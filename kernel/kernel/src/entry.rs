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
    // --- M5.1: kernel-side PCI enumeration (ADR-0021) ---------------------
    // The policy half of the driver split: record bus 0, size the BARs,
    // resolve every VirtIO function's capability structures, and set
    // MEM|BUS MASTER (DMA authorization stays a kernel decision; config
    // space itself is never exposed to ring 3). Non-fatal by design — a
    // machine with nothing on bus 0 still boots; the m5 suite is what
    // asserts the fixture disk.
    let (pci_funcs, virtio_devs) = crate::drivers::pci::enumerate();
    info!(
        "kernel",
        "pci enumeration: {pci_funcs} bus-0 function(s), {virtio_devs} virtio"
    );

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

    // --- M5.1: the driver substrate suite (ADR-0021) ----------------------
    // The PCI record asserted against the harness's virtio-blk fixture,
    // owned untyped frames through ring 3 (alloc / self-map / consume /
    // destroy-returns-frame / exact teardown), a kernel-minted Mmio cap
    // putting the HPET counter under ring-3 eyes, and the IRQ relay's
    // live stub→notify→wake chain proven with a LAPIC self-IPI.
    if !crate::m5::run_suite() {
        crate::halt::halt_machine("milestone 5 suite failed");
    }

    // --- M6.1: the network link proof (ADR-0024) ---------------------------
    // The m6 suite spawns its own short-lived netd (the userspace
    // virtio-net driver, image 6) plus nettest (image 7): a hand-built
    // ARP request for the slirp gateway goes out over the transmit
    // queue and the reply comes back over the receive queue — both
    // completions as MSI-X interrupts relayed into the driver's wait.
    // No virtio-net fixture on the bus → an honest SKIP, never a FAIL
    // (pre-v0.6.0 QEMU invocations stay bootable-green).
    // M7.0 (ADR-0029): the timer facility, installed before the suites
    // that use it and before any protocol exists to need it.
    if let Err(e) = crate::timer::init() {
        crate::halt::halt_machine(e);
    }
    crate::timer::log_ready();
    if !crate::m6::run_suite() {
        crate::halt::halt_machine("milestone 6 suite failed");
    }

    if !crate::m7::run_suite() {
        crate::halt::halt_machine("milestone 7 suite failed");
    }

    // --- Phase 11.0: kernel event plumbing (ADR-0071/0072) --------------------
    // Endpoint-bound notifications, generation-pinned cleanup, the
    // per-process timer quota and direct IPC handoff, proven on the real
    // objects before any production resident exists (every object minted
    // here is destroyed again).
    if !crate::m11::run_suite() {
        crate::halt::halt_machine("milestone 11 suite failed");
    }

    // ADR-0083: an independently linked native runtime consumes a real
    // read-only startup SharedRegion, validates actual cap slots, and proves
    // both valid entry and fail-closed hostile startup cases before residents.
    if !crate::m12::run_suite() {
        crate::halt::halt_machine("milestone 12 startup ABI suite failed");
    }
    // ADR-0089: prove the 32-deep IPC queue admits the simultaneous
    // Phase-12 client burst and refuses caller 33 without losing requests.
    if !crate::m12::run_ipc_queue_capacity_test() {
        crate::halt::halt_machine("milestone 12 IPC queue capacity proof failed");
    }

    // ADR-0056: run the bounded ring-3 SharedRegion capacity/teardown
    // probe before ANY production residents start. Exact resource delta
    // then belongs to this process, not asynchronously starting drivers.
    run_shared_probe();

    // --- M5.2: the production block service (ADR-0022) ----------------------
    // storaged — the userspace virtio-blk driver — starts at boot as a
    // resident service: the kernel mints its device window (an Mmio cap
    // over the BAR carrying the virtio structures), hands it an
    // endpoint's serve side and an interrupt notification, and the
    // driver does EVERYTHING else (virtio handshake, virtqueue setup,
    // IRQ relay arming, zero-copy DMA, completions) from ring 3. It
    // parks in recv; the filesystem daemon below is its first
    // production client. The m5 suite just proved the same image
    // end-to-end on a short-lived test instance.
    let blk_eid = match spawn_storaged() {
        Ok((_pid, eid)) => eid,
        Err(reason) => crate::halt::halt_machine(reason),
    };

    // --- M5.3: the production filesystem service (ADR-0023) -----------------
    // fsd mounts the AFS1 image the host tools formatted (and the m5
    // suite's fs_service test already committed to — that mount, in
    // THIS boot, is the persistence-across-re-open proof), then parks
    // serving the shell's ls/cat/write. Every fsd disk operation is a
    // forwarded block call to storaged; file data DMAs end-to-end
    // between the disk and the CLIENT's frame (zero copy).
    let (fs_pid, fs_eid, fs_diag_nid) = match spawn_fsd(blk_eid) {
        Ok((pid, eid, nid)) => (pid, eid, nid),
        Err(reason) => crate::halt::halt_machine(reason),
    };

    // ADR-0046: a separate ring-3 config reader owns neither fsd nor
    // the update marker. Only the configd process sees the raw FS cap;
    // the inert marker is a receiver-side anchor, NOT an opcode secret.
    let (config_eid, config_marker) =
        spawn_configd(fs_eid).unwrap_or_else(|e| crate::halt::halt_machine(e));

    // --- M6.1: the production network service (ADR-0024) --------------------
    // netd parks serving raw Ethernet frames (NET_SEND/NET_RECV/NET_MAC)
    // on its endpoint — Phase 7's stack will be its first production
    // client. ABSENT virtio-net function → the network service is simply
    // offline: pre-v0.6.0 QEMU invocations boot green without it.
    // ADR-0037: a private manager notification exists before either
    // driver starts, so their DRIVER_OK badges cannot be lost even if
    // they run before the manager's first instruction.
    let manager_nid = crate::ipc::create_notification()
        .unwrap_or_else(|_| crate::halt::halt_machine("servicemgr: notification table full"));
    // A DIFFERENT notification: netd must not be able to assert that
    // rngd is ready merely by writing rngd's badge bit. Authority to
    // signal each ready event is possession of a distinct WRITE cap.
    let rng_ready_nid = crate::ipc::create_notification()
        .unwrap_or_else(|_| crate::halt::halt_machine("servicemgr: rng readiness table full"));
    // ADR-0040: no driver or child may forge a restart-backoff timer.
    // Badge bits are data, not authority: give this private object
    // WRITE only to the manager, separately from netd/rngd readiness.
    let manager_restart_nid = crate::ipc::create_notification()
        .unwrap_or_else(|_| crate::halt::halt_machine("servicemgr: restart timer table full"));
    // ADR-0043: private administrative STOP authority. Unlike the
    // shared event wake, neither netd nor the stack can signal it.
    let manager_admin_nid = crate::ipc::create_notification()
        .unwrap_or_else(|_| crate::halt::halt_machine("servicemgr: admin notification table full"));
    // ADR-0047: proof objects, NEVER badge channels. The same object
    // is held read-only by each receiving service and explicitly
    // delegated by the trusted root to its diagnostic actor.
    let rng_diag_nid = crate::ipc::create_notification()
        .unwrap_or_else(|_| crate::halt::halt_machine("rngd: diagnostic marker table full"));
    let stack_diag_nid = crate::ipc::create_notification()
        .unwrap_or_else(|_| crate::halt::halt_machine("netstackd: diagnostic marker table full"));
    let (net_driver_pid, net_eid) = match spawn_netd(manager_nid) {
        Ok(Some((pid, eid))) => (Some(pid), Some(eid)),
        Ok(None) => {
            info!(
                "kernel",
                "netd: no virtio-net function on bus 0 — the network service stays offline (attach it with: -netdev user,id=net0 -device virtio-net-pci,netdev=net0)"
            );
            (None, None)
        }
        Err(reason) => crate::halt::halt_machine(reason),
    };

    // --- M6.2: the production entropy service (ADR-0025) --------------------
    // rngd parks serving RNG_GET (a caller-LENT frame filled by device
    // DMA) on its endpoint — the third driver on the shared virtio core.
    // ABSENT virtio-rng function → the entropy service is simply
    // offline, exactly as netd's fixture is optional.
    let (rng_driver_pid, rng_eid) = match spawn_rngd(rng_ready_nid, rng_diag_nid) {
        Ok(Some((pid, eid))) => (Some(pid), Some(eid)),
        Ok(None) => {
            info!(
                "kernel",
                "rngd: no virtio-rng function on bus 0 — the entropy service stays offline (attach it with: -device virtio-rng-pci)"
            );
            (None, None)
        }
        Err(reason) => crate::halt::halt_machine(reason),
    };

    // --- M6.3: the production input service (ADR-0026) ----------------------
    // inputd decodes virtio-input keycodes and pushes the bytes into
    // the kernel's console line discipline through a ConsoleInput-gated
    // SYS_CONSOLE_PUSH — the SAME queue the COM1 RX ISR feeds, so the
    // shell needs no changes and serial stays live beside the keyboard.
    // ABSENT virtio-input function → typing simply is not available and
    // the serial console remains the only input, exactly as before.
    let _input_pid = match spawn_inputd() {
        Ok(Some(pid)) => Some(pid),
        Ok(None) => {
            info!(
                "kernel",
                "inputd: no virtio-input function on bus 0 — the keyboard service stays offline (attach it with: -device virtio-keyboard-pci)"
            );
            None
        }
        Err(reason) => crate::halt::halt_machine(reason),
    };

    // --- M6.4: the production console channel (ADR-0027) --------------------
    // consoled turns a virtio-console port into a SECOND console: host
    // bytes enter the kernel's line discipline through the same
    // ConsoleInput gate inputd uses, and the kernel's console output
    // leaves through a ConsoleOutput-gated mirror. Serial remains the
    // kernel's own channel for logs and panics — this adds a channel,
    // it never moves one. ABSENT virtio-console function → the machine
    // is exactly what it was before.
    let _console_pid = match spawn_consoled() {
        Ok(Some(pid)) => Some(pid),
        Ok(None) => {
            info!(
                "kernel",
                "consoled: no virtio-console function on bus 0 — the console channel service stays offline (attach one with: -device virtio-serial-pci,max_ports=1 -device virtconsole,chardev=<id>)"
            );
            None
        }
        Err(reason) => crate::halt::halt_machine(reason),
    };

    info!(
        "kernel",
        "milestones 5–6.5 complete (executable format + image loader + syscall ABI v1 + first user process + IPC v1.1 + spawn protocol + driver substrate + resident block service + the AFS1 filesystem service: persistence and crash consistency proven + the virtio-net link-layer service: a real ARP round trip on the wire every boot + the shared virtio core and the entropy service: device randomness DMA'd into caller frames + the virtio-input keyboard service: decoded keystrokes pushed into the console line discipline beside the serial port + the virtio-console channel service: a second console in both directions, with serial still the kernel's own + supervised restart: a destroyed service answers its callers with a typed status and comes back with its capabilities replayed) — spawning the shell"
    );

    // --- M4.6: the hand-off (ADR-0020) -----------------------------------
    // The shell is the initial service: spawned through the M4.5
    // protocol's kernel-internal entry point (image 1, no parent), with
    // kernel-literal grants in slot order — Power (WRITE, so `shutdown`
    // is an authority the shell HOLDS), Image 0 (READ, so `spawn` can
    // start the test payload), its own notification (READ|WRITE, the
    // exit-badge channel for its children), and the filesystem
    // endpoint's call side (WRITE, M5.3: `ls`/`cat`/`write`). From here the machine
    // stops only through the shell's Power-gated SYS_SHUTDOWN, a panic
    // path, or the harness killing QEMU — the boot sequence no longer
    // halts on its own.
    let shell_nid = match crate::ipc::create_notification() {
        Ok(nid) => nid,
        Err(_) => crate::halt::halt_machine("shell: notification table full"),
    };
    // The manager is a separate ring-3 process, never a second owner
    // of kernel-minted driver grants. It starts the production stack
    // after live-cap and driver-readiness checks. One orderly restart
    // can be tested; full lifecycle/failure proof remains open.
    let (manager_pid, expected_stack_caps, permission_root, package_root) = spawn_servicemgr(
        fs_eid,
        manager_nid,
        rng_ready_nid,
        manager_restart_nid,
        manager_admin_nid,
        net_eid,
        rng_eid,
        rng_diag_nid,
        stack_diag_nid,
    )
    .unwrap_or_else(|e| crate::halt::halt_machine(e));
    crate::image_registry::set_manager(manager_pid);

    // The proof client has the ordinary endpoint only, never the raw
    // filesystem endpoint or a marker. Wait for its exact success exit,
    // then reclaim the boot-root test process before creating the shell.
    let read_grants = [crate::cap::Cap {
        obj: crate::cap::CapObj::Endpoint { eid: config_eid },
        rights: crate::cap::RIGHTS_WRITE | crate::cap::RIGHTS_COPY,
    }];
    let reader_pid = crate::spawn::spawn_init(22, &read_grants, Some((shell_nid, 0xC081)))
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    let rec = crate::spawn::records_snapshot();
    let reader_tid = rec
        .iter()
        .flatten()
        .find(|&&(pid, _)| pid == reader_pid)
        .map(|&(_, tid)| tid)
        .unwrap_or_else(|| crate::halt::halt_machine("configread: spawn record missing"));
    // Before the boot thread becomes idle it must NOT block on a
    // notification: if the fsd/virtio completion is pending, all other
    // threads can be blocked and sched::block_current halts on an empty
    // ready ring. Stay runnable AND IF=1 for the device MSI, like M5's
    // HPET-bounded device-service drain (not a yield-count retry).
    let if_before = crate::arch::x86_64::interrupts_enabled();
    let t0 = crate::timekeeping::now_us();
    let mut last_audited_probe = None;
    crate::arch::x86_64::sti();
    loop {
        // The manager is already running. Its first short-lived worker
        // may exit before the idle loop; audit its REAL installed caps
        // now, not merely after the reader returns. The production
        // child's shell reference is still installed by the idle loop.
        if let Some(wanted) = expected_stack_caps {
            if let Some(ManagerChild::Probe(pid, mode)) =
                audit_manager_child(manager_pid, &wanted, manager_restart_nid)
                    .unwrap_or_else(|e| crate::halt::halt_machine(e))
            {
                if last_audited_probe != Some(pid) {
                    if mode == 0 {
                        info!(
                            "m8",
                            "manager-owned dependency probe pid {pid}: netd/W rngd/W private-notification/W audited, no privileged extras"
                        );
                    } else {
                        info!(
                            "m8",
                            "manager-owned dependency probe pid {pid}: diagnostic mode {mode}, exact attenuated caps audited"
                        );
                    }
                    last_audited_probe = Some(pid);
                }
            }
        }
        if let Some(status) = crate::arch::x86_64::syscall::exit_status_of(reader_tid) {
            if status != 42 {
                crate::halt::halt_machine(
                    "configread: ordinary client refused read/authority proof",
                );
            }
            break;
        }
        if crate::timekeeping::now_us().saturating_sub(t0) > 21_000_000 {
            crate::halt::halt_machine("configread: device-bound reader deadline expired");
        }
        crate::sched::yield_now();
    }
    if !if_before {
        crate::arch::x86_64::cli();
    }
    if crate::ipc::try_wait(shell_nid) != Ok(0xC081) {
        crate::halt::halt_machine("configread: proof client exit badge missing");
    }
    crate::proc::destroy(reader_pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
    crate::spawn::forget(reader_pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
    info!(
        "kernel",
        "configread: boot-root reader reaped; no update authority delegated"
    );

    // A separate updater holds the only transferable update marker.
    // It first reads a marker-gated test plan from configd and SKIPs
    // without any FS writes if the trusted shell staged no intent on
    // a previous boot. The ordinary reader has already exited: neither
    // it nor the shell is ever granted the marker or raw FS by proxy.
    let update_grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid: config_eid },
            rights: crate::cap::RIGHTS_WRITE | crate::cap::RIGHTS_COPY,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid: config_marker },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_COPY | crate::cap::RIGHTS_DESTROY,
        },
    ];
    let updater_pid = crate::spawn::spawn_init(23, &update_grants, Some((shell_nid, 0xC082)))
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    for (slot, &wanted) in update_grants.iter().enumerate() {
        if crate::cap::read(updater_pid, slot).ok() != Some(wanted) {
            crate::halt::halt_machine("configup: boot grant did not match literal policy");
        }
    }
    let up_rec = crate::spawn::records_snapshot();
    let updater_tid = up_rec
        .iter()
        .flatten()
        .find(|&&(pid, _)| pid == updater_pid)
        .map(|&(_, tid)| tid)
        .unwrap_or_else(|| crate::halt::halt_machine("configup: spawn record missing"));
    let if_before = crate::arch::x86_64::interrupts_enabled();
    let t0 = crate::timekeeping::now_us();
    crate::arch::x86_64::sti();
    loop {
        // Keep auditing the manager's initial probe across BOTH child
        // drains; one could exit between the two reader/updater proofs.
        if let Some(wanted) = expected_stack_caps {
            if let Some(ManagerChild::Probe(pid, mode)) =
                audit_manager_child(manager_pid, &wanted, manager_restart_nid)
                    .unwrap_or_else(|e| crate::halt::halt_machine(e))
            {
                if last_audited_probe != Some(pid) {
                    if mode == 0 {
                        info!(
                            "m8",
                            "manager-owned dependency probe pid {pid}: netd/W rngd/W private-notification/W audited, no privileged extras"
                        );
                    } else {
                        info!(
                            "m8",
                            "manager-owned dependency probe pid {pid}: diagnostic mode {mode}, exact attenuated caps audited"
                        );
                    }
                    last_audited_probe = Some(pid);
                }
            }
        }
        if let Some(status) = crate::arch::x86_64::syscall::exit_status_of(updater_tid) {
            if status != 42 {
                crate::halt::halt_machine("configup: updater rejected plan/update proof");
            }
            break;
        }
        if crate::timekeeping::now_us().saturating_sub(t0) > 21_000_000 {
            crate::halt::halt_machine("configup: device-bound updater deadline expired");
        }
        crate::sched::yield_now();
    }
    if !if_before {
        crate::arch::x86_64::cli();
    }
    if crate::ipc::try_wait(shell_nid) != Ok(0xC082) {
        crate::halt::halt_machine("configup: proof client exit badge missing");
    }
    crate::proc::destroy(updater_pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
    crate::spawn::forget(updater_pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
    info!(
        "kernel",
        "configup: boot-root updater reaped; marker never delegated to shell"
    );

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
            rights: crate::cap::RIGHTS_READ
                | crate::cap::RIGHTS_WRITE
                | crate::cap::RIGHTS_COPY
                | crate::cap::RIGHTS_DESTROY,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid: fs_eid },
            rights: crate::cap::RIGHTS_WRITE,
        },
    ];
    // ADR-0040: the Power-holding administrator alone can exercise a
    // production client's call endpoint. No Process cap/driver/MMIO;
    // missing dependencies grant NO partial stack access.
    let mut shell_full = [crate::cap::Cap::EMPTY; 7];
    shell_full[..4].copy_from_slice(&shell_grants);
    let shell_caps: &[crate::cap::Cap] = if let Some(caps) = expected_stack_caps {
        let crate::cap::CapObj::Endpoint { eid } = caps[1].obj else {
            crate::halt::halt_machine("manager: stack grant was not an endpoint");
        };
        shell_full[4] = crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_WRITE,
        };
        shell_full[5] = crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid: manager_nid },
            rights: crate::cap::RIGHTS_WRITE, // wake hint only
        };
        shell_full[6] = crate::cap::Cap {
            obj: crate::cap::CapObj::Notification {
                nid: manager_admin_nid,
            },
            rights: crate::cap::RIGHTS_WRITE, // private STOP request
        };
        &shell_full
    } else {
        &shell_grants
    };
    // A real, parked ring-3 server has already painted and finished its
    // bounded allocation before shell/manager resource commands can run.
    // This avoids a one-frame startup race across historical reboot tests.
    let desktop_frame_nid = crate::ipc::create_notification()
        .unwrap_or_else(|_| crate::halt::halt_machine("desktop: frame notification bound"));
    let mut app_clock_nids = [0u32; 32];
    for nid in &mut app_clock_nids {
        *nid = crate::ipc::create_notification()
            .unwrap_or_else(|_| crate::halt::halt_machine("desktop: app clock capacity"));
    }
    // Phase 11.5 (ADR-0076/0077): the AFS2 file service, desktop profile
    // only (the phase-9 fixture keeps its historical process set).
    let filesd_eid = (option_env!("ARENA_GRAPHICS_FIXTURE") != Some("phase9"))
        .then(|| spawn_filesd(blk_eid, fs_eid).unwrap_or_else(|e| crate::halt::halt_machine(e)));
    // ADR-0091: issue the exact filesd APB1 badge into servicemgr's reserved
    // root-owned slot127. It is outside the manager's fixed 32-slot named
    // child/process inventory, so it does not consume a Process-handle slot;
    // no child receives it before packaged's readiness proof.
    if let (Some(eid), Some(_package)) = (filesd_eid, package_root) {
        use crate::cap::{Cap, CapObj, RIGHTS_COPY as C, RIGHTS_WRITE as W};
        let generation = crate::ipc::endpoint_generation(eid)
            .unwrap_or_else(|| crate::halt::halt_machine("filesd endpoint vanished before install handoff"));
        let eid16 = u16::try_from(eid)
            .unwrap_or_else(|_| crate::halt::halt_machine("filesd endpoint index exceeds badge ABI"));
        crate::sync::without_interrupts(|| {
            if crate::cap::slot_occupied(manager_pid, MGR_SLOT_APB1_INSTALL_AUTH)
                != Some(false)
            {
                crate::halt::halt_machine("servicemgr: reserved APB1 slot127 occupied");
            }
            crate::cap::issue(
                manager_pid,
                MGR_SLOT_APB1_INSTALL_AUTH,
                Cap {
                    obj: CapObj::BadgedEndpoint {
                        eid: eid16,
                        generation,
                        badge: FILESD_APB1_INSTALL_BADGE,
                    },
                    rights: W | C,
                },
            )
            .unwrap_or_else(|e| crate::halt::halt_machine(e));
        });
        crate::ipc::notify(manager_nid, MGR_BADGE_APB1_INSTALL_AUTH)
            .unwrap_or_else(|_| crate::halt::halt_machine("servicemgr: APB1 handoff wake refused"));
        info!(
            "m12",
            "late filesd APB1 BadgedEndpoint issued to servicemgr slot127; authority remains withheld from packaged until receiver READY"
        );
    }
    let mut graphics = start_boot_display(
        _input_pid,
        desktop_frame_nid,
        &app_clock_nids,
        fs_eid,
        filesd_eid,
        package_root.map(|(eid, _)| eid),
    );
    let shell_pid = crate::spawn::spawn_init(1, shell_caps, None)
        .unwrap_or_else(|reason| crate::halt::halt_machine(reason));
    if let Some(eid) = graphics.as_ref().and_then(|g| g.launch_endpoint) {
        crate::cap::issue(
            manager_pid,
            31,
            crate::cap::Cap {
                obj: crate::cap::CapObj::Endpoint { eid },
                rights: crate::cap::RIGHTS_WRITE | crate::cap::RIGHTS_COPY,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
        info!(
            "m10",
            "late explicit manager slot31 grant: desktop launch endpoint W|C; execution still requires a held live Image"
        );
    }
    if expected_stack_caps.is_some() {
        crate::cap::issue(
            shell_pid,
            15,
            crate::cap::Cap {
                obj: crate::cap::CapObj::Notification {
                    nid: stack_diag_nid,
                },
                rights: crate::cap::RIGHTS_READ
                    | crate::cap::RIGHTS_COPY
                    | crate::cap::RIGHTS_DESTROY,
            },
        )
        .unwrap_or_else(|_| crate::halt::halt_machine("shell: diagnostic marker issue refused"));
    }
    if let Some((eid, nid)) = permission_root {
        use crate::cap::{
            Cap, CapObj, RIGHTS_COPY as C, RIGHTS_DESTROY as D, RIGHTS_READ as R, RIGHTS_WRITE as W,
        };
        crate::cap::issue(
            shell_pid,
            16,
            Cap {
                obj: CapObj::Endpoint { eid },
                rights: W | C,
            },
        )
        .unwrap_or_else(|_| crate::halt::halt_machine("shell: mediator issue refused"));
        crate::cap::issue(
            shell_pid,
            17,
            Cap {
                obj: CapObj::Notification { nid },
                rights: R | C | D,
            },
        )
        .unwrap_or_else(|_| crate::halt::halt_machine("shell: approval marker issue refused"));
        // Trusted shell can deliberately transfer its held mediator
        // client cap to an independent proof child of the same app image.
        // That child inherits ONLY the transferred endpoint, not this Image.
        crate::cap::issue(
            shell_pid,
            18,
            Cap {
                obj: CapObj::Image { img_id: 25 },
                rights: R,
            },
        )
        .unwrap_or_else(|_| crate::halt::halt_machine("shell: delegate fixture image refused"));
    }
    // ADR-0053: separate package endpoint and marker. Neither fsd nor
    // permission approval is a substitute, and no app inherits either.
    if let Some((eid, nid)) = package_root {
        use crate::cap::{
            Cap, CapObj, RIGHTS_COPY as C, RIGHTS_DESTROY as D, RIGHTS_READ as R, RIGHTS_WRITE as W,
        };
        crate::cap::issue(
            shell_pid,
            20,
            Cap {
                obj: CapObj::Endpoint { eid },
                rights: W | C,
            },
        )
        .unwrap_or_else(|_| crate::halt::halt_machine("shell: package endpoint issue refused"));
        crate::cap::issue(
            shell_pid,
            21,
            Cap {
                obj: CapObj::Notification { nid },
                rights: R | C | D,
            },
        )
        .unwrap_or_else(|_| crate::halt::halt_machine("shell: package marker issue refused"));
    }
    // ADR-0051: production fsd's separate diagnostic marker is reserved
    // for the trusted Power/raw-FS shell. No broker/app/worker receives
    // this authority; both the FS endpoint and marker are needed.
    crate::cap::issue(
        shell_pid,
        19,
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid: fs_diag_nid },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_COPY | crate::cap::RIGHTS_DESTROY,
        },
    )
    .unwrap_or_else(|_| crate::halt::halt_machine("shell: fsd diagnostic issue refused"));
    info!(
        "kernel",
        "shell spawned: pid {shell_pid} (caps: 0=Power/W 1=Image0/R 2=Notif{shell_nid}/RW 3=Endpoint{fs_eid}/W) — the console is live; type 'help'"
    );
    // ADR-0044: diagnostic Process references only on a full fixture.
    // All four DESTROY targets are structurally protected by the kernel;
    // the foreign production child gets READ ONLY *after* the kernel
    // independently audits its actual Process cap in the manager.
    // Fixed slots 10..14 do not overlap the shell's spawn/File caps.
    if let (Some(net), Some(rng), Some(_)) = (net_driver_pid, rng_driver_pid, expected_stack_caps) {
        use crate::cap::{Cap, CapObj, RIGHTS_DESTROY, RIGHTS_READ};
        for (slot, target, rights) in [
            (10, shell_pid, RIGHTS_READ | RIGHTS_DESTROY),
            (11, manager_pid, RIGHTS_READ | RIGHTS_DESTROY),
            (12, net, RIGHTS_READ | RIGHTS_DESTROY),
            (13, rng, RIGHTS_READ | RIGHTS_DESTROY),
            (14, shell_pid, RIGHTS_READ), // inert reserved placeholder
        ] {
            crate::cap::issue(
                shell_pid,
                slot,
                Cap {
                    obj: CapObj::Process { pid: target },
                    rights,
                },
            )
            .unwrap_or_else(|_| {
                crate::halt::halt_machine("m8: protected Process reference issue refused")
            });
        }
        info!(
            "m8",
            "lifecycle protected refs shell={shell_pid} manager={manager_pid} netd={net} rngd={rng}; foreign placeholder READ-only"
        );
    }

    // ADR-0088: fixed production notifications plus 32 private Desktop
    // clocks occupy 51 of 64 slots on the full fixture. Fill the remaining
    // table without dropping any occupied object, prove the 65th create is
    // mutation-free, then retire only the exact temporary Notifications.
    if net_eid.is_some() && rng_eid.is_some() && _input_pid.is_some() && _console_pid.is_some() {
        let before = crate::ipc::notification_snapshot();
        let fixed_occupied = crate::ipc::notification_occupancy();
        if fixed_occupied != 51 {
            crate::halt::halt_machine("servicemgr: 32-clock notification baseline changed");
        }
        let mut temporary = [u32::MAX; crate::ipc::MAX_NOTIFS];
        let mut count = 0usize;
        while crate::ipc::notification_occupancy() < crate::ipc::MAX_NOTIFS {
            temporary[count] = crate::ipc::create_notification().unwrap_or_else(|_| {
                crate::halt::halt_machine("servicemgr: notification fill refused")
            });
            count += 1;
        }
        let full = crate::ipc::notification_snapshot();
        if count != 13
            || crate::ipc::create_notification().is_ok()
            || crate::ipc::notification_snapshot() != full
        {
            crate::halt::halt_machine(
                "servicemgr: notification bound failed mutation-free sixty-fifth refusal",
            );
        }
        for nid in &temporary[..count] {
            crate::ipc::destroy_notification(*nid).unwrap_or_else(|_| {
                crate::halt::halt_machine("servicemgr: temporary notification cleanup refused")
            });
        }
        if crate::ipc::notification_snapshot() != before {
            crate::halt::halt_machine("servicemgr: notification probe cleanup changed baseline");
        }
        info!(
            "kernel",
            "servicemgr: full fixture notification budget 64/64; sixty-fifth refused, 13 probe slots reclaimed"
        );
    }

    // The bootstrap thread becomes the idle thread: it stays runnable
    // forever, which keeps block_current's no-runnable-thread deadlock
    // halt (ADR-0018) unreachable while the shell parks on console
    // input, and gives every wake (console RX, tick) somewhere to
    // return. Between wakes it halts the CPU with IF=1 — interrupts
    // must flow now: the UART RX path IS the input device.
    let mut last_audited_child = None;
    let mut fs_reaped = false;
    loop {
        // ADR-0061: there is no authorized compositor restart protocol.
        // Preserve possession-based IPC teardown and fail-stop the machine
        // rather than leave inputd blocked or advertise stale windows as
        // usable after either original boot-root graphics service died.
        if let Some(ref mut g) = graphics {
            for pid in [g.compositor, g.display] {
                if crate::sched::proc_live_threads(pid) == 0 {
                    crate::proc::destroy(pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
                    crate::spawn::forget(pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
                    crate::halt::halt_machine(
                        "graphics: original boot service died; IPC failed; fail-closed, no restart",
                    );
                }
            }
            for i in 0..2 {
                if g.clients[i] == 0 {
                    continue;
                }
                if !g.exited[i] && crate::sched::proc_live_threads(g.clients[i]) == 0 {
                    // Real original-child retirement (not a dropped cap).
                    // IPC cancellation is swept by proc::destroy as usual.
                    crate::proc::destroy(g.clients[i])
                        .unwrap_or_else(|e| crate::halt::halt_machine(e));
                    crate::spawn::forget(g.clients[i])
                        .unwrap_or_else(|e| crate::halt::halt_machine(e));
                    g.exited[i] = true;
                    g.deadline[i] = crate::timekeeping::now_us().saturating_add(2_000_000);
                }
                if g.exited[i] && !g.retired[i] {
                    // Only the root-issued comparator may remain. A copy
                    // or mapped surface keeps refs/pins above this bound;
                    // never consume the last witness prematurely.
                    if crate::shared::reference_snapshot(g.regions[i]) == Some((1, 0)) {
                        crate::shared::assert_conservation();
                        crate::cap::consume(g.compositor, 5 + i)
                            .unwrap_or_else(|e| crate::halt::halt_machine(e));
                        crate::cap::consume(g.compositor, 3 + i)
                            .unwrap_or_else(|e| crate::halt::halt_machine(e));
                        if crate::shared::reference_snapshot(g.regions[i]).is_some() {
                            crate::halt::halt_machine("graphics: retired region still referenced");
                        }
                        g.retired[i] = true;
                        let (runs, pages, maps) = crate::shared::usage_snapshot();
                        info!(
                            "m9",
                            "graphics original child {} retired: Process record, region refs, mappings and comparator cap conserved; shared {runs}/{} runs, {pages}/{} pages, {maps}/{} maps; compositor caps {:?}; free frames {}",
                            i,
                            crate::shared::MAX_REGIONS,
                            crate::shared::TOTAL_PAGES,
                            crate::shared::MAX_MAPS,
                            crate::cap::occupancy(g.compositor),
                            crate::frames::free_frames()
                        );
                    } else if crate::timekeeping::now_us() >= g.deadline[i] {
                        crate::halt::halt_machine(
                            "graphics: dead original child never retired its copied region",
                        );
                    }
                }
            }
        }
        // ADR-0051: fsd is a kernel-boot-root service, not a
        // user-Process-cap-managed child. If its LAST thread exits,
        // finish root-owned teardown here (away from its CR3). Without
        // this sweep callers to the orphaned FS endpoint wait forever.
        if !fs_reaped && crate::sched::proc_live_threads(fs_pid) == 0 {
            crate::proc::destroy(fs_pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
            crate::spawn::forget(fs_pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
            fs_reaped = true;
            info!(
                "kernel",
                "fsd root exited; process reaped, endpoint orphaned; filesystem OFFLINE"
            );
        }
        // ADR-0039: independently audit the child created by the
        // MANAGER's syscall. The kernel sees its Process handle in the
        // manager's actual cap table, then compares five installed
        // child caps to the fixed root policy. The probe worker is audited
        // separately and never confused with the production stack.
        if let Some(wanted) = expected_stack_caps {
            match audit_manager_child(manager_pid, &wanted, manager_restart_nid)
                .unwrap_or_else(|e| crate::halt::halt_machine(e))
            {
                Some(ManagerChild::Probe(pid, mode)) => {
                    if last_audited_probe != Some(pid) {
                        if mode == 0 {
                            info!(
                                "m8",
                                "manager-owned dependency probe pid {pid}: netd/W rngd/W private-notification/W audited, no privileged extras"
                            );
                        } else {
                            info!(
                                "m8",
                                "manager-owned dependency probe pid {pid}: diagnostic mode {mode}, exact attenuated caps audited"
                            );
                        }
                        last_audited_probe = Some(pid);
                    }
                }
                Some(ManagerChild::Production(child)) => {
                    if last_audited_child != Some(child) {
                        info!(
                            "m8",
                            "manager-owned netstackd pid {child}: five inherited child caps audited (netd/W stack/R backoff/RW rngd/W diag/R), IPC landings excluded"
                        );
                        // READ-only reference; neither boot policy nor a
                        // manifest grants the shell the child's DESTROY.
                        let previous = last_audited_child.unwrap_or(shell_pid);
                        let want = crate::cap::Cap {
                            obj: crate::cap::CapObj::Process { pid: previous },
                            rights: crate::cap::RIGHTS_READ,
                        };
                        if crate::cap::read(shell_pid, 14).ok() != Some(want) {
                            crate::halt::halt_machine("m8: foreign reference slot was modified");
                        }
                        crate::cap::consume(shell_pid, 14).unwrap_or_else(|_| {
                            crate::halt::halt_machine("m8: foreign reference retire failed")
                        });
                        crate::cap::issue(
                            shell_pid,
                            14,
                            crate::cap::Cap {
                                obj: crate::cap::CapObj::Process { pid: child },
                                rights: crate::cap::RIGHTS_READ,
                            },
                        )
                        .unwrap_or_else(|_| {
                            crate::halt::halt_machine("m8: foreign reference issue failed")
                        });
                        info!(
                            "m8",
                            "lifecycle read-only foreign Process reference installed pid {child} slot 14"
                        );
                        last_audited_child = Some(child);
                    }
                }
                None => {}
            }
        }
        // M7.0: the idle thread is also the SUPERVISOR's hands
        // (ADR-0028 built restart but left `poll` uncalled outside the
        // suite, which made production supervision a promise rather
        // than a fact). This is the right place for it: `poll` spawns,
        // which allocates frames and maps pages, so it must run at
        // plain thread context — never from the death notice inside
        // `proc::destroy`, and never from a tick. It costs one relaxed
        // scan of a small table per wake when nothing has died.
        let restarted = crate::supervise::poll();
        if restarted > 0 {
            info!(
                "kernel",
                "supervisor: restarted {restarted} service(s) — a driver died and came back while the machine kept running"
            );
        }
        crate::sched::yield_now();
        // Halt only when nothing is runnable. A thread made ready by a
        // wake (an IPC server woken by a caller that then blocked, a
        // caller woken by a reply) must run now, not after the next PIT
        // tick: halting with a non-empty ready ring cost every IPC hop up
        // to a full 10 ms tick. The check runs with IF=0 so an
        // interrupt-driven wake cannot slip in between check and halt.
        x86_64::cli();
        if crate::sched::ready_pending() {
            x86_64::sti();
        } else {
            // SAFETY: ring 0; `sti; hlt` is the canonical idle pair —
            // `sti` takes effect after `hlt` begins, so any pending or
            // arriving interrupt resumes the loop right here, and the
            // interrupt gate masks IF again for the handler.
            unsafe { core::arch::asm!("sti", "hlt", options(nomem, nostack)) };
        }
    }
}

/// The exact boot-root objects retained for structural graphics teardown.
/// No user-provided PID, wire handle or cap-slot number enters this record.
struct GraphicsRuntime {
    display: u64,
    compositor: u64,
    clients: [u64; 2],
    regions: [u32; 2],
    exited: [bool; 2],
    retired: [bool; 2],
    deadline: [u64; 2],
    launch_endpoint: Option<u32>,
}

/// One boot-root graphics service: explicit optional GOP and/or modern 2D
/// virtio GPU. A capability BAR, not the PCI numeric address, is delegated.
/// The endpoint is parked before boot continues; graphical readiness is a
/// separate service concern, never inferred from the mere presence of a BAR.
fn start_boot_display(
    input_pid: Option<u64>,
    frame_nid: u32,
    app_clocks: &[u32; 32],
    fs_eid: u32,
    filesd_eid: Option<u32>,
    package_eid: Option<u32>,
) -> Option<GraphicsRuntime> {
    let gop = crate::handoff::display();
    if let Some(mode) = gop {
        if mode.phys & 4095 != 0 || mode.bytes == 0 || mode.bytes > 2 * 1024 * 1024 {
            crate::halt::halt_machine("displayd: invalid GOP handoff bounds");
        }
    }
    // Refusal-only filter before granting any BAR: a single common BAR
    // covers *all* four checked virtio structures, is aligned MMIO, and
    // has a bounded map size. SYS_DEV_INFO rechecks held-cap coverage for
    // every caller, rather than trusting this boot-time discovery.
    let gpu =
        crate::drivers::pci::find_virtio(crate::drivers::pci::VIRTIO_TYPE_GPU).and_then(|v| {
            let bar = v.common.bar as usize;
            let f = crate::drivers::pci::pci_function(v.pci_index)?;
            if v.device_id != 0x1050
                || v.transitional
                || bar >= 6
                || f.bar_is_io[bar]
                || f.bar_base[bar] & 4095 != 0
                || f.bar_size[bar] == 0
                || f.bar_size[bar] > 64 * 1024
                || [v.common, v.notify, v.isr, v.device_cfg].iter().any(|loc| {
                    !loc.present
                        || loc.bar as usize != bar
                        || loc
                            .offset
                            .checked_add(loc.length)
                            .is_none_or(|end| u64::from(end) > f.bar_size[bar])
                })
            {
                return None;
            }
            Some(crate::cap::Cap {
                obj: crate::cap::CapObj::Mmio {
                    phys: f.bar_base[bar],
                    pages: f.bar_size[bar].div_ceil(4096) as u32,
                },
                rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
            })
        });
    if gop.is_none() && gpu.is_none() {
        info!(
            "kernel",
            "displayd: no validated GOP or virtio-gpu; headless boot"
        );
        return None;
    }
    let eid = crate::ipc::create_endpoint()
        .unwrap_or_else(|_| crate::halt::halt_machine("displayd: endpoint table full"));
    let first = if let Some(mode) = gop {
        crate::cap::Cap {
            obj: crate::cap::CapObj::Mmio {
                phys: mode.phys,
                pages: mode.bytes.div_ceil(4096) as u32,
            },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        }
    } else {
        gpu.unwrap_or_else(|| crate::halt::halt_machine("displayd: missing GPU cap"))
    };
    let grants = [
        first,
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_READ,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::MemoryPool,
            rights: crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::SharedDma,
            rights: crate::cap::RIGHTS_READ,
        },
    ];
    let display_pid = if gop.is_some() {
        if let Some(gpu_cap) = gpu {
            crate::spawn::spawn_init_boot(
                0,
                &[grants[0], grants[1], grants[2], grants[3], gpu_cap],
                None,
            )
        } else {
            crate::spawn::spawn_init_boot(0, &grants, None)
        }
    } else {
        crate::spawn::spawn_init_boot(0, &grants, None)
    }
    .unwrap_or_else(|reason| crate::halt::halt_machine(reason));
    info!(
        "kernel",
        "displayd spawned: pid {display_pid}; GOP={} virtio-gpu={}; sole framebuffer/BAR holder",
        gop.is_some(),
        gpu.is_some()
    );
    let was_if = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let deadline = crate::timekeeping::now_us().saturating_add(5_000_000);
    while !crate::ipc::parked_server(eid, display_pid) {
        if crate::sched::proc_live_threads(display_pid) == 0
            || crate::timekeeping::now_us() >= deadline
        {
            crate::halt::halt_machine("displayd: no painted, parked ring-3 service by deadline");
        }
        crate::sched::yield_now();
    }
    if !was_if {
        crate::arch::x86_64::cli();
    }
    let (regions, pages, maps) = crate::shared::usage_snapshot();
    let (held, slots) = crate::cap::occupancy(display_pid)
        .unwrap_or_else(|| crate::halt::halt_machine("displayd: lost capability space"));
    info!(
        "kernel",
        "displayd resources at parked boundary: shared {regions}/{} runs, {pages}/{} pages, {maps}/{} maps; caps {held}/{slots}; free frames {}; live processes {}",
        crate::shared::MAX_REGIONS,
        crate::shared::TOTAL_PAGES,
        crate::shared::MAX_MAPS,
        crate::frames::free_frames(),
        crate::proc::live_count(),
    );
    // The first independent ring-3 client tests receiver-checked MODE and
    // PRESENT while the compositor bridge is still under construction.
    // It has neither GOP/GPU BAR nor DMA bearer. The bootstrap root reaps
    // it and checks this client's own process/record retirement plus
    // global shared/cap conservation. Other production workers may be
    // born/die concurrently, so a global frame delta is not an isolated
    // client credit. The presented pixel stays in displayd's scanout.
    let shared_before = crate::shared::usage_snapshot();
    let display_caps_before = crate::cap::occupancy(display_pid);
    let grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::MemoryPool,
            rights: crate::cap::RIGHTS_WRITE,
        },
    ];
    let client = crate::spawn::spawn_init_boot(2, &grants, None)
        .unwrap_or_else(|reason| crate::halt::halt_machine(reason));
    let tid = crate::spawn::records_snapshot()
        .iter()
        .flatten()
        .find_map(|&(child, thread)| (child == client).then_some(thread))
        .unwrap_or_else(|| crate::halt::halt_machine("displayprobe: missing spawn record"));
    let was_if = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let deadline = crate::timekeeping::now_us().saturating_add(5_000_000);
    while crate::sched::proc_live_threads(client) != 0 {
        if crate::timekeeping::now_us() >= deadline {
            crate::halt::halt_machine("displayprobe: ring-3 client deadline expired");
        }
        crate::sched::yield_now();
    }
    crate::sched::yield_now();
    if !was_if {
        crate::arch::x86_64::cli();
    }
    let observed = crate::arch::x86_64::syscall::exit_status_of(tid);
    if observed != Some(42) {
        error!("m9", "displayprobe ring-3 exit code: {observed:?}");
        crate::halt::halt_machine("displayprobe: typed present/cap refusal failed");
    }
    crate::proc::destroy(client).unwrap_or_else(|e| crate::halt::halt_machine(e));
    crate::spawn::forget(client).unwrap_or_else(|e| crate::halt::halt_machine(e));
    crate::shared::assert_conservation();
    if crate::proc::pml4_of(client).is_some()
        || crate::spawn::records_snapshot()
            .iter()
            .flatten()
            .any(|&(pid, _)| pid == client)
        || crate::shared::usage_snapshot() != shared_before
        || crate::cap::occupancy(display_pid) != display_caps_before
    {
        crate::halt::halt_machine("displayprobe: client record/shared cap teardown mismatch");
    }
    info!(
        "m9",
        "displayprobe: cap-bearing MODE/PRESENT guest; own record retired; shared/cap/PTE accounting conserved PASS"
    );
    // Explicit historical graphics fixture, independent of device names/counts.
    // Shipping builds start the desktop with either keyboard alone or tablet.
    if option_env!("ARENA_GRAPHICS_FIXTURE") != Some("phase9") {
        Some(start_boot_desktop(
            display_pid,
            eid,
            input_pid,
            frame_nid,
            app_clocks,
            fs_eid,
            filesd_eid,
            package_eid,
        ))
    } else {
        Some(start_boot_compositor(display_pid, eid, input_pid))
    }
}

