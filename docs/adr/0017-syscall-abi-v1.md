# ADR-0017: Syscall ABI v1 — register encoding, dispatch, typed status

- Status: accepted (Milestone 4.2)
- Date: 2026-09-26
- Depends on: ADR-0006 (ABI philosophy), ADR-0014 (processes, ring 3, the
  v0 boundary this version replaces), ADR-0016 (executable format — the
  programs that will call this ABI)

## Problem

M3.3a stood the ring-3 boundary up with an ad-hoc *ABI v0*: `RAX` = call
number, three argument registers, `-1` for everything rejected, and two
production calls plus two suite-proof calls. That was the right scope for
proving the machinery. Milestone 4 needs a *versioned contract* user
programs are written against: a register encoding that will not have to
change when IPC (M4.4) needs six arguments and a capability slot, result
values that distinguish *why* a call failed (a program cannot react to
"‑1"), and a numbering policy that makes the surface stable forever
after — because every shipped binary encodes these numbers.

## Approaches considered

1. **Keep v0 and grow it ad hoc.** No migration cost now; every future
   change becomes a wire-compatibility archaeology dig, and programs can
   never distinguish error causes. Rejected: the point of "v1" is that
   it stops moving.
2. **Linux syscall ABI wholesale** (numbers, errno semantics, `fork`/
   `read`/`write` names). Rejected by foundation: POSIX compatibility is
   explicitly not a goal (assignment ground rules; ROADMAP firewall —
   "no errno, ever, unless a superseding ADR"). Borrowing the *register
   slots* is different from borrowing the *interface*.
3. **Message-passing-only kernel (no syscalls; everything is IPC to
   servers).** The end-state direction for most services (M4.4+), but a
   minimal trap-in surface is unavoidable: thread exit, the debug
   console backdoor, and the IPC primitives themselves must be calls
   into the kernel. Rejected as a *replacement*; adopted as the scope
   rule — the syscall surface stays minimal, services move behind IPC.
4. **Versioned register ABI + typed signed status + frozen numbering
   (chosen).**

## Decision

**Syscall ABI v1**, defined below, frozen by this ADR. Additive changes
only from here: new call numbers, new (negative) status codes, new
suite-proof calls. Any change to the meaning of an existing number, the
register roles, or the sign convention requires a superseding ADR and a
major-version bump of the whole ABI.

### Register encoding (v1)

| Register | Role |
|---|---|
| `RAX` (in) | Call number (`u64`) |
| `RDI, RSI, RDX, R10, R8, R9` (in) | Arguments `a0..a5` (up to six, `u64` each) |
| `RAX` (out) | Status (`i64`, see below) |
| `RBX, RBP, R12–R15` | **Preserved** across the call |
| everything else caller-saved | Clobbered |
| `RCX, R11` | Destroyed by hardware (`syscall` stores RIP/RFLAGS there) — never argument registers |

- `R10` carries `a3` instead of `RCX` because the hardware consumes
  `RCX`; the slot order otherwise follows the widespread SysV/Linux
  argument habit — familiarity lowers the cost for every future
  userspace author, and the *numbers and semantics* remain ours.
- User `RFLAGS` is restored on return (the stub re-loads `R11` from the
  recorded frame; `sysretq` restores it) — a call does not disturb the
  user's interrupt flag.
- `SFMASK` clears `TF|DF|IF|NT|AC` on entry: the whole kernel-side
  dispatch runs at IF=0 (a syscall is never preempted mid-handler), and
  the user cannot smuggle `AC` past SMAP.
- The entry stub marshals all six arguments plus the recorded
  `SyscallFrame` into the Win64 convention for the Rust dispatcher
  (four in registers, four on the stack, 16-byte alignment maintained).

### Typed status codes (v1)

The result in `RAX` is a signed 64-bit status:

- `0` (`STATUS_OK`) — plain success.
- **Positive** — call-specific success payload (e.g. `debug_write`
  returns the number of bytes accepted). A call that returns a payload
  documents its range; payloads are always `>= 0`.
