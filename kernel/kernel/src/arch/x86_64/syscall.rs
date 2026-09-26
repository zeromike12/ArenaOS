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
//! notification flags). The spawn protocol (M4.5, ADR-0019) adds
//! `SYS_SPAWN`: build a process from an image capability, hand the
//! child an explicit, attenuated inheritance list, register an exit
//! badge, and start its first thread at the image entry. The console
//! and shell step (M4.6, ADR-0020) adds `SYS_CONSOLE_READ` (blocking
//! line read from the kernel's console input service), `SYS_PROC_LIST`
//! (the live process table's public shape, for `ps`), and
//! `SYS_SHUTDOWN` (machine halt, gated on a Power capability).

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

/// (thread id, exit status) pairs recorded by SYS_THREAD_EXIT, in order.
const EXIT_LOG_CAP: usize = 16;
static EXIT_LOG: SyncCell<[(u64, u64); EXIT_LOG_CAP]> = SyncCell::new([(0, 0); EXIT_LOG_CAP]);
static EXIT_LOG_LEN: SyncCell<usize> = SyncCell::new(0);

/// The status thread `id` exited with (None = not exited / log overflowed
/// before recording — the suite sizes its runs far below the cap).
pub fn exit_status_of(id: u64) -> Option<u64> {
    // SAFETY: read-only; SyncCell contract.
    unsafe {
        let len = *EXIT_LOG_LEN.get();
        for i in 0..len {
            let (tid, status) = (*EXIT_LOG.get())[i];
            if tid == id {
                return Some(status);
            }
        }
        None
    }
}

fn record_exit(id: u64, status: u64) {
    // SAFETY: single writer at IF=0 (dispatcher); capped log.
    unsafe {
        let len = &mut *EXIT_LOG_LEN.get();
        if *len < EXIT_LOG_CAP {
            (*EXIT_LOG.get())[*len] = (id, status);
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
    match nr {
        SYS_DEBUG_WRITE => sys_debug_write(a0, a1) as u64,
        SYS_THREAD_EXIT => sys_thread_exit(a0),
        SYS_PROVE_RING3 => sys_prove_ring3(frame),
        SYS_PROVE_DONE => sys_prove_done(),
        SYS_ABI_ECHO6 => {
            // SAFETY: as above.
            unsafe { (*STATS.get()).echo_calls += 1 };
            echo6_fingerprint([a0, a1, a2, a3, a4, a5])
        }
        SYS_IPC_CALL => sys_ipc_call(a0, a1, a2, a3, a4) as u64,
        SYS_IPC_RECV => sys_ipc_recv(a0, a1) as u64,
        SYS_IPC_REPLY => sys_ipc_reply(a0, a1, a2, a3) as u64,
        SYS_NOTIFY => sys_notify(a0, a1) as u64,
        SYS_WAIT => sys_wait(a0) as u64,
        SYS_SPAWN => sys_spawn(a0, a1, a2, a3, a4) as u64,
        SYS_CONSOLE_READ => sys_console_read(a0, a1) as u64,
        SYS_PROC_LIST => sys_proc_list(a0, a1) as u64,
        SYS_SHUTDOWN => sys_shutdown(a0) as u64,
        _ => {
            // SAFETY: as above.
            unsafe { (*STATS.get()).invalid_nr += 1 };
            STATUS_BAD_CALL as u64
        }
    }
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
    let id = crate::sched::current_thread_id();
    record_exit(id, status);
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

/// SYS_IPC_CALL(ep slot, w0, w1, send-cap slot, reply buf): block until
/// the server replies; the reply lands in the buffer as
/// `[w0, w1, cap-slot-or-CAP_NONE]`. Needs WRITE on the endpoint cap.
fn sys_ipc_call(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG; // kernel threads have no cap space
    };
    let Ok(eid) = endpoint_of(pid, a0, crate::cap::RIGHTS_WRITE) else {
        return STATUS_BAD_ARG;
    };
    let Ok(send_cap) = send_cap_of(pid, a3) else {
        return STATUS_BAD_ARG;
    };
    if !user_range_ok(a4, IPC_BUF_BYTES) {
        return STATUS_BAD_ADDRESS;
    }
    match crate::ipc::call(pid, eid, [a1, a2], send_cap) {
        Ok((words, landed)) => {
            // SAFETY: validated above; `ipc::call` resumes in THIS
            // thread's own context and address space (ADR-0018: user
            // memory is only touched by its owner).
            unsafe { write_user_words(a4, [words[0], words[1], landed]) };
            STATUS_OK
        }
        Err(e) => e,
    }
}

/// SYS_IPC_RECV(ep slot, buf): take the oldest request, blocking while
/// the queue is empty; buf receives `[w0, w1, landed-cap-slot]`. Needs
/// READ on the endpoint cap.
fn sys_ipc_recv(a0: u64, a1: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(eid) = endpoint_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    if !user_range_ok(a1, IPC_BUF_BYTES) {
        return STATUS_BAD_ADDRESS;
    }
    match crate::ipc::recv(pid, eid) {
        Ok((words, landed)) => {
            // SAFETY: as in sys_ipc_call — own context, validated range.
            unsafe { write_user_words(a1, [words[0], words[1], landed]) };
            STATUS_OK
        }
        Err(e) => e,
    }
}

/// SYS_IPC_REPLY(ep slot, w0, w1, send-cap slot): stage the reply into
/// this server's delivered slot and wake the caller. Needs READ.
fn sys_ipc_reply(a0: u64, a1: u64, a2: u64, a3: u64) -> Status {
    let Some(pid) = crate::sched::current_proc_id() else {
        return STATUS_BAD_ARG;
    };
    let Ok(eid) = endpoint_of(pid, a0, crate::cap::RIGHTS_READ) else {
        return STATUS_BAD_ARG;
    };
    let Ok(send_cap) = send_cap_of(pid, a3) else {
        return STATUS_BAD_ARG;
    };
    match crate::ipc::reply(eid, [a1, a2], send_cap) {
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

/// SYS_SPAWN(image slot, spec ptr, spec count, notif slot | CAP_NONE,
/// badge): the spawn protocol of ADR-0019 — validate the image cap and
/// the inheritance spec (an array of `(src_slot, rights)` pairs in the
/// caller's memory, at most `spawn::MAX_INHERIT`), then hand the whole
/// creation sequence to `spawn::spawn_from`. Returns the child's pid as
/// a positive payload. The child's Process cap lands in the caller's
/// first free slot.
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
    let crate::cap::CapObj::Image { img_id } = img_cap.obj else {
        return STATUS_BAD_ARG;
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
    match crate::spawn::spawn_from(pid, img_id, &spec[..n], notif) {
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
