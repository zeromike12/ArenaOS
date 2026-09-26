//! Kernel entry (M2.7, ADR-0011): the boot-info record, the address-space
//! switch trampoline, and `kmain` — the first code of the kernel proper.
//!
//! Sequence (boot stage hands over here):
//!
//! 1. Boot fills [`BootInfo`] via [`prepare`] and computes kernel-view
//!    aliases (everything it passes lives in the image window or the
//!    direct map, so `+KERNEL_OFFSET` is the alias rule).
//! 2. Boot *calls the trampoline at its kernel-view alias* while the
//!    dual-view tables are still live; inside, CR3 switches to the
//!    kernel-only tables, RSP is lifted to its kernel alias, and `kmain`
//!    is entered — the identity view of RAM is gone from this instant.
//! 3. `kmain` validates the record (a real ABI check), runs the
//!    kernel-phase M2 tests (each an observable machine effect: register
//!    read-backs, a deliberate fault through the relocated IDT, live
//!    allocations, live interrupts), emits the combined `m2: RESULT`
//!    line, and shuts the machine down through the runtime services.
//!
//! Nothing here returns to the boot stage; the boot stack's physical
//! pages stay reserved (firmware region) and serve as the entry stack
//! until M3 gives tasks their own.

use crate::arch::x86_64::paging::KERNEL_OFFSET;
use crate::arch::x86_64::{self, cr0, efer, faults};
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::sync::SyncCell;
use core::alloc::Layout;
use core::arch::global_asm;

/// Magic identifying a valid [`BootInfo`] record ("ARENK1\xB0\x07").
pub const BOOTINFO_MAGIC: u64 = 0x4152_454E_4B31_B007;

/// ABI version of [`BootInfo`]; `kmain` refuses anything else.
pub const BOOTINFO_VERSION: u32 = 1;

/// The boot → kernel handoff record: plain data, filled once by the boot
/// stage (see [`crate::handoff`] for the boundary rules). `kmain`
/// validates magic/version and cross-checks the machine state against it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    /// Map key replayed to the successful `ExitBootServices` call.
    pub map_key: u64,
    pub region_count: u64,
    pub conventional_pages: u64,
    /// CR3 of the boot-stage dual-view tables (for the "we left it" check).
    pub dual_view_cr3: u64,
    /// Firmware's own CR3 (entry-time). Shutdown reinstates it for
    /// ResetSystem — runtime services need firmware's mappings (ADR-0011).
    pub fw_cr3: u64,
    /// CR3 the trampoline must install (kernel-only view).
    pub kernel_cr3: u64,
    pub image_base: u64,
    pub image_size: u64,
    pub tsc_hz: u64,
    pub tick_hz: u32,
    /// Physical address of the boot stack at handover (must lie inside
    /// the direct map — asserted by boot before the switch).
    pub stack_phys: u64,
    /// Boot-stage results, carried into the combined RESULT line.
    pub m1_passed: u32,
    pub m1_total: u32,
    /// Pre-entry m2 results *including* the `ebs_exited` marker.
    pub pre_entry_passed: u32,
    pub pre_entry_total: u32,
    /// Frames leaked by the post-EBS reconciliation (map drift).
    pub reconciled_leaks: u64,
    /// Boot-stage heap chunks released back to the frame allocator.
    pub heap_chunks_released: u64,
}

static BOOTINFO: SyncCell<BootInfo> = SyncCell::new(BootInfo {
    magic: 0,
    version: 0,
    map_key: 0,
    region_count: 0,
    conventional_pages: 0,
    dual_view_cr3: 0,
    fw_cr3: 0,
    kernel_cr3: 0,
    image_base: 0,
    image_size: 0,
    tsc_hz: 0,
    tick_hz: 0,
    stack_phys: 0,
    m1_passed: 0,
    m1_total: 0,
    pre_entry_passed: 0,
    pre_entry_total: 0,
    reconciled_leaks: 0,
    heap_chunks_released: 0,
});