- **Negative** — typed error, dense from `-1`, allocated once and never
  reused or renumbered:

| Code | Name | Meaning |
|---|---|---|
| `-1` | `STATUS_BAD_CALL` | Unknown call number |
| `-2` | `STATUS_BAD_ARG` | Argument value invalid (null pointer, `len == 0` where bytes are required, `len` over the call's cap) |
| `-3` | `STATUS_BAD_ADDRESS` | Buffer span not fully inside the caller's registered user regions |

Future codes (`STATUS_BAD_CAPABILITY`, `STATUS_DENIED`,
`STATUS_NO_SPACE`, …) arrive with the milestones that need them. No
errno namespace, no POSIX names, no string payloads — the status *is*
the diagnosis (ADR-0006).

Wire compatibility note: v0's blanket `-1` rejection is v1's
`STATUS_BAD_CALL` bit-for-bit, and v0's success counts are v1's positive
payloads — so the M3 suite's ring-3 assertions (`invalid nr → -1
observed in ring 3`, `write returns count`) hold unchanged under v1.

### Call numbers (v1 registry)

| Nr | Call | Args | Result |
|---|---|---|---|
| 1 | `debug_write` | `a0` = buffer, `a1` = length (1..=256) | bytes accepted, or `-2`/`-3` |
| 2 | `thread_exit` | `a0` = 64-bit exit code | does not return |
| 3 | — | *deliberately unallocated* (v0's gap stays a gap forever) | |
| 4 | `prove_ring3` (suite) | — | `0`; arms the ring-3 #GP expectation (ADR-0014's proof protocol, kept for regression) |
| 5 | `prove_done` (suite) | — | `1` iff the armed #GP was delivered |
| 6 | `abi_echo6` (suite) | `a0..a5` | fingerprint of the six received arguments, MSB masked so even this probe honors the sign domain (proves the stub's six-register marshalling on real hardware) |

- `debug_write` is an explicit **temporary console backdoor**: the
  diagnostic surface until the console service exists behind IPC
  (M4.4+/Phase 6). The name says what it is; it will remain for kernel
  diagnostics even after that, with its cap.
- `thread_exit` terminates the calling thread through the scheduler's
  zombie/reap path (M3.1 semantics) and records `(thread id, code)` —
  process teardown and exit reporting grow on top in M4.5.
- Suite-proof calls are part of the registry on purpose: they are
  stable, documented, and exercised every boot — hidden test hooks that
  bypass the ABI would prove less, not more.
- Buffer validation doctrine (from ADR-0014, unchanged): spans are
  checked page-granularly against the calling thread's *registered*
  user regions; an invalid span is **refused with a typed status, never
  faulted** — user code gets an answer, not an exception it did not ask
  for.

## Consequences / disadvantages

- Six-argument marshalling makes the entry stub's stack choreography
  more delicate (four register moves must happen after the stack
  argument pushes, in an order that loses nothing); the m4 suite proves
  all six registers arrive (call 6) and that the six callee-saved
  registers survive — on real hardware, from ring 3.
- Freezing numbers early means living with the v0→v1 inheritance (1 and
  2 keep their meanings; 3 stays an eternal gap). Accepted: stability
  is worth more than tidiness in a wire format.
- `debug_write` bypasses the future console service; that is its
  documented purpose (diagnostics), and its 256-byte cap keeps it a
  backdoor, not a data channel.

## Future implications

- M4.3 runs the ELF-loaded payload through this ABI (the stub becomes a
  real program: `debug_write` + `thread_exit`).
- M4.4's IPC calls (endpoint call/reply, notification, capability
  transfer) get the next registry numbers under this exact encoding —
  six registers + typed status is sized for them (endpoint cap, tag,
  badge, buffer, length).
- A userspace `libarena` (Phase 8) wraps the raw encoding; the ABI
  itself stays assembly-visible and hand-callable forever — it is the
  contract the whole userspace rests on.