/// Root provides *exactly two* disjoint original-child Process/READ and
/// SharedRegion witnesses. No client inherits an allocation pool or display
/// bearer; the compositor's only display access is the typed endpoint.
fn start_boot_compositor(
    display_pid: u64,
    display_eid: u32,
    input_pid: Option<u64>,
) -> GraphicsRuntime {
    use crate::cap::{Cap, CapObj, RIGHTS_COPY as C, RIGHTS_READ as R, RIGHTS_WRITE as W};
    let eid = crate::ipc::create_endpoint()
        .unwrap_or_else(|_| crate::halt::halt_machine("compositor: endpoint bound"));
    let comp = crate::spawn::spawn_init_boot(
        3,
        &[
            Cap {
                obj: CapObj::Endpoint { eid: display_eid },
                rights: W,
            },
            Cap {
                obj: CapObj::Endpoint { eid },
                rights: R,
            },
            Cap {
                obj: CapObj::ProofToken {
                    id: 0x4152_454e_4149_4e50,
                },
                rights: R,
            },
        ],
        None,
    )
    .unwrap_or_else(|e| crate::halt::halt_machine(e));
    let child_grants = [Cap {
        obj: CapObj::Endpoint { eid },
        rights: W,
    }];
    let a = crate::spawn::spawn_init_boot(4, &child_grants, None)
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    let b = crate::spawn::spawn_init_boot(5, &child_grants, None)
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    let mut region_ids = [0u32; 2];
    for (i, (child, proc_slot, region_slot)) in [(a, 3, 5), (b, 4, 6)].into_iter().enumerate() {
        let (slot, id) = crate::shared::create(child, 19)
            .unwrap_or_else(|| crate::halt::halt_machine("compositor: owned region allocation"));
        if slot != 1 {
            crate::halt::halt_machine("compositor: region not in expected child slot")
        }
        region_ids[i] = id;
        crate::cap::issue(
            comp,
            proc_slot,
            Cap {
                obj: CapObj::Process { pid: child },
                rights: R,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
        crate::cap::issue(
            comp,
            region_slot,
            Cap {
                obj: CapObj::SharedRegion { id },
                rights: R | W | C,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    }
    // The production input instance never serves its inherited READ-only
    // endpoint. Reclaim that object before substituting the compositor call
    // side; the m6 diagnostic input instance is a separate process and
    // retains its existing endpoint/marker topology unchanged.
    if let Some(pid) = input_pid {
        let old = crate::cap::read(pid, 1).unwrap_or_else(|e| crate::halt::halt_machine(e));
        let CapObj::Endpoint { eid: old_eid } = old.obj else {
            crate::halt::halt_machine("compositor: input endpoint grant changed")
        };
        // Kernel root retires its own old READ-only grant; userspace may
        // not destroy it (no DESTROY bit) and receives no authority to do so.
        crate::cap::consume(pid, 1).unwrap_or_else(|e| crate::halt::halt_machine(e));
        crate::ipc::destroy_endpoint(old_eid).unwrap_or_else(|e| crate::halt::halt_machine(e));
        crate::cap::issue(
            pid,
            1,
            Cap {
                obj: CapObj::Endpoint { eid },
                rights: W,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
        crate::cap::issue(
            pid,
            4,
            Cap {
                obj: CapObj::ProofToken {
                    id: 0x4152_454e_4149_4e50,
                },
                rights: R | C,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    }
    let was_if = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let deadline = crate::timekeeping::now_us().saturating_add(5_000_000);
    while crate::shared::usage_snapshot().2 < 6 || !crate::ipc::parked_server(eid, comp) {
        if crate::sched::proc_live_threads(comp) == 0
            || crate::sched::proc_live_threads(a) == 0
            || crate::sched::proc_live_threads(b) == 0
            || crate::timekeeping::now_us() >= deadline
        {
            crate::halt::halt_machine("compositor: clients or service died before owned pixels");
        }
        crate::sched::yield_now();
    }
    if !was_if {
        crate::arch::x86_64::cli()
    }
    let (regions, pages, maps) = crate::shared::usage_snapshot();
    info!(
        "m9",
        "compositor: live processes {}; display/compositor/client cap counts {:?}/{:?}/{:?}/{:?}; shared {regions}/{}, pages {pages}/{}, maps {maps}/{}; free frames {}",
        crate::proc::live_count(),
        crate::cap::occupancy(display_pid),
        crate::cap::occupancy(comp),
        crate::cap::occupancy(a),
        crate::cap::occupancy(b),
        crate::shared::MAX_REGIONS,
        crate::shared::TOTAL_PAGES,
        crate::shared::MAX_MAPS,
        crate::frames::free_frames()
    );
    GraphicsRuntime {
        display: display_pid,
        compositor: comp,
        clients: [a, b],
        regions: region_ids,
        exited: [false; 2],
        retired: [false; 2],
        deadline: [0; 2],
        launch_endpoint: None,
    }
}

/// A short-lived *guest* capacity proof, not a kernel-side allocator model.
/// The kernel checks the observed exit code and independently reclaims its
/// page tables, caps, mapping pins, contiguous physical runs and record.
fn run_shared_probe() {
    let frames_before = crate::frames::free_frames();
    let records_before = crate::spawn::records_snapshot()
        .iter()
        .filter(|rec| rec.is_some())
        .count();
    let processes_before = crate::proc::live_count();
    let pool = crate::cap::Cap {
        obj: crate::cap::CapObj::MemoryPool,
        rights: crate::cap::RIGHTS_WRITE,
    };
    // ADR-0058 negative: a one-page *prefix* of the boot fixture's real
    // four-page block BAR is insufficient for the device's notify window
    // in page four. Grant it only to this short-lived probe process;
    // displayd, shell and manager never inherit it. Absent virtio-blk
    // keeps the old one-grant/headless fixture unchanged.
    let truncated = crate::drivers::pci::find_virtio(crate::drivers::pci::VIRTIO_TYPE_BLOCK)
        .and_then(|v| {
            let bar = v.common.bar as usize;
            let f = crate::drivers::pci::pci_function(v.pci_index)?;
            (bar < 6 && !f.bar_is_io[bar] && f.bar_base[bar] != 0 && f.bar_size[bar] >= 2 * 4096)
                .then_some(crate::cap::Cap {
                    obj: crate::cap::CapObj::Mmio {
                        phys: f.bar_base[bar],
                        pages: 1,
                    },
                    rights: crate::cap::RIGHTS_READ,
                })
        });
    let pid = if let Some(short_bar) = truncated {
        crate::spawn::spawn_init_boot(1, &[pool, short_bar], None)
    } else {
        crate::spawn::spawn_init_boot(1, &[pool], None)
    }
    .unwrap_or_else(|reason| crate::halt::halt_machine(reason));
    let tid = crate::spawn::records_snapshot()
        .iter()
        .flatten()
        .find_map(|&(child, thread)| (child == pid).then_some(thread))
        .unwrap_or_else(|| crate::halt::halt_machine("sharedprobe: no spawn thread record"));
    let was_if = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let deadline = crate::timekeeping::now_us().saturating_add(5_000_000);
    while crate::sched::proc_live_threads(pid) != 0 {
        if crate::timekeeping::now_us() >= deadline {
            crate::halt::halt_machine("sharedprobe: guest did not exit by deadline");
        }
        crate::sched::yield_now();
    }
    crate::sched::yield_now();
    if !was_if {
        crate::arch::x86_64::cli();
    }
    let observed = crate::arch::x86_64::syscall::exit_status_of(tid);
    if observed != Some(42) {
        error!("m9", "sharedprobe ring-3 exit code: {observed:?}");
        crate::halt::halt_machine("sharedprobe: ring3 refusal/rights/zero assertion failed");
    }
    crate::proc::destroy(pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
    crate::spawn::forget(pid).unwrap_or_else(|e| crate::halt::halt_machine(e));
    crate::shared::assert_conservation();
    if crate::frames::free_frames() != frames_before
        || crate::spawn::records_snapshot()
            .iter()
            .filter(|rec| rec.is_some())
            .count()
            != records_before
        || crate::proc::live_count() != processes_before
    {
        crate::halt::halt_machine("sharedprobe: teardown not resource-exact");
    }
    info!(
        "m9",
        "SharedRegion guest 512-page zero/authority/capacity refusals; dead-process mapping/cap sweep frame-exact; RESULT PASS (1/1)"
    );
}

/// The fixed kernel trust root for the Phase 8.0 manager. On a machine
/// without BOTH drivers, give it only its image and event channel; it
/// must report OFFLINE and must never receive partial/phantom grants.
/// On a full fixture, all 4 netstackd grants are COPY-able *manager*
/// caps. The manager still may only attenuate them through SYS_SPAWN.
fn spawn_servicemgr(
    fs_eid: u32,
    manager_nid: u32,
    rng_ready_nid: u32,
    manager_restart_nid: u32,
    manager_admin_nid: u32,
    net: Option<u32>,
    rng: Option<u32>,
    rng_diag_nid: u32,
    stack_diag_nid: u32,
) -> Result<
    (
        u64,
        Option<[crate::cap::Cap; 5]>,
        Option<(u32, u32)>,
        Option<(u32, u32)>,
    ),
    &'static str,
> {
    use crate::cap::{Cap, CapObj, RIGHTS_COPY as C, RIGHTS_READ as R, RIGHTS_WRITE as W};
    let root = [
        Cap {
            obj: CapObj::Image { img_id: 17 },
            rights: R,
        },
        Cap {
            obj: CapObj::Notification { nid: manager_nid },
            rights: R | W,
        },
    ];
    let (grants, stack, permission, package) = match (net, rng) {
        (Some(net_eid), Some(rng_eid)) => {
            let stack_eid = crate::ipc::create_endpoint()
                .map_err(|_| "servicemgr: stack endpoint table full")?;
            let backoff_nid = crate::ipc::create_notification()
                .map_err(|_| "servicemgr: backoff notification table full")?;
            let permission_eid =
                crate::ipc::create_endpoint().map_err(|_| "permissiond: endpoint table full")?;
            let approval_nid = crate::ipc::create_notification()
                .map_err(|_| "permissiond: approval marker table full")?;
            let package_eid =
                crate::ipc::create_endpoint().map_err(|_| "packaged: endpoint table full")?;
            let package_nid = crate::ipc::create_notification()
                .map_err(|_| "packaged: separate marker table full")?;
            let lifecycle_nid = crate::ipc::create_notification()
                .map_err(|_| "packaged: lifecycle marker table full")?;
            let all = [
                root[0],
                root[1],
                Cap {
                    obj: CapObj::Endpoint { eid: net_eid },
                    rights: W | C,
                },
                Cap {
                    obj: CapObj::Endpoint { eid: stack_eid },
                    rights: R | C,
                },
                Cap {
                    obj: CapObj::Notification { nid: backoff_nid },
                    rights: R | W | C,
                },
                Cap {
                    obj: CapObj::Endpoint { eid: rng_eid },
                    rights: W | C,
                },
                Cap {
                    obj: CapObj::Notification { nid: rng_ready_nid },
                    rights: R | W,
                },
                Cap {
                    obj: CapObj::Notification {
                        nid: manager_restart_nid,
                    },
                    rights: R | W | C, // worker gets WRITE only; stack never sees this object
                },
                Cap {
                    obj: CapObj::Notification {
                        nid: manager_admin_nid,
                    },
                    rights: R, // not transferable; only shell can request
                },
                Cap {
                    obj: CapObj::Image { img_id: 20 },
                    rights: R,
                },
                Cap {
                    obj: CapObj::Notification { nid: rng_diag_nid },
                    rights: R | C | crate::cap::RIGHTS_DESTROY, // transferable, disposable proof
                },
                Cap {
                    obj: CapObj::Notification {
                        nid: stack_diag_nid,
                    },
                    rights: R | C | crate::cap::RIGHTS_DESTROY, // stack receives READ only
                },
                // ADR-0048 exact five manager-held permission sources.
                Cap {
                    obj: CapObj::Image { img_id: 24 },
                    rights: R,
                },
                Cap {
                    obj: CapObj::Image { img_id: 25 },
                    rights: R,
                },
                Cap {
                    obj: CapObj::Endpoint { eid: fs_eid },
                    rights: W | C,
                },
                Cap {
                    obj: CapObj::Endpoint {
                        eid: permission_eid,
                    },
                    rights: R | W | C,
                },
                Cap {
                    obj: CapObj::Notification { nid: approval_nid },
                    rights: R | C,
                },
                // ADR-0053 exact package sources at manager slots 17..19.
                Cap {
                    obj: CapObj::Image { img_id: 26 },
                    rights: R,
                },
                Cap {
                    obj: CapObj::Endpoint { eid: package_eid },
                    rights: R | W | C,
                },
                Cap {
                    obj: CapObj::Notification { nid: package_nid },
                    rights: R | C,
                },
                // ADR-0055: possession (not pid) is registration authority.
                Cap {
                    obj: CapObj::ImageRegistrar,
                    rights: W | C,
                },
                // Distinct from STAGE; one receiver-verified 8.5 marker.
                Cap {
                    obj: CapObj::Notification { nid: lifecycle_nid },
                    rights: R | C | crate::cap::RIGHTS_DESTROY, // full sender marker; child gets READ only
                },
            ];
            let child = [
                Cap {
                    obj: CapObj::Endpoint { eid: net_eid },
                    rights: W,
                },
                Cap {
                    obj: CapObj::Endpoint { eid: stack_eid },
                    rights: R,
                },
                Cap {
                    obj: CapObj::Notification { nid: backoff_nid },
                    rights: R | W,
                },
                Cap {
                    obj: CapObj::Endpoint { eid: rng_eid },
                    rights: W,
                },
                Cap {
                    obj: CapObj::Notification {
                        nid: stack_diag_nid,
                    },
                    rights: R,
                },
            ];
            (
                Some(all),
                Some((stack_eid, child)),
                Some((permission_eid, approval_nid)),
                Some((package_eid, package_nid)),
            )
        }
        _ => (None, None, None, None),
    };
    let pid = if let Some(all) = grants {
        crate::spawn::spawn_init(19, &all, None)?
    } else {
        crate::spawn::spawn_init(19, &root, None)?
    };
    // Post-install audit: no extra authority and no rights widened by
    // bootstrap. A manifest cannot authorize something not present in
    // this actual process cap table.
    let expected: &[Cap] = if let Some(ref all) = grants {
        all
    } else {
        &root
    };
    for (slot, &wanted) in expected.iter().enumerate() {
        if crate::cap::read(pid, slot).ok() != Some(wanted) {
            return Err("servicemgr: actual boot cap did not match literal grant");
        }
    }
    for slot in expected.len()..crate::cap::CAP_SLOTS {
        if crate::cap::read(pid, slot).is_ok() {
            return Err("servicemgr: unexpected boot cap (authority leak)");
        }
    }
    info!(
        "kernel",
        "servicemgr spawned: pid {pid}, image19; stack endpoint {:?}; audited {} literal caps; no device/Power/Process grants",
        stack.map(|(eid, _)| eid),
        expected.len()
    );
    Ok((pid, stack.map(|(_, child)| child), permission, package))
}

/// True after the manager has spawned one child and all four caps
/// match the kernel's fixed policy. No guessing a Process-cap slot
/// from a pid: inspect actual held manager handles first. Child-owned
/// Untyped frames are allowed; privileged or unexpected grants are not.
enum ManagerChild {
    Production(u64),
    Probe(u64, u8),
}

fn audit_manager_child(
    manager_pid: u64,
    expected: &[crate::cap::Cap; 5],
    private_nid: u32,
) -> Result<Option<ManagerChild>, &'static str> {
    use crate::cap::{CapObj, RIGHTS_DESTROY};
    let mut child = None;
    for slot in 7..crate::cap::CAP_SLOTS {
        if let Ok(c) = crate::cap::read(manager_pid, slot) {
            if let CapObj::Process { pid } = c.obj {
                if c.rights & RIGHTS_DESTROY == 0 {
                    return Err("servicemgr: child Process cap lacks DESTROY");
                }
                // This historical audit selects only the netd-sourced
                // stack or its worker. The manager now also owns a
                // separate broker and app; audit those independently.
                if crate::cap::read(pid, 0)
                    .ok()
                    .is_some_and(|first| first.obj == expected[0].obj)
                    && child.replace(pid).is_some()
                {
                    return Err("servicemgr: stack child Process cap ambiguous");
                }
            }
        }
    }
    let Some(pid) = child else {
        return Ok(None);
    };
    if !crate::spawn::has_record(pid) {
        return Err("servicemgr: child Process cap has no live spawn record");
    }
    let worker = [
        expected[0], // netd Endpoint/WRITE
        expected[3], // rngd Endpoint/WRITE
        crate::cap::Cap {
            obj: CapObj::Notification { nid: private_nid },
            rights: crate::cap::RIGHTS_WRITE,
        },
    ];
    if crate::cap::read(pid, 2).ok() == Some(worker[2]) {
        if !crate::spawn::has_user_child_record(pid) {
            return Err("servicemgr: dependency probe has no user-child record");
        }
        let Some(first) = crate::cap::read(pid, 0).ok() else {
            return Err("servicemgr: probe lacks netd grant");
        };
        if first.obj != worker[0].obj
            || ![worker[0].rights, worker[0].rights | crate::cap::RIGHTS_COPY]
                .contains(&first.rights)
            || !matches!(crate::cap::read(pid, 1).ok(), Some(c) if c.obj == worker[1].obj
                && [worker[1].rights, worker[1].rights | crate::cap::RIGHTS_COPY].contains(&c.rights)
                && !(first.rights & crate::cap::RIGHTS_COPY != 0 && c.rights & crate::cap::RIGHTS_COPY != 0))
        {
            return Err("servicemgr: probe grant differs from root policy");
        }
        let diagnostic = first.rights & crate::cap::RIGHTS_COPY != 0
            || crate::cap::read(pid, 1).ok().unwrap().rights & crate::cap::RIGHTS_COPY != 0;
        if diagnostic {
            let wrong =
                crate::cap::read(pid, 3).map_err(|_| "probe missing wrong-marker fixture")?;
            let correct = crate::cap::read(pid, 4).map_err(|_| "probe missing rngd marker")?;
            let root = crate::cap::read(manager_pid, 10).map_err(|_| "manager lost rngd marker")?;
            if wrong.obj != expected[4].obj
                || wrong.rights
                    != crate::cap::RIGHTS_READ
                        | crate::cap::RIGHTS_COPY
                        | crate::cap::RIGHTS_DESTROY
                || correct != root
                || correct.obj == wrong.obj
            {
                return Err("probe diagnostic authority shape differs from root policy");
            }
        }
        // Later slots are allocated/copied by the live worker itself;
        // inherited authority is bounded by SYS_SPAWN, not by a
        // snapshot taken during the worker's own negative-space calls.
        let mode = if first.rights & crate::cap::RIGHTS_COPY != 0 {
            1
        } else if crate::cap::read(pid, 1).ok().unwrap().rights & crate::cap::RIGHTS_COPY != 0 {
            2
        } else {
            0
        };
        return Ok(Some(ManagerChild::Probe(pid, mode)));
    }
    for (slot, &want) in expected.iter().enumerate() {
        if crate::cap::read(pid, slot).ok() != Some(want) {
            return Err("servicemgr: actual child cap differs from attenuated policy");
        }
    }
    // SYS_SPAWN accepts at most five inherited references. Later slots
    // may contain caller-controlled IPC landings (including malformed
    // references); treating them as bootstrap grants lets an ordinary
    // client halt the entire kernel by sending a cap during an audit.
    Ok(Some(ManagerChild::Production(pid)))
}

/// Spawn the production block service (M5.2, ADR-0022): registry image 2
/// with kernel-literal grants in slot order — the device window (Mmio,
/// READ|WRITE: the handshake writes registers), the endpoint's serve
/// side (READ), and its interrupt notification (READ|WRITE — the relay
/// target `SYS_IRQ_RELAY` arms). The virtio record comes from the boot
/// PCI scan; the driver re-discovers the structure offsets itself
/// through `SYS_DEV_INFO` (config space is never exposed to ring 3).
fn spawn_storaged() -> Result<(u64, u32), &'static str> {
    let v = crate::drivers::pci::find_virtio(crate::drivers::pci::VIRTIO_TYPE_BLOCK)
        .ok_or("storaged: no virtio-block function on bus 0")?;
    let f = crate::drivers::pci::pci_function(v.pci_index)
        .ok_or("storaged: recorded function vanished from the table")?;
    let bar = v.common.bar as usize;
    if bar > 5 || f.bar_is_io[bar] || f.bar_base[bar] == 0 || f.bar_size[bar] < 4096 {
        return Err("storaged: the virtio structure BAR is unusable");
    }
    let eid = crate::ipc::create_endpoint().map_err(|_| "storaged: endpoint table full")?;
    let nid = crate::ipc::create_notification().map_err(|_| "storaged: notification table full")?;
    let grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Mmio {
                phys: f.bar_base[bar],
                pages: (f.bar_size[bar] / 4096) as u32,
            },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_READ,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
    ];
    let pid = crate::spawn::spawn_init(2, &grants, None)?;
    info!(
        "kernel",
        "storaged spawned: pid {pid} (caps: 0=Mmio bar{bar} phys {:#x} RW, 1=Endpoint{eid}/R, 2=Notif{nid}/RW) — the block service is live; fsd is its first production client",
        f.bar_base[bar]
    );
    Ok((pid, eid))
}

/// Spawn the 8.1 resident config reader with no delegated updater.
fn spawn_configd(fs_eid: u32) -> Result<(u32, u32), &'static str> {
    use crate::cap::{Cap, CapObj, RIGHTS_READ as R, RIGHTS_WRITE as W};
    let eid = crate::ipc::create_endpoint().map_err(|_| "configd: endpoint table full")?;
    let nid = crate::ipc::create_notification().map_err(|_| "configd: marker table full")?;
    let grants = [
        Cap {
            obj: CapObj::Endpoint { eid: fs_eid },
            rights: W,
        },
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: R,
        },
        Cap {
            obj: CapObj::Notification { nid },
            rights: R,
        },
    ];
    let pid = crate::spawn::spawn_init(21, &grants, None)?;
    for (slot, &wanted) in grants.iter().enumerate() {
        if crate::cap::read(pid, slot).ok() != Some(wanted) {
            return Err("configd: boot grant did not match literal policy");
        }
    }
    info!(
        "kernel",
        "configd spawned: pid {pid}, image21, FS/W + cfg/R + marker/R anchor; updater separately granted"
    );
    Ok((eid, nid))
}

/// Spawn the production filesystem service (M5.3, ADR-0023): registry
/// image 4 with kernel-literal grants in slot order — the block
/// endpoint's CALL side (WRITE: every fsd disk operation is a
/// forwarded block call) and its own FS endpoint's serve side (READ).
/// Returns the fsd pid and its FS endpoint id (the shell gets the call
/// side). No Mmio, no notification: fsd never sees the device.
fn spawn_fsd(blk_eid: u32) -> Result<(u64, u32, u32), &'static str> {
    let eid = crate::ipc::create_endpoint().map_err(|_| "fsd: endpoint table full")?;
    let diag_nid =
        crate::ipc::create_notification().map_err(|_| "fsd: diagnostic marker table full")?;
    let grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid: blk_eid },
            rights: crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_READ,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid: diag_nid },
            rights: crate::cap::RIGHTS_READ,
        },
    ];
    let pid = crate::spawn::spawn_init(4, &grants, None)?;
    info!(
        "kernel",
        "fsd spawned: pid {pid} (caps: 0=Endpoint{blk_eid}/W 1=Endpoint{eid}/R 2=Notif{diag_nid}/R) — AFS1 mount and serve from ring 3"
    );
    Ok((pid, eid, diag_nid))
}

