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

mod arch;
mod bootinfo;
mod drivers;
mod frames;
mod halt;
mod log;
mod m1;
mod m2;
mod panic;
mod sync;
mod timekeeping;
mod uefi;

use arch::x86_64::{self, gdt, tss};
use log::{log_error as error, log_info as info, log_warn as warn};

/// UEFI image entry point (UEFI 2.10 §2.1 EFI_IMAGE_ENTRY_POINT; win64/
/// efiapi ABI on x86_64, entry symbol fixed to `efi_main` by our target's
/// linker args).
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(
    _image_handle: usize,
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
    unsafe { drivers::serial::init() };

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
    let cpu = x86_64::cpu_info();
    info!("boot", "cpu: {cpu}");
    info!(
        "boot",
        "cpustate: cr0={:#x} cr3={:#x} cr4={:#x} efer={:#x}",
        x86_64::read_cr0(),
        x86_64::read_cr3(),
        x86_64::read_cr4(),
        x86_64::read_efer()
    );

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
    unsafe { arch::x86_64::idt::init() };
    info!(
        "boot",
        "idt: installed own IDT (base={:#x}, 256 vectors: 0-31 diagnostics+halt, 32-255 absorb+EOI)",
        arch::x86_64::idt::expected_idt().0
    );

    // --- Step 2.5: capture the firmware memory map (boot-info seed) --------
    // Done once here for production consumers (frame allocator, later the
    // ExitBootServices map-key replay); the M1 test suite re-captures
    // independently as part of its assertions.
    match bootinfo::capture() {
        Ok(summary) => info!(
            "boot",
            "bootinfo: captured {} regions (map_key={:#x}, conventional={}MiB)",
            summary.region_count,
            summary.map_key,
            summary.conventional_mib()
        ),
        Err(status) => {
            error!("boot", "memory map capture failed: status={status:#x}");
            halt::halt_machine("memory map capture failed");
        }
    }

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

    let (passed2, total2) = m2::run_all();

    // --- Step 4: halt safely ------------------------------------------------
    info!(
        "boot",
        "milestones finished (m1 {passed}/{total}, m2 {passed2}/{total2}); halting via UEFI ResetSystem(shutdown)"
    );
    uefi::reset_shutdown();

    // reset_shutdown() only returns if the firmware hand-off failed. Fall
    // back to parking the CPU: interrupts are off, so HLT is permanent.
    // SAFETY: CLI+HLT loop, single CPU, nothing else runnable.
    unsafe {
        x86_64::cli();
        loop {
            core::arch::asm!("hlt", options(nostack, nomem, preserves_flags));
        }
    }
}
