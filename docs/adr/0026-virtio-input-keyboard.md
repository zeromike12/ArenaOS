# ADR-0026: virtio-input keyboard — `inputd`, the console-input capability, and live typing

Status: accepted (M6.3)

## Context

Phase 6's third driver step asks for a keyboard: the machine should be
typed into, not just fed. Two questions had to be answered before any
code, and the roadmap named both — **which transport**, and **how a
ring-3 input driver reaches the shell** without breaking the serial
console every harness and headless user depends on.

The second question is the load-bearing one. The shell today blocks in
`SYS_CONSOLE_READ`, and the kernel owns the line discipline (echo,
backspace, CR→line, the queue). Every automated boot — all eleven test
scripts, the crash gate, the persistence gate, the 100-boot stability
loop, the release verify-boot — types `shutdown` into that path over
the serial chardev. A keyboard must ADD a source, never replace one.

## Approaches considered

### Transport

1. **i8042 PS/2 (the documented fallback).** Universally emulated and
   present on real x86 hardware. Rejected as the primary: it is legacy
   port I/O, which ring 3 cannot do without inventing a new capability
   kind (an I/O-port-range cap) and a new kernel policy for it; the
   8042 is a two-device controller with a mode-byte state machine,
   output-buffer-ownership ambiguity between keyboard and mouse, and
   translation quirks (scancode set 1 vs 2 vs translation enabled);
   and none of it generalizes — real modern hardware is USB HID, not
   8042. It stays the documented fallback if virtio-input ever
   disappoints; it did not.
2. **USB HID (`-device usb-kbd`).** The honest future of real
   keyboards, and exactly the wrong milestone: it requires an entire
   xHCI host-controller driver first. Phase 6 is the VirtIO family;
   USB is its own multi-milestone project later.
3. **virtio-input-hid, `virtio-keyboard-pci` (chosen).** It is the
   fourth driver on the shared virtio core (ADR-0025) — which is the
   point: the core was extracted at N=3 and this validates it at N=4
   with a device shape nobody designed it around (two queues, but the
   *event* queue is device-writable with many small buffers posted,
   unlike net's RX/TX split or block's request chain). QEMU forces
   modern virtio on it (`virtio_pci_force_virtio_1`), so there is no
   transitional ID to accept: device 0x1052, type 18. Events are the
   8-byte `{le16 type, le16 code, le32 value}` evdev triple, batched
   by the device and flushed on `EV_SYN`/`SYN_REPORT`.

### Reaching the shell