/// filesd's user-root badge (`filesd_wire::USER_ROOT_BADGE`): record 1,
/// record generation 1.
const FILESD_USER_ROOT_BADGE: u32 = 1 | 1 << 16;
/// ADR-0091 filesd-reserved APB1 install record 2/generation 1.
const FILESD_APB1_INSTALL_BADGE: u32 = 2 | 1 << 16;
/// Reserved high manager slot for late authority, outside its low32 inventory.
const MGR_SLOT_APB1_INSTALL_AUTH: usize = 127;
const _: () = assert!(MGR_SLOT_APB1_INSTALL_AUTH == crate::cap::CAP_SLOTS - 1);
/// Wake hint only: servicemgr rechecks actual slot-127 capability possession.
const MGR_BADGE_APB1_INSTALL_AUTH: u64 = 1 << 22;

/// Spawn the AFS2 file service (Phase 11.5, ADR-0076/0077): boot image 9
/// with storaged's block endpoint (call side), its own endpoint (serve
/// side), the read-only RTC and fsd's endpoint (call side, read-only use:
/// the one-shot AFS1 import). Returns its endpoint.
fn spawn_filesd(blk_eid: u32, fs_eid: u32) -> Result<u32, &'static str> {
    use crate::cap::{Cap, CapObj, RIGHTS_READ as R, RIGHTS_WRITE as W};
    let eid = crate::ipc::create_endpoint().map_err(|_| "filesd: endpoint table full")?;
    let grants = [
        Cap {
            obj: CapObj::Endpoint { eid: blk_eid },
            rights: W,
        },
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: R,
        },
        Cap {
            obj: CapObj::Rtc,
            rights: R,
        },
        Cap {
            obj: CapObj::Endpoint { eid: fs_eid },
            rights: W,
        },
    ];
    let pid = crate::spawn::spawn_init_boot(9, &grants, None)?;
    info!(
        "kernel",
        "filesd spawned: pid {pid} (caps: 0=Endpoint{blk_eid}/W 1=Endpoint{eid}/R 2=Rtc/R 3=Endpoint{fs_eid}/W) — AFS2 file service"
    );
    Ok(eid)
}