/// Store the record; returns a pointer to it, valid in the *current*
/// (dual) view — boot adds `KERNEL_OFFSET` for the trampoline argument.
pub fn prepare(record: BootInfo) -> *const BootInfo {
    // SAFETY: boot contract — single sequential writer, IF=0.
    unsafe {
        *BOOTINFO.get() = record;
        BOOTINFO.get() as *const BootInfo
    }
}

global_asm!(
    // Address-space switch trampoline (M2.7). Invoked AT ITS KERNEL-VIEW
    // ALIAS while the dual-view tables are live, so the fetch of these
    // instructions survives the CR3 write mid-body. Win64/"C" arguments:
    //   rcx = kernel-view PML4 phys, rdx = kmain kernel-view address,
    //   r8  = BootInfo kernel-view address (becomes kmain's rcx argument).
    // Between `mov cr3` and `add rsp` NO instruction touches memory: the
    // old RSP's identity alias is unmapped from the CR3 write onward, and
    // the stack returns as its direct-map alias. The trailing `jmp` (not
    // `call`) means nothing ever returns here — the pushed return address
    // of the trampoline call dies with the identity stack view.
    ".section .text",
    ".p2align 4",
    ".globl arena_switch_to_kernel_view",
    "arena_switch_to_kernel_view:",
    "mov rax, rdx",                // kmain address
    "mov rdx, rcx",                // PML4 phys into scratch
    "mov rcx, r8",                 // kmain arg0 = BootInfo
    "mov cr3, rdx",                // kernel-only tables live
    "mov r10, 0xffffffff80000000", // KERNEL_OFFSET
    "add rsp, r10",                // stack to its kernel-view alias
    "jmp rax",                     // enter kmain; never returns
);

unsafe extern "C" {
    /// The trampoline (see asm above).
    fn arena_switch_to_kernel_view();
}

/// Link-time address of the trampoline (an identity/boot-view value; the
/// caller adds `KERNEL_OFFSET` and black-boxes it — the M2.4 lea-folding
/// lesson).
pub fn switch_trampoline_addr() -> u64 {
    arena_switch_to_kernel_view as *const () as u64
}

