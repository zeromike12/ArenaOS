//! The syscall boundary (M3.3, ADR-0014): `syscall`/`sysret` MSRs, the
//! entry stub, the dispatcher, and first entry into ring 3.
//!
//! Entry contract (all decided in ADR-0014):
//!
//! * `syscall` arrives at [`arena_syscall_entry`] with RCX = user RIP and
//!   R11 = user RFLAGS; SFMASK has already cleared TF/DF/IF/NT/AC, so the
//!   whole kernel-side path runs at IF=0 — a syscall is never preempted
//!   mid-handler, which keeps the per-CPU scratch and TSS RSP0
//!   single-valued.
//! * The stub finds the kernel stack via `swapgs` + the per-CPU
//!   [`CpuScratch`] (offset 0 = the stashed user RSP, offset 8 = this
//!   thread's kernel stack top — maintained by the scheduler *together
//!   with* RSP0, so the two never disagree).
//! * The dispatcher returns the user-visible result in RAX; the stub
//!   tears the frame down and `sysretq`s to the *recorded* RIP/RFLAGS —
//!   user code never supplies a return target (the non-canonical-RCX
//!   `sysretq` fault class is closed by construction).
//! * First entry into ring 3 is [`enter_user`]: an `iretq` frame with the
//!   user selectors and RFLAGS IF=1 — user code is interruptible, and
//!   therefore preemptible, from its first instruction. Interrupts that
//!   land in ring 3 take TSS RSP0 (hardware path), not the scratch.
//!
//! The syscall ABI (v1, ADR-0017 — ours, deliberately not POSIX): RAX =
//! call number, RDI/RSI/RDX/R10/R8/R9 = up to six arguments, result in
//! RAX as a typed `i64` status (0 = OK, positive = call-specific success
//! payload, negative = dense typed error); RBX/RBP/R12–R15 are preserved
//! across the call, RCX/R11 are consumed by the hardware, and the user's
//! RFLAGS is restored on return. The v1 registry: `SYS_DEBUG_WRITE`
//! (validated, SMAP-aware copy-out to the serial console — the temporary
//! diagnostics backdoor until a console service exists),
//! `SYS_THREAD_EXIT` (terminate through the scheduler), the two ring-3
//! *proof* calls the M3 suite uses (`SYS_PROVE_RING3` arms a #GP
//! expectation at the user's next instruction — a privileged `cli` the
//! payload then executes faults at CPL 3 and resumes, which cannot
//! happen at CPL 0; `SYS_PROVE_DONE` records whether that fault was
//! actually observed), and `SYS_ABI_ECHO6` (fingerprint of all six
//! received argument registers — the stub's marshalling proof). IPC v1
//! (M4.4, ADR-0018) adds `SYS_IPC_CALL`/`SYS_IPC_RECV`/`SYS_IPC_REPLY`
//! (endpoint rendezvous: two-word messages, one transferred capability,
//! blocking call/reply) and `SYS_NOTIFY`/`SYS_WAIT` (badged, merged
//! notification flags). IPC v1.1 (M5.3, ADR-0023) extends those three
//! calls with an OPTIONAL trailing 64-byte inline message buffer
//! (`CALL` a5 in/out, `RECV` a2 out, `REPLY` a4 in); a NULL pointer
//! keeps exact v1.0 behavior. Every non-null buffer is range-checked
//! against the calling process's regions and touched ONLY in its
//! owner's context (ADR-0018 discipline), under STAC. The spawn protocol (M4.5, ADR-0019) adds
//! `SYS_SPAWN`: build a process from an image capability, hand the
//! child an explicit, attenuated inheritance list, register an exit
//! badge, and start its first thread at the image entry. The console
//! and shell step (M4.6, ADR-0020) adds `SYS_CONSOLE_READ` (blocking
//! line read from the kernel's console input service), `SYS_PROC_LIST`
//! (the live process table's public shape, for `ps`), and
//! `SYS_SHUTDOWN` (machine halt, gated on a Power capability). The
//! driver substrate (M5.1, ADR-0021) adds `SYS_ALLOC_FRAME` (the
//! allocator mints an OWNED untyped-frame capability) and
//! `SYS_MAP_MEMORY` (self-map a memory cap at a kernel-chosen VA —
//! windows join the calling thread's registered region table).

use super::gdt;
use core::arch::global_asm;

use crate::log::{log_error as error, log_info as info};
use crate::sync::SyncCell;

// ---- MSR addresses (SDM Vol. 4) -------------------------------------------

pub const MSR_EFER: u32 = 0xC000_0080;
pub const MSR_STAR: u32 = 0xC000_0081;
pub const MSR_LSTAR: u32 = 0xC000_0082;
pub const MSR_SFMASK: u32 = 0xC000_0084;
pub const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;

/// STAR's user-selector base: `sysretq` computes SS = base+8|RPL3
/// (USER_DATA at 0x28) and CS = base+16|RPL3 (USER_CODE at 0x30). The
/// base value itself (0x20 — the TSS descriptor's high qword) is never
/// loaded as a selector (ADR-0014, GDT layout rationale in gdt.rs).
const STAR_USER_BASE: u64 = 0x20;

/// Flags SFMASK clears on `syscall` entry: TF|DF|IF|NT|AC. IF=0 makes the
/// handler non-preemptible; AC=0 closes the SMAP-bypass-via-EFLAGS hole
/// (user could otherwise set AC with `pushf/popf`? — it cannot at CPL 3,
/// but a stale AC from kernel paths must not leak into the handler).
const SFMASK_VALUE: u64 = (1 << 10) | // DF
    (1 << 9) | //  IF
    (1 << 8) | //  TF
    (1 << 14) | // NT
    (1 << 18); //  AC

/// RFLAGS image for first entry into ring 3: reserved-one + IF — user
/// code runs interruptible (ticks land via TSS RSP0; preemption reaches
/// ring 3 exactly like ring 0 — the M3 suite proves both).
const USER_RFLAGS: u64 = 0x202;

// ---- call numbers (ABI v1 registry — ADR-0017; frozen: additive only) -----

pub const SYS_DEBUG_WRITE: u64 = 1;
pub const SYS_THREAD_EXIT: u64 = 2;
// 3 is deliberately unallocated — v0's gap stays a gap forever (ADR-0017).
pub const SYS_PROVE_RING3: u64 = 4;
pub const SYS_PROVE_DONE: u64 = 5;
pub const SYS_ABI_ECHO6: u64 = 6;
// IPC v1 (M4.4, ADR-0018): endpoint rendezvous and notifications.
pub const SYS_IPC_CALL: u64 = 7;
pub const SYS_IPC_RECV: u64 = 8;
pub const SYS_IPC_REPLY: u64 = 9;
pub const SYS_NOTIFY: u64 = 10;
pub const SYS_WAIT: u64 = 11;
// Spawn protocol v1 (M4.5, ADR-0019): create a process from an image
// capability with explicit handle inheritance and an exit notification.
pub const SYS_SPAWN: u64 = 12;
// Console + shell v1 (M4.6, ADR-0020): blocking console line input,
// the live process table for `ps`, and the Power-gated machine halt.
pub const SYS_CONSOLE_READ: u64 = 13;
pub const SYS_PROC_LIST: u64 = 14;
pub const SYS_SHUTDOWN: u64 = 15;
// Driver substrate v1 (M5.1, ADR-0021): owned frames for DMA-capable
// userspace drivers and self-map windows at kernel-chosen VAs.
pub const SYS_ALLOC_FRAME: u64 = 16;
pub const SYS_MAP_MEMORY: u64 = 17;
// Block-service substrate (M5.2, ADR-0022): device-interrupt relays,
// zero-copy buffer references (phys query / discard / attenuated copy),
// and the resolved virtio record of a granted device.
pub const SYS_IRQ_RELAY: u64 = 18;
pub const SYS_CAP_PHYS: u64 = 19;
pub const SYS_CAP_DESTROY: u64 = 20;
pub const SYS_CAP_COPY: u64 = 21;
pub const SYS_DEV_INFO: u64 = 22;
// Input substrate (M6.3, ADR-0026): a capability-gated injection point
// into the console's line discipline, so a ring-3 keyboard driver
// feeds the SAME queue the UART RX ISR does — one line editor, one
// blocking read contract, two hardware sources.
pub const SYS_CONSOLE_PUSH: u64 = 23;
/// SYS_CONSOLE_ATTACH (M6.4, ADR-0027): bind a notification to the
/// console's output mirror, turning it on.
pub const SYS_CONSOLE_ATTACH: u64 = 24;
/// SYS_CONSOLE_PULL (M6.4, ADR-0027): drain mirrored console output.
pub const SYS_CONSOLE_PULL: u64 = 25;
/// SYS_CLOCK_NOW (M7.0, ADR-0029): monotonic microseconds.
pub const SYS_CLOCK_NOW: u64 = 26;
/// SYS_TIMER_ARM (M7.0, ADR-0029): deliver a badge after a delay.
pub const SYS_TIMER_ARM: u64 = 27;
/// SYS_TIMER_CANCEL (M7.0, ADR-0029).
pub const SYS_TIMER_CANCEL: u64 = 28;
// Phase 8.0: inspect only an authority already held in this process,
// and finish only a Process cap with DESTROY (ADR-0037).
pub const SYS_CAP_DESCRIBE: u64 = 29;
pub const SYS_PROC_FINISH: u64 = 30;
/// ADR-0041: read-only kernel accounting, held Power/WRITE only.
pub const SYS_RESOURCE_SNAPSHOT: u64 = 31;
/// Nonblocking take of a held Notification/READ's pending badge.
pub const SYS_TRY_WAIT: u64 = 32;
/// ADR-0055: possession-gated volatile Image registry (additive ABI v1).
pub const SYS_IMAGE_REGISTER: u64 = 33;
pub const SYS_IMAGE_REVOKE: u64 = 34;
/// ADR-0056: information only for a held exact GOP framebuffer Mmio/READ.
pub const SYS_DISPLAY_INFO: u64 = 35;
/// ADR-0056: bounded generic RAM sharing; all operations are cap-gated.
pub const SYS_SHARED_CREATE: u64 = 36;
pub const SYS_SHARED_MAP: u64 = 37;
pub const SYS_SHARED_PHYS: u64 = 38;
/// ADR-0057: read-only size/generation of a *held* generic SharedRegion.
pub const SYS_SHARED_INFO: u64 = 39;
pub const SYS_SHARED_UNMAP: u64 = 40;
/// ADR-0060: only a held Process/READ witness may inspect thread liveness.
pub const SYS_PROC_LIVE: u64 = 41;
pub const SYS_IPC_TRY_RECV: u64 = 42;
pub const SYS_IPC_REPLY_CHECKED: u64 = 43;
pub const SYS_OBSERVE: u64 = 44;
pub const SYS_SPAWN_CHECK: u64 = 45;
/// Phase 11.0 (ADR-0071): bind a notification to an endpoint's serve side.
pub const SYS_ENDPOINT_BIND: u64 = 46;
pub const SYS_ENDPOINT_UNBIND: u64 = 47;
/// Phase 11.2 (ADR-0074): mint a badged client endpoint cap (serve side).
pub const SYS_ENDPOINT_MINT: u64 = 48;
/// Phase 11.2 (ADR-0074): receive (blocking or not) returning the badge.
pub const SYS_IPC_RECV_BADGED: u64 = 49;

/// Largest `SYS_DEBUG_WRITE` the dispatcher accepts (bytes). The console
/// is a diagnostic surface; a real byte-stream API arrives with the FS
/// milestone.
const WRITE_MAX: u64 = 256;

/// IPC message buffers are exactly three u64 words — [w0, w1, cap-slot]
/// — in both directions (ADR-0018). x86-64 tolerates the unaligned word
/// stores, so the ABI imposes no buffer alignment.
const IPC_BUF_BYTES: u64 = 24;

/// ABI v1 typed status (ADR-0017): `0` plain OK, positive a
/// call-specific success payload, negative a typed error — dense from
/// `-1`, allocated once, never reused, never renumbered.
pub type Status = i64;
pub const STATUS_OK: Status = 0;
pub const STATUS_BAD_CALL: Status = -1;
pub const STATUS_BAD_ARG: Status = -2;
pub const STATUS_BAD_ADDRESS: Status = -3;
/// IPC v1 (ADR-0018): a bounded object refused rather than blocked —
/// the endpoint's caller queue is full, a second server parked on one
/// endpoint, or a second waiter parked on one notification.
pub const STATUS_BUSY: Status = -4;

/// M6.5 (ADR-0028): the service you called is GONE — its process was
/// destroyed while your request was queued or in its hands.
///
/// This exists so that a dead driver is an ANSWER rather than a hang.
/// A client blocked in `SYS_IPC_CALL` has no timeout to fall back on
/// and no way to observe the server's liveness; without this it waits
/// forever on a reply nobody will ever stage. The endpoint itself
/// survives (the kernel owns it), so a client that handles this status
/// may simply call again once the service is back — which is exactly
/// what a supervised restart makes possible.
///
/// It says nothing about whether the request was performed. A client
/// whose operation is not idempotent must treat it as "unknown", not
/// as "did not happen" — the ADR is explicit that the kernel cannot
/// know, and a status that pretended otherwise would be a lie.
pub const STATUS_SERVICE_GONE: Status = -5;
pub const STATUS_CALLER_GONE: Status = -6;
/// Phase 11.0 (ADR-0071): a per-process bound refused the request (for
/// example a fifth armed timer) although the shared table may have room.
pub const STATUS_QUOTA: Status = -7;

/// `SYS_ABI_ECHO6`'s mix of the six received arguments (call 6). Public
/// so the m4 suite computes its expectation with the very function the
/// dispatcher returns — the six-register marshalling proof shares no
/// duplicated arithmetic. The top bit is masked off: EVERY call result
/// honors the status sign domain (ADR-0017) — a success payload with
/// the MSB set would be indistinguishable from a typed error.
pub fn echo6_fingerprint(a: [u64; 6]) -> u64 {
    (a[0]
        ^ a[1].rotate_left(11)
        ^ a[2].rotate_left(22)
        ^ a[3].rotate_left(33)
        ^ a[4].rotate_left(44)
        ^ a[5].rotate_left(55))
        & 0x7FFF_FFFF_FFFF_FFFF
}