/// Spawn the production network service (M6.1, ADR-0024): registry
/// image 6 with storaged's three-cap driver shape — an `Mmio` cap over
/// the virtio-net structure BAR (R|W), its endpoint serve side (READ),
/// and interrupt notification (READ|WRITE, both MSI-X relays) — plus a
/// fourth, WRITE-only manager-readiness notification (ADR-0038). Returns
/// `Ok(None)` when bus 0 carries no virtio-net function: the network
/// service is optional until Phase 7 (an honest SKIP, never a fake
/// init — the caller logs the absence).
fn spawn_netd(manager_nid: u32) -> Result<Option<(u64, u32)>, &'static str> {
    let Some(v) = crate::drivers::pci::find_virtio(crate::drivers::pci::VIRTIO_TYPE_NET) else {
        return Ok(None);
    };
    let f = crate::drivers::pci::pci_function(v.pci_index)
        .ok_or("netd: recorded function vanished from the table")?;
    let bar = v.common.bar as usize;
    if bar > 5 || f.bar_is_io[bar] || f.bar_base[bar] == 0 || f.bar_size[bar] < 4096 {
        return Err("netd: the virtio structure BAR is unusable");
    }
    let eid = crate::ipc::create_endpoint().map_err(|_| "netd: endpoint table full")?;
    let nid = crate::ipc::create_notification().map_err(|_| "netd: notification table full")?;
    let grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Mmio {
                phys: f.bar_base[bar],
                pages: (f.bar_size[bar] / 4096) as u32,
            },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_READ,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        // Production only: a WRITE-only readiness signal to the
        // manager after this driver reaches DRIVER_OK. The kernel
        // supervisor replays it if the driver is restarted.
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid: manager_nid },
            rights: crate::cap::RIGHTS_WRITE,
        },
    ];
    let pid = crate::spawn::spawn_init(6, &grants, None)?;
    crate::supervise::register("netd", 6, &grants, pid)
        .map_err(|_| "netd: production supervisor registration refused")?;
    info!(
        "kernel",
        "netd spawned: pid {pid} (caps: 0=Mmio bar{bar} phys {:#x} RW, 1=Endpoint{eid}/R, 2=Notif{nid}/RW, 3=Notif{manager_nid}/W) — kernel-owned link-layer driver; signals manager when DRIVER_OK",
        f.bar_base[bar]
    );
    Ok(Some((pid, eid)))
}