4. **Shell multiplexes two sources.** Keep `SYS_CONSOLE_READ` for
   serial and add a non-blocking `INPUT_POLL` IPC call to inputd,
   spinning between them. Rejected: it forces the shell to poll (the
   project's drivers are interrupt-driven by rule), it duplicates the
   line discipline — echo, backspace, and the line buffer would exist
   twice and drift — and it turns a blocking kernel contract into a
   busy loop.
5. **Replace the serial read path with input events.** What the
   roadmap literally suggested. Rejected as a regression: it breaks
   every automated boot and every headless user, and it makes the
   keyboard mandatory for a machine that must stay bootable without
   one. Pushing back on this was the right call — the goal is *live
   typing*, not *only* typing.
6. **inputd feeds the kernel's line discipline (chosen).** `console::
   feed(b)` is already the byte entry point the serial ISR calls and
   the m4 suite drives. inputd decodes evdev codes to ASCII and pushes
   bytes through a new syscall into that same discipline. One line
   editor, one queue, one blocking read contract; the shell needs
   **zero changes**; serial and keyboard are both live simultaneously;
   and a machine with no keyboard behaves exactly as before.

### Guarding the push

7. **Unguarded syscall.** Any ring-3 process could forge keystrokes
   into the console the shell trusts — a privilege-escalation seam
   handed out for free. Refused.
8. **A capability, mirroring `CapObj::Power` (chosen).**
   `CapObj::ConsoleInput` is a singleton authority with no identity:
   holding it with WRITE *is* the right to inject console input, and
   the kernel hands it to exactly one process (the production inputd
   spawned by `entry.rs`). The m6 suite's short-lived instance does
   NOT get it, which is why the same binary must work both ways —
   hence the probe below.

## Decision

**`SYS_CONSOLE_PUSH = 23`** `(slot, buf, len) -> bytes pushed`. Gated
on `CapObj::ConsoleInput` + `RIGHTS_WRITE` in `slot`; copies from the
caller's address space (SMAP-aware, bounded by `PUSH_MAX = 64`) and
feeds each byte to `console::feed` — the identical path the COM1 RX
ISR uses. **A zero-length push is the documented capability probe**:
it pushes nothing and returns 0 if the cap is valid, a typed refusal
if not. That is how one binary discovers which mode it is in, using
only the ABI it already has.

**`inputd`** (registry image 10, `userspace/inputd`): the fourth
driver on the shared core. Handshake asserts modern virtio-input
(0x1052, no transitional form), VERSION_1 only (virtio-input defines
no device feature bits), two virtqueues. Queue 0 (eventq) is set up
with `RingMem::Packed` and **32 device-writable 8-byte event buffers
posted from one owned frame** — posting matters: QEMU's
`virtio_input_send` drops an ENTIRE event batch if the ring cannot
absorb all of it, so an under-posted ring silently loses keystrokes.
Queue 1 (statusq) is not used in v1 (no LED/repeat control) and is
left unenabled. One MSI-X entry → one relay vector; a batch of events
raises ONE interrupt, so the driver harvests until the used ring is
drained rather than assuming one event per notification.

Decoded keys land in a 64-byte ring inside inputd, so keys that arrive
while no client is asking are not lost (netd's single hold slot was a
link-probe simplification; a keyboard is asynchronous by nature).
The keymap is the US-ASCII subset of evdev codes 1..57 with a shift
table, tracking `LEFTSHIFT`/`RIGHTSHIFT` press and release; key
*presses* produce bytes, releases only update modifier state, and
everything else is ignored honestly rather than mapped to noise.

**Two modes, chosen by the zero-length probe:**

- **Console mode** (production; `entry.rs` grants the ConsoleInput
  cap in slot 3): loop = wait on the relay badge → harvest the used
  ring → decode → `SYS_CONSOLE_PUSH` the bytes. Typing in a real QEMU
  window drives `arena>` through the kernel's own line discipline.
- **Service mode** (the m6 suite; endpoint serve side, no console
  cap): `INPUT_OP_READ` replies with up to 64 buffered key bytes,
  blocking on the relay until at least one exists; `INPUT_OP_SHUTDOWN`
  is the poison, replying with the interrupt-delivered batch count.

**`inputtest`** (image 11): asks for keys until it has five bytes and
asserts they are exactly `arena` — the fixture contract, injected by
the harness over QMP, in the same spirit as slirp's fixed 10.0.2.2
gateway. Exit 42 on match; 62..67 for the failure stages.

**A keyboard needs a typist — so the supervisor, not the driver,
decides when to stop waiting.** Every other fixture in the suite
answers by itself: slirp replies to ARP, the entropy backend fills
buffers. A keyboard produces nothing unless someone uses it, and the
first implementation made that a hang: attach `virtio-keyboard-pci`,
boot, type nothing, and the suite's 21 s drain expired and HALTED the
machine. Attaching a device must never make a machine unusable, so:
the relay notification carries one word besides the device's badge,
`INPUT_BADGE_GIVE_UP`, and whoever spawned inputd (the only holder of
that notification's write side) may send it to abandon a pending
READ. inputd answers `INPUT_S_NO_KEYS`; inputtest poisons the service
and exits 68; the suite reports an honest **SKIP** and tears down
frame-exactly on that path too.

The alternative — a timeout inside the driver — was rejected on
principle: a key that never comes is indistinguishable from a key that
comes later, so a driver inventing a deadline would be inventing
policy it cannot possibly have. The supervisor knows whether anyone is
expected to type; the driver does not. (The suite's window is ~1 s of
HPET wall clock, which is also a reminder that QEMU's HPET runs at
100 MHz, not the 14.318 MHz the older comments assumed.)

Reading the delivery counter to detect "someone typed" needed one
correction the first boot found: relay vector 48 is RECYCLED across
this suite's tests, so the rng test's count was still sitting there
when the window opened and the wait ended instantly. The window now
waits for THIS driver to arm the vector (which resets the counter) and
then compares against a baseline taken at that moment.

**The harness gains QMP** (`tools/qmp.py`): a ~60-line client over a
unix socket (greeting → `qmp_capabilities` → `input-send-event` with
`qcode` keys). `mtest.run_qemu(keys=[(marker, nth, "text")])` injects
a string as press/release pairs once a serial marker has appeared N
times — marker-paced, never sleep-based, the same discipline as the
crash gate's killer. Injection must follow inputd's DRIVER_OK (before
that, QEMU has no buffers to fill and drops the batch), so the marker
is inputd's own ready line.

**Proofs — two, both real:**

1. `m6:test:input_service` — the suite spawns inputd + inputtest, the
   harness injects `arena`, the client decodes it through the service
   boundary and verifies byte-for-byte; the kernel witnesses at least
   one relay delivery (a batch count, not a fixed number — QEMU may
   coalesce), exact exit badges, both children exit 42, the dead
   driver's relay swept, teardown frame-exact.
2. `tools/test_m6_typing.py` — the PRODUCTION path end to end: no
   serial input at all, the harness types `echo typed-on-the-keyboard`
   and `shutdown` on the virtual keyboard, and the boot must show the
   shell echoing the characters and executing both commands. This is
   the milestone's actual claim — a user typing into a QEMU window —
   and it is asserted, not described.

## Consequences

- The shell is unchanged: one line discipline, one blocking read, two
  hardware sources feeding it. Serial keeps working everywhere, so no
  existing gate needed a rewrite and headless use is unaffected.
- `CapObj::ConsoleInput` is the second singleton authority cap after
  `Power`. The pattern is now established for "the kernel delegates a
  privileged kernel-side action to exactly one ring-3 server."
- The registry is full again: images 10 (inputd) and 11 (inputtest)
  consume the last two of `MAX_IMAGES = 12`. **6.4 (consoled) must
  raise the cap** — recorded here so it is a decision, not a surprise,
  exactly as ADR-0024 did for 6.2.
- v1 keyboard limits, stated plainly: US layout only, no caps lock, no
  key repeat (QEMU's `input-send-event` and a real user's host
  keyboard both generate the repeats), no LED control (statusq
  unenabled), no mouse/tablet/multitouch even though the same driver
  shape would serve them. Non-ASCII and unmapped keys are dropped, not
  guessed at.
- A boot with a keyboard attached and nobody at it is a first-class,
  tested configuration (`test_m6.py`'s third boot): honest SKIP,
  shell prompt reached, clean halt. The give-up badge is also the
  shape a future supervisor (6.5) will need for "abandon this
  client's blocked request" in general — recorded here as a pattern,
  not just a patch.
- QMP is new harness infrastructure and the first time the test suite
  drives QEMU through a control channel rather than only the serial
  chardev. It is test-only: nothing in the OS knows it exists.
- A batch-count assertion replaced net's exact-interrupt-count one:
  the device coalesces events per `EV_SYN`, and asserting an exact MSI
  count would be asserting QEMU's batching policy rather than our
  driver's behavior. The suite asserts what is genuinely ours — every
  injected key arrived, in order, by interrupt.