// ---- per-CPU scratch (the swapgs target) ----------------------------------

/// GS-relative scratch consumed by the entry stub. Field order is ABI:
/// the stub uses `[gs]` = `user_rsp` and `[gs + 8]` = `kernel_rsp`.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct CpuScratch {
    /// User RSP stashed by the entry stub (consumed by the `sysretq`
    /// teardown via the frame; the slot itself is scratch).
    pub user_rsp: u64,
    /// Current thread's kernel stack top — written by the scheduler on
    /// every switch (same place, same value as TSS RSP0).
    pub kernel_rsp: u64,
}
const _: () = assert!(core::mem::size_of::<CpuScratch>() == 64);

static SCRATCH: SyncCell<[CpuScratch; crate::sched::MAX_CPUS]> = SyncCell::new(
    [CpuScratch {
        user_rsp: 0,
        kernel_rsp: 0,
    }; crate::sched::MAX_CPUS],
);

/// Point the entry stub's `kernel_rsp` at `top` (the incoming thread's
/// kernel stack top). Called by `plan_switch` together with
/// `tss::set_rsp0` — the pair must never disagree (ADR-0014).
///
/// # Safety
/// IF=0 (the scheduler's decision phase), `top` 16-byte aligned.
pub unsafe fn set_cpu_kernel_stack(top: u64) {
    // SAFETY: single writer under IF=0; fixed CPU index until SMP.
    unsafe {
        (*SCRATCH.get())[crate::sched::this_cpu()].kernel_rsp = top;
    }
}

/// IA32_GS_BASE — the current GS.base, readable in ring 0.
const MSR_GS_BASE: u32 = 0xC000_0101;

/// This CPU's scratch address — the kernel-side GS.base value (the
/// stub's swapgs target).
///
/// # Safety
/// IF=0.
pub unsafe fn cpu_scratch_addr() -> u64 {
    // SAFETY: pointer arithmetic on a 'static cell; fixed CPU index.
    unsafe { SCRATCH.get().add(crate::sched::this_cpu()) as u64 }
}

/// Whether the per-CPU GS pair is currently flipped to the stub's
/// kernel side (GS.base = scratch; KERNEL_GS_BASE = the user-side
/// value). The pair is per-CPU, NOT per-thread — `switch_context` does
/// not save it — so the scheduler normalizes it around every block
/// (ADR-0018): all kernel code outside the stub runs canonical
/// (GS.base ≠ scratch), and a thread that blocks inside the stub
/// restores its flipped side when it resumes.
///
/// # Safety
/// Ring 0, IF=0.
pub unsafe fn gs_is_kernel_side() -> bool {
    // SAFETY: caller contract; rdmsr is ring-0 only.
    unsafe { super::rdmsr(MSR_GS_BASE) == cpu_scratch_addr() }
}

/// Flip the GS pair to the stub's kernel side (no-op if already there).
///
/// # Safety
/// Ring 0, IF=0, and the pair one `swapgs` away from canonical.
pub unsafe fn gs_to_kernel_side() {
    // SAFETY: caller contract.
    unsafe {
        if !gs_is_kernel_side() {
            core::arch::asm!("swapgs", options(nostack, preserves_flags));
        }
    }
}

/// Flip the GS pair back to canonical (scratch parked in
/// KERNEL_GS_BASE — the state `enter_user`, the preempt path, and
/// `SYS_THREAD_EXIT`'s diverging swapgs all leave behind). No-op if
/// already canonical.
///
/// # Safety
/// Ring 0, IF=0, and the pair one `swapgs` away from the kernel side.
pub unsafe fn gs_to_canonical_side() {
    // SAFETY: caller contract.
    unsafe {
        if gs_is_kernel_side() {
            core::arch::asm!("swapgs", options(nostack, preserves_flags));
        }
    }
}

// ---- the syscall frame -----------------------------------------------------

/// What the entry stub pushes before calling the dispatcher (ascending
/// from the frame pointer): the user state `sysretq` needs on return.
#[repr(C)]
pub struct SyscallFrame {
    pub usr_rsp: u64,
    pub usr_rip: u64,
    pub usr_rflags: u64,
}

// ---- observability (the M3 suite asserts on these) -------------------------

/// Cumulative dispatcher facts since boot.
#[derive(Clone, Copy)]
pub struct SyscallStats {
    pub calls: u64,
    pub write_calls: u64,
    pub write_bytes: u64,
    pub write_rejected: u64,
    pub exit_calls: u64,
    pub invalid_nr: u64,
    pub echo_calls: u64,
    pub prove_ring3_calls: u64,
    /// Set when SYS_PROVE_DONE found the armed #GP actually delivered.
    pub ring3_proved: bool,
    /// Error code the proved #GP carried (meaningful when ring3_proved).
    pub ring3_gp_ec: u64,
}

static STATS: SyncCell<SyscallStats> = SyncCell::new(SyscallStats {
    calls: 0,
    write_calls: 0,
    write_bytes: 0,
    write_rejected: 0,
    exit_calls: 0,
    invalid_nr: 0,
    echo_calls: 0,
    prove_ring3_calls: 0,
    ring3_proved: false,
    ring3_gp_ec: 0,
});

/// Copy-out of the cumulative stats (IF-agnostic: single-CPU, all writes
/// happen at IF=0 inside the dispatcher).
pub fn stats() -> SyscallStats {
    // SAFETY: copy-out of one small struct; SyncCell contract.
    unsafe { *STATS.get() }
}

/// Most recent SYS_DEBUG_WRITE payload (first bytes + length) — the test side
/// asserts on the *copied* buffer, not on serial text.
static WRITE_BUF: SyncCell<[u8; WRITE_MAX as usize]> = SyncCell::new([0; WRITE_MAX as usize]);
static WRITE_BUF_LEN: SyncCell<usize> = SyncCell::new(0);

pub fn last_write() -> ([u8; WRITE_MAX as usize], usize) {
    // SAFETY: copy-out; SyncCell contract (writes are IF=0 dispatcher-side).
    unsafe { (*WRITE_BUF.get(), *WRITE_BUF_LEN.get()) }
}

/// (thread id, exit status) pairs recorded by SYS_THREAD_EXIT — a
/// NEWEST-WINS ring (M5.3): one full boot (m1–m5 suites + production
/// spawn) records far more exits than the original 16-entry prefix
/// log held, and the suites always query their OWN just-exited
/// children. Oldest entries are overwritten; `exit_status_of` scans
/// newest-first, so the latest exit of a thread id always wins.
const EXIT_LOG_CAP: usize = 64;
static EXIT_LOG: SyncCell<[(u64, u64); EXIT_LOG_CAP]> = SyncCell::new([(0, 0); EXIT_LOG_CAP]);
static EXIT_LOG_LEN: SyncCell<usize> = SyncCell::new(0);
static EXIT_LOG_POS: SyncCell<usize> = SyncCell::new(0);

/// The status thread `id` exited with (None = not exited, or evicted
/// from the ring by `EXIT_LOG_CAP` newer exits — a suite that queries
/// its children promptly never sees eviction).
pub fn exit_status_of(id: u64) -> Option<u64> {
    // SAFETY: read-only; SyncCell contract.
    unsafe {
        let len = *EXIT_LOG_LEN.get();
        let pos = *EXIT_LOG_POS.get();
        for k in 0..len {
            let idx = (pos + EXIT_LOG_CAP - 1 - k) % EXIT_LOG_CAP;
            let (tid, status) = (*EXIT_LOG.get())[idx];
            if tid == id {
                return Some(status);
            }
        }
        None
    }
}

fn record_exit(id: u64, status: u64) {
    // SAFETY: single writer at IF=0 (dispatcher); ring write.
    unsafe {
        let pos = &mut *EXIT_LOG_POS.get();
        (*EXIT_LOG.get())[*pos] = (id, status);
        *pos = (*pos + 1) % EXIT_LOG_CAP;
        let len = &mut *EXIT_LOG_LEN.get();
        if *len < EXIT_LOG_CAP {
            *len += 1;
        }
    }
}

// ---- boot-time arming ------------------------------------------------------

/// Program the syscall MSRs, enable EFER.SCE, and turn on SMEP/SMAP when
/// the CPU has them. Verifies every write by read-back — a control MSR
/// that did not take is a dead boundary, not a warning (ADR-0005: no
/// decorative success). Called once from `kmain` before the M3 suite.
///
/// # Safety
/// Ring 0, IF=0, our GDT live (the STAR selectors must exist), scheduler
/// structures initialized far enough for `this_cpu()`.
pub unsafe fn init() -> Result<(), &'static str> {
    let entry_addr = syscall_entry_addr();
    let star = (STAR_USER_BASE << 48) | ((gdt::KERNEL_CODE_SELECTOR as u64) << 32);
    // SAFETY: ring 0, IF=0; every MSR below is architectural for a
    // long-mode CPU, values verified by read-back immediately after.
    unsafe {
        let efer = super::read_efer();
        super::wrmsr(MSR_EFER, efer | super::efer::SCE);
        super::wrmsr(MSR_STAR, star);
        super::wrmsr(MSR_LSTAR, entry_addr);
        super::wrmsr(MSR_SFMASK, SFMASK_VALUE);
        // The stub's swapgs target: this CPU's scratch. Per-CPU identity
        // never changes, so this is programmed once.
        let scratch = &(*SCRATCH.get())[crate::sched::this_cpu()] as *const CpuScratch;
        super::wrmsr(MSR_KERNEL_GS_BASE, scratch as u64);

        if super::rdmsr(MSR_EFER) & super::efer::SCE == 0 {
            return Err("EFER.SCE did not stick");
        }
        if super::rdmsr(MSR_STAR) != star {
            return Err("STAR read-back mismatch");
        }
        if super::rdmsr(MSR_LSTAR) != entry_addr {
            return Err("LSTAR read-back mismatch");
        }
        if super::rdmsr(MSR_SFMASK) != SFMASK_VALUE {
            return Err("SFMASK read-back mismatch");
        }
        if super::rdmsr(MSR_KERNEL_GS_BASE) != scratch as u64 {
            return Err("KERNEL_GS_BASE read-back mismatch");
        }

        // SMEP/SMAP when available (ADR-0014): enforcement, not aspiration
        // — the reference QEMU recipe enables both CPU features.
        let cpu = super::cpu_info();
        let cr4 = super::read_cr4();
        let mut want = cr4;
        if cpu.has_smep {
            want |= super::cr4::SMEP;
        }
        if cpu.has_smap {
            want |= super::cr4::SMAP;
        }
        if want != cr4 {
            super::write_cr4(want);
        }
        let now = super::read_cr4();
        if cpu.has_smep && now & super::cr4::SMEP == 0 {
            return Err("CR4.SMEP did not stick");
        }
        if cpu.has_smap && now & super::cr4::SMAP == 0 {
            return Err("CR4.SMAP did not stick");
        }
        super::set_smap_active(cpu.has_smap && now & super::cr4::SMAP != 0);

        info!(
            "syscall",
            "ring-3 boundary armed: STAR={:#x} LSTAR={entry_addr:#x} SFMASK={SFMASK_VALUE:#x} kernel_gs_base={:#x} smep={} smap={} (cpu: smep={} smap={})",
            super::rdmsr(MSR_STAR),
            scratch as u64,
            now & super::cr4::SMEP != 0,
            now & super::cr4::SMAP != 0,
            cpu.has_smep,
            cpu.has_smap,
        );
    }
    Ok(())
}

// ---- first entry into ring 3 ------------------------------------------------

/// Enter ring 3 for the first time on this thread: `iretq` frame with the
/// user selectors, RFLAGS IF=1, and the recorded entry RIP/stack. NEVER
/// RETURNS — the thread comes back to ring 0 only through the syscall
/// stub (or dies with an exception), and leaves existence through
/// `SYS_THREAD_EXIT`.
///
/// # Safety
/// Ring 0, IF=0, syscall MSRs initialized ([`init`]), the current thread
/// owns a kernel stack (the stub's scratch/RSP0 pair is reprogrammed here
/// defensively as well), `rip` and `user_rsp` canonical and inside the
/// thread's registered user regions (`sched::set_current_user_regions` —
/// the dispatcher validates syscall buffers against exactly those), and
/// the pages backing both are mapped U/S in the live address space.
pub unsafe fn enter_user(rip: u64, user_rsp: u64) -> ! {
    // Defensive re-program of the ring-3→ring-0 stack pair for THIS
    // thread (plan_switch already did it at switch-in; enter_user is the
    // last gate before user mode and refuses to trust ordering).
    if let Some(top) = crate::sched::current_kernel_stack_top() {
        // SAFETY: IF=0 caller contract; `top` is this thread's stack top.
        unsafe {
            super::tss::set_rsp0(top);
            set_cpu_kernel_stack(top);
        }
    } else {
        error!("syscall", "enter_user on a thread without a kernel stack");
        crate::halt::halt_machine("enter_user: bootstrap tried to become user code");
    }
    // SAFETY: caller contract; the asm block pushes a well-formed iretq
    // frame (SS/RSP/RFLAGS/CS/RIP) and never falls through.
    unsafe {
        core::arch::asm!(
            "push {ss}",
            "push {ursp}",
            "push {rfl}",
            "push {cs}",
            "push {urip}",
            "iretq",
            ss = in(reg) gdt::USER_DATA_SELECTOR_RPL3 as u64,
            ursp = in(reg) user_rsp,
            rfl = in(reg) USER_RFLAGS,
            cs = in(reg) gdt::USER_CODE_SELECTOR_RPL3 as u64,
            urip = in(reg) rip,
            options(noreturn),
        );
    }
}

// ---- the entry stub ---------------------------------------------------------

