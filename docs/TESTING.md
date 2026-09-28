# ArenaOS — Testing Strategy

Governing rule (ADR-0005): **a subsystem exists only when a test would fail
if it were faked.** "Printing success" is not a test. Tests assert observable
machine effects: bytes that came back out of a UART in loopback mode, register
values read back after a GDT load, firmware structures parsed from real UEFI
memory, privilege-level changes, VM exit behavior.

## The testing pyramid

```
        ┌────────────────────────┐
        │ in-guest self-tests    │  hardware behaviors: UART loopback,
        │ (m<N>:test:* markers)  │  sgdt read-back, #PF traps, ring transitions
        ├────────────────────────┤
        │ boot integration tests │  build → fresh ESP → QEMU/EDK2 → serial
        │ (tools/test_m<N>.py)   │  marker assertions → clean-exit check
        ├────────────────────────┤
        │ host unit tests        │  arch-independent logic: allocators, rings,
        │ (cargo test --target   │  parsers, data structures
        │  x86_64-unknown-linux) │
        └────────────────────────┘
```

### Host-side unit tests (M2.5+)

Pure-logic crates under `kernel/libs/` compile for the host too and carry
their own `#[cfg(test)]` suites — fast, deterministic, and they can use
`std` helpers (reference models, maps) that do not exist in the kernel:

```
cd kernel && cargo test -p arena-heap --lib --target x86_64-unknown-linux-gnu
cd kernel && cargo test -p arena-sync --lib --target x86_64-unknown-linux-gnu
```

The sync suite runs on real host threads (four-way contention with
non-atomic increments, recursion `#[should_panic]`) — parallel behavior
the single-CPU guest cannot honestly generate; the guest proves the lock
*state machine* and the irqsave freeze on real PIT-tick hardware
(`crit_section`: zero interrupts may cross a 25 ms section entered with
IF=1, delivery must resume after).

(`--lib` because the offline toolchain has no `rustdoc`; these crates have
no doctests.) `tools/run_tests.sh` runs the host suites *and* every
milestone boot harness. In-guest tests still re-prove the same logic on
real hardware semantics — e.g. `heap_guards` deliberately triggers
double-free / red-zone / foreign-pointer rejections; their ERROR lines on
serial are **expected evidence**, not failures (the harness fails only on
PANIC or missing/failed markers).

## Marker grammar (serial)

Every milestone emits machine-checkable lines:

```
m1:test:<name>: PASS
m1:test:<name>: FAIL (<reason>)
m1: RESULT PASS (8/8)
m1: RESULT FAIL (6/8)
m2:test:<name>: PASS                        ← same grammar per milestone
m2: RESULT PASS (3/3)
[arena PANIC <file>:<line>] <message>       ← any panic fails the run
```

The harness (`tools/test_m<N>.py`, a thin wrapper over the shared pipeline in
`tools/mtest.py`) requires: all expected test names present with PASS, RESULT
line PASS *and covering exactly the expected test count*, no PANIC anywhere,
QEMU exits by itself (the kernel calls UEFI `ResetSystem(EfiResetShutdown)`;
`-no-reboot` makes QEMU exit 0) within the timeout. A timeout means hang; a
nonzero QEMU exit means crash/reset-loop — distinct diagnoses. All milestone
harnesses run against the same image in one boot: the kernel executes every
milestone suite in order, so M1 markers prove the older guarantees still hold
while M2 code is present.

