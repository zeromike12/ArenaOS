# ADR-0020: Console input and the minimal shell

- Status: accepted
- Date: 2026-09-26
- Milestone: M4.6 (ROADMAP phase 4 — "Userspace & first program")
- Supersedes: — (extends ADR-0017 syscall registry, ADR-0019 spawn protocol)

## Problem

Through M4.5 the system only ever talks *outbound*: the 16550 console is
polled TX, and every ring-3 program is a fixed test artifact started by
the suite. There is no way for a human (or a test harness) to say
something *to* the running system, and no program whose behavior is
driven by input. Milestone 4's last step demands the first "real"
userspace surface: a minimal shell — line input from a console service,
builtins (`help`, `ps`, `echo`, `shutdown`), and spawning images through
the M4.5 protocol.

Four sub-problems, each needing a decision:

1. **Where does input come from?** The machine has two candidate input
   devices in scope: the serial port (already owned for output) and the
   PS/2 keyboard (not yet driven at all).
2. **Who owns the line discipline?** Kernel subsystem vs. userspace
   console/keyboard server over IPC.
3. **How does a program read a line?** New syscall surface (registry
   slots 13+), including how blocking interacts with the scheduler, and
   what happens with concurrent readers.
4. **Who may turn the machine off, and what does the boot flow become?**
   Today the bootstrap thread halts the machine after the suites. A
   living system must instead hand over to an initial service — and
   `shutdown` must remain an authority, not a public verb.

## Decision

### 1. Input device v1: COM1 serial RX, interrupt-driven (IRQ4 → IOAPIC pin 4 → vector 33)

The console a user already watches is the console they type into: with
QEMU's `-serial mon:stdio` (what `tools/run.sh` already uses), output
and input share one terminal, headless-friendly, no second device
driver. The kernel enables the UART's RX interrupt (IER.ERBFI) after
ExitBootServices — the same point where the timer chain is reclaimed
(ADR-0011) — and routes ISA IRQ4 through the IOAPIC (pin 4,
edge-triggered, fixed delivery to the BSP) to a new IDT vector 33 with
the timer stub's frame discipline (ADR-0013): full caller-saved save,
RSP realignment, Rust handler, dual-controller EOI *before* any work
that could block or switch.

The ISR drains the RBR while LSR.DR is set and hands every byte to the
line discipline. Bytes that arrive while interrupts are masked (e.g.
during the suites) are not lost: the 16550 FIFO holds them and the
IOAPIC latches the edge; delivery happens at the first `sti`.

### 2. The line discipline is a kernel subsystem (`console.rs`), not a userspace server (yet)

`console.rs` implements the classic line discipline: printable ASCII
(0x20..=0x7E) accumulates into an edit buffer (bound 120 bytes, excess
dropped and counted); 0x08/0x7F erase; CR or LF commits a nonempty line
into a 4-line ring queue (when full, the *oldest* line is dropped and
counted — input is never back-pressured into the ISR); every accepted
byte is echoed to TX as it arrives, and a commit echoes CRLF, so the
terminal shows what the user typed. Empty lines are consumed, never
queued (Enter on a blank line just re-prompts).

A userspace console/keyboard *server* (driver isolation per ADR-0002)
is the right eventual home — it arrives with the desktop phase, when
PS/2/GOP input makes the console a multiplexed resource worth a
service. For v1 the kernel-owned discipline is the smallest honest
version: one device, one reader class, no policy to delegate.

### 3. Three new frozen registry slots (13/14/15)

- **13 `SYS_CONSOLE_READ(buf, max)`** — pops the next complete line into
  the caller's buffer (STAC-bracketed, region-validated), returning its
  byte count as a positive status (terminator never included). Blocks
  (`block_current`, with the ADR-0018 GS-side capture/restore) while
  the queue is empty. **One reader at a time**: a second concurrent
  reader gets `STATUS_BUSY` (-4) rather than interleaved garbage — the
  same bounded-refusal idiom as IPC v1. If `max` is shorter than the
  line, the line is truncated (the remainder is discarded, not
  re-queued) and the truncation is counted.
- **14 `SYS_PROC_LIST(buf, max_pairs)`** — writes up to `max_pairs`
  `(pid, live_thread_count)` u64 pairs for every live process, returns
  the pair count. `ps` needs no more than the process table's public
  shape; names stay kernel-side for now (they are `&'static str`
  diagnostics, not a user contract).
- **15 `SYS_SHUTDOWN(power_slot)`** — requires a `CapObj::Power` cap
  with WRITE in the caller's slot, logs the requesting pid, and enters
  the existing `halt::reset_shutdown()` path — the same farewell island
  and the same canonical "halting via UEFI ResetSystem(shutdown)"
  declaration the harness grammar keys on.

**`CapObj::Power` (singleton)**: the authority to stop the machine is
an object reached through a capability, never a public verb — the
capability discipline (ADR-0015) applied to the one irreversible
system-wide act. The kernel's own panic/suite halt paths are unchanged
(they are ring-0 internal, not syscall surface).

### 4. The shell is a real userspace image, spawned by the kernel at boot