/// Enter the kernel proper: called by the trampoline at this function's
/// kernel-view alias, on the lifted stack, under kernel-only tables.
/// Never returns — the machine shuts down via runtime services.
pub extern "C" fn kmain(boot_info: &'static BootInfo) -> ! {
    // The trampoline just switched CR3 to the kernel-only view: from this
    // instruction on, the identity alias of RAM is GONE, and page-table
    // accesses must go through the kernel-view alias (paging::table_ptr).
    // SAFETY: called exactly once, after the CR3 switch, ring 0, IF=0.
    unsafe { x86_64::paging::note_identity_torn_down() };

    // --- record validation: a real ABI check, not ceremony ---------------
    if boot_info.magic != BOOTINFO_MAGIC
        || boot_info.version != BOOTINFO_VERSION
        || boot_info.fw_cr3 == 0
    {
        error!(
            "kernel",
            "boot-info record invalid: magic={:#x} version={} (want {:#x} v{})",
            boot_info.magic,
            boot_info.version,
            BOOTINFO_MAGIC,
            BOOTINFO_VERSION
        );
        crate::halt::halt_machine("boot-info record invalid");
    }
    info!(
        "kernel",
        "arena kernel proper entered: boot-info v{} map_key={:#x} regions={} conventional={}MiB tsc={}Hz tick={}Hz",
        boot_info.version,
        boot_info.map_key,
        boot_info.region_count,
        boot_info.conventional_pages * 4096 / (1024 * 1024),
        boot_info.tsc_hz,
        boot_info.tick_hz
    );
    info!(
        "kernel",
        "handover: boot m1 {}/{}, m2 pre-entry {}/{}; post-EBS reconcile leaked {} frame(s); {} heap chunk(s) recycled",
        boot_info.m1_passed,
        boot_info.m1_total,
        boot_info.pre_entry_passed,
        boot_info.pre_entry_total,
        boot_info.reconciled_leaks,
        boot_info.heap_chunks_released
    );

    // --- reclaim the timer and the interrupt chain -------------------------
    // The firmware's ExitBootServices teardown killed the tick three ways
    // (each measured live during the M2.7 bring-up, QEMU 11.0.2 + OVMF):
    //   1. The IOAPIC RTE carrying the PIT (pin 2 — the PC convention
    //      for ISA IRQ0) was masked with its vector zeroed — the
    //      PIT→LAPIC route itself was torn down.
    //   2. The 8254's IRQ output was suppressed at the QEMU level: while
    //      HPET legacy replacement mode is on, i8254.c `pit_irq_control`
    //      holds `irq_disabled` — the counter keeps advancing (we measured
    //      it) but no IRQ edges ever fire. Pre-EBS ticks were HPET
    //      legacy edges on IRQ0; at EBS firmware quiesced its timer and
    //      the PIT stayed suppressed. Clearing GEN_CONF.LEGACY_ENABLE
    //      re-enables the PIT's IRQ output.
    //   3. The PIT divisor/mode itself needed reprogramming (done first).
    // Port I/O is view-independent; IOAPIC/LAPIC/HPET register windows go
    // through their kernel-view aliases (GCD MMIO, memory-map-absent —
    // build_kernel_view maps them explicitly). Single CPU, IF=0; the IRQ
    // test below enables interrupts itself, bounded.
    // SAFETY: kernel owns these devices post-handover; all aliases are
    // mapped RW in the kernel view.
    unsafe {
        use crate::drivers::intc;
        crate::drivers::pit::set_periodic_hz(crate::timekeeping::KERNEL_TICK_HZ);
        // MMIO phys > 2 GiB: kernel-half alias rule (mmio_alias_va) — the
        // same arithmetic build_kernel_view used to place the mappings.
        let ioapic_va = x86_64::paging::mmio_alias_va(intc::IOAPIC_PHYS);
        let lapic_va = x86_64::paging::mmio_alias_va(x86_64::paging::apic_base_phys());
        let hpet_va = x86_64::paging::mmio_alias_va(intc::HPET_PHYS);
        // 2. HPET legacy replacement off: IRQ0 belongs to the PIT again.
        let hpet_conf0 = intc::hpet_read(hpet_va, intc::HPET_GEN_CONF);
        intc::hpet_write(
            hpet_va,
            intc::HPET_GEN_CONF,
            hpet_conf0 & !intc::HPET_LEGACY_ENABLE,
        );
        let hpet_conf1 = intc::hpet_read(hpet_va, intc::HPET_GEN_CONF);
        // LAPIC delivery-state ownership: TPR down, stale in-service
        // cleared (EOI with an empty ISR is a no-op by spec), software
        // enable guaranteed.
        intc::lapic_write(lapic_va, intc::LAPIC_TPR, 0);
        intc::lapic_write(lapic_va, intc::LAPIC_EOI_REG, 0);
        let svr = intc::lapic_enable(lapic_va);
        // 1. IOAPIC pin PIT_IOAPIC_PIN (=2: the PC convention for ISA
        // IRQ0, QEMU-hardcoded in ioapic_set_irq) → our absorb vector,
        // unmasked. Firmware's EBS teardown left this RTE masked with its
        // vector zeroed — read back as evidence.
        let rte_lo = intc::ioapic_read(ioapic_va, intc::rte_lo_index(intc::PIT_IOAPIC_PIN));
        let rte_hi = intc::ioapic_read(ioapic_va, intc::rte_hi_index(intc::PIT_IOAPIC_PIN));
        intc::route_pin_to_vector(ioapic_va, intc::PIT_IOAPIC_PIN, intc::PIT_VECTOR);
        let rte_now = intc::ioapic_read(ioapic_va, intc::rte_lo_index(intc::PIT_IOAPIC_PIN));
        // Evidence: the PIT oscillator really advances over 2 ms.
        let c1 = crate::drivers::pit::latch_count_ch0();
        crate::timekeeping::busy_wait_us(2_000);
        let c2 = crate::drivers::pit::latch_count_ch0();
        // COM1 RX (M4.6, ADR-0020): the console's input half, reclaimed
        // in the same shape as the PIT — route ISA IRQ4 (IOAPIC pin 4)
        // to our vector, then unmask the UART's receive interrupt.
        // Bytes typed while IF=0 wait in the 16550 FIFO with the edge
        // latched in the IOAPIC; the idle loop's first `sti` delivers
        // them. The hook (console::init) drains RBR → line discipline.
        intc::route_pin_to_vector(ioapic_va, intc::SERIAL_IOAPIC_PIN, intc::SERIAL_RX_VECTOR);
        crate::console::init();
        let serial_rte = intc::ioapic_read(ioapic_va, intc::rte_lo_index(intc::SERIAL_IOAPIC_PIN));
        info!(
            "kernel",
            "timer chain reclaimed: pit {KERNEL_TICK_HZ} Hz (ch0 {c1} -> {c2} over 2ms); ioapic pin {} rte {rte_lo:#x}/{rte_hi:#x} -> {rte_now:#x}; hpet gen_conf {hpet_conf0:#x} -> {hpet_conf1:#x}; lapic svr={svr:#x}",
            intc::PIT_IOAPIC_PIN,
            KERNEL_TICK_HZ = crate::timekeeping::KERNEL_TICK_HZ
        );
        info!(
            "kernel",
            "console input armed: com1 rx -> ioapic pin {} -> vector {} (rte {serial_rte:#x}), line discipline live",
            intc::SERIAL_IOAPIC_PIN,
            intc::SERIAL_RX_VECTOR
        );
    }

    // --- kernel-phase M2 tests (observable effects only) ------------------
    let mut passed = 0u32;
    let checks: [(&str, fn(&BootInfo) -> Result<(), &'static str>); 4] = [
        ("kernel_entry", test_kernel_entry),
        ("identity_torn_down", test_identity_torn_down),
        ("kernel_heap_live", test_kernel_heap_live),
        ("kernel_irq_live", test_kernel_irq_live),
    ];
    for (name, test) in checks {
        match test(boot_info) {
            Ok(()) => {
                passed += 1;
                write_marker(format_args!("m2:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("kernel", "test {name} failed: {reason}");
                write_marker(format_args!("m2:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = checks.len() as u32;
    let all_passed = boot_info.pre_entry_passed + passed;
    let all_total = boot_info.pre_entry_total + total;
    if all_passed == all_total {
        write_marker(format_args!("m2: RESULT PASS ({all_passed}/{all_total})"));
    } else {
        write_marker(format_args!("m2: RESULT FAIL ({all_passed}/{all_total})"));
        crate::halt::halt_machine("kernel-entry test suite failed");
    }

    info!(
        "kernel",
        "milestone 2 complete — starting the milestone 3 suite"
    );

    // --- M3.1: kernel threads + context switch (ADR-0012) ----------------
    // The bootstrap thread is kmain itself; init is a real ABI step and a
    // failure (double-init, corrupt state) halts with diagnostics.
    if let Err(reason) = crate::sched::init() {
        error!("kernel", "scheduler init failed: {reason}");
        crate::halt::halt_machine("scheduler init failed");
    }

    // --- M3.3: the ring-3 boundary (ADR-0014) -----------------------------
    // syscall/sysret MSRs, STAR selectors, SFMASK, KERNEL_GS_BASE scratch,
    // SMEP/SMAP when the CPU has them — every write verified by read-back
    // inside init (a control MSR that did not take is a dead boundary).
    // SAFETY: ring 0, IF=0, our GDT (with the ring-3 pair) and scheduler
    // are live; the image is at its final kernel-view addresses.
    if let Err(reason) = unsafe { x86_64::syscall::init() } {
        error!("kernel", "syscall boundary init failed: {reason}");
        crate::halt::halt_machine("syscall boundary init failed");
    }

    if !crate::m3::run_suite() {
        crate::halt::halt_machine("milestone 3 suite failed");
    }

    // --- M4.1/M4.2: userspace images — ELF strict-subset validator +
    // loader (ADR-0016): the embedded rust-lld artifact parsed, its
    // rejection corpus attacked, and a real process address space
    // loaded, verified under its own CR3, and reclaimed exactly. Then
    // the syscall ABI v1 (ADR-0017): six-register marshalling, typed
    // status codes, and the callee-saved promise proven from ring 3.
    // And then the milestone capstone: the first user process — the
    // real image loaded into its own address space runs in ring 3,
    // writes through debug_write, and exits through thread_exit. IPC v1
    // (ADR-0018) follows: two processes rendezvous over an endpoint —
    // blocking call/reply, a transferred capability, badged
    // notifications — with the GS-side invariant now part of the
    // context switch.
    if !crate::m4::run_suite() {
        crate::halt::halt_machine("milestone 4 suite failed");
    }

    info!(
        "kernel",
        "milestone 4 complete (executable format + image loader + syscall ABI v1 + first user process + IPC v1 + spawn protocol + minimal shell) — spawning the shell"
    );

    // --- M4.6: the hand-off (ADR-0020) -----------------------------------
    // The shell is the initial service: spawned through the M4.5
    // protocol's kernel-internal entry point (image 1, no parent), with
    // kernel-literal grants in slot order — Power (WRITE, so `shutdown`
    // is an authority the shell HOLDS), Image 0 (READ, so `spawn` can
    // start the test payload), and its own notification (READ|WRITE,
    // the exit-badge channel for its children). From here the machine
    // stops only through the shell's Power-gated SYS_SHUTDOWN, a panic
    // path, or the harness killing QEMU — the boot sequence no longer
    // halts on its own.
    let shell_nid = match crate::ipc::create_notification() {
        Ok(nid) => nid,
        Err(_) => crate::halt::halt_machine("shell: notification table full"),
    };
    let shell_grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Power,
            rights: crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Image { img_id: 0 },
            rights: crate::cap::RIGHTS_READ,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid: shell_nid },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
    ];
    match crate::spawn::spawn_init(1, &shell_grants, None) {
        Ok(pid) => info!(
            "kernel",
            "shell spawned: pid {pid} (caps: 0=Power/W 1=Image0/R 2=Notif{shell_nid}/RW) — the console is live; type 'help'"
        ),
        Err(reason) => crate::halt::halt_machine(reason),
    }

    // The bootstrap thread becomes the idle thread: it stays runnable
    // forever, which keeps block_current's no-runnable-thread deadlock
    // halt (ADR-0018) unreachable while the shell parks on console
    // input, and gives every wake (console RX, tick) somewhere to
    // return. Between wakes it halts the CPU with IF=1 — interrupts
    // must flow now: the UART RX path IS the input device.
    loop {
        crate::sched::yield_now();
        // SAFETY: ring 0; `sti; hlt` is the canonical idle pair — any
        // pending or arriving interrupt resumes the loop right here,
        // and the interrupt gate masks IF again for the handler.
        unsafe { core::arch::asm!("sti", "hlt", options(nomem, nostack)) };
    }
}

/// We are running in the kernel view: RIP and RSP are higher-half, CR3 is
/// the kernel-view PML4 (and NOT the boot dual-view one), and the
/// enforcement bits (CR0.WP/PG, EFER.NXE) survived the switch.
fn test_kernel_entry(info: &BootInfo) -> Result<(), &'static str> {
    let rip: u64;
    let rsp: u64;
    // SAFETY: pure register reads.
    unsafe {
        core::arch::asm!("lea {r}, [rip]", r = lateout(reg) rip,
            options(nostack, nomem, preserves_flags));
        core::arch::asm!("mov {r}, rsp", r = lateout(reg) rsp,
            options(nostack, nomem, preserves_flags));
    }
    if rip < KERNEL_OFFSET {
        return Err("RIP is not in the kernel view");
    }
    if rsp < KERNEL_OFFSET {
        return Err("RSP is not in the kernel view");
    }
    let live_cr3 = x86_64::read_cr3();
    if live_cr3 != info.kernel_cr3 {
        return Err("CR3 is not the kernel-view PML4 from the boot-info record");
    }
    if live_cr3 == info.dual_view_cr3 {
        return Err("still on the boot-stage dual-view tables");
    }
    let cr0_v = x86_64::read_cr0();
    if cr0_v & cr0::WP == 0 || cr0_v & cr0::PG == 0 {
        return Err("CR0 WP/PG not enforced in the kernel view");
    }
    let efer_v = x86_64::read_efer();
    if efer_v & efer::NXE == 0 || efer_v & efer::LMA == 0 {
        return Err("EFER NXE/LMA not set in the kernel view");
    }
    info!(
        "kernel",
        "kernel_entry: rip={rip:#x} rsp={rsp:#x} cr3={live_cr3:#x} (dual was {:#x}) cr0={cr0_v:#x} efer={efer_v:#x}",
        info.dual_view_cr3
    );
    Ok(())
}

/// #PF fault site for the armed protocol (M2.1): write this instruction
/// stream's resume address into `faults::RESUME` *first*, then execute the
/// faulting 8-byte write — the handler rewrites the frame RIP to the
/// resume label and the test continues. The kernel crate keeps its own
/// copy because boot's fault sites are crate-private.
///
/// # Safety
/// Call only with `faults::arm(14)` in effect, and only with `addr`
/// guaranteed unmapped under the live view (otherwise the store silently
/// succeeds and the armed expectation leaks).
unsafe fn write_probe(addr: u64) {
    // SAFETY: as above; clobbers declared; the resume slot is our own
    // static (single CPU, IF=0 — SyncCell contract).
    unsafe {
        core::arch::asm!(
            "lea rax, [rip + 2f]",
            "mov [rip + {resume}], rax",
            "mov [{addr}], rax", // #PF — handler resumes execution at 2:
            "2:",
            addr = in(reg) addr,
            resume = sym crate::arch::x86_64::faults::RESUME,
            out("rax") _,
            options(nostack),
        );
    }
}

/// The identity view of RAM is really gone: a write to a low identity
/// address (page 0x1000 — never a runtime-services region) must raise
/// #PF (not-present, write) and be recovered through the *relocated* IDT.
/// This is the observable teardown proof, not a log line saying so.
fn test_identity_torn_down(_info: &BootInfo) -> Result<(), &'static str> {
    const PROBE_VA: u64 = 0x1000;
    faults::arm(14);
    // SAFETY: armed expected-fault protocol (M2.1); 0x1000 is unmapped in
    // the kernel view by construction (identity RAM deliberately absent).
    unsafe { write_probe(core::hint::black_box(PROBE_VA)) };
    let obs = faults::observed();
    faults::disarm();
    if !obs.valid || obs.vector != 14 {
        return Err("write to an identity address did NOT fault — teardown did not happen");
    }
    if obs.cr2 != PROBE_VA {
        return Err("CR2 does not name the probed identity address");
    }
    // Not-present (P=0) + write (W=1) → error code 0b10.
    if obs.error_code & 0b11 != 0b10 {
        return Err("error code is not not-present+write (mapping still there?)");
    }
    info!(
        "kernel",
        "identity_torn_down: write to {PROBE_VA:#x} → #PF ec={:#x} cr2={:#x}, recovered at a kernel-view RIP",
        obs.error_code,
        obs.cr2
    );
    Ok(())
}

/// The recycled heap works in the kernel view: allocation returns a
/// higher-half pointer backed by a post-EBS frame allocation, payloads
/// round-trip, frees are clean — and the frame allocator itself still
/// allocates/frees after ExitBootServices.
fn test_kernel_heap_live(_info: &BootInfo) -> Result<(), &'static str> {
    let layout = Layout::from_size_align(128, 16).map_err(|_| "invalid layout")?;
    let p = crate::heap::alloc(layout).ok_or("kernel heap alloc failed post-EBS")?;
    let va = p.as_ptr() as u64;
    if va < KERNEL_OFFSET {
        return Err("heap pointer is not a kernel-view address");
    }
    // SAFETY: p is a live owned 128-byte allocation.
    unsafe {
        core::ptr::write_bytes(p.as_ptr(), 0x7E, 128);
        if *p.as_ptr() != 0x7E || *p.as_ptr().add(127) != 0x7E {
            return Err("kernel heap payload round-trip failed");
        }
        crate::heap::free(p).map_err(|_| "kernel heap free was rejected")?;
    }
    let Some(frame) = crate::frames::alloc() else {
        return Err("frame alloc failed post-EBS");
    };
    crate::frames::free(frame).map_err(|_| "frame free failed post-EBS")?;
    info!(
        "kernel",
        "kernel_heap_live: alloc/free at {va:#x} (chunk #{}, reserved {}KiB), frame {frame:#x} round-trip",
        crate::heap::chunk_count(),
        crate::heap::bytes_reserved() / 1024
    );
    Ok(())
}

/// The interrupt path fully relocated: gates at kernel-view aliases, the
/// absorb stub's EOI through the LAPIC's kernel-view alias. Two PIT ticks
/// must flow in a bounded IF=1 window (same discipline as the pre-EBS
/// tests) — a missed EOI or a stale gate address wedges this immediately.
fn test_kernel_irq_live(_info: &BootInfo) -> Result<(), &'static str> {
    const REQUIRED_TICKS: u64 = 2;
    const WINDOW_CAP_US: u64 = 500_000;
    let before = x86_64::idt::absorbed_irq_count();
    let t0 = crate::timekeeping::now_us();
    // SAFETY: bounded IF window, cli on every exit path; the PIT was
    // re-armed by kmain after the firmware EBS teardown, the PIT →
    // IOAPIC → LAPIC → IDT chain was proven pre-EBS (tick_rate), and the
    // descriptors were relocated before the switch.
    x86_64::sti();
    let mut ticks;
    loop {
        ticks = x86_64::idt::absorbed_irq_count() - before;
        if ticks >= REQUIRED_TICKS || crate::timekeeping::now_us() - t0 > WINDOW_CAP_US {
            break;
        }
        core::hint::spin_loop();
    }
    // Post-window LAPIC evidence: irr1 nonzero with zero absorptions means
    // the wire works but the CPU is not taking the interrupt (IF/delivery);
    // all-zero means nothing reaches the LAPIC at all (IOAPIC/wiring).
    let lapic_va = x86_64::paging::mmio_alias_va(x86_64::paging::apic_base_phys());
    // SAFETY: mapped RW alias in the kernel view; IF=0 again; MMIO reads.
    let irr1 =
        unsafe { crate::drivers::intc::lapic_read(lapic_va, crate::drivers::intc::LAPIC_IRR1) };
    let isr1 =
        unsafe { crate::drivers::intc::lapic_read(lapic_va, crate::drivers::intc::LAPIC_ISR1) };
    let ppr =
        unsafe { crate::drivers::intc::lapic_read(lapic_va, crate::drivers::intc::LAPIC_PPR) };
    x86_64::cli();
    if ticks < REQUIRED_TICKS {
        info!(
            "kernel",
            "irq window closed with irr1={irr1:#x} isr1={isr1:#x} ppr={ppr:#x} ticks={ticks}"
        );
        return Err("timer interrupts stopped flowing after the kernel-view switch");
    }
    info!(
        "kernel",
        "kernel_irq_live: {ticks} ticks through the relocated IDT + kernel-alias LAPIC EOI"
    );
    Ok(())
}