Since M4.6 (ADR-0020) a healthy boot no longer halts by itself — it ends at
the shell — so the pipeline also FEEDS the console: serial runs through a
stdio chardev (no monitor interleaving) and a marker-paced feeder thread
types `shutdown` when the shell's first `arena> ` prompt appears. Every
milestone boot thereby proves the full console chain: UART RX IRQ → kernel
line discipline → blocking `SYS_CONSOLE_READ` → shell dispatch → Power-gated
`SYS_SHUTDOWN` → `ResetSystem`. `tools/test_m4_shell.py` overrides the feed
script to drive a full interactive session (help/echo/ps/ls/write/cat/ls/
spawn/unknown/shutdown) and asserts every response — including the spawned
payload's pinned message appearing mid-session, and, since M5.3, the
filesystem builtins: `ls` must already show the m5 suite's committed
`arena.txt` (the production fsd mounted the same on-disk state the suite
wrote EARLIER IN THE SAME BOOT), `write note.txt hello-fs` creates a real
file through fsd + storaged, `cat note.txt` streams the exact bytes back,
and the second `ls` counts both files; since M5.4 `rm note.txt`
unlinks it transactionally (and `rm nosuch.txt` gets an honest
refusal) before the final `ls` counts only the suite's file.
`tools/stability_loop.sh` feeds the same way from bash (marker-paced,
never sleep-based).