/// Spawn the production entropy service (M6.2, ADR-0025): registry
/// image 8 with the same first three driver grants: Mmio/RW, serve
/// Endpoint/READ and IRQ Notification/RW. The fourth, WRITE-only cap
/// names a *different* manager-readiness notification (ADR-0038).
/// Returns `Ok(None)`
/// when bus 0 carries no virtio-rng function: the entropy service is
/// optional (an honest offline note, never a fake init — the caller
/// logs the absence).
fn spawn_rngd(rng_ready_nid: u32, rng_diag_nid: u32) -> Result<Option<(u64, u32)>, &'static str> {
    let Some(v) = crate::drivers::pci::find_virtio(crate::drivers::pci::VIRTIO_TYPE_ENTROPY) else {
        return Ok(None);
    };
    let f = crate::drivers::pci::pci_function(v.pci_index)
        .ok_or("rngd: recorded function vanished from the table")?;
    let bar = v.common.bar as usize;
    if bar > 5 || f.bar_is_io[bar] || f.bar_base[bar] == 0 || f.bar_size[bar] < 4096 {
        return Err("rngd: the virtio structure BAR is unusable");
    }
    let eid = crate::ipc::create_endpoint().map_err(|_| "rngd: endpoint table full")?;
    let nid = crate::ipc::create_notification().map_err(|_| "rngd: notification table full")?;
    let grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Mmio {
                phys: f.bar_base[bar],
                pages: (f.bar_size[bar] / 4096) as u32,
            },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_READ,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        // Production only: a WRITE-only readiness signal to the
        // manager after this driver reaches DRIVER_OK. The kernel
        // supervisor replays it if the driver is restarted.
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid: rng_ready_nid },
            rights: crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid: rng_diag_nid },
            rights: crate::cap::RIGHTS_READ,
        },
    ];
    let pid = crate::spawn::spawn_init(8, &grants, None)?;
    crate::supervise::register("rngd", 8, &grants, pid)
        .map_err(|_| "rngd: production supervisor registration refused")?;
    info!(
        "kernel",
        "rngd spawned: pid {pid} (caps: 0=Mmio bar{bar} phys {:#x} RW, 1=Endpoint{eid}/R, 2=Notif{nid}/RW, 3=Notif{rng_ready_nid}/W) — kernel-owned entropy driver; signals manager when DRIVER_OK",
        f.bar_base[bar]
    );
    Ok(Some((pid, eid)))
}