global_asm!(
    ".section .text",
    // Ring-3 origin is a hard contract of this stub (ADR-0014): the
    // `swapgs` pair assumes GS.base=0 in user mode and the scratch in
    // KERNEL_GS_BASE. The kernel never executes SYSCALL itself.
    //
    // Stack discipline (ABI v1, ADR-0017): kernel_rsp is the thread's
    // stack top (16-aligned). Layout, from the frame base F (= top-24):
    // pad at F-8 (the frame's 3 pushes make the arg count odd — WITHOUT
    // this pad every stack arg lands 8 bytes off its Win64 slot, a bug
    // the M4.2 bring-up caught live), arg8 &frame at F-16, arg7 a5 at
    // F-24, arg6 a4 at F-32, arg5 a3 at F-40, Win64 shadow below →
    // RSP = F-72, 16-aligned at the `call`, and the callee reads
    // [rsp+40..64] = a3, a4, a5, frame exactly. The four Win64 register
    // args are moved ONLY after the pushes that consume r8/r9 have
    // happened. SFMASK already cleared IF — the whole path is
    // non-preemptible.
    ".p2align 4",
    ".globl arena_syscall_entry",
    "arena_syscall_entry:",
    "swapgs",
    "mov gs:[0], rsp",          // stash the user RSP
    "mov rsp, gs:[8]",          // become the kernel stack (top)
    "push r11",                 // frame: user RFLAGS
    "push rcx",                 //        user RIP
    "mov rcx, gs:[0]",
    "push rcx",                 //        user RSP   (frame base F = RSP now)
    "mov rcx, rsp",             // rcx = F (captured BEFORE the pad push)
    "push rax",                 // pad at F-8 (aligns the arg slots; rax=nr survives)
    "push rcx",                 // arg8: &SyscallFrame (F)   at F-16
    "push r9",                  // arg7: a5 (user's r9 — before it is reused)
    "push r8",                  // arg6: a4 (user's r8 — before it is reused)
    "push r10",                 // arg5: a3                            at F-40
    "sub rsp, 32",              // Win64 shadow space → RSP = F-72 (aligned)
    "mov r9, rdx",              // a2 (before rdx becomes a0's target)
    "mov rdx, rdi",             // a0
    "mov r8, rsi",              // a1
    "mov rcx, rax",             // nr
    "call {dispatch}",
    // RAX = user-visible status. Tear down: skip shadow (32), args (32)
    // and pad (8), reload the recorded user state (never a user-supplied
    // target — ADR-0014), leave the kernel stack, swap back, sysretq.
    "add rsp, 72",
    "mov rcx, [rsp + 8]",       // user RIP
    "mov r11, [rsp + 16]",      // user RFLAGS
    "mov rsp, [rsp]",           // user RSP
    "swapgs",
    "sysretq",
    dispatch = sym syscall_dispatch,
);

// Assembly entry symbol (defined in the global_asm! block above).
unsafe extern "C" {
    static arena_syscall_entry: u8;
}

/// LSTAR value: the entry stub's address in the live view (the image is
/// at its final kernel-view addresses when `init` runs).
fn syscall_entry_addr() -> u64 {
    core::ptr::addr_of!(arena_syscall_entry) as u64
}

// ---- the dispatcher ---------------------------------------------------------

/// Rust half of the boundary: validate, act, return the user-visible
/// typed status in RAX (ADR-0017). Runs at IF=0 on the calling thread's
/// kernel stack with the frame the stub built; all six user argument
/// registers arrive marshalled per the Win64 convention. SYS_THREAD_EXIT
/// diverges through the scheduler.
extern "C" fn syscall_dispatch(
    nr: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    frame: *const SyscallFrame,
) -> u64 {
    // SAFETY: stats writes are single-writer at IF=0 (SyncCell contract);
    // the frame pointer came from the stub and lives on this stack.
    unsafe {
        (*STATS.get()).calls += 1;
    }
    let result = match nr {
        SYS_DEBUG_WRITE => sys_debug_write(a0, a1) as u64,
        SYS_THREAD_EXIT => sys_thread_exit(a0),
        SYS_PROVE_RING3 => sys_prove_ring3(frame),
        SYS_PROVE_DONE => sys_prove_done(),
        SYS_ABI_ECHO6 => {
            // SAFETY: as above.
            unsafe { (*STATS.get()).echo_calls += 1 };
            echo6_fingerprint([a0, a1, a2, a3, a4, a5])
        }
        SYS_IPC_CALL => sys_ipc_call(a0, a1, a2, a3, a4, a5) as u64,
        SYS_IPC_RECV => sys_ipc_recv(a0, a1, a2) as u64,
        SYS_IPC_TRY_RECV if a3 == 0 && a4 == 0 && a5 == 0 => {
            sys_ipc_receive(a0, a1, a2, false) as u64
        }
        SYS_IPC_REPLY => sys_ipc_reply(a0, a1, a2, a3, a4) as u64,
        SYS_IPC_REPLY_CHECKED => sys_ipc_reply_policy(a0, a1, a2, a3, a4, true) as u64,
        SYS_NOTIFY => sys_notify(a0, a1) as u64,
        SYS_WAIT => sys_wait(a0) as u64,
        SYS_SPAWN => sys_spawn(a0, a1, a2, a3, a4) as u64,
        SYS_CONSOLE_READ => sys_console_read(a0, a1) as u64,
        SYS_PROC_LIST => sys_proc_list(a0, a1) as u64,
        SYS_SHUTDOWN => sys_shutdown(a0) as u64,
        SYS_ALLOC_FRAME => sys_alloc_frame(a0) as u64,
        SYS_MAP_MEMORY => sys_map_memory(a0, a1) as u64,
        SYS_IRQ_RELAY => sys_irq_relay(a0, a1, a2, a3) as u64,
        SYS_CAP_PHYS => sys_cap_phys(a0) as u64,
        SYS_CAP_DESTROY => sys_cap_destroy(a0) as u64,
        SYS_CAP_COPY => sys_cap_copy(a0, a1, a2) as u64,
        SYS_DEV_INFO => sys_dev_info(a0, a1) as u64,
        SYS_CONSOLE_PUSH => sys_console_push(a0, a1, a2) as u64,
        SYS_CONSOLE_ATTACH => sys_console_attach(a0, a1, a2) as u64,
        SYS_CONSOLE_PULL => sys_console_pull(a0, a1, a2) as u64,
        SYS_CLOCK_NOW => sys_clock_now() as u64,
        SYS_TIMER_ARM => sys_timer_arm(a0, a1, a2) as u64,
        SYS_TIMER_CANCEL => sys_timer_cancel(a0) as u64,
        SYS_CAP_DESCRIBE => sys_cap_describe(a0, a1) as u64,
        SYS_PROC_FINISH => sys_proc_finish(a0, a1) as u64,
        SYS_RESOURCE_SNAPSHOT => sys_resource_snapshot(a0, a1) as u64,
        SYS_OBSERVE if [a2, a3, a4, a5] == [0; 4] => sys_observe(a0, a1) as u64,
        SYS_SPAWN_CHECK if [a1, a2, a3, a4, a5] == [0; 5] => sys_spawn_check(a0) as u64,
        SYS_TRY_WAIT => sys_try_wait(a0) as u64,
        SYS_IMAGE_REGISTER => sys_image_register(a0, a1, a2, a3, a4, a5) as u64,
        SYS_IMAGE_REVOKE => sys_image_revoke(a0, a1, a2, a3, a4, a5) as u64,
        SYS_DISPLAY_INFO => sys_display_info(a0, a1) as u64,
        SYS_SHARED_CREATE => sys_shared_create(a0, a1, a2) as u64,
        SYS_SHARED_MAP => sys_shared_map(a0, a1) as u64,
        SYS_SHARED_PHYS => sys_shared_phys(a0, a1, a2) as u64,
        SYS_SHARED_INFO => sys_shared_info(a0, a1, [a2, a3, a4, a5]) as u64,
        SYS_SHARED_UNMAP => sys_shared_unmap(a0, [a1, a2, a3, a4, a5]) as u64,
        SYS_PROC_LIVE => sys_proc_live(a0) as u64,
        SYS_ENDPOINT_BIND if [a3, a4, a5] == [0; 3] => sys_endpoint_bind(a0, a1, a2) as u64,
        SYS_ENDPOINT_UNBIND if [a1, a2, a3, a4, a5] == [0; 5] => sys_endpoint_unbind(a0) as u64,
        SYS_ENDPOINT_MINT if [a3, a4, a5] == [0; 3] => sys_endpoint_mint(a0, a1, a2) as u64,
        SYS_IPC_RECV_BADGED if a5 == 0 => sys_ipc_recv_badged(a0, a1, a2, a3, a4) as u64,
        _ => {
            // SAFETY: as above.
            unsafe { (*STATS.get()).invalid_nr += 1 };
            STATUS_BAD_CALL as u64
        }
    };
    crate::image_registry::assert_conservation();
    crate::shared::assert_conservation();
    result
}

