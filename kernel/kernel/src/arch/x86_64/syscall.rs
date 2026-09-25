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
//! The syscall ABI (ours, deliberately not POSIX): RAX = number,
//! RDI/RSI/RDX = up to three arguments, result in RAX, `-1` (all ones) =
//! rejected. All caller-saved registers clobber; RCX/R11 are consumed by
//! the hardware. The 3.3 surface: `SYS_WRITE` (validated, SMAP-aware
//! copy-out to the serial console), `SYS_EXIT` (terminate through the
//! scheduler), and the two ring-3 *proof* calls the M3 suite uses
//! (`SYS_PROVE_RING3` arms a #GP expectation at the user's next
//! instruction — a privileged `cli` the payload then executes faults at
//! CPL 3 and resumes, which cannot happen at CPL 0; `SYS_PROVE_DONE`
//! records whether that fault was actually observed).

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

// ---- syscall numbers (ABI v0 — ADR-0014) ----------------------------------

pub const SYS_WRITE: u64 = 1;
pub const SYS_EXIT: u64 = 2;
pub const SYS_PROVE_RING3: u64 = 4;
pub const SYS_PROVE_DONE: u64 = 5;

/// Largest `SYS_WRITE` the dispatcher accepts (bytes). The console is a
/// diagnostic surface in M3; a real byte-stream API arrives with the FS
/// milestone.
const WRITE_MAX: u64 = 256;

/// The value returned to user code for a rejected syscall.
const SYS_REJECTED: u64 = u64::MAX;

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

/// Most recent SYS_WRITE payload (first bytes + length) — the test side
/// asserts on the *copied* buffer, not on serial text.
static WRITE_BUF: SyncCell<[u8; WRITE_MAX as usize]> = SyncCell::new([0; WRITE_MAX as usize]);
static WRITE_BUF_LEN: SyncCell<usize> = SyncCell::new(0);

pub fn last_write() -> ([u8; WRITE_MAX as usize], usize) {
    // SAFETY: copy-out; SyncCell contract (writes are IF=0 dispatcher-side).
    unsafe { (*WRITE_BUF.get(), *WRITE_BUF_LEN.get()) }
}

/// (thread id, exit status) pairs recorded by SYS_EXIT, in order.
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
/// `SYS_EXIT`.
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
    // Stack discipline: kernel_rsp is the thread's stack top (16-aligned).
    // Pushes: frame (3 qwords) + 5th arg (1) + Win64 shadow (4) = 8 qwords,
    // so RSP is 16-aligned at the `call` and the callee sees the standard
    // [ret][shadow][arg5] layout. SFMASK already cleared IF — the whole
    // path is non-preemptible.
    ".p2align 4",
    ".globl arena_syscall_entry",
    "arena_syscall_entry:",
    "swapgs",
    "mov gs:[0], rsp",          // stash the user RSP
    "mov rsp, gs:[8]",          // become the kernel stack (top)
    "push r11",                 // frame: user RFLAGS
    "push rcx",                 //        user RIP
    "mov rcx, gs:[0]",
    "push rcx",                 //        user RSP   (frame base = RSP now)
    "mov rcx, rsp",
    "push rcx",                 // 5th arg: &SyscallFrame
    "sub rsp, 32",              // Win64 shadow space
    "mov r9, rdx",              // a2 (before rdx is overwritten)
    "mov rcx, rax",             // nr
    "mov rdx, rdi",             // a0
    "mov r8, rsi",              // a1
    "call {dispatch}",
    // RAX = user-visible result. Tear down: skip shadow + arg5, reload the
    // recorded user state (never a user-supplied target — ADR-0014), leave
    // the kernel stack, swap back, and sysretq.
    "add rsp, 40",
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
/// result in RAX. Runs at IF=0 on the calling thread's kernel stack with
/// the frame the stub built. SYS_EXIT diverges through the scheduler.
extern "C" fn syscall_dispatch(
    nr: u64,
    a0: u64,
    a1: u64,
    _a2: u64,
    frame: *const SyscallFrame,
) -> u64 {
    // SAFETY: stats writes are single-writer at IF=0 (SyncCell contract);
    // the frame pointer came from the stub and lives on this stack.
    unsafe {
        (*STATS.get()).calls += 1;
    }
    match nr {
        SYS_WRITE => sys_write(a0, a1),
        SYS_EXIT => sys_exit(a0),
        SYS_PROVE_RING3 => sys_prove_ring3(frame),
        SYS_PROVE_DONE => sys_prove_done(),
        _ => {
            // SAFETY: as above.
            unsafe { (*STATS.get()).invalid_nr += 1 };
            SYS_REJECTED
        }
    }
}

/// SYS_WRITE(buf, len): copy `buf[..len]` from the *calling thread's*
/// address space (SMAP-aware) to the serial console, raw. Validation is
/// per page against the thread's registered user regions — a span
/// crossing an unmapped hole is rejected, never faulted (ADR-0014).
fn sys_write(buf: u64, len: u64) -> u64 {
    // SAFETY: stats/record writes single-writer at IF=0.
    unsafe {
        (*STATS.get()).write_calls += 1;
    }
    // (buf needs no alignment — byte streams may start anywhere; only
    // the page-span check below matters.)
    if len == 0 || len > WRITE_MAX || buf == 0 {
        // SAFETY: as above.
        unsafe { (*STATS.get()).write_rejected += 1 };
        return SYS_REJECTED;
    }
    if !user_range_ok(buf, len) {
        // SAFETY: as above.
        unsafe { (*STATS.get()).write_rejected += 1 };
        return SYS_REJECTED;
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
    n as u64
}

/// SYS_EXIT(status): record (id, status), then terminate through the
/// scheduler's normal zombie/reap path. Diverges — the kernel stack frame
/// this call sits on dies with the thread (the stub never resumes).
fn sys_exit(status: u64) -> ! {
    // SAFETY: single writer at IF=0.
    unsafe {
        (*STATS.get()).exit_calls += 1;
    }
    let id = crate::sched::current_thread_id();
    record_exit(id, status);
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
    0
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
