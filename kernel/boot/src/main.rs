//! ArenaOS boot stage — Milestones 1–2.
//!
//! This binary *is* the kernel image at M1: a UEFI application
//! (`EFI/BOOT/BOOTX64.EFI`) that firmware loads in 64-bit long mode. It
//! follows the boot contract from ADR-0003:
//!
//! 1. Establish CPU state: interrupts off, own GDT (far-jump CS reload).
//! 2. Initialize runtime support: polled 16550 serial console, structured
//!    logging, panic diagnostics.
//! 3. Produce verified diagnostics: CPU identity/state, firmware identity,
//!    real UEFI memory-map parsing — each checked by the M1 self-test suite.
//! 4. Halt safely: UEFI `ResetSystem(EfiResetShutdown)`.
//!
//! At M2.7 this crate splits into boot stage + kernel proper; until then,
//! per the roadmap, it is the smallest bootable kernel that honestly
//! demonstrates each capability.

#![no_std]
#![no_main]

mod bootinfo;
mod m1;
mod m2;
mod panic;
mod uefi;

use arena_kernel::arch::x86_64::paging::{DIRECT_MAP_BYTES, KERNEL_OFFSET};
use arena_kernel::arch::x86_64::{self, gdt, tss};
use arena_kernel::entry::{self, BOOTINFO_MAGIC, BOOTINFO_VERSION, BootInfo};
use arena_kernel::handoff;
use arena_kernel::log::{log_error as error, log_info as info, log_warn as warn};
use arena_kernel::{frames, halt, heap, timekeeping};