/// Spawn the production input service (M6.3, ADR-0026): registry image
/// 10 with the three-cap driver shape every service gets — an `Mmio`
/// cap over the virtio-input structure BAR (R|W), its own endpoint's
/// serve side (READ), and an interrupt notification (READ|WRITE) —
/// plus a FOURTH grant no other driver holds: `ConsoleInput` (WRITE),
/// the singleton authority to inject bytes into the console's line
/// discipline. That cap is both the permission and the mode switch:
/// inputd probes for it with a zero-length `SYS_CONSOLE_PUSH` and,
/// finding it, runs as the console feeder instead of an IPC service.
/// Exactly ONE process ever receives it — this one.
///
/// Returns `Ok(None)` when bus 0 carries no virtio-input function: the
/// keyboard is optional (an honest offline note, never a fake init),
/// and the serial console keeps the machine fully usable without it.
fn spawn_inputd() -> Result<Option<u64>, &'static str> {
    let Some(v) = crate::drivers::pci::find_virtio(crate::drivers::pci::VIRTIO_TYPE_INPUT) else {
        return Ok(None);
    };
    let f = crate::drivers::pci::pci_function(v.pci_index)
        .ok_or("inputd: recorded function vanished from the table")?;
    let bar = v.common.bar as usize;
    if bar > 5 || f.bar_is_io[bar] || f.bar_base[bar] == 0 || f.bar_size[bar] < 4096 {
        return Err("inputd: the virtio structure BAR is unusable");
    }
    let eid = crate::ipc::create_endpoint().map_err(|_| "inputd: endpoint table full")?;
    let nid = crate::ipc::create_notification().map_err(|_| "inputd: notification table full")?;
    let grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Mmio {
                phys: f.bar_base[bar],
                pages: (f.bar_size[bar] / 4096) as u32,
            },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_READ,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::ConsoleInput,
            rights: crate::cap::RIGHTS_WRITE,
        },
    ];
    let pid = crate::spawn::spawn_init(10, &grants, None)?;
    // A single input service owns both queues and consumes their common
    // IRQ notification. The second MMIO cap never reaches applications.
    if let Some(second) = (0..crate::drivers::pci::virtio_count())
        .filter_map(crate::drivers::pci::virtio_device)
        .find(|d| {
            d.virtio_type == crate::drivers::pci::VIRTIO_TYPE_INPUT && d.pci_index != v.pci_index
        })
    {
        let sf = crate::drivers::pci::pci_function(second.pci_index)
            .ok_or("inputd: second function vanished")?;
        let sb = second.common.bar as usize;
        if sb > 5 || sf.bar_is_io[sb] || sf.bar_base[sb] == 0 || sf.bar_size[sb] < 4096 {
            return Err("inputd: second BAR invalid");
        }
        crate::cap::issue(
            pid,
            5,
            crate::cap::Cap {
                obj: crate::cap::CapObj::Mmio {
                    phys: sf.bar_base[sb],
                    pages: (sf.bar_size[sb] / 4096) as u32,
                },
                rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
            },
        )?;
    }
    info!(
        "kernel",
        "inputd spawned: pid {pid} (caps: 0=Mmio bar{bar} phys {:#x} RW, 1=Endpoint{eid}/R, 2=Notif{nid}/RW, 3=ConsoleInput/W) — the keyboard is live; keystrokes feed the shell's line discipline beside the serial port",
        f.bar_base[bar]
    );
    Ok(Some(pid))
}