Since M5.1 (ADR-0021) every harness boot also attaches the **scratch-disk
fixture**: `arena_env.scratch_disk_args()` re-creates a fresh 8 MiB
`build/scratch.img` per run — since M5.3 (ADR-0023) FORMATTED as AFS1 by
`tools/afs1.py`'s host-side `mkfs` (checksummed superblock, first
ping-pong commit, empty object table + allocation bitmap) — and attaches
it as `virtio-blk-pci`: the device the kernel's bus-0 PCI scan must find
(`m5:test:pci_scan`) and the medium the userspace services really read
and write. `m5:test:block_service` spawns storaged (image 2) and blktest
(image 3); the client's write→clear→read-back→verify cycle goes through
the service boundary onto this disk — the pattern is cleared between the
two calls, so the verified bytes can only have come from the device's
DMA (the raw-block cycle claims the LAST sector: with a formatted
filesystem on the image, a raw write must not touch FS structures).
`m5:test:fs_service` then spawns a fresh storaged, fsd (image 4), and
fstest (image 5). Since M5.4 fstest PROBES the volume first (OPEN
`arena.txt`), and the answer picks one of two derived contracts: on a
FRESH volume it creates the file, writes 512 pattern bytes (its LENT
frame forwarded through fsd — the device DMAs the CLIENT's page),
closes, RE-OPENs by name, reads back, verifies byte-for-byte, walks a
strict single-file `ls`, and exits 42 — 34 device ops (mount 12:
superblock 1 + commit slots 2 + superseded-slot probe 1 + objtab 4 +
bitmap 4; create-commit 9; write 11; read 2). On a volume that
SURVIVED A REBOOT the persisted branch verifies the committed file
with ZERO writes, walks a tolerant `ls` (other files legitimately
share the namespace), and exits 43 — 14 device ops. The kernel asserts
three exact badges and relay deliveries EXACTLY equal to the contract
the exit code selects (fsd's reported count and storaged's completions
must agree — three witnesses, one number). Every dirty boot's suite is
thus a persistence witness. AFTER the boot, `test_m5.py` parses the
committed image with the same host-side layout module: newest commit
seq 3, `arena.txt` size 512, extents resolving to bitmap-marked
sectors, and the literal pattern bytes in those sectors — the on-disk
layout proven from the host side.
Fresh-per-run remains the DEFAULT fixture discipline (no boot may
silently inherit another's disk by accident); the two M5.4 scripts own
their disk's lifecycle EXPLICITLY via `mtest.boot()` (an explicit
scratch path, marker-paced feeding, and an arming kill switch):
`test_m5_persist.py` formats once and boots twice — boot 1's shell
writes `persist.txt`, boot 2's suite takes the persisted branch, the
shell `cat`s the boot-1 bytes back exactly and `rm`s them across the
reboot, and the host verifies the committed sectors after each boot.
`test_m5_crash.py` is the crash-consistency gate: after a clean seed
boot it SIGKILLs QEMU at five observed points of an in-flight shell
write (the guest experiences a strict PREFIX of its issued device
operations — the process-crash model AFS1's superblock-last commit is
designed for) and verifies every reboot WITHOUT any repair tool: the
suite passes 6/6 on the crashed volume, the crashed file comes back
never-committed, committed-empty, or committed-full (never torn),
older commits stay byte-exact, and `afs1.audit()` — the host-side
fsck-lite over the newest committed generation (checksums, live-run
bitmap marks, extent/bitmap agreement, no double-claims; leaks are
legal and never flagged) — finds zero problems. Interactive boots
(`tools/run.sh`), the stability loop (which reformats per boot, so it
always exercises the fresh contract), and the release-bundle
verification (which boots the SHIPPED `scratch-template.img`) attach
the same fixture family; the ESP stays the boot medium throughout.

Since M6.1 (ADR-0024) every harness boot also attaches the **slirp NIC
fixture**: `arena_env.net_args()` adds `-netdev user,id=net0 -device
virtio-net-pci,netdev=net0` — QEMU's user-mode networking, no host
privileges, and it answers ARP for its built-in gateway 10.0.2.2, the
one peer netd's link proof needs. `m6:test:net_service` spawns netd
(image 6) and nettest (image 7): the client fetches the device MAC
through the service (the device-config read path, DEV_INFO word [6]),
hand-builds the 42-byte Ethernet/ARP frame (who-has 10.0.2.2 tell
10.0.2.15), and SENDs it — the frame is LENT through IPC and the
device DMAs the client's own page chained behind netd's virtio header
(zero-copy TX). slirp's reply arrives on the receive queue as an MSI-X
interrupt (never a poll), is held in netd's single-frame hold slot,
and is delivered in the REPLY's inline 64-byte message; the client
verifies ethertype, opcode, sender IP, target MAC, and the
sender-MAC==Ethernet-source consistency at their exact wire offsets.
The kernel counts the machine side: exactly ONE hardware delivery per
relay vector (48 RX + 49 TX), both exit badges, both exit 42s, the
dead driver's relays swept, frame-exact teardown.
The fixture is OPTIONAL by design: the net device is new in v0.6.0 and
pre-v0.6.0 invocations must stay bootable-green, so `test_m6.py` boots
TWICE — with the NIC (the full proof above, plus the production netd
spawn asserted) and WITHOUT it (`run_qemu(..., net=False)`), where the
boot must show the honest SKIP markers (`m6:test:net_service: SKIP`,
`m6: RESULT SKIP`), the kernel's "network service stays offline" line,
no netd instance at all, and m1–m5 green in the same boot. A skip is
never laundered into a pass count, and a compatibility regression
never hides.

Since M6.2 (ADR-0025) the family gained the **entropy fixture**:
`arena_env.rng_args()` adds a bare `-device virtio-rng-pci`, which
QEMU backs with its own `rng-builtin` source (the platform CSPRNG) —
no host files, no privileges, portable across host OSes.
`m6:test:rng_service` spawns rngd (image 8) and rngtest (image 9):
the client takes TWO 4 KiB draws into SEPARATE frames, each LENT
through IPC so the device DMAs entropy directly into the client's own
page (zero-copy fill — the write direction of storaged's read path).
Randomness cannot be asserted against expected values — a fixed
expectation would be fixed entropy, i.e. a fake — so the client
asserts what every genuine source satisfies and every broken one
fails: the device wrote the FULL length (its own used-ring count),
neither draw is all-zero (an ignored descriptor), neither is a single
repeated byte (a stuck source), and the two draws DIFFER (a looping
or cached source). The kernel witnesses the mechanics: exactly TWO
relay deliveries on the driver's vector (one MSI per draw — no
polling), both exit badges exact, both children exit 42, the dead
driver's relay swept by `proc::destroy`, teardown frame-exact. The
serial line `rngtest: draw A … vs draw B …` prints fresh fingerprints
every boot, and `test_m6.py` asserts they are present and different —
evidence in the log, not a claim. Entropy QUALITY (distribution,
unpredictability) belongs to the host backend and is deliberately NOT
claimed by the suite. This fixture is optional too: the no-fixture
boot (`run_qemu(..., net=False, rng=False)`) must show BOTH honest
SKIPs, both "service stays offline" lines, no driver instance of
either kind, and m1–m5 green in the same boot. All four fixture
combinations (both / net-only / rng-only / neither) are verified to
boot green.

### The second input channel: the virtual keyboard (M6.3, ADR-0026)

Until v0.8.0 every automated boot drove ArenaOS through ONE channel —
bytes written to the serial chardev. The input milestone adds a
second: `arena_env.input_args()` attaches `-device
virtio-keyboard-pci`, `arena_env.qmp_args()` exposes a QMP unix
socket, and `tools/qmp.py` types on that keyboard the same way a
person at a QEMU window does (press/release pairs, shift held around
capitals, one keystroke per QMP command). It is TEST infrastructure
only — nothing inside ArenaOS knows QMP exists; the guest sees device
interrupts and evdev events.

A typing script has the same shape as a serial feed — `(marker,
count, text)` — and is injected once the marker has appeared that many
times: **marker-paced, never sleep-based**, the discipline the serial
feeder has always used. `mtest.DEFAULT_KEYS` types the m6 input
fixture at inputd's DRIVER_OK marker, so every boot in the gate now
exercises the whole keyboard chain (device interrupt → evdev event →
keymap → IPC), just as every boot has exercised the serial console
chain since M4.6.

`m6:test:input_service` spawns inputd (image 10) and inputtest (image
11), and deliberately WITHHOLDS the `ConsoleInput` capability from the
driver — so the same binary that feeds the console in production runs
as an IPC service here, and the capability probe that chooses between
them is itself under test. The harness types `arena`; the client reads
the DECODED bytes back through the service and verifies them
byte-for-byte. The interrupt assertion is a LOWER bound (≥1 delivery),
unlike net's and rng's exact counts: the device coalesces events per
`EV_SYN`, so an exact number would assert QEMU's batching policy
rather than our driver's behavior.

**A keyboard is the one fixture that needs a typist**, which makes
"nobody typed" a real configuration rather than an error. The suite
waits a bounded window (~1 s of HPET wall clock) and then sends
inputd's spawner-only give-up badge; the children wind themselves up
and the test reports an honest SKIP. `test_m6.py`'s THIRD boot asserts
exactly that: keyboard attached, `keys=[]`, and the machine must still
reach the shell prompt, halt cleanly, and report SKIP — never FAIL,
never a fake PASS. Attaching a device must never make a machine
unusable.

One rule the stability loop learned the hard way: **one typist, one
boot.** The loop recreates its serial log every iteration, so a typist
left over from a finished boot finds the NEXT boot's marker and types
into it — and a stray `arena` landing in the same line as the feeder's
`shutdown` hangs a perfectly good kernel. Each boot's typist is now
reaped before the next begins (the first 100-run with the keyboard
scored 99/100 with one boot that never reached the kernel at all; the
reaped rerun was 100/100).

`tools/test_m6_typing.py` is the milestone's real claim, asserted: a
boot with `feed=[]` — the serial input channel completely dead — where
`echo Hello-From-The-Keyboard` (capitals prove modifier tracking),
`psX<backspace>` (the line discipline erases the typo before the shell
reads the line, so `ps` runs and `psX` never does), and `shutdown` are
all typed on the virtual keyboard. The machine halts because someone
typed it.

### The third channel: the virtio-console port (M6.4, ADR-0027)

A boot now has three ways in and two ways out. `arena_env.console_args()`
attaches `-device virtio-serial-pci,max_ports=1 -device virtconsole` on a
unix-socket chardev, and `tools/vcon.py` is whoever is sitting at the
other end of that socket: it records everything the guest sends (that
capture IS the evidence for guest→host assertions) and sends scripted
replies when markers appear in that stream. Marker-paced like every
other actor here — the console's own output is the clock.

The fixture chardev uses **`server=on,wait=on`**: QEMU discards a
console port's output while nobody is attached, so with `wait=off` the
guest could transmit its fixture into a socket the harness had not
reached yet, and the test would report an honest-but-useless SKIP about
one boot in five. Blocking QEMU's startup until the actor is attached
removes the race rather than papering over it with a sleep. Users get
`wait=off` in docs/RUNNING.md — their machine must boot whether or not
anyone connects, and THAT configuration is tested too (the
console_service SKIP path).

`m6:test:console_service` spawns consoled (image 12) and contest
(image 13) with NEITHER console capability, so the driver serves the
port over IPC and the capability probe that chooses its mode is itself
under test. contest sends a fixture line out the transmit queue — the
harness asserts those exact bytes on its socket, where the evidence is
— and then reads the harness's answer back off the receive queue and
verifies it byte-for-byte. Both directions must be interrupt-completed;
teardown must be frame-exact and must sweep BOTH relay vectors.

`tools/test_m6_console.py` is the milestone's real claim: a boot with
`feed=[]` and `keys=[]` — serial input dead, keyboard untouched — where
the shell's banner and prompt ARRIVE on the port and `echo`,
`psX<backspace>`, and `shutdown` are all typed INTO it. The machine
halts because someone typed it on a virtio-console port.

**A harness that perturbs its subject is a broken instrument.** Two
lessons, both paid for in flaky runs: an out-of-line call added to
`serial::putc` (tens of thousands of calls per boot) and an O(n²)
capture rewrite in the actor each slowed the guest enough to fail the
m2 suite's TSC calibration — QEMU's TSC follows HOST time while the PIT
follows virtual time, so host load shows up INSIDE the machine as a
clock disagreement. The tap's inert path is now three inlined
instructions and the actor appends to an open file. The kernel's own
calibration (and the m2 cross-check) additionally retry up to three
rounds, because a stalled host must not be able to fail a boot.

### Fault injection: killing a service on purpose (M6.5, ADR-0028)

Every other test in this suite checks what happens when things work.
Two do not. `m6:test:service_death` and `m6:test:service_restart`
destroy a live service while a client is blocked in a call to it, and
assert that the machine says so and recovers.

They need no device and no host actor, which makes them the only
fully DETERMINISTIC tests in the m6 family — and the reason they use
their own trivial image pair (`userspace/faultd`) rather than a real
driver. Killing storaged mid-I/O would prove the same thing while
risking the filesystem; that experiment belongs with the
crash-consistency work, not here.

The fault lands at a moment the service NOMINATES rather than one the
harness guesses: `faultd` takes a request it will never answer,
signals the suite on a write-only notification, and parks forever on
a read-only one it can never signal itself. (The first version
signalled and parked on the SAME notification and swallowed its own
badge — rights now make that unrepresentable.) Whether the kill
arrives while it is still runnable or already parked, the client's
request is `Delivered` either way, which is the only state the proof
depends on.

What is asserted: the caller is woken with `STATUS_SERVICE_GONE`
rather than left blocked; the kernel logs exactly one failed
in-flight call; the parked thread is KILLED, not left live forever;
the endpoint OUTLIVES its server; the supervisor respawns the image
with its capabilities replayed under a new pid; a fresh client
reaches the restarted instance through the capability it was granted
before the crash; the spawn-record table is flat across the cycle;
and frames return to baseline. Counting frames here needs one step no
earlier test needed — a killed thread is a zombie holding its 32 KiB
kernel stack until the next scheduler entry, so the test yields
before counting rather than reporting a leak that does not exist.

### Timing, measured rather than asserted (M7.0, ADR-0029)

`m7:test:timer_facility` is the first test whose subject is time
itself, and it is written as a MEASUREMENT: `timertest` reads
`SYS_CLOCK_NOW` before and after a real 50 ms timer and reports the
elapsed microseconds, which `tools/test_m7.py` then parses and checks
from the host side. A boot prints, for example:

    timertest: PASS — 50000us timer delivered after 52096us
    (never early, 2096us of lag against a 10000us tick)

The property that matters most is the negative one: a deadline must
never fire EARLY. A timer that can fire early makes every
retransmission and aging rule built on it wrong in a way that only
appears under load, so the test asserts `elapsed >= requested`
unconditionally, and bounds lateness separately with slack sized for
a loaded host.

The rest of the facility is checked the same way — a cancelled timer
must stay silent (or a TCP stack would retransmit segments it had
already acknowledged), a second cancel of the same timer must be
REFUSED (a protocol cancelling a retransmission that already went out
needs to tell the difference), and two due timers on one notification
must both deliver, since that merge is what lets a Phase 7 service
wait for "work OR timeout" in a single blocking call. The client then
leaves a timer armed on purpose and exits, so the suite can prove the
kernel sweeps it.

This suite needs no device fixture and no host actor: it is about the
kernel's own clock, so it runs and must pass on the barest machine.

### Proving a cache (M7.1, ADR-0030)

`m7:test:arp_service` resolves a real address over the real wire, and
then does the thing that actually tests a cache: it resolves the SAME
address again and asserts that the number of ARP requests netstackd
has PUT ON THE WIRE did not move. A cache that still sends the packet
would pass any test that merely checked the returned value; the only
honest evidence is silence on the wire, so that is what is measured.

The third check is the one Phase 7 was reordered for. `arptest` asks
for an address nothing answers (10.0.2.77) and the call must come
back `UNREACHABLE` — which above all means it must come back at all.
Before M7.0, `NET_OP_RECV` blocked until a frame arrived, so this
test could not have terminated; netd now takes a deadline and arms a
timer on its own notification, because a client blocked in
`SYS_IPC_CALL` cannot observe its own (ADR-0029's erratum). The test
also reads netstackd's own timeout counter, so the timeouts are the
service's account of itself rather than the harness's inference.

The kernel cross-checks the wire independently: transmit and receive
relay deliveries must both be non-zero, so the exchange cannot have
been a lookup table with good manners.

### A proof that cannot fail is not a proof (M7.1b)

The re-attach test took three attempts to become honest, and the two
failures are more instructive than the success.

Attempt one restarted netd and *then* released the client. It passed
— and proved only that a capability survives a restart, because the
stack called into a driver that was already back. Its own counter
said `0 re-attach(es)`, which is the tell.

Attempt two polled the supervisor inside the drain loop, which is
what production does. Same result: the first poll ran before the
client had even woken.

The test now kills netd, releases the client immediately, and waits
for the KERNEL'S OWN evidence that the stack has met the corpse —
with netd dead, the only thing left in the system that arms a timer
is netstackd's backoff, so a rise in `timer::stats().armed_total` is
unambiguous — and only then lets the supervisor work. The assertion
that the stack re-attached is read from the service's own accounting,
not inferred by the harness.

When a test of a failure path passes on the first run, the right
question is whether the failure actually happened.

### Exception-path testing (M2.1+)

Exception tests use *real* faulting instructions (divide-by-zero, writes to
unmapped memory) and must survive them: the suite arms an expected vector
(`arch/x86_64/faults.rs`), the fault site records its own resume address,
the handler verifies/recovers, and the test asserts the **measured** delivery
(vector, error code, CR2) — never the expectation. Unarmed faults still take
the full diagnostics-and-halt path, so injection cannot mask real bugs.
Since M3.3b that path also dumps the stub-saved caller-saved registers and a
bounded best-effort scan of the faulting stack for text-range return
addresses — a frame-pointer-less backtrace, valid under any CR3 (it is what
caught the wrapped MMIO-alias EOI fault: the trace led into the paging
walk reading a "table" at the LAPIC's physical address).

**Standing invariant:** `0x0000_6000_0000_0000` (m2.rs
`UNMAPPED_CANONICAL_ADDR`) must remain unmapped in *every* address space this
project ever builds — firmware's, the M2.4 kernel tables, and any later user
address space. It is the #PF test's faulting address.

## Determinism

- Fixed machine (`q35`), fixed RAM (512M), fixed CPU model (`qemu64,+nx`),
  fixed firmware blobs (pinned QEMU package).
- No wall-clock assertions; time sources are treated as monotonic counters.
- Scheduler tests use an explicit deterministic round-robin mode (M3).
- Fresh OVMF_VARS and fresh ESP image every run (no state survives between
  runs).

## Regression discipline

- Milestone test scripts are never deleted; `tools/run_tests.sh` runs *all*
  of them. Green means: every milestone ever completed still completes.
- Any change that makes an old test flaky is treated as a kernel bug until
  proven otherwise.

## Case study: the M1 timer test (post-mortem, kept as a teaching artifact)

`m1:test:timer_irq_absorbed` went through three failure modes before it was
deterministic; each taught a rule now baked into this document.

1. **Triple fault on the very first interrupt.** Our `IdtGate` struct encoded
   the type/attribute byte at the wrong offset (a packed-layout mistake), so
   every gate looked non-present/ill-typed to the CPU: `#GP(vector*8+2)` →
   `#DF` → triple fault, with *no* serial output. Rule: **tests must assert
   architectural byte layouts read back from hardware** — `test_idt_installed`
   now audits all 256 live gates (selector, attribute, IST, handler offset)
   against independently computed expectations, never against the same struct
   that wrote them (a self-consistency audit would have passed).
2. **Flaky single-tick pass/fail.** Probes showed firmware hands off with both
   8259 IMRs at 0xFF and the PIT tick routed IOAPIC → LAPIC. Our stub EOI'd
   only the 8259, so the LAPIC in-service bit stayed set: exactly one tick
   could ever be absorbed per boot, and the test passed only when that tick
   landed after the `before` snapshot. Rule: **a test that can pass on a
   single lucky hardware edge is not a test** — it now requires *two*
   absorbed ticks, which only happens if the full
   deliver → absorb → EOI → re-deliver cycle works.
3. **A #DE "divide error" from an instruction with no division.** An interim
   fix unmasked 8259 IRQ0; because firmware never programmed the 8259 vector
   base (irq_base=0), the ExtINT acknowledge delivered *vector 0* — our own
   gate-0 diagnostic stub, reporting a #DE at a plain `mov` load. The
   diagnostic stub earned its keep (vector, error code, RIP, CS, RFLAGS all
   on serial), and RIP-to-source mapping worked without symbols by anchoring
   the runtime image base via the printed GDT address and disassembling the
   PE at the computed offset. Rule: **probe the machine, not the manual** —
   firmware handoff state (both interrupt controllers, masks, vector bases)
   is a measured fact in this repo, documented in `ARCHITECTURE.md`.

Known quirk (not a bug): the `total_described=` MiB figure in
`m1:test:memory_map` exceeds installed RAM because firmware memory-map
descriptors include huge reserved MMIO spans (64-bit PCI window); the RAM
figures are `conventional=`/`reclaimable=`.

## Adding tests for a new milestone

1. Define markers in the kernel (`mN:test:<name>`), each backed by a real
   assertion path (no `assert!(true)` theater).
2. Add `tools/test_mN.py` modeled on `test_m1.py` (shared helpers live in
   `tools/qemu_env.py` / `tools/espimg.py`).
3. Wire it into `tools/run_tests.sh`.
4. Document exit criteria in `docs/ROADMAP.md`.

## Future layers (scheduled, not speculative)

- **Fault injection** (M2+): deliberate exceptions, corrupted structures fed
  to parsers, double-frees against debug allocators.
- **Stress loops**: the 100-boot stability loop shipped in M2.7
  (`tools/stability_loop.sh`, ADR-0011) — every boot must show every
  milestone's RESULT line (`m2: RESULT PASS (21/21)` … since v0.7.0
  `m6: RESULT PASS (2/2)`), the clean-halt declaration, no PANIC, QEMU
  exit 0, with the full fixture family attached (scratch disk + slirp
  NIC + entropy source — stability means the SHIPPING configuration). The v0.6.0 loop
  caught its first host-coupled flake at 1-in-100: the device-bound
  suite drains were bounded by a *yield count*, which under host load
  burns out before QEMU's iothread delivers the awaited MSI — they are
  now bounded by an HPET wall-clock deadline (~21 s; see
  `DRAIN_DEADLINE_TICKS` in `m5.rs`/`m6.rs`); allocator churn tests.
- **Host fuzzing** (Phase 5+): boot-info parser, FS metadata parser, image
  loader — all reachable from host unit-test binaries.
- **KVM acceleration** when the host allows it; test semantics unchanged.
- **Packet capture assertions** (Phase 7): QEMU slirp/tap + tcpdump-grade
  checks that bytes actually went on the wire.