/// UEFI image entry point (UEFI 2.10 §2.1 EFI_IMAGE_ENTRY_POINT; win64/
/// efiapi ABI on x86_64, entry symbol fixed to `efi_main` by our target's
/// linker args).
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(
    image_handle: usize,
    system_table: *const uefi::SystemTable,
) -> usize {
    // --- Step 1: establish CPU state -------------------------------------
    x86_64::cli();

    // Capture the firmware interface *first*: the panic handler's safe-halt
    // path depends on it, and everything below can (hypothetically) fail.
    uefi::init(system_table);

    // --- Step 2: runtime support -----------------------------------------
    // SAFETY: we are the only code running (single CPU, interrupts off);
    // COM1 programming follows the driver's port-I/O contract.
    unsafe { arena_kernel::drivers::serial::init() };

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    info!(
        "boot",
        "ArenaOS boot stage v{} ({profile} build, x86_64-unknown-uefi) — milestones 1–2",
        env!("CARGO_PKG_VERSION")
    );

    // Firmware identity (real UCS-2 string from the EFI system table).
    // SAFETY: uefi::init() ran above with firmware's live system table.
    unsafe {
        if let Some((vendor, revision)) = uefi::firmware_info() {
            info!(
                "boot",
                "firmware: vendor=\"{vendor}\" revision={revision:#x}"
            );
        } else {
            warn!("boot", "firmware: system table pointer was null (!)");
        }
    }

    // CPU identity and architectural state, read back from the CPU itself.
    // The firmware's CR3 is captured here for M2.7: the ExitBootServices
    // farewell window runs under firmware's own page tables (its code
    // touches GCD MMIO our map-described identity view deliberately
    // lacks — ADR-0008/0011).
    let fw_cr3 = x86_64::read_cr3();
    // Deposit it for the shutdown farewell island (halt.rs): ResetSystem
    // runs in firmware's world, under firmware's tables.
    // SAFETY: fw_cr3 was just read from CR3; firmware's tables live for
    // the machine's remaining lifetime (we never free firmware frames).
    unsafe { halt::set_firmware_cr3(fw_cr3) };
    let cpu = x86_64::cpu_info();
    info!("boot", "cpu: {cpu}");
    info!(
        "boot",
        "cpustate: cr0={:#x} cr3={fw_cr3:#x} cr4={:#x} efer={:#x}",
        x86_64::read_cr0(),
        x86_64::read_cr4(),
        x86_64::read_efer()
    );

    // SAFETY: system table captured above; read-only table diagnostics.
    if let Some(bs) = unsafe { uefi::boot_services() } {
        info!(
            "boot",
            "uefi: boot services rev={:#x} hdr_size={:#x}",
            bs.spec_revision(),
            bs.hdr.header_size
        );
    }

    // Replace the firmware's GDT with ours (ADR-0003 step 1). The M1 test
    // suite independently verifies this via sgdt read-back.
    // SAFETY: ring 0, interrupts disabled, image identity-mapped by firmware
    // — exactly the contract of gdt::load().
    unsafe { gdt::load() };
    info!(
        "boot",
        "gdt: installed own GDT (base={:#x}, code_sel={:#x}, data_sel={:#x}, tss_sel={:#x})",
        gdt::expected_gdt().0,
        gdt::KERNEL_CODE_SELECTOR,
        gdt::KERNEL_DATA_SELECTOR,
        gdt::TSS_SELECTOR
    );

    // TSS (M2.1): IST stacks for the exceptions that must survive a broken
    // stack (#DF/NMI/#MC → dedicated 16 KiB fault stack). Must be live
    // before the IDT goes in, since those gates carry IST=1.
    // SAFETY: ring 0, IF=0, our GDT is live — tss::init()'s contract.
    unsafe { tss::init() };
    info!(
        "boot",
        "tss: installed (IST1 fault stack top={:#x})",
        tss::ist1_stack_top()
    );

    // The IDT must change hands together with the GDT: EDK2 re-enables
    // interrupts inside boot-service calls (TPL restore = `sti`), and its
    // gates reference firmware selectors absent from our GDT (see idt.rs
    // header for the triple-fault post-mortem). Our M1 IDT: exceptions get
    // full serial diagnostics + safe halt; external interrupts are absorbed
    // with 8259 + LAPIC EOI until M2 replaces them with real dispatch.
    // SAFETY: ring 0, IF=0, installed immediately after the GDT swap and
    // before the first firmware call — idt::init()'s contract.
    unsafe { x86_64::idt::init() };
    info!(
        "boot",
        "idt: installed own IDT (base={:#x}, 256 vectors: 0-31 diagnostics+halt, 32-255 absorb+EOI)",
        x86_64::idt::expected_idt().0
    );

    // --- Step 2.5: capture the firmware memory map (boot-info seed) --------
    // Done once here for production consumers (frame allocator, later the
    // ExitBootServices map-key replay); the M1 test suite re-captures
    // independently as part of its assertions.
    let _summary = match bootinfo::capture() {
        Ok(summary) => {
            info!(
                "boot",
                "bootinfo: captured {} regions (map_key={:#x}, conventional={}MiB)",
                summary.region_count,
                summary.map_key,
                summary.conventional_mib()
            );
            summary
        }
        Err(status) => {
            error!("boot", "memory map capture failed: status={status:#x}");
            halt::halt_machine("memory map capture failed");
        }
    };

    // --- Step 3: verified diagnostics -------------------------------------
    info!("m1", "running milestone-1 self-tests");
    let (passed, total) = m1::run_all();

    // --- Step 3.5: take over timekeeping (M2.2) ----------------------------
    // After the M1 suite (which observes firmware-owned hardware state) and
    // before the M2 suite (which verifies and consumes the clock): calibrate
    // the TSC against the PIT oscillator and start the kernel tick. A boot
    // without a trustworthy clock is not a boot we want to continue — fail
    // loudly (ADR-0005).
    if let Err(reason) = timekeeping::init() {
        error!("boot", "timekeeping init failed: {reason}");
        halt::halt_machine("timekeeping init failed");
    }

    // --- Step 3.6: physical memory ownership (M2.3) -------------------------
    // Build the frame allocator from the captured conventional regions.
    if let Err(reason) = frames::init() {
        error!("boot", "frame allocator init failed: {reason}");
        halt::halt_machine("frame allocator init failed");
    }
    info!(
        "boot",
        "frames: managing {} frames ({} MiB free)",
        frames::total_frames(),
        frames::free_frames() * 4096 / (1024 * 1024)
    );

    // --- Step 3.7: kernel address space (M2.4) ------------------------------
    // Install our own PML4 (ADR-0008): identity view of every described
    // region (firmware keeps running under it until M2.7 tears it down),
    // higher-half direct map at KERNEL_OFFSET, our image window mapped per
    // PE section (.text R+X, .rdata RO, rest RW+NX), CR0.WP + EFER.NXE
    // enforced. The identity view keeps every executing address valid
    // across the CR3 switch, so boot simply continues.
    // Since M2.7 the kernel crate never touches UEFI: the boot stage
    // resolves the Loaded Image Protocol here and deposits plain data
    // (base/size) into the handoff record; paging parses the PE section
    // table from those facts.
    let layout = match uefi::loaded_image_info(image_handle) {
        Some((base, size)) => {
            let l = arena_kernel::handoff::ImageLayout { base, size };
            arena_kernel::handoff::set_image_layout(l);
            l
        }
        None => {
            error!("boot", "Loaded Image Protocol did not report our image");
            halt::halt_machine("no loaded-image information");
        }
    };
    // SAFETY: ring 0, interrupts off, frame allocator live, handoff record
    // filled by the capture above and the image layout just deposited.
    if let Err(reason) = unsafe { x86_64::paging::init(&layout) } {
        error!("boot", "paging init failed: {reason}");
        halt::halt_machine("paging init failed");
    }

    // --- Step 3.8: kernel heap (M2.5) ---------------------------------------
    // arena-heap core (host-tested) wired to the frame allocator; guards
    // always on at boot stage, chunks grown lazily on first allocation.
    heap::init();

    let (passed2, total2) = m2::run_all();

    // --- Step 4: ExitBootServices (M2.7) ------------------------------------
    info!("boot", "exiting UEFI boot services (M2.7 handover)");
    // The farewell window (watchdog off, final capture, EBS itself) runs
    // under the FIRMWARE's page tables, restored from the entry-time CR3.
    // Why: EDK2's boot-service internals touch GCD MMIO that never appears
    // in the memory map (observed live: #PF reading 0x81060000 from DXE
    // code during this window), and our identity view maps only
    // map-described regions by design (ADR-0008). Firmware's tables map
    // firmware's world completely — including our image (LoaderCode/Data)
    // and every conventional page our statics live in. We switch back to
    // our dual view the moment EBS succeeds.
    info!(
        "boot",
        "restoring firmware page tables for the EBS window (cr3={fw_cr3:#x})"
    );
    // SAFETY: fw_cr3 was read at entry, before we installed any tables of
    // our own; firmware's tables are intact (nothing freed or remapped
    // them — our frames come from the bitmap, not from firmware).
    unsafe { x86_64::write_cr3(fw_cr3) };
    // Watchdog off: from here on, nothing outside the kernel may reset the
    // machine — shutdown is ours (runtime ResetSystem).
    // SAFETY: boot services are live until the successful EBS call below.
    unsafe {
        match uefi::boot_services() {
            Some(bs) => {
                let status = (bs.set_watchdog_timer)(0, 0, 0, core::ptr::null());
                if status != uefi::EFI_SUCCESS {
                    warn!("boot", "SetWatchdogTimer(0) returned {status:#x}");
                }
            }
            None => halt::halt_machine("boot services vanished before ExitBootServices"),
        }
    }
    // The EBS handshake: the map key must come from the most recent
    // GetMemoryMap. Re-capture (which also refreshes the handoff record
    // with the FINAL map) and replay; firmware that changed the map in
    // between rejects with INVALID_PARAMETER — re-capture and retry,
    // bounded. A failed attempt leaves boot services fully alive, so the
    // retry and halt paths remain legal.
    const EBS_ATTEMPTS: u32 = 3;
    let mut ebs_status = usize::MAX;
    let mut final_summary = None;
    for attempt in 1..=EBS_ATTEMPTS {
        let summary = match bootinfo::capture() {
            Ok(s) => s,
            Err(status) => {
                error!("boot", "final memory-map capture failed: {status:#x}");
                halt::halt_machine("final memory-map capture failed");
            }
        };
        // SAFETY: image_handle is ours; map_key is from the capture just
        // completed; a failing call leaves boot services functional.
        ebs_status = unsafe {
            match uefi::boot_services() {
                Some(bs) => (bs.exit_boot_services)(image_handle, handoff::map_key() as usize),
                None => usize::MAX,
            }
        };
        if ebs_status == uefi::EFI_SUCCESS {
            final_summary = Some(summary);
            info!(
                "boot",
                "ExitBootServices succeeded (attempt {attempt}, map_key={:#x}, {} regions, conventional={}MiB)",
                handoff::map_key(),
                summary.region_count,
                summary.conventional_mib()
            );
            break;
        }
        // EDK2 restores TPL (which means `sti`) on the way out of the
        // failed call — re-mask before anything else runs.
        x86_64::cli();
        warn!(
            "boot",
            "ExitBootServices attempt {attempt} rejected: {ebs_status:#x}"
        );
        if ebs_status != uefi::EFI_INVALID_PARAMETER {
            break; // not the stale-key case; retrying cannot help
        }
    }
    if ebs_status != uefi::EFI_SUCCESS {
        error!("boot", "ExitBootServices failed: {ebs_status:#x}");
        halt::halt_machine("ExitBootServices failed");
    }
    let final_summary = final_summary.expect("EBS success path stored the summary");
    // Farewell window closed: back onto our own dual-view tables (untouched
    // since step 3.7). Firmware's tables are never used again.
    let dual_cr3 = x86_64::paging::cr3_phys();
    // SAFETY: our dual-view PML4 is live state from step 3.7; IF=0.
    unsafe { x86_64::write_cr3(dual_cr3) };
    info!(
        "boot",
        "back on kernel dual-view tables (cr3={dual_cr3:#x}); boot services are dead"
    );
    // The 16550, the PIT, the APICs and the frame bitmap are ours; runtime
    // services remain, for shutdown only.
    arena_kernel::log::write_marker(format_args!("m2:test:ebs_exited: PASS"));

    // --- Step 4.5: reconcile + recycle for the kernel view -------------------
    // Firmware may have shifted the map between the allocator's capture and
    // the final one. Rule: a frame still free here but not conventional in
    // the FINAL map is leaked — never handed out.
    let leaks = match frames::reconcile_final_map() {
        Ok(n) => n,
        Err(reason) => {
            error!("boot", "post-EBS frame reconcile failed: {reason}");
            halt::halt_machine("post-EBS frame reconcile failed");
        }
    };
    info!(
        "boot",
        "frames: post-EBS reconcile leaked {leaks} frame(s); {} free ({} MiB)",
        frames::free_frames(),
        frames::free_frames() * 4096 / (1024 * 1024)
    );
    // The boot heap's chunks live at identity addresses — recycle them
    // into the frame pool; the kernel regrows its heap at direct-map
    // aliases on first allocation.
    let released = match heap::reinit_for_kernel_view() {
        Ok(n) => n,
        Err(reason) => {
            error!("boot", "heap kernel-view reinit failed: {reason}");
            halt::halt_machine("heap kernel-view reinit failed");
        }
    };
    info!(
        "boot",
        "heap: recycled {released} boot chunk(s); kernel-view heap armed"
    );

    // --- Step 5: enter the kernel proper ------------------------------------
    // Kernel-only tables: higher-half everything, runtime services kept at
    // their identity addresses (RT code references its own VAs internally),
    // the identity view of RAM deliberately absent.
    // M2.7 bring-up probe: what does the FINAL map call the interrupt
    // controllers' MMIO pages? build_kernel_view only aliases
    // map-described MMIO, and the kernel's reclaimed IRQ chain needs both.
    for target in [0xFEC0_0000u64, x86_64::paging::apic_base_phys()] {
        let mut found = None;
        for i in 0..handoff::region_count() {
            if let Some(r) = handoff::region(i) {
                let end = r.base + r.pages * 4096;
                if r.base <= target && target < end {
                    found = Some((r.kind, r.base, r.pages));
                    break;
                }
            }
        }
        match found {
            Some((kind, base, pages)) => info!(
                "boot",
                "map probe {target:#x}: kind={kind} region_base={base:#x} pages={pages}"
            ),
            None => info!("boot", "map probe {target:#x}: NOT DESCRIBED"),
        }
    }
    let kernel_cr3 = match unsafe { x86_64::paging::build_kernel_view() } {
        Ok(cr3) => cr3,
        Err(reason) => {
            error!("boot", "kernel-view build failed: {reason}");
            halt::halt_machine("kernel-view build failed");
        }
    };
    // Relocate the descriptor tables and the LAPIC EOI target to
    // kernel-view aliases while BOTH views are still live (this code
    // executes at identity addresses; the relocated structures are valid
    // under the dual-view tables immediately).
    // SAFETY: ring 0, IF=0, dual-view tables active; the kernel view maps
    // the whole image window, so every relocated address is valid right
    // after the switch.
    // Relocation must be atomic against the 100 Hz PIT: a tick landing
    // mid-loop is delivered through a half-rewritten table (observed live
    // in the M2.7 bring-up: vector 0x20 hit while the loop sat at vector
    // 20). IF stays 0 from here through the trampoline; kmain re-enables
    // interrupts under the kernel view, where every descriptor is high.
    x86_64::cli();
    unsafe {
        x86_64::idt::set_lapic_eoi_addr(x86_64::paging::apic_base_phys() + KERNEL_OFFSET + 0xB0);
        x86_64::tss::relocate_for_kernel(KERNEL_OFFSET);
        x86_64::idt::relocate_for_kernel(KERNEL_OFFSET);
    }
    info!(
        "boot",
        "descriptors relocated to the kernel view (GDT/TSS/IDT, LAPIC EOI)"
    );
    // Readback proof: what the CPU will walk (via sidt) must equal what we
    // wrote (via the static), and both must sit at the high alias.
    // SAFETY: ring 0, dual view maps the table at both aliases.
    let (v14_static, v14_idtr, idtr_base) = unsafe { x86_64::idt::readback_gate(14) };
    let (v32_static, v32_idtr, _) = unsafe { x86_64::idt::readback_gate(0x20) };
    info!(
        "boot",
        "idt readback: idtr={idtr_base:#x} vec14={v14_static:#x}/{v14_idtr:#x} vec32={v32_static:#x}/{v32_idtr:#x}"
    );
    // The PE's absolute slots (dyn-dispatch vtables, fn pointers) still
    // hold the identity addresses UEFI's relocation pass baked in. Re-apply
    // the .reloc table with delta=+KERNEL_OFFSET so they point at the high
    // alias — the only alias the kernel view maps. Without this, kmain's
    // first writeln! dispatches through a vtable slot to identity
    // serial-write code and dies (M2.7 bring-up root cause; ADR-0011).
    // The log line right after this call is itself the canary: it runs
    // through the re-relocated vtables under the dual view.
    let (patched, skipped) = match unsafe { x86_64::paging::apply_base_relocations(KERNEL_OFFSET) }
    {
        Ok(counts) => counts,
        Err(e) => halt::halt_machine(e),
    };
    info!(
        "boot",
        "image re-relocated for the kernel view: {patched} DIR64 slot(s) +KERNEL_OFFSET, {skipped} other-kind skipped"
    );

    // The boot stack's identity address IS its physical address (identity
    // view); the trampoline lifts RSP into the direct map, so the stack
    // must live inside the direct-map span.
    let stack_phys = x86_64::read_rsp();
    if stack_phys >= DIRECT_MAP_BYTES {
        error!(
            "boot",
            "boot stack {stack_phys:#x} lies outside the direct map"
        );
        halt::halt_machine("boot stack not direct-mapped");
    }
    let record = entry::prepare(BootInfo {
        magic: BOOTINFO_MAGIC,
        version: BOOTINFO_VERSION,
        map_key: handoff::map_key(),
        region_count: handoff::region_count() as u64,
        conventional_pages: final_summary.conventional_pages,
        dual_view_cr3: x86_64::paging::cr3_phys(),
        fw_cr3,
        kernel_cr3,
        image_base: layout.base,
        image_size: layout.size,
        tsc_hz: timekeeping::tsc_hz(),
        tick_hz: timekeeping::KERNEL_TICK_HZ,
        stack_phys,
        m1_passed: passed as u32,
        m1_total: total as u32,
        pre_entry_passed: (passed2 + 1) as u32, // + the ebs_exited marker
        pre_entry_total: (total2 + 1) as u32,
        reconciled_leaks: leaks,
        heap_chunks_released: released,
    });
    // Kernel-view aliases of everything the trampoline touches. Black-box
    // the bases so nothing folds `sym + KERNEL_OFFSET` into one relocated
    // address (the M2.4 disp32-wrapping lesson).
    let tramp = core::hint::black_box(entry::switch_trampoline_addr());
    let kmain_base = core::hint::black_box(entry::kmain as *const () as u64);
    let tramp_high = tramp + KERNEL_OFFSET;
    let kmain_high = kmain_base + KERNEL_OFFSET;
    // After the re-relocation, data-static addresses may materialize at
    // EITHER alias (patched GOT slots yield high addresses; RIP-relative
    // LEAs still resolve identity-side). Normalize to the physical record
    // and lift exactly once.
    let record_high = x86_64::paging::kernel_view_phys(record as u64) + KERNEL_OFFSET;
    info!(
        "boot",
        "entering kernel proper: trampoline={tramp_high:#x} kmain={kmain_high:#x} boot-info={record_high:#x} kernel_cr3={kernel_cr3:#x}"
    );
    // SAFETY: the trampoline is invoked at its kernel-view alias while the
    // dual-view tables are live, so both aliases execute; inside, it
    // switches CR3, lifts RSP into the direct map, and jumps to kmain —
    // never returning (documented in entry.rs). All three arguments are
    // kernel-view addresses of live mappings.
    unsafe {
        let switch =
            core::mem::transmute::<u64, unsafe extern "C" fn(u64, usize, usize) -> !>(tramp_high);
        switch(kernel_cr3, kmain_high as usize, record_high as usize);
    }
}
