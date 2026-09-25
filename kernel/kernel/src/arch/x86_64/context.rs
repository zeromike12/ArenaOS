//! Thread context switch (M3.1, ADR-0012) — quarantined unsafe leaf.
//!
//! The switch frame is exactly the Win64 callee-saved set plus RFLAGS:
//! `rbx, rbp, rdi, rsi, r12–r15` (8 × 8 B) + `pushfq` (8 B) + the return
//! address the `call` pushed (8 B) = 80 bytes. Caller-saved registers
//! need no frame: `switch_context` is an ordinary Win64 call, so the
//! compiler already spilled everything live around it. No FPU/SSE state
//! is saved — the kernel image contains zero FPU/SSE/MMX instructions
//! (soft-float target, integer-only code) and `tools/build.sh` *fails the
//! build* if that invariant ever breaks (ADR-0012).
//!
//! A brand-new thread's first context is a synthesized image of the same
//! 80-byte frame (zeroed registers, RFLAGS=0x2 — reserved bit set, IF=0 —
//! return address = the trampoline), so the very first switch into it is
//! indistinguishable from a resume.

use core::arch::global_asm;

/// The complete saved state of a switched-out kernel thread: its stack
/// pointer. Everything else lives on that stack in the 80-byte frame
/// documented above. `0` = never switched out (a fresh thread carries its
/// synthesized frame RSP instead; the bootstrap thread's slot is filled
/// by its first switch).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    pub rsp: u64,
}

/// Size of the switch frame: 8 callee-saved registers + RFLAGS + the
/// return address ([`new_thread_stack`] writes all ten slots).
const FRAME_BYTES: u64 = 80;

/// RFLAGS image for a brand-new thread: bit 1 is reserved-one (SDM Vol. 1
/// §3.4.3), IF=0 per the kernel's interrupt discipline — the scheduler
/// controls when interrupts exist at all (3.2).
const INITIAL_RFLAGS: u64 = 0x2;

global_asm!(
    ".section .text",
    // ---- the switch ------------------------------------------------------
    // Win64/"C" arguments: rcx = save slot (*mut u64), rdx = restore RSP.
    // Both are raw values by contract (ADR-0012: no live borrow may cross
    // the switch — the scheduler decides, ends its borrow, then calls).
    // Push order defines the frame layout new_thread_stack synthesizes.
    ".p2align 4",
    ".globl arena_context_switch",
    "arena_context_switch:",
    "push rbx",
    "push rbp",
    "push rdi",
    "push rsi",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "pushfq",
    "mov [rcx], rsp",   // save outgoing stack
    "mov rsp, rdx",     // ... and become the incoming thread
    "popfq",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rsi",
    "pop rdi",
    "pop rbp",
    "pop rbx",
    "ret",              // resume: into the trampoline (first run) or the
                        // return address of the thread's own prior switch
    // ---- first-run trampoline --------------------------------------------
    // Entered by `ret` with RSP = the thread's stack top, 16-byte aligned
    // (the 80-byte frame preserves the alignment of a 16-aligned top).
    // Win64 wants RSP ≡ 0 (mod 16) *before* a call and 0x20 shadow space
    // for the callee; `sub rsp, 0x20` provides both. thread_main never
    // returns (it exits through the scheduler); the epilogue is dead code
    // kept so the unwinder-shaped frame stays well-formed.
    ".p2align 4",
    ".globl arena_thread_trampoline",
    "arena_thread_trampoline:",
    "sub rsp, 0x20",
    "call {thread_main}",
    "add rsp, 0x20",
    "ret",
    thread_main = sym crate::sched::thread_main,
);

unsafe extern "C" {
    /// The assembly switch (frame layout above). Arguments are raw by
    /// contract; neither may point into memory the other thread frees.
    fn arena_context_switch(save: *mut u64, restore: u64);
    /// First-run entry point synthesized into new thread frames.
    fn arena_thread_trampoline();
}

/// Perform a context switch: save this thread's RSP into `save`, load
/// `restore` and continue wherever that stack's frame says.
///
/// Returns twice — once per thread participation: the call "returns"
/// immediately in the *incoming* thread (resuming its own earlier switch)
/// and again in *this* thread whenever it is switched back to.
///
/// # Safety
/// - `save` must point to a writable u64 owned by the outgoing thread
///   (its context slot) and stay valid until that thread resumes.
/// - `restore` must be an RSP produced by a prior `switch_context` save
///   or by [`new_thread_stack`], on a stack that is mapped RW in the live
///   address space.
/// - Call with interrupts disabled (the scheduler's irqsave section):
///   between the decision and the switch no interrupt may observe a
///   half-switched scheduler state.
pub unsafe fn switch_context(save: *mut u64, restore: u64) {
    unsafe { arena_context_switch(save, restore) };
}

/// Synthesize a brand-new thread's first switch frame at the top of its
/// stack and return the RSP to hand to [`switch_context`].
///
/// Frame (ascending from the returned RSP): RFLAGS, r15, r14, r13, r12,
/// rsi, rdi, rbp, rbx (all zeroed), return address = the trampoline —
/// exactly what `arena_context_switch` would have left behind, so the
/// first switch-in runs the thread's entry through `thread_main`.
///
/// # Safety
/// `stack_top` must be the exclusive top of a ≥ [`FRAME_BYTES`]-byte,
/// 16-byte-aligned, mapped-RW region owned by the new thread.
pub unsafe fn new_thread_stack(stack_top: u64) -> u64 {
    let rsp = stack_top - FRAME_BYTES;
    // SAFETY: caller contract — the frame region is ours, aligned, RW.
    unsafe {
        let slot = |i: u64| (rsp + i * 8) as *mut u64;
        *slot(0) = INITIAL_RFLAGS; // popfq
        for i in 1..=8 {
            *slot(i) = 0; // pop r15..rbx
        }
        *slot(9) = arena_thread_trampoline as *const () as u64; // ret
    }
    rsp
}

/// Address of the trampoline (for diagnostics/tests: the synthesized
/// return address must equal it).
pub fn trampoline_addr() -> u64 {
    arena_thread_trampoline as *const () as u64
}
