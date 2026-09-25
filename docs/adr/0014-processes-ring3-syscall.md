# ADR-0014: Processes, ring 3, and the syscall boundary

Status: accepted (M3.3, 2026-09)

## Problem

Milestone 3.3 turns the thread machinery (ADR-0012/0013) into an OS with a
kernel/user boundary. Concretely, the roadmap demands:

* **Processes** as first-class objects: an address space (own page tables)
  plus — from 3.4 — a capability space. Threads belong to exactly one
  process; no `fork`, no signals (deliberate non-goals, ARCHITECTURE §5).
* **The privilege machinery**: ring-3 code segments, the ring-3→ring-0
  stack switch (TSS RSP0), a fast system-call path (`syscall`/`sysret`
  MSRs: STAR/LSTAR/SFMASK, EFER.SCE), and SMAP/SMEP when the CPU offers
  them.
* **Proof it is real**: actual ring-3 instructions executing, actual
  system calls crossing the boundary in both directions, actual address
  isolation between two processes at the *same* virtual addresses, and
  interrupts/preemption landing on ring-3 code without breaking any of
  it. A "user mode" that is really kernel code in disguise is exactly
  the fake functionality this project forbids.

Constraints inherited from earlier decisions: no userspace binary format
or loader yet (4.1), single core (SMP structures only, M5), the IF=0
scheduler invariant (ADR-0013), W^X everywhere we own the mapping
(ADR-0008), exact frame accounting (the M3 suites assert it), and the
"old tests never deleted" regression contract (ADR-0005).

## Approaches considered

### 1. System-call gate: `syscall`/`sysret` vs `int 0x80`-style gate vs `sysenter`

* **Interrupt/Trap gate (`int n`)**: works everywhere, pushes a full
  iretq frame, costs ~2× a modern `syscall` (gate lookup, more pushes),
  and the return path (`iretq`) is slow. It also cannot mask flags
  atomically on entry the way SFMASK does (an `int` gate keeps the
  user's IF unless the gate type is *interrupt* — workable, but every
  knob is separate).
* **`sysenter`/`sysexit`**: the 32-bit-era fast path; on x86-64 its
  64-bit mode support is awkward (it targets compat mode; Linux uses it
  only for 32-bit userlands). We have no 32-bit userland and never
  plan one (ARCHITECTURE: 64-bit only).
* **`syscall`/`sysret` (selected)**: the canonical AMD64 path: one MSR
  programmed at boot (EFER.SCE), entry at LSTAR with RCX/R11 holding
  the user's RIP/RFLAGS, SFMASK clearing IF (and AC/TF/DF/NT)
  atomically on entry, `sysretq` restoring everything from the same two
  registers. Universally implemented on every x86-64 CPU that can run
  this kernel.

Known `sysretq` sharp edge (documented, defended): a non-canonical RCX
at `sysretq` faults **in ring 0 with user-controlled state** (the
historic CVE class). Our return path only ever restores RIPs the kernel
itself recorded when entering user mode — user code never supplies the
return target — so the class is closed by construction, and the entry
code records the canonical check obligation.

### 2. Finding the kernel stack at syscall entry: GS scratch vs IST vs per-CPU TSS

`syscall` does **not** switch stacks or push anything: entry code must
find a kernel RSP with nothing but the MSRs. Options:

* **IST for the syscall gate**: IST belongs to exceptions that must
  survive broken stacks; `syscall` arrives on a healthy user stack and
  IST would bypass RSP0 semantics for no gain (and IST entries are a
  scarce, special-purpose resource we already reserve for #DF/NMI/#MC).
* **`IA32_TSC_AUX`/`RDGSBASE`**: needs a CPUID feature bit not present
  on our baseline CPU model.
* **`swapgs` + per-CPU scratch (selected)**: `MSR_KERNEL_GS_BASE`
  points at `CpuScratch { user_rsp, kernel_rsp }` for this CPU; entry is
  `swapgs; mov [gs:0], rsp; mov rsp, [gs:8]`. `kernel_rsp` is the
  *current thread's* kernel stack top and is maintained by the
  scheduler itself — the same write that programs TSS RSP0 in
  `plan_switch`, so the two can never disagree. All user threads run
  with GS.base = 0, so no per-thread GS save is needed; the scratch is
  per-CPU and never changes identity.

TSS **RSP0** remains the hardware path for *interrupts and exceptions*
that land while in ring 3 (the CPU loads RSP0 for any privilege-level
stack switch). RSP0 = current thread's kernel stack top, updated in
`plan_switch` for every incoming thread that owns a stack. Consequence,
accepted: **syscalls run entirely at IF=0** (SFMASK clears IF; the
handler never re-enables it). A syscall therefore can never be
preempted mid-handler, which keeps the GS scratch and RSP0 single-valued
and removes any need to make the syscall frame preemptible. When
blocking syscalls arrive (M4+), the handler may sleep *explicitly*
through the scheduler, which updates RSP0/scratch on every switch
anyway — the invariant survives.

### 3. Entering ring 3 the first time: `iretq` vs `sysretq`

`sysretq` cannot *start* a user context (it has no way to set a new RSP
and its RFLAGS come from a register, not memory) — every OS uses an
`iretq` frame for first entry and `sysretq` for syscall returns. We do
the same: `enter_user` pushes SS/RSP/RFLAGS/CS/RIP with the user
selectors and RFLAGS = IF=1 — user code is interruptible from its first
instruction, which the M3 suite *proves* by landing timer ticks (and
therefore preemption, including switches away and back) on a spinning
ring-3 payload.

### 4. User address spaces: per-process PML4 with shared kernel half

Standard and forced: each process gets its own PML4; entries 256–511
(the kernel half: direct map, image window, MMIO aliases) are *shared*
by copying the kernel view's entries at creation, so a ring-3→ring-0
transition never needs a CR3 switch and kernel code running inside a
process context sees exactly one kernel address space. User half
(entries 0–255) starts empty; the process's code/data/stack pages are
mapped there with U/S, W^X enforced (code R+X, data/stack RW+NX).

`plan_switch` loads the incoming thread's CR3 when it differs from the
live one. The switch itself only touches kernel-half memory (thread
stacks live in the direct map; the save slots live in the image), so
the CR3 write *before* the assembly switch is safe under every view —
and the outgoing thread's state needs no TLB care because the next
entry reloads CR3 anyway (no PCID in 3.3; PCID is a later optimization,
noted under implications).

Two processes running the *same payload at the same VAs* with different
data-page contents is the isolation test: each syscall's `SYS_WRITE`
copies from *the calling process's* address space, so the kernel-side
captured buffers differing per process is machine-checked proof that
the VA→PA translation is per-process. Teardown walks the user half and
frees every leaf page, every intermediate table frame, and the PML4
itself — the accounting test asserts `free_frames()` returns exactly to
baseline.

The M3.3a machinery slice runs its ring-3 tests on user pages mapped
temporarily into the kernel's *own* view (with exact unmap-and-free
accounting at test end) so the privilege machinery can be proven
before, and independently of, the process-object layer.

### 5. SMAP/SMEP: enable when present, prove when enabled