`userspace/shell` is a second genuine cargo/rust-lld artifact
(x86_64-unknown-none, `relocation-model=static`, `-no-pie`), laid out
by its own linker script at 0x400000 (text, R+X) / 0x410000 (data+bss,
R+W, NOLOAD bss) — the same ADR-0016 strict-subset contract the payload
image proves. It is embedded via `include_bytes!` as `elf::SHELL_IMAGE`
and registered as **image 1** in the spawn registry (image 0 remains
the test payload).

The shell is a plain program: no libc, no allocator — fixed buffers,
byte-wise command matching, decimal/hex formatters, output chunked to
`WRITE_MAX`. Builtin set v1: `help`, `ps`, `echo <text>`, `spawn`
(SYS_SPAWN image 0 with an empty inheritance spec, exit badge lent from
its own notification cap, then SYS_WAIT and report), `shutdown`.
Unknown commands answer with a hint, never silence.

**Boot hand-off (`spawn_init`)**: after every suite passes, the
bootstrap thread spawns the shell through a new kernel-internal entry
point of the spawn module — the ADR-0019 creation sequence *without a
parent*: no inheritance spec (there is nothing above), grants come from
kernel literals: `Power` (WRITE), `Image{0}` (READ, so `spawn` works),
`Notification` (READ|WRITE, its own exit-badge channel). The shell's
stack page is derived from its image exactly as any spawned child's.
The bootstrap thread then **parks in an idle loop** (`yield_now` +
`sti;hlt`): it stays runnable forever, which (a) keeps
`block_current`'s no-runnable-thread deadlock halt unreachable while
the shell blocks on input, and (b) gives every wake (UART ISR, timer)
a thread to return to. The machine now only stops via the shell's
`shutdown`, a kernel panic path, or the harness killing QEMU.

### 5. The harness feeds the console; every boot now proves the shell

`mtest.py` switches QEMU's serial from `file:` to a stdio chardev
(`-chardev stdio,signal=off -serial chardev:…` — no monitor
interleaving), captures stdout to the same log file, and gains a
prompt-paced feeder: when the shell's `arena>` prompt appears in the
output, input lines are written to stdin. All six existing milestone
scripts inherit one default feed — `shutdown` at the first prompt — so
*every* boot in *every* test now ends by exercising the full chain:
UART RX IRQ → line discipline → blocking console_read → shell dispatch
→ Power-gated shutdown syscall → ResetSystem. The clean-halt grammar
the harness keys on is unchanged. `test_m4_shell.py` drives the
interactive session (help/echo/ps/spawn/shutdown, each paced on prompt
count) and asserts the responses, including the payload's M4.3 message
appearing mid-session as the visible proof that `spawn` works from the
shell. The stability loop feeds `shutdown` per boot the same way
(marker-paced from bash).

## Approaches considered

- **PS/2 keyboard + QEMU-monitor `sendkey` injection** — the
  desktop-oriented device, and it will return with the graphics phase.
  Rejected for v1: a second driver (i8042 + scancode set 1), and input
  would arrive on a *different* device than the console output, which
  breaks the single-terminal experience headless users (and CI) have
  today.
- **Polling RX inside `SYS_CONSOLE_READ`** — no IRQ work at all.
  Rejected: a busy-wait that cannot coexist with `block_current`
  semantics; the shell would burn the only CPU while a human thinks.
- **Userspace console server now (kernel relays raw bytes over
  notifications)** — architecturally the eventual shape (ADR-0002
  driver isolation), but for one device and one reader it triples the
  moving parts (server process, byte-flow IPC, reconnection) before any
  policy needs them. Deferred to the desktop/input phase.
- **`shutdown` as an ungated syscall** — trivially simpler, and
  rejected on the project's security-first rule: an unholdable
  authority is a design flaw, not a convenience. The Power cap costs
  one enum variant and one rights check.
- **Shell as hand-assembled payload (test-7/8 style)** — rejected: a
  command parser in hand-assembled machine code is unmaintainable, and
  the milestone's whole point is a *real* userspace program.
- **Keeping the post-suite auto-halt and testing the shell only in a
  special mode** — rejected as fake-completion-adjacent: the shipped
  system must BE the interactive system; the harness adapts instead.

## Consequences

- The ABI registry grows to 15 frozen slots; `console_read` is the
  first *blocking* syscall outside IPC, reusing (and thereby
  re-proving on every boot) the GS-side block/wake invariant.
- Wake-from-interrupt-context enters the codebase: `sched::wake` was
  already IF=0-safe and enqueue-only (no switch inside the ISR), but
  the shell phase is its first live exercise — the stability loop now
  hammers it 100 boots at a time.
- Boot time grows by roughly the suite→prompt→feed→shutdown round trip
  (~0.5 s under TCG); the harness's marker pacing keeps it
  deterministic rather than sleep-based.
- The bootstrap thread becomes a permanent idle thread; future
  scheduling work (work queues, preemption for user processes) has a
  natural home to build on, and the deadlock-halt invariant stays
  meaningful (it can still fire if the shell ever dies — which is the
  honest outcome).
- `spawn_init` is the seed of the root-task story: when boot-time
  policy arrives (bootinfo caps, a real init), it grows here, not in
  the syscall path.
- Deferred (documented, deliberate): line editing beyond backspace,
  input from PS/2, a userspace console server, `ps` names/states,
  shell job control, command arguments beyond `echo`, history.
