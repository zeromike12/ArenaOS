# ArenaOS

A general-purpose operating system for x86-64 UEFI computers, written from
scratch in Rust. Not based on Linux. Not a Unix clone. A capability-based
hybrid microkernel with userspace drivers, explicit permissions, and its own
APIs — built milestone by milestone, where every milestone is demonstrated by
automated tests that boot the real system in QEMU.

> This is a serious long-term engineering project. The architecture is
> documented before it is built, every decision that matters has an ADR, and
> the tree is bootable at every commit.

## Status board

| Phase / Milestone | State | Evidence |
|---|---|---|
| Phase 0 — Architecture (vision, ADRs, toolchain) | ✅ complete | `docs/` |
| **Milestone 1 — First boot** (UEFI → kernel → verified diagnostics → safe halt) | ✅ **complete** | `tools/test_m1.py` (8/8 in-guest self-tests, clean QEMU exit) |
| **Milestone 2 — Kernel foundations** (exceptions, timers, frames, paging, heap, locks, boot split) | ✅ **complete** — 2.1 (TSS/IST, exception recovery) · 2.2 (PIT/TSC, monotonic clock, 100 Hz tick) · 2.3 (frame allocator, ADR-0007) · 2.4 (own page tables, higher-half, W^X/WP/NX enforced, ADR-0008) · 2.5 (kernel heap, ADR-0009) · 2.6 (spinlocks, irqsave critical sections, ADR-0010) · 2.7 (boot split: ExitBootServices → kernel proper, reclaimed timer chain, farewell-island shutdown, ADR-0011) | `tools/test_m2.py` (21/21) + 100/100-boot stability loop + host suites (arena-heap 12/12, arena-sync 6/6) |
| **Milestone 3 — Multitasking** (threads, scheduler, processes, capabilities) | ✅ **complete** — 3.1 (kernel threads + context switch: callee-saved frame, no-FPU invariant build-enforced, canaried 32 KiB stacks, exact-accounting reap, ADR-0012) · 3.2 (timer-driven preemption: tick hook, nested cooperative switch, per-CPU run queues, exact-RR proof on yield-free threads, ADR-0013) · 3.3 (ring-3 threads, syscall/sysret boundary, TSS RSP0, SMEP/SMAP armed and fault-tested; processes = address-space objects: private PML4, cloned kernel half, exact teardown, ADR-0014) · 3.4 (capability spaces: per-process slot tables, attenuation-only delegation, right-gated destroy, gated `process_root`/`map_memory` invokes, ADR-0015) | `tools/test_m3.py` (13/13) + m1/m2 regressions green + 100/100-boot stability + release [v0.3.0](https://github.com/zeromike12/ArenaOS/releases/tag/v0.3.0) |
| **Milestone 5 — Storage** (userspace VirtIO-blk driver → own filesystem design → persistence & crash consistency) | ✅ **complete** — 5.1 done (driver substrate: Untyped/Mmio capability kinds, `SYS_ALLOC_FRAME`/`SYS_MAP_MEMORY` self-map windows, IRQ relay vectors 48..63 → notifications, kernel-side PCI enumeration with the virtio capability walk and MEM\|BUS MASTER as kernel policy; the harness grows a fresh scratch `virtio-blk-pci` disk, ADR-0021) · **5.2 done** (`userspace/storaged`: the resident userspace virtio-blk driver — full ring-3 virtio 1.0 handshake, one split virtqueue over self-allocated frames, zero-copy DMA through LENT buffer caps, MSI-X completions relayed into `SYS_WAIT`, block service spawned at boot; write→read-back→verify proven THROUGH the service boundary by `blktest`, ADR-0022) · **5.3 done** (`userspace/fsd`: AFS1 — the own-design filesystem with extent-based data and CoW transactional metadata, host-side `mkfs`, IPC v1.1 inline messages, end-to-end zero-copy file I/O by cap forwarding, shell `ls`/`cat`/`write`, ADR-0023) · **5.4 done** (two-boot persistence — written in boot N, read byte-exact in boot N+1 with the host parsing the committed sectors; the crash-consistency gate — five SIGKILL-mid-write rounds, every reboot recovers with NO repair tool, crashed writes return never-committed/committed-empty/committed-full and never torn; transactional UNLINK behind the shell's `rm`; mount-time reclamation of superseded commits; fstest's dual fresh/persisted op contracts → **v0.5.0**) | `tools/test_m5.py` (6/6) + `test_m5_persist.py` (two boots) + `test_m5_crash.py` (5 crash rounds) + m1–m4 regressions green in the same boot + release [v0.5.0](https://github.com/zeromike12/ArenaOS/releases/tag/v0.5.0) |
| **Milestone 6 — Drivers** (VirtIO family: net → rng → input → console, then supervised restarts) | ✅ **complete** — **6.5 done** (supervised restart, ADR-0028: isolation is only half a promise, and this is the other half. A service that dies now answers everyone it owed a reply to with a typed `STATUS_SERVICE_GONE` — found by CAPABILITY, since the endpoints a process served are exactly the ones it holds `Endpoint`+READ for — instead of leaving clients blocked forever with no timeout and no way to see it. A process with PARKED threads can finally be killed, which `State::Blocked` has promised since M3.1 and which a supervisor needs, because a driver worth restarting is almost always blocked. And `kernel/src/supervise.rs` respawns the image with its grant list REPLAYED while the endpoint — and therefore every client capability — survives untouched, so clients never learn a pid and a restart costs them one retry. `faultd`/`faulttest` drive the whole cycle on real processes: live service, blocked client, kill, restart with 3 caps replayed, the blocked client answered and exiting cleanly, a NEW client reaching the restarted instance through the capability it held before the crash — frame-exact, with the spawn-record table flat across it (closing ADR-0025's GC debt). Not claimed: back-off, dependency ordering, state recovery, or a ring-3 supervisor — minting an Mmio cap is the one authority the kernel has never delegated) · **6.4 done** (`userspace/consoled`: the resident userspace virtio-console driver, the fifth on the shared virtio core and the first with a queue in each direction. **The roadmap said move the shell's stdout/stdin onto it; that was decided against** — a port is not a console. The kernel keeps ONE line discipline and drivers attach CHANNELS to it: inbound reuses M6.3's `ConsoleInput` gate unchanged, outbound is a new `ConsoleOutput`-gated MIRROR of the console's byte stream (`SYS_CONSOLE_ATTACH`/`SYS_CONSOLE_PULL`), so `nc -U` on the host socket is a full second console — what the machine prints arrives there and what you type there drives the same `arena>` prompt, with serial still the kernel's own for logs and panics. Two separate capabilities on purpose: a keyboard must not thereby gain the power to read everything the machine prints. MULTIPORT declined (two queues, no control protocol); buffers posted AFTER DRIVER_OK because QEMU pauses a chardev the guest cannot yet read. **Two latent kernel bugs surfaced and were fixed**: the M5.1 relay interrupt stubs clobbered `rcx` before saving it — any code interrupted with a live `rcx` resumed with the vector number in its place — and boot-time TSC calibration halted the machine whenever a host stall skewed one PIT window, so a busy laptop could fail to boot; ADR-0027) · **6.3 done** (`userspace/inputd`: the resident userspace virtio-input keyboard driver, the fourth on the shared virtio core and the first with a device-writable queue that must stay stocked — 32 event buffers posted from the ring frame, because QEMU drops an ENTIRE event batch it cannot place; evdev events harvested to the used ring's end per interrupt, decoded through a US-ASCII keymap with modifier tracking. **The roadmap said switch the shell's read path off serial; that was rejected** — it would have broken every automated boot and every headless user. Instead a new `SYS_CONSOLE_PUSH`, gated on a new `CapObj::ConsoleInput` singleton, feeds decoded bytes into the kernel's EXISTING line discipline — the same entry point the UART RX ISR uses — so the shell needed zero changes and serial and keyboard are both live. A zero-length push is the capability probe that selects console mode from service mode, and the m6 suite withholds the cap so the switch is under test. Proofs: `inputtest` verifies harness-typed keystrokes decoded through the service boundary, and `test_m6_typing.py` drives the PRODUCTION path with the serial input channel dead — capitals prove shift, a backspace erases a typo before the shell sees it, and the machine halts because someone typed `shutdown`. A keyboard attached with nobody typing is a tested configuration too: a spawner-only give-up badge ends the wait and the suite reports an honest SKIP, because attaching a device must never make a machine unusable; ADR-0026) · **6.2 done** (`userspace/virtio.rs`: the shared virtio 1.0 core — ADR-0024's third-driver rule executed mechanically under the full suite, so discovery/window-map/handshake/frame-budget/queue-setup/ring primitives exist ONCE and storaged + netd shed 268 lines each with zero behavior change · `userspace/rngd`: the resident userspace virtio-rng driver — one request queue, `RNG_GET` pointing a device-writable descriptor at the caller's LENT frame so the device DMAs entropy straight into the client's own page, MSI-X-completed; `rngtest` proves real variance — two 4 KiB draws, full-length by the device's own count, not all-zero, not one repeated byte, and different from each other, with the kernel counting exactly one interrupt per draw; entropy QUALITY stays the host backend's business and is never claimed; registry caps raised 8 → 12, ADR-0025) · **6.1 done** (`userspace/netd`: the resident userspace virtio-net driver — link layer only, raw Ethernet frames; two split virtqueues packed one frame per queue under the cap-slot budget, two MSI-X relay badges into one notification, zero-copy TX chaining the caller's LENT frame behind the driver's own virtio header, the config-space MAC through DEV_INFO word [6], and order-independent device discovery that storaged adopted too; the m6 suite proves the wire — a hand-built 42-byte ARP request to slirp's 10.0.2.2 and the reply verified field-by-field at exact wire offsets, exactly one counted interrupt per relay vector, frame-exact teardown; NO NIC attached → an honest SKIP with the service offline, so pre-v0.6.0 invocations stay green, ADR-0024) | `tools/test_m6.py` (6/6 + the no-fixture SKIP boot + the nobody-typing SKIP boot) + `tools/test_m6_typing.py` (the shell driven from the keyboard) + `tools/test_m6_console.py` (the shell driven entirely over the console port, serial input dead) + m1–m5 regressions green in every boot + release [v0.10.0](https://github.com/zeromike12/ArenaOS/releases/tag/v0.10.0) |
| Milestone 4 — Userspace & first program | ✅ complete — 4.1 (executable format + image loader: ELF64 container with ArenaOS strict-subset semantics — ET_EXEC-only validator, W^X segments, zero-filled NOLOAD bss, exact-accounting load into a process space; test image is a genuine cargo/rust-lld artifact in `userspace/payload`, ADR-0016) + 4.2 (syscall ABI v1: six argument registers, typed i64 status codes, frozen call registry — `debug_write`/`thread_exit` — with the marshalling and callee-saved promises proven from ring 3, ADR-0017) + 4.3 (first user process: the real rust-lld image runs at ring 3 in its own address space — writes its pinned message via debug_write byte-identical to the file, stamps bss from ring 3, exits via thread_exit with META's own success code) + 4.4 (IPC v1: endpoint rendezvous with blocking call/reply, badged merged notifications, capability transfer in messages — an echo-server demo across two real processes, ADR-0018) + 4.5 (spawn protocol v1: SYS_SPAWN from image capabilities with explicit attenuating inheritance, Process handles, exit-badge notifications — a ring-3 supervisor spawns the real image twice and the restart is visible as its console message appearing twice, ADR-0019) + 4.6 (minimal shell: interrupt-driven console input with a kernel line discipline, a real second userspace image spawned at boot as the initial service, builtins help/ps/echo/spawn/shutdown, the machine halt gated on a Power capability — every test boot now ends by typing `shutdown` into the running shell, ADR-0020) done | `tools/test_m4.py` (9/9) + `tools/test_m4_shell.py` (interactive session, 26 checks) + m1/m2/m3 regressions green |
| Phases 5–10 — Storage, drivers, net, userspace maturity, graphics, desktop | ⬜ | `docs/ROADMAP.md` |

## Quickstart (this sandbox)

```bash
tools/dev-env/bootstrap.sh     # one-time: Rust 1.97 + bare-metal sysroots + QEMU/EDK2 + musl (idempotent)
source tools/dev-env/env.sh    # shell environment
tools/run_tests.sh             # build → boot in QEMU → assert milestone markers → verdict
tools/run.sh                   # interactive: watch the system boot on serial (Ctrl-A X to exit)
```

On a normal workstation with system packages (`qemu-system-x86`, `ovmf`,
Rust with the `x86_64-unknown-uefi` target), the same scripts work
unmodified — see `docs/DEV-ENV.md` for resolution order and overrides.

## Run a released build in your own QEMU

Every completed milestone ships as a GitHub release: a prebuilt boot
image plus the exact EDK2 firmware pair it was tested against. See
**[docs/RUNNING.md](docs/RUNNING.md)** — two `cp`s, one
`qemu-system-x86_64` command, serial is the console in BOTH directions:
after the boot-time test suites pass, the kernel spawns the shell and
the machine waits for you at the `arena> ` prompt — type `help`, and
`shutdown` when you are done.

## What just booted (current: Phase 6 in progress — an interactive shell over its own AFS1 filesystem, served entirely from ring 3 — files written in one boot come back byte-exact in the next, crashes mid-write recover with no repair tool — plus a ring-3 NIC driver that proves itself on the wire every boot with a real ARP round trip, and a ring-3 entropy driver that DMAs fresh randomness into its clients' pages)

QEMU/OVMF loads `EFI/BOOT/BOOTX64.EFI` (our Rust boot stage). It brings
up serial, GDT/IDT/TSS, the 16550 UART, and the real UEFI memory map;
calibrates the TSC against the PIT oscillator; installs its own page
tables (higher-half direct map, per-section W^X, WP+NXE enforced and
fault-tested); grows a guarded kernel heap from the frame allocator; and
proves spinlock/irqsave critical sections against live PIT hardware —
re-running every Milestone-1 self-test on the way (`m1: RESULT PASS
(8/8)`).

Then the M2.7 handoff (ADR-0011): `ExitBootServices()`, CR3 switches to
the kernel-only view through a trampoline (identity mapping torn down,
verified by a recovered fault), PE base relocations are replayed at
`+KERNEL_OFFSET`, and `kmain` validates the `BootInfo` ABI record. The
kernel reclaims the timer chain firmware left behind — the PIT arrives on
IOAPIC **pin 2** (the classic ISA-IRQ0→GSI-2 override), re-routed to our
vector — and proves two live ticks through the relocated IDT with
kernel-alias LAPIC EOI (`m2: RESULT PASS (21/21)`).

Then Milestone 3 begins: `kmain` registers itself as the bootstrap
thread and the scheduler runs real kernel threads (ADR-0012) — exact
round-robin interleave order, callee-saved registers round-tripped
*through* a live context switch, canaried 32 KiB stacks that stay
disjoint under depth-200 recursion, and a 127-thread churn whose frame
and heap accounting returns exactly to baseline. Then the 100 Hz PIT
tick starts driving the scheduler itself (ADR-0013): three threads that
contain *no yield call at all* are rotated in exact round-robin order
by timer preemption (~200k loop iterations each), and cooperative
yields provably compose with 10 ms quanta. Then the ring-3 boundary
comes up (ADR-0014): hand-assembled user payloads execute at CPL 3 and
cross it through real `syscall`s with SMEP/SMAP armed — a privileged
instruction in user mode faults and resumes, a bare kernel read of a
user page takes the SMAP #PF — and processes become address-space
objects: private PML4s with cloned kernel halves, same VA over distinct
frames per process, exact create/destroy accounting. Every process also
anchors a 16-slot capability space (ADR-0015): rights attenuate on
copy (amplification is a loud refusal), destroy is right-gated and
removes references only — dangling caps refuse cleanly — and two
gated invokes (`process_root`, `map_memory`) bind untyped frames into
a target's address space under WRITE rights on both caps
(`m3: RESULT PASS (13/13)`). Milestone 4 begins with the executable
format (ADR-0016): a genuine cargo/rust-lld ELF artifact
(`userspace/payload`, embedded at compile time) is validated field by
field against the ArenaOS strict ELF subset, 25 mutation classes of it
are refused, and a real process loads it — 3 pages, 7 frames, PTEs
W^X-exact, entry stub + META manifest + 4 KiB of zeroed NOLOAD bss
read back under the *target's* CR3 through STAC-bracketed accesses,
double-load refused at zero cost, teardown exact. The syscall ABI v1
(ADR-0017) then proves six-register marshalling and typed status codes
from ring 3; the payload image RUNS as the first user process (M4.3);
IPC v1 (ADR-0018) lands endpoints, blocking call/reply, badged
notifications and capability transfer with an echo-server demo across
two processes; the spawn protocol (ADR-0019) lets a ring-3 supervisor
create processes from image capabilities with explicit attenuating
inheritance and restart a worker through its exit badge; and the
console learns to LISTEN (ADR-0020): COM1 RX on IRQ4/vector 33, a
kernel line discipline, and a real second userspace image — the shell —
spawned at boot as the initial service (`m4: RESULT PASS (9/9)`).

Milestone 5 has begun (ADR-0021): the kernel now enumerates PCI bus 0
at boot — every function logged, BARs sized, each VirtIO device's
capability structures resolved, MEM|BUS MASTER enabled as a kernel
POLICY decision (config space is never exposed to ring 3) — and the m5
suite proves the substrate userspace drivers will run on: ring-3-owned
untyped frames (`SYS_ALLOC_FRAME` → `SYS_MAP_MEMORY` self-map → the
mapped cap is consumed → destroying an unmapped cap returns exactly its
frame → teardown frame-exact), device registers under ring-3 eyes (the
HPET main counter through a kernel-minted `Mmio` cap, read-only,
strictly increasing), and IRQ relay vectors 48..63 turning a live
LAPIC IPI into a notification badge that wakes a parked waiter.

Step 5.2 put the first SERVICE in userspace (ADR-0022): `storaged`,
a real virtio-blk driver process spawned at boot, runs the entire
virtio 1.0 handshake and virtqueue mechanism in ring 3 on a
kernel-granted MMIO window, arms its MSI-X interrupt through
`SYS_IRQ_RELAY` (the kernel programs the table and the enable bit —
routing stays policy), and serves a zero-copy block protocol: callers
LEND a buffer cap with the request, the device DMAs the caller's own
frame, and every completion is an interrupt relayed into the driver's
`SYS_WAIT`. The m5 suite proves the boundary from the other side —
`blktest`, a separate process, wrote a 512-byte pattern to the scratch
disk, cleared its buffer, read the sector back through the service,
and verified every byte, with exactly two interrupt-delivered
completions counted and frame-exact teardown. The production storaged
instance parks resident beside the shell — `ps` lists both.

Step 5.3 built the FILESYSTEM on top of it (ADR-0023): AFS1, an own
design — extent-based file data written in place, copy-on-write
metadata (object table + allocation bitmap CoW'd to fresh runs per
transaction), and a ping-pong commit record that flips generations in
ONE sector write, with two-generation-delayed freeing so a crash can
only leak, never corrupt. `tools/afs1.py` formats the scratch disk
host-side and mirrors the layout byte-for-byte with `userspace/fsd`
(registry image 4, spawned between storaged and the shell). Names and
dirents ride IPC v1.1's optional 64-byte inline message; file data
never touches fsd at all — the client LENDS its frame, fsd FORWARDS
the untouched cap to storaged, and the device DMAs between the disk
and the CLIENT's page (lent caps cannot be mapped, so the capability
system itself enforces the zero copy). The m5 suite proves it
(`m5: RESULT PASS (6/6)`): `fstest` ran create→write→close→RE-OPEN→
read→byte-for-byte verify→ls entirely in ring 3, with relay
deliveries exactly matching the derived 33-operation contract; after
the boot, the harness parses the committed image and verifies the
file's real sectors on disk. The production fsd mounts what the suite
committed — the same boot's persistence-across-re-open proof — and
the shell serves `ls`, `cat NAME`, and `write NAME TXT` through its
new endpoint cap.

Step 5.4 made that filesystem DURABLE (ADR-0023 addendum). The same
scratch.img now survives boots: written in boot N, read back
byte-for-byte in boot N+1 (`tools/test_m5_persist.py` proves it with
the host parsing the committed sectors after each boot). The suite
itself became a persistence witness — fstest probes the volume first:
a fresh disk runs the create contract (34 device ops, exit 42); a disk
that survived a reboot runs the persisted contract, verifying the
committed file with ZERO writes (14 device ops, exit 43). The
crash-consistency gate (`tools/test_m5_crash.py`) SIGKILLs QEMU at
five points of an in-flight write; every reboot recovers with no
repair tool — the crashed write comes back never-committed,
committed-empty, or committed-full, never torn, and the host-side
fsck-lite (`afs1.audit()`) finds zero problems. The shell grew `rm`
(transactional UNLINK: the name vanishes atomically with the commit
flip; its sectors are reclaimed two generations later), and fsd's
mount reclaims the superseded ping-pong generation, so long-lived
volumes stop bleeding metadata sectors.

Phase 6 opened with step 6.1: a real NIC driver (ADR-0024).
`userspace/netd` — spawned at boot as the third resident service —
drives virtio-net entirely in ring 3, link layer only: raw Ethernet
frames in and out, two virtqueues packed into the capability-slot
budget, two MSI-X interrupt badges into one notification, zero-copy
transmit (the frame is LENT through IPC and the device DMAs the
caller's own page, chained behind netd's own virtio header), and the
MAC read from the device's config region — no new syscalls were
needed, because the M5.2 substrate already generalized. Every boot
proves it on the wire: the m6 suite's `nettest` hand-builds a 42-byte
ARP request for QEMU's built-in gateway 10.0.2.2 and verifies the
reply that comes back — field by field at exact wire offsets, one
counted interrupt per relay vector, no polling anywhere. No NIC
attached? An honest SKIP and the network service stays offline —
pre-v0.6.0 invocations boot green, and a skip is never laundered into
a pass count.

Step 6.2 asked the question every growing driver family faces: when
does duplication become a library? ADR-0024 had answered it in
advance — at the THIRD driver, because two instances cannot tell you
which seams are real. So `userspace/virtio.rs` now holds the virtio
1.0 core once (discovery, the window map, the §3.1 handshake, the
frame budget, queue setup, the ring primitives), storaged and netd
each shed 268 lines, and the extraction was proven by running the
entire suite green BEFORE any new feature was added — sameness first,
behavior second. Then the third driver: `userspace/rngd`, the entropy
service, whose `RNG_GET` points a device-writable descriptor at the
caller's LENT frame so the device DMAs randomness directly into the
client's own page. Proving an RNG honestly is its own puzzle — you
cannot assert expected bytes without fixing the entropy, which would
be the fake this project refuses — so the suite asserts what every
genuine source satisfies and every broken one fails: two 4 KiB draws,
each the full length the DEVICE reports writing, neither all-zero,
neither a single repeated byte, and the two different from each
other, while the kernel counts exactly one interrupt per draw. The
serial log prints fresh fingerprints every boot. Entropy *quality*
belongs to the host's CSPRNG and the docs say so plainly.

The machine no longer shuts itself down: it ends the suites by handing
the console to the shell and waits at the `arena> ` prompt. It stops
only when a process holding the Power capability asks — `shutdown` in
the shell routes through the farewell island that hands control back to
firmware's `ResetSystem` in firmware's own address space (the test
harnesses type `shutdown` for you, marker-paced, so every automated
boot proves the whole chain). Every claim above is a machine-checked
serial marker; nothing is decorative.

## Documentation map

- [docs/VISION.md](docs/VISION.md) — what ArenaOS is and what it refuses to be
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — system architecture, kernel
  architecture comparison & choice, memory/process/IPC/driver/security/FS/API
  philosophy
- [docs/adr/](docs/adr/README.md) — architecture decision records
  (language, kernel architecture, boot strategy, dependency policy, testing,
  ABI philosophy, allocator, address space, heap, sync, boot split, threads,
  preemption, processes/ring-3, capabilities, executable format)
- [docs/ROADMAP.md](docs/ROADMAP.md) — milestones with exit criteria + the
  "do not build yet" firewall
- [docs/RISKS.md](docs/RISKS.md) — risk register
- [docs/DEV-ENV.md](docs/DEV-ENV.md) — toolchain bootstrap, build/boot/debug
- [docs/RUNNING.md](docs/RUNNING.md) — running a release build in your own QEMU
- [docs/TESTING.md](docs/TESTING.md) — testing doctrine and marker grammar
- [docs/CODING-CONVENTIONS.md](docs/CODING-CONVENTIONS.md),
  [docs/REPO-LAYOUT.md](docs/REPO-LAYOUT.md)

## Ground rules (enforced, not aspirational)

1. **Always bootable.** Every commit builds and passes all milestone tests.
2. **Never fake functionality.** A subsystem is done when a test would fail
   if it were faked (ADR-0005).
3. **Control scope.** Smallest useful version of everything; the firewall
   list in ROADMAP.md is binding.
4. **Decisions get ADRs.** Architecture does not live in chat history.