/// Spawn the production console channel service (M6.4, ADR-0027).
///
/// consoled receives the driver shape every virtio service gets — an
/// `Mmio` cap over the structure BAR (R|W), an endpoint's serve side
/// (READ), an interrupt notification (READ|WRITE) — plus BOTH console
/// singletons: `ConsoleInput` (WRITE) and `ConsoleOutput` (READ).
///
/// The pair is the point. One alone would be half a console: input
/// without output is a keyboard with no screen, output without input
/// is a log tap. Granting them separately is also what keeps them
/// honest as SEPARATE authorities — inputd holds the first and never
/// the second, so a keyboard driver cannot read everything the machine
/// prints. Exactly one process ever holds both: this one.
///
/// Returns `Ok(None)` when bus 0 carries no virtio-console function.
fn spawn_consoled() -> Result<Option<u64>, &'static str> {
    let Some(v) = crate::drivers::pci::find_virtio(crate::drivers::pci::VIRTIO_TYPE_CONSOLE) else {
        return Ok(None);
    };
    let f = crate::drivers::pci::pci_function(v.pci_index)
        .ok_or("consoled: recorded function vanished from the table")?;
    let bar = v.common.bar as usize;
    if bar > 5 || f.bar_is_io[bar] || f.bar_base[bar] == 0 || f.bar_size[bar] < 4096 {
        return Err("consoled: the virtio structure BAR is unusable");
    }
    let eid = crate::ipc::create_endpoint().map_err(|_| "consoled: endpoint table full")?;
    let nid = crate::ipc::create_notification().map_err(|_| "consoled: notification table full")?;
    let grants = [
        crate::cap::Cap {
            obj: crate::cap::CapObj::Mmio {
                phys: f.bar_base[bar],
                pages: (f.bar_size[bar] / 4096) as u32,
            },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_READ,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::Notification { nid },
            rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::ConsoleInput,
            rights: crate::cap::RIGHTS_WRITE,
        },
        crate::cap::Cap {
            obj: crate::cap::CapObj::ConsoleOutput,
            rights: crate::cap::RIGHTS_READ,
        },
    ];
    let pid = crate::spawn::spawn_init(12, &grants, None)?;
    info!(
        "kernel",
        "consoled spawned: pid {pid} (caps: 0=Mmio bar{bar} phys {:#x} RW, 1=Endpoint{eid}/R, 2=Notif{nid}/RW, 3=ConsoleInput/W, 4=ConsoleOutput/R) — the port is a second console; what the machine prints goes there too, and what you type there reaches the shell",
        f.bar_base[bar]
    );
    Ok(Some(pid))
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