Runtime detection (CPUID.7:EBX[20:21]); CR4.SMEP/CR4.SMAP set when
available, and the reference QEMU recipe gains `+smap,+smep` so the
enforcement is *exercised*, not just compiled. Kernel access to user
memory (the `SYS_WRITE` copy) is wrapped in `stac`/`clac` — no-ops when
SMAP is absent (the instructions would #UD). With SMAP live, an armed
fault test proves a kernel read of a user page *without* AC faults
(#PF, CR2 = the user VA) and recovers. SMEP needs no code-path change:
the kernel never executes user pages, and its presence is logged as
read-back evidence.

### 6. Syscall ABI (ours, deliberately not POSIX)

`rax` = number, `rdi`/`rsi`/`rdx` = up to three arguments, return in
`rax` (`-1` = rejected). All caller-saved registers clobber (the C ABI
does it for us); `rcx`/`r11` are consumed by the hardware. The 3.3
surface is minimal and real:

* `1 SYS_WRITE(buf, len)` — validate `[buf, buf+len)` page-by-page
  against the calling thread's registered user regions (≤ 4 KiB), copy
  under STAC, write the raw bytes to the serial console, return `len`.
  Validation is per *mapped region*, not one flat range: a span crossing
  an unmapped hole must be rejected, not faulted.
* `2 SYS_EXIT(status)` — record `(thread id, status)`, terminate through
  the scheduler's normal zombie/reap path. Never returns. **GS
  discipline:** because it diverges, the stub's exit-side `swapgs` is
  never reached — `SYS_EXIT` restores the canonical user-side GS state
  (`GS.base = 0`, `KERNEL_GS_BASE = scratch`) itself before handing off
  to the scheduler. Bring-up proved why: skipping it left the machine on
  the kernel side of the swap, and the *next* syscall's entry `swapgs`
  landed backwards — `mov gs:[0], rsp` wrote through `GS.base = 0`
  (#PF), whose exception frame then tried to push onto the still-loaded
  user stack, which SMAP correctly blocked (nested #PF → #DF → triple
  fault, all captured with `qemu -d int`). Any future syscall that
  diverges from the stub inherits this obligation.

Unknown numbers return `-1` and are counted; the test payload
*deliberately* issues one and checks the `-1` in ring 3 (rejection is
part of the contract, proven from the user side).

## Decision

`syscall`/`sysret` with SFMASK = TF|DF|IF|NT|AC; STAR wiring kernel CS
0x08/SS 0x10 and user base 0x20 (GDT: `udata` at 0x28, `ucode` at
0x30, appended *after* the TSS descriptor pair so no existing selector
moves); LSTAR → a hand-written entry stub that finds the kernel stack
via `swapgs` + per-CPU scratch, builds a `SyscallFrame { usr_rsp,
usr_rip, usr_rflags }` plus Win64 shadow space, and calls a Rust
dispatcher; `sysretq` returns with the recorded RIP/RFLAGS. TSS RSP0
and the scratch `kernel_rsp` are written together by `plan_switch` on
every switch to a stack-owning thread. First entry to ring 3 is an
`iretq` frame built by `enter_user` (RFLAGS IF=1 — user code is
interruptible and preemptible from instruction one). Processes own a
PML4 whose kernel half is cloned from the kernel view; `plan_switch`
switches CR3 only when the incoming thread's address space differs from
the live one; teardown frees the user half exactly. SMAP/SMEP enabled
when present (reference recipe: present), STAC/CLAC around every kernel
touch of user memory. Syscalls run at IF=0 end-to-end.

Because user threads made IF=1 execution the normal case for real work,
two global-state fixes ride along, both required for correctness under
preemption:

* the **frame allocator** (plain `SyncCell` state, previously protected
  only by the boot contract's IF=0) wraps its public operations in
  `without_interrupts` — a tick between a bitmap read and write would
  corrupt allocation state;
* new threads' first-run frames already carry IF=1 (ADR-0013's
  amendment of ADR-0012); this ADR relies on it.

## Downsides accepted

* **Syscalls are non-preemptible (IF=0)**: a long-running syscall
  delays ticks for its whole duration. Acceptable while syscalls are
  microseconds long; the explicit-sleep path (blocking syscalls, M4+)
  is where preemption returns, via the scheduler rather than by
  re-enabling IF inside handler bodies.
* **No PCID**: every CR3 switch flushes the TLB. On a single-core
  milestone VM this is invisible; PCID becomes worth it when process
  switch frequency rises (M4+), and the per-process-PML4 layout
  already matches what PCID needs.
* **GS.base = 0 in user mode**: userspace gets no GS-based TLS yet.
  TLS arrives with the C runtime story (M6+); the scratch-MSR design
  does not constrain it (KERNEL_GS_BASE is ours; a future swapgs-aware
  scheduler can carry a per-thread user GS value).
* **The entry stub trusts its origin is ring 3** (`swapgs`
  unconditional). A ring-0 `syscall` would corrupt the scratch
  swap-slot and fault loudly; the kernel contains no such call, and the
  convention is documented at the stub. (A saved-CS RPL check is a
  one-instruction hardening if that ever changes.)
* **Two hand-encoded ring-3 payloads** (byte arrays assembled by the
  test itself) are the "userspace" until 4.1's loader exists. They are
  real ring-3 instructions executed by the CPU — the milestone's whole
  point — but they are test fixtures, not a user programming surface.
* **Cloned kernel half is a snapshot**: kernel-half PML4 entries copied
  at process creation would miss *later* kernel-half changes. Today the
  kernel half is static after M2.7 (no runtime kernel mapping changes),
  and the invariant is documented where it is enforced; if the kernel
  half ever becomes dynamic, sharing the PML4 *entries* (not copying)
  or a shootdown protocol becomes mandatory.

## Future implications

* 3.4 capability spaces attach to the `Process` object created here —
  the struct is the anchor the roadmap promised ("address space +
  capability space").
* M4's loader replaces the hand-encoded payloads: ELF parsing, mapping
  segments into `build_process_view`, and setting the entry RIP — every
  piece of machinery it needs (user page mapping, region registration,
  enter_user, syscall dispatch) exists after this step.
* The per-thread `regions` table becomes the process's VMA list when
  demand paging / mmap-like calls arrive; the page-granular validation
  rule (reject spans crossing holes) is already the right semantics.
* SMP (M5): the scratch, RSP0 writes, and CR3 loads are all per-CPU or
  per-switch already; the missing piece is TLB shootdown for
  cross-CPU address-space changes (and a real answer for the cloned
  kernel half).