/// SYS_DEBUG_WRITE(buf, len): copy `buf[..len]` from the *calling
/// thread's* address space (SMAP-aware) to the serial console, raw —
/// the temporary diagnostics backdoor (ADR-0017). Returns the accepted
/// byte count, `STATUS_BAD_ARG` (null buf / len 0 / len over cap), or
/// `STATUS_BAD_ADDRESS`. Validation is per page against the thread's
/// registered user regions — a span crossing an unmapped hole is
/// rejected with a typed status, never faulted (ADR-0014).
fn sys_debug_write(buf: u64, len: u64) -> Status {
    // SAFETY: stats/record writes single-writer at IF=0.
    unsafe {
        (*STATS.get()).write_calls += 1;
    }
    // (buf needs no alignment — byte streams may start anywhere; only
    // the page-span check below matters.)
    if len == 0 || len > WRITE_MAX || buf == 0 {
        // SAFETY: as above.
        unsafe { (*STATS.get()).write_rejected += 1 };
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(buf, len) {
        // SAFETY: as above.
        unsafe { (*STATS.get()).write_rejected += 1 };
        return STATUS_BAD_ADDRESS;
    }
    let mut tmp = [0u8; WRITE_MAX as usize];
    let n = len as usize;
    // SAFETY: range fully validated against the thread's regions (every
    // page present U/S in the live address space); STAC/CLAC bracket the
    // touch when SMAP is live; IF=0, single CPU — no concurrent mutation
    // of the source pages exists in 3.3 (no demand paging, no mprotect).
    unsafe {
        super::stac();
        core::ptr::copy_nonoverlapping(buf as *const u8, tmp.as_mut_ptr(), n);
        super::clac();
    }
    // SAFETY: single writer at IF=0.
    unsafe {
        core::ptr::copy_nonoverlapping(tmp.as_ptr(), WRITE_BUF.get() as *mut u8, n);
        *WRITE_BUF_LEN.get() = n;
        (*STATS.get()).write_bytes += n as u64;
    }
    crate::log::write_raw(&tmp[..n]);
    n as Status
}

/// SYS_THREAD_EXIT(code): record (id, code), then terminate through the
/// scheduler's normal zombie/reap path (ADR-0017 call 2). Diverges — the
/// kernel stack frame this call sits on dies with the thread (the stub
/// never resumes).
fn sys_thread_exit(status: u64) -> ! {
    // SAFETY: single writer at IF=0.
    unsafe {
        (*STATS.get()).exit_calls += 1;
    }
    manager_check_last_thread(); // before even recording the exit status
    let id = crate::sched::current_thread_id();
    record_exit(id, status);
    notify_last_thread_exit();
    // This syscall diverges: the stub's exit-side `swapgs` never runs.
    // Restore the canonical user-side GS state HERE (GS.base = 0,
    // KERNEL_GS_BASE = scratch) before the scheduler takes over —
    // otherwise the machine stays on the kernel side of the swap and the
    // NEXT syscall's entry `swapgs` lands backwards, writing through
    // GS.base = 0 (observed live during bring-up: #PF at LSTAR+3 →
    // SMAP-blocked exception push onto the user stack → #DF → triple
    // fault). Every other syscall returns through the stub and swaps
    // back there.
    // SAFETY: swapgs exchanges GS.base ↔ MSR_KERNEL_GS_BASE; we are
    // inside the stub's swapped window (IF=0), single CPU.
    unsafe {
        core::arch::asm!("swapgs", options(nostack, preserves_flags));
    }
    crate::sched::terminate()
}

/// Shared last-thread hook for both voluntary exit and an unarmed CPL3
/// exception (ADR-0042). The exception entered via IDT, not the syscall
/// swapgs window, so its caller must NOT swap GS again.
fn manager_check_last_thread() {
    if let Some(pid) = crate::sched::current_proc_id() {
        if crate::sched::proc_live_threads(pid) == 1 {
            crate::image_registry::manager_death_check(pid);
        }
    }
}

fn notify_last_thread_exit() {
    // Spawn-protocol hook (ADR-0019): if this is the LAST live thread of
    // a process with a registered exit notification, badge it now — the
    // supervisor's `wait` consumes the badge through the ADR-0018
    // primitive (merged, so it survives even if the supervisor is
    // mid-syscall). This thread is still Running, so "last" means the
    // live count is exactly 1: us.
    if let Some(pid) = crate::sched::current_proc_id() {
        if crate::sched::proc_live_threads(pid) == 1 {
            if let Some((nid, badge)) = crate::proc::exit_notif_of(pid) {
                if let Err(e) = crate::ipc::notify(nid, badge) {
                    error!("syscall", "exit notification failed for pid {pid}: {e}");
                }
            }
        }
    }
}

/// A genuine ring-3 CPU fault kills ONLY the faulting process thread.
/// The parent receives its ordinary child-exit notification; its held
/// Process cap must still authorize teardown, including failing a
/// client IPC already delivered to this server. Called IF=0 from IDT.
pub(crate) fn exit_on_user_fault(vector: u64) -> ! {
    manager_check_last_thread();
    record_exit(crate::sched::current_thread_id(), 0x100 + vector);
    notify_last_thread_exit();
    // The driver supervisor cannot destroy a process in its own live
    // CR3 here. Mark the last faulting thread's driver for deferred
    // idle-thread teardown; poll will release outstanding IPC BEFORE
    // replaying grants. A non-supervised user child stays manager-owned.
    if let Some(pid) = crate::sched::current_proc_id() {
        if crate::sched::proc_live_threads(pid) == 1 && crate::supervise::is_supervised(pid) {
            crate::supervise::note_death(pid);
        }
    }
    crate::sched::terminate()
}

// ---- IPC v1 handlers (ADR-0018) -------------------------------------------
//
// The handlers own the ABI surface: caller identity (thread → process),
// cap resolution (slot → object + rights), user-buffer validation, and
// every STAC-bracketed user-memory touch — always in the owner thread's
// own context. Object logic lives in `crate::ipc`.

/// Resolve an endpoint cap: in-bounds slot, occupied, `CapObj::Endpoint`,
/// and holding `right` (WRITE = call side, READ = serve side).
fn endpoint_of(pid: u64, slot: u64, right: u32) -> Result<u32, Status> {
    if slot >= crate::cap::CAP_SLOTS as u64 {
        return Err(STATUS_BAD_ARG);
    }
    // SAFETY: IF=0 dispatch context; cap::read is bounds/occupancy-safe.
    let c = crate::cap::read(pid, slot as usize).map_err(|_| STATUS_BAD_ARG)?;
    if c.rights & right == 0 {
        return Err(STATUS_BAD_ARG);
    }
    match c.obj {
        crate::cap::CapObj::Endpoint { eid } => Ok(eid),
        _ => Err(STATUS_BAD_ARG),
    }
}

/// SYS_ENDPOINT_BIND(endpoint slot, notification slot, badge) — ADR-0071.
///
/// Authority is held capabilities only: READ on the endpoint (the serve
/// side; a WRITE-only client cannot redirect its server's wakes) and
/// READ|WRITE on the notification (the binder can both wait on it and
/// signal it, so binding grants nothing it did not already hold). From
/// then on a CALL queued while no server is parked in RECV ORs `badge`
/// into that notification. One binding per endpoint; rebinding replaces.
fn sys_endpoint_bind(a0: u64, a1: u64, a2: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(eid) = endpoint_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    let Ok(nid) = notification_of(pid, a1, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    if notification_of(pid, a1, crate::cap::RIGHTS_WRITE).is_err() {
        return STATUS_BAD_ARG;
    }
    match crate::ipc::bind(eid, nid, a2) {
        Ok(()) => STATUS_OK,
        Err(e) => e,
    }
}

/// SYS_ENDPOINT_MINT(endpoint slot, badge, rights) — ADR-0074. The slot
/// must hold a plain endpoint cap with READ (the serve side); returns the
/// slot of the new badged cap (rights ⊆ WRITE|COPY, WRITE required).
fn sys_endpoint_mint(a0: u64, a1: u64, a2: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 || a1 > u64::from(u32::MAX) || a2 > u64::from(u32::MAX) {
        return STATUS_BAD_ARG;
    }
    match crate::cap::mint_badged(pid, a0 as usize, a1 as u32, a2 as u32) {
        Ok(slot) => slot as Status,
        Err(_) => STATUS_BAD_ARG,
    }
}

/// SYS_IPC_RECV_BADGED(endpoint slot, out 32 B, msg buf, blocking, 0) —
/// ADR-0074: as RECV (blocking = 1) or TRY_RECV (0), with `out` receiving
/// `[w0, w1, landed-cap-slot, badge]`.
fn sys_ipc_recv_badged(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a3 > 1 || a4 != 0 {
        return STATUS_BAD_ARG;
    }
    let Ok(eid) = endpoint_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    if !user_range_ok(a1, 32) {
        return STATUS_BAD_ADDRESS;
    }
    let has_msg = a2 != 0;
    if has_msg && !user_range_ok(a2, crate::ipc::MSG_BYTES as u64) {
        return STATUS_BAD_ADDRESS;
    }
    match crate::ipc::recv_badged(pid, eid, a3 == 1) {
        Ok((words, landed, msg, badge)) => {
            // SAFETY: as in sys_ipc_call — own context, validated range.
            unsafe {
                write_user_words(a1, [words[0], words[1], landed]);
                super::stac();
                core::ptr::write_volatile((a1 as *mut u64).add(3), u64::from(badge));
                super::clac();
                if has_msg {
                    write_user_msg(a2, &msg);
                }
            }
            STATUS_OK
        }
        Err(e) => e,
    }
}

/// SYS_ENDPOINT_UNBIND(endpoint slot) — READ on the endpoint (ADR-0071).
fn sys_endpoint_unbind(a0: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(eid) = endpoint_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    match crate::ipc::unbind(eid) {
        Ok(()) => STATUS_OK,
        Err(e) => e,
    }
}

/// Resolve a notification cap (WRITE = notify, READ = wait).
fn notification_of(pid: u64, slot: u64, right: u32) -> Result<u32, Status> {
    if slot >= crate::cap::CAP_SLOTS as u64 {
        return Err(STATUS_BAD_ARG);
    }
    let c = crate::cap::read(pid, slot as usize).map_err(|_| STATUS_BAD_ARG)?;
    if c.rights & right == 0 {
        return Err(STATUS_BAD_ARG);
    }
    match c.obj {
        crate::cap::CapObj::Notification { nid } => Ok(nid),
        _ => Err(STATUS_BAD_ARG),
    }
}

/// The optional per-message transferred cap: `CAP_NONE` = none sent;
/// otherwise the slot must hold a cap with COPY (transfer is a copy —
/// attenuation-only, ADR-0015/0018).
fn send_cap_of(pid: u64, slot: u64) -> Result<Option<crate::cap::Cap>, Status> {
    if slot == crate::ipc::CAP_NONE {
        return Ok(None);
    }
    if slot >= crate::cap::CAP_SLOTS as u64 {
        return Err(STATUS_BAD_ARG);
    }
    let c = crate::cap::read(pid, slot as usize).map_err(|_| STATUS_BAD_ARG)?;
    if c.rights & crate::cap::RIGHTS_COPY == 0 {
        return Err(STATUS_BAD_ARG);
    }
    Ok(Some(c))
}

/// Copy a user inline-message buffer into kernel scratch (IPC v1.1).
///
/// # Safety
/// `[a, a+64)` was validated by `user_range_ok` against the CURRENT
/// thread's regions and the thread's own address space is live.
unsafe fn read_user_msg(a: u64) -> [u8; crate::ipc::MSG_BYTES] {
    let mut msg = [0u8; crate::ipc::MSG_BYTES];
    // SAFETY: caller contract; STAC brackets the SMAP-guarded reads.
    unsafe {
        super::stac();
        core::ptr::copy_nonoverlapping(a as *const u8, msg.as_mut_ptr(), crate::ipc::MSG_BYTES);
        super::clac();
    }
    msg
}

/// Copy a kernel inline-message buffer to validated user memory (v1.1).
///
/// # Safety
/// As [`read_user_msg`].
unsafe fn write_user_msg(a: u64, msg: &[u8; crate::ipc::MSG_BYTES]) {
    // SAFETY: caller contract.
    unsafe {
        super::stac();
        core::ptr::copy_nonoverlapping(msg.as_ptr(), a as *mut u8, crate::ipc::MSG_BYTES);
        super::clac();
    }
}

/// Write the three message words to a validated user buffer.
///
/// # Safety
/// `[buf, buf+24)` was validated by `user_range_ok` against the CURRENT
/// thread's regions, the thread's own address space is live, and IF=0.
unsafe fn write_user_words(buf: u64, words: [u64; 3]) {
    // SAFETY: caller contract; STAC brackets the SMAP-guarded writes.
    unsafe {
        super::stac();
        let p = buf as *mut u64;
        core::ptr::write_volatile(p, words[0]);
        core::ptr::write_volatile(p.add(1), words[1]);
        core::ptr::write_volatile(p.add(2), words[2]);
        super::clac();
    }
}

/// SYS_IPC_CALL(ep slot, w0, w1, send-cap slot, reply buf, msg buf):
/// block until the server replies; the reply lands in the buffer as
/// `[w0, w1, cap-slot-or-CAP_NONE]` and the server's inline reply
/// message overwrites `msg buf` (NULL = none; IPC v1.1). Needs WRITE
/// on the endpoint cap.
fn sys_ipc_call(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    // A plain endpoint cap (badge 0) or a live badged one (ADR-0074).
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok((eid, badge)) = crate::cap::call_target(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    let Ok(send_cap) = send_cap_of(pid, a3) else {
        return STATUS_BAD_ARG;
    };
    if !user_range_ok(a4, IPC_BUF_BYTES) {
        return STATUS_BAD_ADDRESS;
    }
    // IPC v1.1: optional inline message; the buffer is IN/OUT —
    // snapshotted now (caller context), reply written on resume (same
    // context: `ipc::call` blocks and wakes THIS thread).
    let has_msg = a5 != 0;
    if has_msg && !user_range_ok(a5, crate::ipc::MSG_BYTES as u64) {
        return STATUS_BAD_ADDRESS;
    }
    let msg = if has_msg {
        // SAFETY: validated above; own context.
        unsafe { read_user_msg(a5) }
    } else {
        [0u8; crate::ipc::MSG_BYTES]
    };
    match crate::ipc::call_badged(pid, eid, badge, [a1, a2], send_cap, msg) {
        Ok((words, landed, reply_msg)) => {
            // SAFETY: validated above; `ipc::call` resumes in THIS
            // thread's own context and address space (ADR-0018: user
            // memory is only touched by its owner).
            unsafe {
                write_user_words(a4, [words[0], words[1], landed]);
                if has_msg {
                    write_user_msg(a5, &reply_msg);
                }
            }
            STATUS_OK
        }
        Err(e) => e,
    }
}

/// SYS_IPC_RECV(ep slot, buf, msg buf): take the oldest request,
/// blocking while the queue is empty; buf receives
/// `[w0, w1, landed-cap-slot]` and the request's inline message lands
/// in `msg buf` (NULL = not wanted; IPC v1.1). Needs READ.
fn sys_ipc_recv(a0: u64, a1: u64, a2: u64) -> Status {
    sys_ipc_receive(a0, a1, a2, true)
}

fn sys_ipc_receive(a0: u64, a1: u64, a2: u64, blocking: bool) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(eid) = endpoint_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    if !user_range_ok(a1, IPC_BUF_BYTES) {
        return STATUS_BAD_ADDRESS;
    }
    // IPC v1.1: optional inline-message out buffer. Validated BEFORE
    // blocking so a bad pointer fails fast with a typed status.
    let has_msg = a2 != 0;
    if has_msg && !user_range_ok(a2, crate::ipc::MSG_BYTES as u64) {
        return STATUS_BAD_ADDRESS;
    }
    let result = if blocking {
        crate::ipc::recv(pid, eid)
    } else {
        crate::ipc::try_recv(pid, eid)
    };
    match result {
        Ok((words, landed, msg)) => {
            // SAFETY: as in sys_ipc_call — own context, validated range.
            unsafe {
                write_user_words(a1, [words[0], words[1], landed]);
                if has_msg {
                    write_user_msg(a2, &msg);
                }
            }
            STATUS_OK
        }
        Err(e) => e,
    }
}

/// SYS_IPC_REPLY(ep slot, w0, w1, send-cap slot, msg buf): stage the
/// reply into this server's delivered slot and wake the caller; the
/// inline reply message is snapshotted from `msg buf` in the SERVER's
/// context now (NULL = none; IPC v1.1). Needs READ.
fn sys_ipc_reply(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> Status {
    sys_ipc_reply_policy(a0, a1, a2, a3, a4, false)
}
fn sys_ipc_reply_policy(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, checked: bool) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(eid) = endpoint_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    let Ok(send_cap) = send_cap_of(pid, a3) else {
        return STATUS_BAD_ARG;
    };
    // IPC v1.1: optional inline reply message, snapshotted in the
    // server's own context (ADR-0018: never touch foreign memory).
    let has_msg = a4 != 0;
    if has_msg && !user_range_ok(a4, crate::ipc::MSG_BYTES as u64) {
        return STATUS_BAD_ADDRESS;
    }
    let msg = if has_msg {
        // SAFETY: validated above; own context.
        unsafe { read_user_msg(a4) }
    } else {
        [0u8; crate::ipc::MSG_BYTES]
    };
    match crate::ipc::reply_policy(eid, [a1, a2], send_cap, msg, checked) {
        Ok(()) => STATUS_OK,
        Err(e) => e,
    }
}

/// SYS_NOTIFY(notif slot, badge): OR a nonzero badge into the pending
/// word, wake a parked waiter. Non-blocking. Needs WRITE.
fn sys_notify(a0: u64, a1: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(nid) = notification_of(pid, a0, crate::cap::RIGHTS_WRITE) else {
        return STATUS_BAD_ARG;
    };
    match crate::ipc::notify(nid, a1) {
        Ok(()) => STATUS_OK,
        Err(e) => e,
    }
}

/// SYS_WAIT(notif slot): take the pending badge word (clearing it),
/// blocking while zero. Returns the badge as a positive payload. Needs
/// READ.
fn sys_wait(a0: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(nid) = notification_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    match crate::ipc::wait(nid) {
        Ok(badge) => badge as Status, // positive payload: the merged badge word
        Err(e) => e,
    }
}

/// SYS_TRY_WAIT(notification READ slot): return pending merged badge
/// and clear it, or zero if absent; NEVER park or register a waiter.
fn sys_try_wait(a0: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(nid) = notification_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    match crate::ipc::try_wait(nid) {
        Ok(badge) => badge as Status,
        Err(e) => e,
    }
}

/// SYS_SPAWN(image slot, spec ptr, spec count, notif slot | CAP_NONE,
/// badge): the spawn protocol of ADR-0019 — validate the image cap and
/// the inheritance spec (an array of `(src_slot, rights)` pairs in the
/// caller's memory, at most `spawn::MAX_INHERIT`), then hand the whole
/// creation sequence to `spawn::spawn_from`. Returns the child's pid as
/// a positive payload. The child's Process cap lands in the caller's
/// first free slot.
/// ADR-0055: registrar possession, exact copied user bytes, preflight,
/// production ELF validation, then atomic publish and cap mint under IF=0.
fn sys_image_register(reg: u64, addr: u64, len: u64, dst: u64, r8: u64, r9: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if reg >= crate::cap::CAP_SLOTS as u64
        || dst >= crate::cap::CAP_SLOTS as u64
        || len == 0
        || len > crate::image_registry::MAX_BYTES as u64
        || r8 != 0
        || r9 != 0
    {
        return STATUS_BAD_ARG;
    }
    let Ok(c) = crate::cap::read(pid, reg as usize) else {
        return STATUS_BAD_ARG;
    };
    if c.obj != crate::cap::CapObj::ImageRegistrar
        || c.rights & crate::cap::RIGHTS_WRITE == 0
        || !crate::image_registry::registrar_alive()
        || crate::cap::read(pid, dst as usize).is_ok()
    {
        return STATUS_BAD_ARG;
    }
    // Explicit user-half/overflow check, then registered readable regions
    // checked page by page before STAC. No recoverable #PF is assumed.
    let Some(end) = addr.checked_add(len) else {
        return STATUS_BAD_ADDRESS;
    };
    if addr == 0 || end > 0x0000_8000_0000_0000 || !user_range_ok(addr, len) {
        return STATUS_BAD_ADDRESS;
    }
    let Some(idx) = crate::image_registry::reserve() else {
        return STATUS_BUSY;
    };
    // SAFETY: the full user span has been checked against current regions;
    // IF=0, no intervening unmap; the reserved destination is kernel-owned.
    unsafe {
        super::stac();
        crate::image_registry::copy_from_user(idx, addr as *const u8, len as usize);
        super::clac();
    }
    if !crate::image_registry::validate_reserved(idx) {
        crate::image_registry::abandon(idx);
        return STATUS_BAD_ARG;
    }
    let id = crate::image_registry::begin_mint(idx);
    let cap = crate::cap::Cap {
        obj: crate::cap::CapObj::Image { img_id: id },
        rights: crate::cap::RIGHTS_READ | crate::cap::RIGHTS_COPY | crate::cap::RIGHTS_DESTROY,
    };
    if crate::cap::issue(pid, dst as usize, cap).is_err() {
        crate::image_registry::undo_mint(idx);
        return STATUS_BUSY;
    }
    crate::image_registry::commit_mint(idx);
    id as Status
}

fn sys_image_revoke(reg: u64, id: u64, rdx: u64, r10: u64, r8: u64, r9: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if reg >= crate::cap::CAP_SLOTS as u64
        || id < crate::image_registry::FIRST as u64
        || id > u32::MAX as u64
        || rdx != 0
        || r10 != 0
        || r8 != 0
        || r9 != 0
    {
        return STATUS_BAD_ARG;
    }
    let Ok(c) = crate::cap::read(pid, reg as usize) else {
        return STATUS_BAD_ARG;
    };
    if c.obj != crate::cap::CapObj::ImageRegistrar
        || c.rights & crate::cap::RIGHTS_WRITE == 0
        || !crate::image_registry::registrar_alive()
    {
        return STATUS_BAD_ARG;
    }
    if !crate::image_registry::revoke(id as u32) {
        return STATUS_BAD_ARG;
    }
    STATUS_OK
}

/// Descriptive preflight before an ordinary broker allocates auxiliary
/// resources. The actual spawn repeats all checks and retains rollback;
/// this read-only observation is never a reservation or extra authority.
fn sys_spawn_check(slot: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if slot >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(cap) = crate::cap::read(pid, slot as usize) else {
        return STATUS_BAD_ARG;
    };
    if cap.rights & crate::cap::RIGHTS_READ == 0 {
        return STATUS_BAD_ARG;
    }
    match cap.obj {
        crate::cap::CapObj::Image { img_id } if img_id < crate::image_registry::FIRST => {
            if crate::spawn::image_bytes(img_id).is_none() {
                return STATUS_BAD_ARG;
            }
        }
        crate::cap::CapObj::Image { img_id } => {
            if !crate::image_registry::live(img_id) {
                return STATUS_BAD_ARG;
            }
            if crate::spawn::dynamic_children_full() {
                return STATUS_BUSY;
            }
        }
        crate::cap::CapObj::BootImage { index } if crate::spawn::boot_image_live(index) => {}
        _ => return STATUS_BAD_ARG,
    }
    if crate::spawn::records_snapshot().iter().flatten().count() >= crate::spawn::MAX_SPAWN_RECS
        || crate::proc::live_count() >= crate::proc::MAX_PROCESSES
        || !crate::cap::occupancy(pid).is_some_and(|(used, total)| used < total)
    {
        return STATUS_BUSY;
    }
    STATUS_OK
}
fn sys_spawn(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    // The image cap: READ = may spawn from it.
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(img_cap) = crate::cap::read(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    if img_cap.rights & crate::cap::RIGHTS_READ == 0 {
        return STATUS_BAD_ARG;
    }
    let source = match img_cap.obj {
        crate::cap::CapObj::Image { img_id } => {
            // Full-ID liveness wins over BUSY for stale copies. A second
            // over-capacity child refuses before any loader/resource reservation.
            if img_id >= crate::image_registry::FIRST {
                if !crate::image_registry::live(img_id) {
                    return STATUS_BAD_ARG;
                }
                if crate::spawn::dynamic_children_full() {
                    return STATUS_BUSY;
                }
            }
            (img_id, false)
        }
        crate::cap::CapObj::BootImage { index } if crate::spawn::boot_image_live(index) => {
            (index, true)
        }
        _ => return STATUS_BAD_ARG,
    };
    // The inheritance spec lives in the caller's memory: page-validate,
    // then read it under STAC in THIS (the caller's own) context.
    if a2 > crate::spawn::MAX_INHERIT as u64 {
        return STATUS_BAD_ARG;
    }
    let n = a2 as usize;
    let mut spec = [(0u64, 0u64); crate::spawn::MAX_INHERIT];
    if n > 0 {
        if !user_range_ok(a1, a2 * 16) {
            return STATUS_BAD_ADDRESS;
        }
        // SAFETY: the span was validated against this thread's regions;
        // own address space live; STAC brackets the SMAP-guarded reads;
        // IF=0. Unaligned u64 loads are fine on x86-64.
        unsafe {
            super::stac();
            for i in 0..n {
                let p = (a1 + (i * 16) as u64) as *const u64;
                spec[i] = (
                    core::ptr::read_volatile(p),
                    core::ptr::read_volatile(p.add(1)),
                );
            }
            super::clac();
        }
    }
    // The exit notification: the parent lends the notify side (WRITE)
    // of one of its notification caps; the badge must be nonzero.
    let notif = if a3 == crate::ipc::CAP_NONE {
        if a4 != 0 {
            return STATUS_BAD_ARG; // a badge with nowhere to send it
        }
        None
    } else {
        let Ok(nid) = notification_of(pid, a3, crate::cap::RIGHTS_WRITE) else {
            return STATUS_BAD_ARG;
        };
        if a4 == 0 {
            return STATUS_BAD_ARG;
        }
        Some((nid, a4))
    };
    let created = if source.1 {
        crate::spawn::spawn_boot_from(pid, source.0, &spec[..n], notif)
    } else {
        crate::spawn::spawn_from(pid, source.0, &spec[..n], notif)
    };
    match created {
        Ok(child_pid) => child_pid as Status, // pid > 0 = success payload
        Err(status) => status,
    }
}

/// SYS_CONSOLE_READ(buf, max): pop the next complete console line into
/// the caller's buffer, blocking while the queue is empty (ADR-0020).
/// Returns the line's byte count as a positive payload — the terminator
/// is never included. `max` is bounded by `console::LINE_MAX`; a
/// shorter `max` truncates the line (counted, remainder discarded).
/// One reader at a time: a second concurrent reader gets STATUS_BUSY.
fn sys_console_read(a0: u64, a1: u64) -> Status {
    if a1 == 0 || a1 > crate::console::LINE_MAX as u64 {
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(a0, a1) {
        return STATUS_BAD_ADDRESS;
    }
    let mut line = [0u8; crate::console::LINE_MAX];
    match crate::console::read_line(&mut line[..a1 as usize]) {
        Ok(n) => {
            // SAFETY: the span was validated against this thread's
            // regions; own address space live; STAC brackets the
            // SMAP-guarded writes; IF=0. The copy source is this
            // stack's own scratch. read_line may have blocked (GS-side
            // invariant handled inside, ADR-0018) — on resume we are
            // back in this same dispatcher frame.
            unsafe {
                super::stac();
                core::ptr::copy_nonoverlapping(line.as_ptr(), a0 as *mut u8, n);
                super::clac();
            }
            n as Status
        }
        Err(status) => status,
    }
}

/// SYS_PROC_LIST(buf, max_pairs): write up to `max_pairs`
/// `(pid, live_thread_count)` u64 pairs for every live process into the
/// caller's buffer; returns the pair count as a positive payload
/// (ADR-0020 — `ps`'s whole view of the process table).
fn sys_proc_list(a0: u64, a1: u64) -> Status {
    if a1 == 0 || a1 > crate::proc::MAX_PROCESSES as u64 {
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(a0, a1 * 16) {
        return STATUS_BAD_ADDRESS;
    }
    let mut pairs = [(0u64, 0usize); crate::proc::MAX_PROCESSES];
    let n = crate::proc::list_live(&mut pairs[..a1 as usize]);
    // SAFETY: validated span, own address space, STAC bracket, IF=0;
    // the scratch is this stack's own.
    unsafe {
        super::stac();
        for i in 0..n {
            let p = (a0 + (i * 16) as u64) as *mut u64;
            core::ptr::write_volatile(p, pairs[i].0);
            core::ptr::write_volatile(p.add(1), pairs[i].1 as u64);
        }
        super::clac();
    }
    n as Status
}

/// SYS_SHUTDOWN(power slot): halt the machine through the firmware's
/// ResetSystem — gated on a `CapObj::Power` capability with WRITE in
/// the caller's slot (ADR-0020: the authority to stop the machine is
/// an object, never a public verb). Logs the requesting pid, then
/// enters the same farewell-island path the boot sequence's clean halt
/// uses; the canonical halt declaration on the wire is the harness's
/// discriminator, unchanged.
fn sys_shutdown(a0: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(c) = crate::cap::read(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    if !matches!(c.obj, crate::cap::CapObj::Power) || c.rights & crate::cap::RIGHTS_WRITE == 0 {
        return STATUS_BAD_ARG;
    }
    info!(
        "kernel",
        "shutdown requested by pid {pid} through its Power cap — goodnight"
    );
    crate::halt::reset_shutdown()
}

/// Largest byte run one `SYS_CONSOLE_PUSH` accepts. A keyboard
/// produces bytes one keystroke at a time; the bound exists so the
/// copy lands in a fixed stack buffer with no allocator and no
/// unbounded loop under IF=0.
pub const CONSOLE_PUSH_MAX: u64 = 64;

/// SYS_CONSOLE_PUSH(slot, buf, len): feed `len` bytes from the
/// caller's buffer into the console's line discipline — byte for byte
/// the path COM1's RX ISR takes (`console::feed`), so echo, backspace,
/// the line queue, and the parked reader's wake all behave identically
/// whether the byte came from the serial port or from a keyboard
/// driver in ring 3 (M6.3, ADR-0026).
///
/// Gated on [`crate::cap::CapObj::ConsoleInput`] + `RIGHTS_WRITE` in
/// `slot`: without it any process could forge the keystrokes the shell
/// trusts. `len == 0` is the documented CAPABILITY PROBE — it pushes
/// nothing and returns 0 when the cap is valid, a typed refusal when
/// it is not, which is how one driver image discovers whether it was
/// spawned as the production console feeder or as a suite's service
/// instance. Returns the byte count pushed.
fn sys_console_push(a0: u64, a1: u64, a2: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(c) = crate::cap::read(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    if !matches!(c.obj, crate::cap::CapObj::ConsoleInput)
        || c.rights & crate::cap::RIGHTS_WRITE == 0
    {
        return STATUS_BAD_ARG;
    }
    if a2 == 0 {
        return 0; // the capability probe: authority confirmed, nothing pushed
    }
    if a2 > CONSOLE_PUSH_MAX {
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(a1, a2) {
        return STATUS_BAD_ADDRESS;
    }
    let mut buf = [0u8; CONSOLE_PUSH_MAX as usize];
    // SAFETY: the span was validated against this thread's regions;
    // own address space live; STAC brackets the SMAP-guarded read;
    // IF=0. The destination is this stack's own scratch.
    unsafe {
        super::stac();
        core::ptr::copy_nonoverlapping(a1 as *const u8, buf.as_mut_ptr(), a2 as usize);
        super::clac();
    }
    for &b in &buf[..a2 as usize] {
        // Exactly what rx_isr does per byte: the discipline's own
        // entry point. `commit`'s wake only ENQUEUES the parked
        // reader (no switch), so this stays a plain syscall return.
        crate::console::feed(b);
    }
    a2 as Status
}

/// Largest run one `SYS_CONSOLE_PULL` returns. Matches WRITE_MAX: a
/// driver that pulls console text is about to push it at a device in
/// buffers of that order, and a bound keeps the copy on the syscall
/// stack.
const CONSOLE_PULL_MAX: u64 = 256;

/// SYS_CONSOLE_ATTACH(cap_slot, notif_slot, badge): attach this
/// process as THE console output channel. `cap_slot` must hold
/// `ConsoleOutput` with READ; `notif_slot` a notification with WRITE
/// (the notify side, exactly as `SYS_IRQ_RELAY` takes it); `badge`
/// must be nonzero (the merged-badge protocol has no empty word).
///
/// Returns 0. `STATUS_BUSY` = another channel is already attached.
/// The attachment is swept when the process is destroyed, so a dead
/// driver cannot leave the kernel mirroring into a dead notification
/// (`proc::destroy`, the same place relay vectors are swept).
fn sys_console_attach(a0: u64, a1: u64, a2: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(c) = crate::cap::read(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    if !matches!(c.obj, crate::cap::CapObj::ConsoleOutput)
        || c.rights & crate::cap::RIGHTS_READ == 0
    {
        return STATUS_BAD_ARG;
    }
    if a2 == 0 {
        return STATUS_BAD_ARG;
    }
    let Ok(nid) = notification_of(pid, a1, crate::cap::RIGHTS_WRITE) else {
        return STATUS_BAD_ARG;
    };
    match crate::console::attach_output(pid, nid, a2) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// SYS_CONSOLE_PULL(cap_slot, buf, len): copy out up to `len` bytes of
/// mirrored console output (at most [`CONSOLE_PULL_MAX`]), returning
/// the count — 0 means the mirror is empty, which also re-arms the
/// notification.
///
/// Non-blocking by construction (see `console::pull_output`). Refused
/// unless the caller both HOLDS the capability and IS the attached
/// channel: the cap is the authority, the attachment is the identity.
fn sys_console_pull(a0: u64, a1: u64, a2: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(c) = crate::cap::read(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    if !matches!(c.obj, crate::cap::CapObj::ConsoleOutput)
        || c.rights & crate::cap::RIGHTS_READ == 0
    {
        return STATUS_BAD_ARG;
    }
    if !crate::console::output_owner_is(pid) {
        return STATUS_BAD_ARG; // holds the cap, never attached
    }
    if a2 == 0 {
        return 0; // the capability probe, mirroring `SYS_CONSOLE_PUSH`
    }
    let n = core::cmp::min(a2, CONSOLE_PULL_MAX) as usize;
    if !user_range_ok(a1, n as u64) {
        return STATUS_BAD_ADDRESS;
    }
    let mut buf = [0u8; CONSOLE_PULL_MAX as usize];
    let got = crate::console::pull_output(&mut buf[..n]);
    if got > 0 {
        // SAFETY: the span was validated against this thread's
        // regions; own address space live; STAC brackets the
        // SMAP-guarded write; IF=0. The source is this stack's scratch.
        unsafe {
            super::stac();
            core::ptr::copy_nonoverlapping(buf.as_ptr(), a1 as *mut u8, got);
            super::clac();
        }
    }
    got as Status
}

/// SYS_CLOCK_NOW(): monotonic microseconds since boot.
///
/// No capability: a clock reading is not an authority over anything,
/// and every timeout, RTT measurement and retry decision in Phase 7
/// needs it. It is the SAME clock `timekeeping` calibrated in M2.2 and
/// the m2 suite cross-checks every boot, so a client and the kernel
/// cannot disagree about how much time passed.
///
/// The status domain is signed and this is a count, so it saturates at
/// `i64::MAX` rather than ever returning a value that would read as an
/// error — about 292,000 years of uptime, which is not the bug anyone
/// will hit first.
fn sys_clock_now() -> Status {
    let us = crate::timekeeping::now_us();
    if us > i64::MAX as u64 {
        i64::MAX
    } else {
        us as Status
    }
}

/// SYS_TIMER_ARM(notif slot, badge, delay_us): deliver `badge` on that
/// notification once `delay_us` of monotonic time has passed. Returns
/// the timer id (>= 0).
///
/// Gated by the NOTIFICATION, not by a new capability kind: WRITE on
/// the notification is the notify side, exactly as `SYS_IRQ_RELAY`
/// requires. A process can only aim a timer at something it was
/// already trusted to signal.
fn sys_timer_arm(a0: u64, a1: u64, a2: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    let Ok(nid) = notification_of(pid, a0, crate::cap::RIGHTS_WRITE) else {
        return STATUS_BAD_ARG;
    };
    match crate::timer::arm(pid, nid, a1, a2) {
        // The id carries a generation in its high bits (ADR-0029
        // erratum): masked to 31 bits there, so it is always a
        // positive status and never collides with the error domain.
        Ok(id) => id as Status,
        Err(crate::timer::ArmError::Quota) => STATUS_QUOTA,
        Err(_) => STATUS_BUSY,
    }
}

/// SYS_TIMER_CANCEL(timer id): disarm a timer this process armed.
///
/// Cancelling a timer that already fired is an ERROR, not a silent
/// success. A protocol cancelling a retransmission that has in fact
/// already gone out needs to be able to tell the difference.
///
/// A STALE id — one whose slot has since been handed to another timer
/// — is refused for the same reason and with the same status. Ids
/// carry a generation precisely so that this case is distinguishable
/// from "you just cancelled a stranger's timer", which is what a bare
/// slot index would have silently done.
fn sys_timer_cancel(a0: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    match crate::timer::cancel(pid, a0) {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_BAD_ARG,
    }
}

// ---- driver substrate handlers (M5.1, ADR-0021) ----------------------------

/// `SYS_MAP_MEMORY`'s window allocator bounds: the kernel hands VAs out
/// linearly from 1 GiB in 2 MiB strides — far above any v1 image/stack
/// layout, far below the kernel-half bound. The stride keeps small
/// windows from sharing a 2 MiB block with unrelated mappings; the
/// limit contains a runaway scan.
const MMAP_BASE: u64 = 0x0000_0000_4000_0000;
const MMAP_STRIDE: u64 = 2 * 1024 * 1024;
const MMAP_LIMIT: u64 = 0x0000_0100_0000_0000;

/// SYS_ALLOC_FRAME(slot): take one physical frame from the kernel
/// allocator and mint an OWNED `Untyped` cap (full rights) in the
/// caller's `slot`, which must be empty. Returns the frame's physical
/// address as a positive payload (no secret: the holder is about to
/// map it). `STATUS_BUSY` when the allocator is exhausted or the slot
/// is occupied — on issuance failure the frame goes straight back
/// (no leak through the error path).
fn sys_alloc_frame(a0: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Some(phys) = crate::frames::alloc() else {
        return STATUS_BUSY; // frame allocator exhausted
    };
    let cap = crate::cap::Cap {
        obj: crate::cap::CapObj::Untyped { phys, owned: true },
        rights: crate::cap::RIGHTS_ALL,
    };
    match crate::cap::issue(pid, a0 as usize, cap) {
        Ok(()) => phys as Status,
        Err(_) => {
            // Rollback: the frame never left kernel ownership.
            if crate::frames::free(phys).is_err() {
                error!("syscall", "alloc_frame rollback: free of {phys:#x} refused");
            }
            STATUS_BUSY
        }
    }
}

/// SYS_MAP_MEMORY(slot, writable): map the memory cap in the caller's
/// own `slot` into the caller's OWN address space at a kernel-chosen VA
/// (self-map only — cross-process binding stays the `cap::map_memory`
/// invoke). Accepted kinds: `Untyped` (one frame; ownership TRANSFERS
/// to the address space and the cap slot goes empty — teardown reclaims
/// the frame) and `Mmio` (uncached, NX; the cap survives). Rights: the
/// access mode decides — `writable` needs WRITE, read-only needs READ —
/// plus DESTROY for Untyped.
/// Executable windows are never handed out. Returns the window VA as a
/// positive payload; `STATUS_BUSY` when the region table is full or no
/// VA is left under the limit.
fn sys_map_memory(a0: u64, a1: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let writable = a1 != 0;
    let Ok(c) = crate::cap::read(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    let (phys, pages, owned) = match c.obj {
        // A LENT Untyped cap (copy/IPC-landed) is refused: mapping
        // consumes the cap and hands the frame to this address space,
        // whose teardown would free a frame owned by ANOTHER cap
        // (ADR-0022 — ownership is never duplicated).
        crate::cap::CapObj::Untyped { phys, owned: true } => (phys, 1u32, true),
        crate::cap::CapObj::Mmio { phys, pages } => (phys, pages, false),
        _ => return STATUS_BAD_ARG, // only owned/device memory self-maps
    };
    if pages == 0 {
        return STATUS_BAD_ARG;
    }
    // The access mode decides the right (ADR-0021, same rule as the
    // cap::map_memory invoke): writable needs WRITE, read-only READ.
    let need = if writable {
        crate::cap::RIGHTS_WRITE
    } else {
        crate::cap::RIGHTS_READ
    };
    if c.rights & need == 0 {
        return STATUS_BAD_ARG;
    }
    if owned && c.rights & crate::cap::RIGHTS_DESTROY == 0 {
        return STATUS_BAD_ARG;
    }
    let Some(root) = crate::proc::pml4_of(pid) else {
        return STATUS_BAD_ARG; // own space must be live; defensive
    };
    let span = u64::from(pages) * crate::arch::x86_64::paging::PAGE;

    // Region-table capacity FIRST: a window that cannot be registered
    // must never be mapped (the dispatcher validates user pointers
    // against exactly that table — an unregistered window would be a
    // leak no legitimate syscall buffer can reach).
    if !crate::sched::current_user_regions()
        .iter()
        .any(|&(lo, hi)| lo == 0 && hi == 0)
    {
        return STATUS_BUSY;
    }

    // The kernel-chosen VA: first stride-aligned window overlapping
    // neither a registered region nor a live page-table leaf. Runs at
    // IF=0 (the stub's SFMASK guarantee), so scan → map → register is
    // one non-preemptible decision for this thread; two threads of one
    // process scanning on different CPUs is a documented v1 limitation
    // (ADR-0021 — driver processes are single-threaded).
    let regions = crate::sched::current_user_regions();
    let mut va = MMAP_BASE;
    let chosen = loop {
        let Some(end) = va.checked_add(span) else {
            return STATUS_BUSY;
        };
        if end > MMAP_LIMIT {
            return STATUS_BUSY;
        }
        let overlaps = regions
            .iter()
            .any(|&(lo, hi)| (lo != 0 || hi != 0) && va < hi && lo < end);
        // SAFETY: IF=0; `root` is this thread's own live address space
        // (the probe reads its tables through the kernel-view aliases).
        let mapped = (0..u64::from(pages))
            .any(|i| unsafe { crate::arch::x86_64::paging::user_va_mapped(root, va + i * 4096) });
        if !overlaps && !mapped {
            break va;
        }
        va += MMAP_STRIDE;
    };

    // Install the leaves: RAM frames through the normal user mapper,
    // device registers through the uncached/NX MMIO mapper (ADR-0021).
    for i in 0..u64::from(pages) {
        // SAFETY: IF=0; `root` is the caller's own live space; the VA
        // was just probed unmapped; phys comes from a validated cap.
        let r = unsafe {
            if owned {
                crate::arch::x86_64::paging::map_user_page_4k(
                    root,
                    chosen + i * 4096,
                    phys + i * 4096,
                    writable,
                    false,
                )
            } else {
                crate::arch::x86_64::paging::map_mmio_page_4k(
                    root,
                    chosen + i * 4096,
                    phys + i * 4096,
                    writable,
                )
            }
        };
        if r.is_err() {
            // Unreachable with kernel-chosen arguments (fresh VA,
            // aligned, never exec); if it ever fires, the window stays
            // partial and UNREGISTERED — unreachable to the caller and
            // accounted at teardown. Loud, not decorative.
            error!(
                "syscall",
                "map_memory: page {i} of window at {chosen:#x} refused"
            );
            return STATUS_BUSY;
        }
        // SAFETY: the VA belongs to this CPU's live address space;
        // invlpg drops any stale not-present caching of the fresh leaf.
        unsafe { super::invlpg(chosen + i * 4096) };
    }

    if let Err(e) = crate::sched::append_current_user_region(chosen, chosen + span) {
        // Unreachable under IF=0 (capacity and overlap were checked
        // against the same snapshot above); loud on the impossible.
        error!("syscall", "map_memory: region registration failed: {e}");
        return STATUS_BUSY;
    }
    if owned {
        // Ownership transferred: the frame now belongs to this address
        // space (teardown reclaims it); the cap slot goes empty.
        if let Err(e) = crate::cap::consume(pid, a0 as usize) {
            error!("syscall", "map_memory: untyped consume failed: {e}");
        }
    }
    info!(
        "syscall",
        "map_memory: pid {pid} window va={chosen:#x} pages={pages} writable={writable} mmio={}",
        !owned
    );
    chosen as Status
}

// ---- block-service substrate handlers (M5.2, ADR-0022) ----------------------

/// Number of u64 words `SYS_DEV_INFO` writes (the layout is frozen in
/// ADR-0022 and mirrored by `userspace/storaged`).
const DEV_INFO_WORDS: u64 = 12;

/// The device gate: resolve virtio record `dev_idx` and require that the
/// CALLER HOLDS an Mmio cap over the BAR carrying the device's virtio
/// structures — the proof that the kernel granted this process this
/// device (caps are the only authority; device indices are not
/// guessable secrets, but the gate keeps the surface honest). v1
/// requires all four structures on ONE BAR (the reference fixture puts
/// them all in BAR4); a multi-BAR layout is a typed refusal, documented
/// in ADR-0022. Returns (device, structure-bar index, function record).
fn virtio_for_caller(
    pid: u64,
    dev_idx: u64,
    required_rights: u32,
) -> Result<
    (
        crate::drivers::pci::VirtioDevice,
        usize,
        crate::drivers::pci::PciFunction,
    ),
    Status,
> {
    use crate::drivers::pci;
    let Some(v) = pci::virtio_device(dev_idx as usize) else {
        return Err(STATUS_BAD_ARG);
    };
    if !v.common.present
        || !v.notify.present
        || !v.isr.present
        || !v.device_cfg.present
        || v.notify.bar != v.common.bar
        || v.isr.bar != v.common.bar
        || v.device_cfg.bar != v.common.bar
    {
        return Err(STATUS_BAD_ARG); // absent or split across BARs (v1: one window)
    }
    let bar = v.common.bar as usize;
    let Some(f) = pci::pci_function(v.pci_index) else {
        return Err(STATUS_BAD_ARG);
    };
    if bar > 5 || f.bar_is_io[bar] || f.bar_base[bar] == 0 || f.bar_size[bar] == 0 {
        return Err(STATUS_BAD_ARG);
    }
    // ADR-0058: an untrusted PCI capability cannot describe registers
    // outside its recorded BAR or the caller's actually granted MMIO cap.
    // Device-info alone does not grant the right to map or dereference.
    let bounds = [
        (v.common, 0x38u32),
        (v.notify, 2),
        (v.isr, 1),
        (v.device_cfg, 1),
    ];
    let Some(last_byte) = bounds.iter().try_fold(0u64, |max_end, &(loc, minimum)| {
        if loc.length < minimum || loc.bar as usize != bar {
            return None;
        }
        let end = u64::from(loc.offset).checked_add(u64::from(loc.length))?;
        (end <= f.bar_size[bar]).then_some(max_end.max(end))
    }) else {
        return Err(STATUS_BAD_ARG);
    };
    // The gate itself: held cap, exact BAR base, requested right and
    // coverage of every capability span. A one-page prefix of a four-page
    // BAR is not authority to address its notify doorbell in page four.
    let base = f.bar_base[bar];
    let holds = (0..crate::cap::CAP_SLOTS).any(|slot| {
        crate::cap::read(pid, slot).is_ok_and(|c| {
            matches!(c.obj, crate::cap::CapObj::Mmio { phys, pages }
                if phys == base && u64::from(pages) * 4096 >= last_byte)
                && c.rights & required_rights == required_rights
        })
    });
    if !holds {
        return Err(STATUS_BAD_ARG);
    }
    Ok((v, bar, f))
}

/// SYS_DEV_INFO(dev_idx, buf): write the kernel's RESOLVED virtio record
/// for a granted device into the caller's buffer — `DEV_INFO_WORDS` u64
/// words, little-endian, the layout of ADR-0022:
/// `[0]`=pci_index, `[1]`=structure-BAR base, `[2..7]`=common/notify/isr/
/// device offset|length<<32 pairs plus the notify multiplier,
/// `[7]`=msix present|table_size<<32, `[8]`=device_id|transitional<<32.
/// Config space itself is never exposed — this record IS the ring-3 view
/// of the kernel's scan. Returns the word count as a positive payload.
fn sys_dev_info(a0: u64, a1: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    if !user_range_ok(a1, DEV_INFO_WORDS * 8) {
        return STATUS_BAD_ADDRESS;
    }
    let Ok((v, bar, f)) = virtio_for_caller(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    let loc =
        |c: crate::drivers::pci::VirtioCapLoc| u64::from(c.offset) | (u64::from(c.length) << 32);
    let words = [
        v.pci_index as u64,
        f.bar_base[bar],
        loc(v.common),
        loc(v.notify),
        u64::from(v.notify_off_multiplier),
        loc(v.isr),
        loc(v.device_cfg),
        u64::from(v.msix.present) | (u64::from(v.msix.table_size) << 32),
        u64::from(v.device_id) | (u64::from(v.transitional) << 32),
        0,
        0,
        0,
    ];
    // SAFETY: validated span against this thread's regions, own address
    // space live, STAC brackets the write, IF=0; the source is this
    // stack's own scratch.
    unsafe {
        super::stac();
        let p = a1 as *mut u64;
        for (i, w) in words.iter().enumerate() {
            core::ptr::write_volatile(p.add(i), *w);
        }
        super::clac();
    }
    DEV_INFO_WORDS as Status
}

/// SYS_IRQ_RELAY(dev_idx, msix_entry, notif_slot, badge): arm the
/// device-interrupt → notification bridge (ADR-0022). The kernel — the
/// only party that may touch config space and interrupt routing —
/// allocates a free relay vector (48..63), programs MSI-X table entry
/// `msix_entry` to deliver it to this CPU, enables MSI-X on the
/// function, and registers (vector → notification, badge) OWNED by the
/// calling process (a dead driver's relay dies with it, via
/// `proc::destroy`'s sweep). The caller needs WRITE on its notification
/// cap and the device gate of [`virtio_for_caller`]. Returns the vector
/// as a positive payload; `STATUS_BUSY` when all relay vectors are
/// armed or the device refused the programming.
fn sys_irq_relay(a0: u64, a1: u64, a2: u64, a3: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    // The notification the deliveries will wake: WRITE is the notify side.
    let Ok(nid) = notification_of(pid, a2, crate::cap::RIGHTS_WRITE) else {
        return STATUS_BAD_ARG;
    };
    if a3 == 0 {
        return STATUS_BAD_ARG; // the merged-badge protocol has no empty word
    }
    let Ok((v, _bar, f)) = virtio_for_caller(pid, a0, crate::cap::RIGHTS_WRITE) else {
        return STATUS_BAD_ARG;
    };
    if !v.msix.present || a1 >= u64::from(v.msix.table_size) {
        return STATUS_BAD_ARG;
    }
    let tbar = v.msix.table_bar as usize;
    if tbar > 5 || f.bar_is_io[tbar] || f.bar_base[tbar] == 0 {
        return STATUS_BAD_ARG;
    }
    // First free relay vector (the range itself belongs to idt.rs).
    let base = super::idt::RELAY_VECTOR_BASE as u64;
    let Some(vector) = (base..base + super::idt::RELAY_VECTOR_COUNT as u64)
        .find(|&vec| !crate::relay::registered(vec))
    else {
        return STATUS_BUSY; // all 16 relay vectors armed
    };
    let table_phys = f.bar_base[tbar] + u64::from(v.msix.table_offset);
    if let Err(e) = crate::drivers::pci::msix_write_entry(table_phys, a1 as u16, vector) {
        error!("syscall", "irq_relay: MSI-X entry programming failed: {e}");
        return STATUS_BUSY;
    }
    if let Err(e) = crate::drivers::pci::msix_enable(v.pci_index, v.msix.cap_ptr) {
        error!("syscall", "irq_relay: MSI-X enable failed: {e}");
        return STATUS_BUSY;
    }
    if let Err(e) = crate::relay::register_owned(vector, nid, a3, pid) {
        // Unreachable in practice (the vector was just probed free, the
        // badge is nonzero) — loud on the impossible, no half-armed state
        // matters: without the registration the vector is spurious-only.
        error!("syscall", "irq_relay: registration failed: {e}");
        return STATUS_BUSY;
    }
    info!(
        "syscall",
        "irq_relay: pid {pid} dev {a0} msix entry {a1} → vector {vector}, notification {nid} badge {a3:#x}"
    );
    vector as Status
}

/// ADR-0056: scalar geometry is disclosable only to the holder of the
/// *exact* GOP Mmio window; this never mints a mapping or a device cap.
fn sys_display_info(slot: u64, output: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if slot >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Some(d) = crate::handoff::display() else {
        return STATUS_BAD_ARG;
    };
    let Ok(cap) = crate::cap::read(pid, slot as usize) else {
        return STATUS_BAD_ARG;
    };
    let crate::cap::CapObj::Mmio { phys, pages } = cap.obj else {
        return STATUS_BAD_ARG;
    };
    if cap.rights & crate::cap::RIGHTS_READ == 0
        || phys != d.phys
        || u64::from(pages) * 4096 < d.bytes
    {
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(output, 40) {
        return STATUS_BAD_ADDRESS;
    }
    // SAFETY: validated caller-owned 40-byte region, five plain scalars;
    // no untrusted pointer is dereferenced without paired STAC/CLAC.
    unsafe {
        super::stac();
        let dst = output as *mut u64;
        for (i, value) in [
            u64::from(d.width),
            u64::from(d.height),
            u64::from(d.pitch_pixels),
            u64::from(d.format),
            d.bytes,
        ]
        .iter()
        .enumerate()
        {
            core::ptr::write_unaligned(dst.add(i), *value);
        }
        super::clac();
    }
    STATUS_OK
}

/// SYS_SHARED_CREATE(pool_slot, pages, out[3]): allocates one zeroed,
/// generation-safe object and mints its first cap. No numeric ID grants
/// authority; output contains (slot, full ID, byte length).
fn sys_shared_create(pool: u64, pages: u64, out: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if pool >= crate::cap::CAP_SLOTS as u64
        || pages == 0
        || pages > u64::from(crate::shared::MAX_PAGES)
    {
        return STATUS_BAD_ARG;
    }
    let Ok(authority) = crate::cap::read(pid, pool as usize) else {
        return STATUS_BAD_ARG;
    };
    if authority.obj != crate::cap::CapObj::MemoryPool
        || authority.rights & crate::cap::RIGHTS_WRITE == 0
    {
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(out, 24) {
        return STATUS_BAD_ADDRESS;
    }
    // All bounded capacity checks precede physical allocation. No staged
    // registry entry, ID or cap is consumed when the slot table is full.
    if !crate::cap::occupancy(pid).is_some_and(|(used, total)| used < total) {
        return STATUS_BUSY;
    }
    let Some((slot, id)) = crate::shared::create(pid, pages as u32) else {
        return STATUS_BUSY;
    };
    // SAFETY: full 24-byte caller-owned span prevalidated under IF=0.
    unsafe {
        super::stac();
        let dst = out as *mut u64;
        for (i, value) in [slot as u64, u64::from(id), pages * 4096]
            .iter()
            .enumerate()
        {
            core::ptr::write_unaligned(dst.add(i), *value);
        }
        super::clac();
    }
    STATUS_OK
}

/// SYS_SHARED_MAP(region_slot, writable): a single NX registered user
/// window. The bound is one region-table entry, not one entry per page.
fn sys_shared_map(slot: u64, writable: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if slot >= crate::cap::CAP_SLOTS as u64 || writable > 1 {
        return STATUS_BAD_ARG;
    }
    let Ok(cap) = crate::cap::read(pid, slot as usize) else {
        return STATUS_BAD_ARG;
    };
    let crate::cap::CapObj::SharedRegion { id } = cap.obj else {
        return STATUS_BAD_ARG;
    };
    let required = if writable == 1 {
        crate::cap::RIGHTS_WRITE
    } else {
        crate::cap::RIGHTS_READ
    };
    if cap.rights & required == 0 {
        return STATUS_BAD_ARG;
    }
    let Some((phys, pages)) = crate::shared::backing(id) else {
        return STATUS_BAD_ARG;
    };
    let Some(root) = crate::proc::pml4_of(pid) else {
        return STATUS_BAD_ARG;
    };
    let span = u64::from(pages) * 4096;
    // The 2MiB-aligned map slot uses at most one new PT per 2 MiB it
    // spans and two parent tables (a region above 2 MiB spans several
    // PTs; ADR-0075). IF=0 from preflight through publication; the
    // existing mapper's OOM assertion is unreachable with this reserve.
    let tables = span.div_ceil(MMAP_STRIDE) + 2;
    if !crate::shared::has_map_slot()
        || crate::frames::free_frames() < tables
        || !crate::sched::current_user_regions()
            .iter()
            .any(|&(lo, hi)| lo == 0 && hi == 0)
    {
        return STATUS_BUSY;
    }
    let regions = crate::sched::current_user_regions();
    let mut va = MMAP_BASE;
    // At most MAX_MAPS live shared maps plus USER_REGIONS_MAX windows can
    // occupy distinct strides. Never search an unbounded 1-TiB VA range
    // under IF=0 on malformed or adversarial input.
    let mut probes = 0usize;
    let chosen = loop {
        if probes >= crate::shared::MAX_MAPS + crate::sched::USER_REGIONS_MAX + 1 {
            return STATUS_BUSY;
        }
        probes += 1;
        let Some(end) = va.checked_add(span) else {
            return STATUS_BUSY;
        };
        if end > MMAP_LIMIT {
            return STATUS_BUSY;
        }
        let overlap = regions
            .iter()
            .any(|&(lo, hi)| (lo != 0 || hi != 0) && va < hi && lo < end);
        let used = (0..u64::from(pages))
            .any(|i| unsafe { crate::arch::x86_64::paging::user_va_mapped(root, va + i * 4096) });
        if !overlap && !used {
            break va;
        }
        va += MMAP_STRIDE;
    };
    for i in 0..u64::from(pages) {
        // SAFETY: exclusive IF=0, validated fresh VA and allocator-owned
        // contiguous physical range; NX regardless of cap rights.
        unsafe {
            crate::arch::x86_64::paging::map_user_page_4k(
                root,
                chosen + i * 4096,
                phys + i * 4096,
                writable == 1,
                false,
            )
            .unwrap_or_else(|_| crate::halt::halt_machine("SharedRegion map after preflight"));
            super::invlpg(chosen + i * 4096);
        }
    }
    crate::sched::append_current_user_region(chosen, chosen + span)
        .unwrap_or_else(|_| crate::halt::halt_machine("SharedRegion region after preflight"));
    crate::shared::pin_map(id, pid, chosen);
    chosen as Status
}

/// SYS_SHARED_UNMAP(exact_own_va, zero x5): remove only the exact
/// registry-backed self-map, not an arbitrary VA or another process's
/// mapping. Last-pin release follows PTE/TLB and region removal.
fn sys_shared_unmap(va: u64, reserved: [u64; 5]) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if va % 4096 != 0 || reserved != [0; 5] {
        return STATUS_BAD_ARG;
    }
    let Some((id, phys, pages)) = crate::shared::own_mapping(pid, va) else {
        return STATUS_BAD_ARG;
    };
    let Some(root) = crate::proc::pml4_of(pid) else {
        return STATUS_BAD_ARG;
    };
    let Some(span) = u64::from(pages).checked_mul(4096) else {
        return STATUS_BAD_ARG;
    };
    let Some(end) = va.checked_add(span) else {
        return STATUS_BAD_ARG;
    };
    if !crate::sched::current_user_regions()
        .iter()
        .any(|&(lo, hi)| lo == va && hi == end)
    {
        return STATUS_BAD_ARG;
    }
    // Entire run preflight precedes *any* PTE/region/pin mutation. A
    // corrupt or replaced leaf never permits partially unmapping a run.
    for i in 0..u64::from(pages) {
        let off = i * 4096;
        let Some(expected) = phys.checked_add(off) else {
            return STATUS_BAD_ARG;
        };
        // SAFETY: current process's root, exact registry record, IF=0.
        if !unsafe {
            crate::arch::x86_64::paging::shared_user_leaf_matches(root, va + off, expected)
        } {
            return STATUS_BAD_ARG;
        }
    }
    for i in 0..u64::from(pages) {
        let off = i * 4096;
        // SAFETY: complete prior preflight, single CPU/IF=0. A failure
        // here is an internal broken invariant, never a partial refusal.
        let removed =
            unsafe { crate::arch::x86_64::paging::unmap_shared_user_page(root, va + off) };
        if removed != Some(phys + off) {
            crate::halt::halt_machine("SharedRegion unmap diverged after preflight");
        }
    }
    crate::sched::remove_current_user_region(va, end).unwrap_or_else(|_| {
        crate::halt::halt_machine("SharedRegion user span changed during unmap")
    });
    crate::shared::unpin_own_mapping(pid, va, id);
    STATUS_OK
}

/// SYS_PROC_LIVE(slot): read-only check through a *held* Process/READ cap.
/// A still-recorded process whose last thread exited returns 0. A destroyed
/// process, wrong kind or insufficient right returns BAD_ARG; no numeric PID
/// supplied by a caller is ever treated as an authority witness.
fn sys_proc_live(slot: u64) -> Status {
    let Some(caller) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if slot >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(cap) = crate::cap::read(caller, slot as usize) else {
        return STATUS_BAD_ARG;
    };
    let crate::cap::CapObj::Process { pid: target } = cap.obj else {
        return STATUS_BAD_ARG;
    };
    if cap.rights & crate::cap::RIGHTS_READ == 0 || crate::proc::pml4_of(target).is_none() {
        return STATUS_BAD_ARG;
    }
    i64::from(crate::sched::proc_live_threads(target) != 0)
}

/// SYS_SHARED_INFO(region_slot, out[2], zero, zero, zero, zero):
/// descriptor-only bound, NOT physical backing or allocation authority.
/// Clients can determine the actual page count of a received region before
/// accepting untrusted surface geometry. Reserved args and the entire
/// caller output are validated before any write.
fn sys_shared_info(slot: u64, out: u64, reserved: [u64; 4]) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if slot >= crate::cap::CAP_SLOTS as u64 || reserved != [0; 4] {
        return STATUS_BAD_ARG;
    }
    let Ok(cap) = crate::cap::read(pid, slot as usize) else {
        return STATUS_BAD_ARG;
    };
    let crate::cap::CapObj::SharedRegion { id } = cap.obj else {
        return STATUS_BAD_ARG;
    };
    if cap.rights & crate::cap::RIGHTS_READ == 0 {
        return STATUS_BAD_ARG;
    }
    let Some((_, pages)) = crate::shared::backing(id) else {
        return STATUS_BAD_ARG;
    };
    if !user_range_ok(out, 16) {
        return STATUS_BAD_ADDRESS;
    }
    // SAFETY: one IF=0 owner-context write to a prevalidated 16-byte span;
    // neither the physical address nor a new bearer is disclosed.
    unsafe {
        super::stac();
        let dst = out as *mut u64;
        core::ptr::write_unaligned(dst, u64::from(id));
        core::ptr::write_unaligned(dst.add(1), u64::from(pages));
        super::clac();
    }
    STATUS_OK
}

/// DMA is a second, independent designation. The caller must hold both a
/// READ region reference and the display server's SharedDma/READ bearer.
fn sys_shared_phys(region: u64, dma: u64, out: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if region >= crate::cap::CAP_SLOTS as u64 || dma >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(rc) = crate::cap::read(pid, region as usize) else {
        return STATUS_BAD_ARG;
    };
    let Ok(dc) = crate::cap::read(pid, dma as usize) else {
        return STATUS_BAD_ARG;
    };
    let crate::cap::CapObj::SharedRegion { id } = rc.obj else {
        return STATUS_BAD_ARG;
    };
    if rc.rights & crate::cap::RIGHTS_READ == 0
        || dc.obj != crate::cap::CapObj::SharedDma
        || dc.rights & crate::cap::RIGHTS_READ == 0
    {
        return STATUS_BAD_ARG;
    }
    let Some((phys, pages)) = crate::shared::backing(id) else {
        return STATUS_BAD_ARG;
    };
    if !user_range_ok(out, 24) {
        return STATUS_BAD_ADDRESS;
    }
    // SAFETY: prevalidated caller output and two independent held caps.
    unsafe {
        super::stac();
        let dst = out as *mut u64;
        for (i, value) in [phys, u64::from(pages), u64::from(id)].iter().enumerate() {
            core::ptr::write_unaligned(dst.add(i), *value);
        }
        super::clac();
    }
    STATUS_OK
}

/// SYS_CAP_PHYS(slot): the physical address a memory-kind cap names
/// (`Untyped` owned OR lent, `Memory`, `Mmio`). Needs READ. This is the
/// zero-copy DMA seam: a driver points a device at a caller's buffer
/// through the phys of the LENT cap the caller handed over — never
/// through a bare number in a message word (which would let any process
/// aim a bus master at any frame). Returns the phys as a positive
/// payload.
fn sys_cap_phys(a0: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    match crate::cap::phys_of(pid, a0 as usize) {
        Ok(phys) => phys as Status,
        Err(_) => STATUS_BAD_ARG,
    }
}

/// SYS_CAP_DESCRIBE(slot, out): query only a cap the caller possesses.
/// `out` receives [kind, object id, rights] (three u64s). Object ids
/// are descriptive, never grant handles. Refuse other cap kinds — in
/// particular MMIO addresses are neither queryable nor mintable here.
/// Kind 1 = Image, 2 = Endpoint, 3 = Notification, 4 = Process.
fn sys_cap_describe(a0: u64, a1: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(a1, 24) {
        return STATUS_BAD_ADDRESS;
    }
    let Ok(cap) = crate::cap::read(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    let (kind, object) = match cap.obj {
        crate::cap::CapObj::Image { img_id }
            if img_id < crate::image_registry::FIRST || crate::image_registry::live(img_id) =>
        {
            (1u64, u64::from(img_id))
        }
        crate::cap::CapObj::ImageRegistrar if crate::image_registry::registrar_alive() => (5, 0),
        crate::cap::CapObj::BootImage { index } if crate::spawn::boot_image_live(index) => {
            (6, u64::from(index))
        }
        crate::cap::CapObj::SharedRegion { id } if crate::shared::backing(id).is_some() => {
            (7, u64::from(id))
        }
        // Held one-frame LENT type/geometry; no physical address is exposed.
        crate::cap::CapObj::Untyped { owned: false, .. } => (11, 1),
        crate::cap::CapObj::MemoryPool => (8, 0),
        crate::cap::CapObj::SharedDma => (9, 0),
        crate::cap::CapObj::ProofToken { id } if id != 0 => (10, id),
        crate::cap::CapObj::Endpoint { eid } => (2, u64::from(eid)),
        // ADR-0074: kind 12; the badge is the server's business, not shown.
        crate::cap::CapObj::BadgedEndpoint {
            eid, generation, ..
        } if crate::ipc::endpoint_generation(u32::from(eid)) == Some(generation) => {
            (12, u64::from(eid))
        }
        crate::cap::CapObj::Notification { nid } => (3, u64::from(nid)),
        crate::cap::CapObj::Process { pid: target } if crate::proc::pml4_of(target).is_some() => {
            (4, target)
        }
        _ => return STATUS_BAD_ARG,
    };
    // SAFETY: caller's live, validated 24-byte user buffer; STAC is
    // paired with CLAC while the kernel writes all three words.
    unsafe {
        super::stac();
        let out = a1 as *mut u64;
        core::ptr::write_volatile(out, kind);
        core::ptr::write_volatile(out.add(1), object);
        core::ptr::write_volatile(out.add(2), u64::from(cap.rights));
        super::clac();
    }
    STATUS_OK
}

/// SYS_PROC_FINISH(slot, mode): a Process cap with DESTROY, not a pid.
/// Mode 0 reaps only an exited child; mode 1 explicitly stops a live
/// child and then reaps it. Kernel-bootstrapped roots (including the
/// manager), supervised drivers waiting to restart, and self are never
/// targets: a live USER-child spawn record is required, but caller parent
/// identity is not. The spawn record, child address space and handle are all
/// retired, so repeated restarts cannot exhaust the bounded tables.
fn sys_proc_finish(a0: u64, a1: u64) -> Status {
    let Some(owner) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 || a1 > 1 {
        return STATUS_BAD_ARG;
    }
    let Ok(cap) = crate::cap::read(owner, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    let crate::cap::CapObj::Process { pid: target } = cap.obj else {
        return STATUS_BAD_ARG;
    };
    if cap.rights & crate::cap::RIGHTS_DESTROY == 0
        || target == owner
        || crate::supervise::owns_pid(target)
        || crate::proc::pml4_of(target).is_none()
        || !crate::spawn::has_user_child_record(target)
    {
        return STATUS_BAD_ARG;
    }
    let live_threads = crate::sched::proc_live_threads(target);
    if (a1 == 0 && live_threads != 0) || (a1 == 1 && live_threads == 0) {
        return STATUS_BUSY; // mode 1 is a genuine LIVE stop, not a dead reap
    }
    if a1 == 1 {
        info!(
            "syscall",
            "proc_finish mode1: owner {owner} target {target} live_threads={live_threads} held Process/DESTROY"
        );
    }
    if crate::proc::destroy(target).is_err() {
        return STATUS_BUSY;
    }
    if crate::spawn::forget(target).is_err() {
        return STATUS_BUSY;
    }
    if crate::cap::destroy(owner, a0 as usize).is_err() {
        return STATUS_BUSY;
    }
    STATUS_OK
}

/// SYS_RESOURCE_SNAPSHOT(power slot, output pointer): three scalar
/// counts [free frames, live spawn records, occupied process slots].
/// An observation is NOT lifecycle authority. Wrong-kind caps fail
/// before touching output; Power/WRITE is the existing admin gate.
fn sys_resource_snapshot(a0: u64, a1: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    let Ok(c) = crate::cap::read(pid, a0 as usize) else {
        return STATUS_BAD_ARG;
    };
    if !matches!(c.obj, crate::cap::CapObj::Power) || c.rights & crate::cap::RIGHTS_WRITE == 0 {
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(a1, 24) {
        return STATUS_BAD_ADDRESS;
    }
    let counts = crate::sync::without_interrupts(|| {
        [
            crate::frames::free_frames(),
            crate::spawn::records_snapshot().iter().flatten().count() as u64,
            crate::proc::live_count() as u64,
        ]
    });
    // SAFETY: output is the caller's validated, live 24-byte user span;
    // the SMAP window is paired in this synchronous syscall.
    unsafe {
        super::stac();
        let out = a1 as *mut u64;
        for (i, &value) in counts.iter().enumerate() {
            core::ptr::write_volatile(out.add(i), value);
        }
        super::clac();
    }
    STATUS_OK
}

/// Read-only pool diagnostics through a held MemoryPool/READ reference.
/// No physical addresses, handles or destructive authority are disclosed.
fn sys_observe(slot: u64, out: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(cap) = crate::cap::read(pid, slot as usize) else {
        return STATUS_BAD_ARG;
    };
    if cap.obj != crate::cap::CapObj::MemoryPool || cap.rights & crate::cap::RIGHTS_READ == 0 {
        return STATUS_BAD_ARG;
    }
    if !user_range_ok(out, 9 * 8) {
        return STATUS_BAD_ADDRESS;
    }
    let (regions, pages, maps) = crate::shared::usage_snapshot();
    let counts = [
        crate::frames::free_frames(),
        crate::frames::total_frames(),
        crate::spawn::records_snapshot().iter().flatten().count() as u64,
        crate::proc::live_count() as u64,
        regions as u64,
        pages as u64,
        maps as u64,
        crate::cap::occupancy(pid)
            .map(|(used, _)| u64::from(used))
            .unwrap_or(0),
        crate::timekeeping::now_us(),
    ];
    unsafe {
        super::stac();
        for (i, value) in counts.into_iter().enumerate() {
            core::ptr::write_unaligned((out as *mut u64).add(i), value);
        }
        super::clac();
    }
    STATUS_OK
}

/// SYS_CAP_DESTROY(slot): discard a cap reference (the ring-3 twin of
/// `cap::destroy`, DESTROY right required). An OWNED Untyped cap frees
/// its frame; a LENT reference (IPC-landed DMA buffer) just goes away —
/// the frame stays with its owner. `STATUS_OK` or a typed refusal.
fn sys_cap_destroy(a0: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    match crate::cap::destroy(pid, a0 as usize) {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_BAD_ARG,
    }
}

/// SYS_CAP_COPY(src, dst, rights): delegation by copy INSIDE the
/// caller's own space — attenuation-only (`cap::copy` refuses
/// amplification loudly), `dst` must be empty, the source needs COPY.
/// A copy of an Untyped cap is LENT (ownership never duplicates): the
/// pattern every zero-copy client uses — keep the owned cap, hand a
/// copy to the driver, keep the local mapping alive. `STATUS_OK` or a
/// typed refusal.
fn sys_cap_copy(a0: u64, a1: u64, a2: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    if a0 >= crate::cap::CAP_SLOTS as u64 || a1 >= crate::cap::CAP_SLOTS as u64 {
        return STATUS_BAD_ARG;
    }
    if a2 == 0 || a2 & !u64::from(crate::cap::RIGHTS_ALL) != 0 {
        return STATUS_BAD_ARG; // unknown right bits are a contract error
    }
    match crate::cap::copy(pid, a0 as usize, pid, a1 as usize, a2 as u32) {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_BAD_ARG,
    }
}

/// SYS_PROVE_RING3: arm a #GP expectation whose resume address is the
/// *recorded* user RIP + 1 — the payload's next instruction is a
/// privileged `cli`, which faults at CPL 3 and cannot fault at CPL 0.
/// The kernel (not user code) computes the resume address, extending the
/// M2.1 armed-fault protocol to ring-3 sites (ADR-0014).
fn sys_prove_ring3(frame: *const SyscallFrame) -> u64 {
    // SAFETY: frame is the stub-built struct on this kernel stack; the
    // hardware-saved user RIP points at the instruction after `syscall`
    // — the payload contract places the 1-byte `cli` exactly there.
    let usr_rip = unsafe { (*frame).usr_rip };
    crate::arch::x86_64::faults::arm_with_resume(13, usr_rip + 1);
    // SAFETY: single writer at IF=0.
    unsafe { (*STATS.get()).prove_ring3_calls += 1 };
    STATUS_OK as u64
}

/// SYS_PROVE_DONE: record whether the armed #GP was actually delivered
/// (the fault handler consumed the expectation and stored the facts).
/// Returns 1 when the proof holds, 0 otherwise; the test asserts both
/// this and the recorded vector/error-code.
fn sys_prove_done() -> u64 {
    let obs = crate::arch::x86_64::faults::observed();
    let proved = obs.valid && obs.vector == 13;
    // SAFETY: single writer at IF=0.
    unsafe {
        let st = &mut *STATS.get();
        if proved {
            st.ring3_proved = true;
            st.ring3_gp_ec = obs.error_code;
        }
    }
    if proved { 1 } else { 0 }
}

/// Page-granular validation of `[buf, buf+len)` against the calling
/// thread's registered user regions: every 4 KiB page the span touches
/// must lie entirely inside one region (spans crossing holes are
/// rejected, not faulted — ADR-0014).
fn user_range_ok(buf: u64, len: u64) -> bool {
    let regions = crate::sched::current_user_regions();
    let end = match buf.checked_add(len) {
        Some(e) => e,
        None => return false,
    };
    let mut page = buf & !0xFFF;
    while page < end {
        let page_end = page + 4096;
        let ok = regions
            .iter()
            .any(|&(lo, hi)| lo != 0 && page >= lo && page_end <= hi);
        if !ok {
            return false;
        }
        page = page_end;
    }
    true
}