/// Desktop's explicit broker clock slots preserve the Phase-10 layout
/// (7..=12 and 14..=19), with the filesd endpoint at 13 and the `/Users/user`
/// lineage at 20. Slots 30..=31 stay open for the first-free display IPC
/// reply cap; additional Phase-12 clocks use 21..=29 and 32..=42.
const fn desktop_clock_slot(index: usize) -> usize {
    if index < 6 {
        7 + index
    } else if index < 12 {
        8 + index
    } else if index < 21 {
        9 + index
    } else {
        11 + index
    }
}

/// Ordinary userspace broker/compositor: kernel policy delegates explicit
/// image, pool, endpoint and clock authorities; window policy remains ring 3.
fn start_boot_desktop(
    display: u64,
    display_eid: u32,
    input_pid: Option<u64>,
    frame_nid: u32,
    app_clocks: &[u32; 32],
    fs_eid: u32,
    filesd_eid: Option<u32>,
    package_eid: Option<u32>,
) -> GraphicsRuntime {
    use crate::cap::{Cap, CapObj, RIGHTS_COPY as C, RIGHTS_READ as R, RIGHTS_WRITE as W};
    let eid = crate::ipc::create_endpoint().unwrap_or_else(|e| crate::halt::halt_machine(e));
    let token = CapObj::ProofToken {
        id: 0x4152_454e_5031_3049,
    };
    let comp = crate::spawn::spawn_init_boot(
        6,
        &[
            Cap {
                obj: CapObj::Endpoint { eid: display_eid },
                rights: W,
            },
            Cap {
                obj: CapObj::Endpoint { eid },
                rights: R,
            },
            Cap {
                obj: token,
                rights: R,
            },
            Cap {
                obj: CapObj::MemoryPool,
                rights: W,
            },
            // COPY: the broker lends its clock to filesd for the Desktop
            // directory watch (ADR-0079).
            Cap {
                obj: CapObj::Notification { nid: frame_nid },
                rights: R | W | C,
            },
            Cap {
                obj: CapObj::Endpoint { eid },
                rights: W | C,
            },
            Cap {
                obj: CapObj::BootImage { index: 8 },
                rights: R,
            },
        ],
        None,
    )
    .unwrap_or_else(|e| crate::halt::halt_machine(e));
    // Client clocks occupy 7..=12 and 14..=19, then the extension range
    // 21..=29 and 32..=42. Slots 13 (filesystem endpoint), 20 (filesd
    // lineage), and 30..=31 (first-free display reply window) stay fixed;
    // the arithmetic is shared with Desktop's clock(i) mapping.
    for (i, nid) in app_clocks.iter().enumerate() {
        crate::cap::issue(
            comp,
            desktop_clock_slot(i),
            Cap {
                obj: CapObj::Notification { nid: *nid },
                rights: R | W | C,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    }
    crate::cap::issue(
        comp,
        13,
        Cap {
            obj: CapObj::Endpoint { eid: fs_eid },
            rights: W,
        },
    )
    .unwrap_or_else(|e| crate::halt::halt_machine(e));
    // Slot 20: the broker's file grant — a badged capability naming the
    // filesd record of `/Users/user` (all rights). `/System` has no badge
    // anyone holds.
    if let Some(eid) = filesd_eid {
        let generation = crate::ipc::endpoint_generation(eid)
            .unwrap_or_else(|| crate::halt::halt_machine("filesd endpoint vanished"));
        crate::cap::issue(
            comp,
            20,
            Cap {
                obj: CapObj::BadgedEndpoint {
                    eid: eid as u16,
                    generation,
                    badge: FILESD_USER_ROOT_BADGE,
                },
                rights: W | C,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    }
    // ADR-0091: the trusted Desktop broker alone receives the package
    // receiver endpoint. It can submit one exact filesd File cap, but receives
    // no APKG marker or filesd install badge.
    if let Some(eid) = package_eid {
        crate::cap::issue(
            comp,
            43,
            Cap {
                obj: CapObj::Endpoint { eid },
                rights: W,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    }
    crate::cap::consume(comp, 3).unwrap_or_else(|e| crate::halt::halt_machine(e));
    crate::cap::issue(
        comp,
        3,
        Cap {
            obj: CapObj::MemoryPool,
            rights: R | W | C,
        },
    )
    .unwrap_or_else(|e| crate::halt::halt_machine(e));
    if let Some(pid) = input_pid {
        let old = crate::cap::read(pid, 1).unwrap_or_else(|e| crate::halt::halt_machine(e));
        let CapObj::Endpoint { eid: old_eid } = old.obj else {
            crate::halt::halt_machine("desktop input endpoint invalid")
        };
        crate::cap::consume(pid, 1).unwrap_or_else(|e| crate::halt::halt_machine(e));
        crate::ipc::destroy_endpoint(old_eid).unwrap_or_else(|e| crate::halt::halt_machine(e));
        crate::cap::issue(
            pid,
            1,
            Cap {
                obj: CapObj::Endpoint { eid },
                rights: W,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
        crate::cap::issue(
            pid,
            4,
            Cap {
                obj: token,
                rights: R | C,
            },
        )
        .unwrap_or_else(|e| crate::halt::halt_machine(e));
    }
    info!(
        "m10",
        "desktop broker spawned with explicit delegated grants; application Process/region ownership remains userspace"
    );
    GraphicsRuntime {
        display,
        compositor: comp,
        clients: [0; 2],
        regions: [0; 2],
        exited: [false; 2],
        retired: [false; 2],
        deadline: [0; 2],
        launch_endpoint: Some(eid),
    }
}
