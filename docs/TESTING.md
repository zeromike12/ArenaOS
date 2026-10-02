# ArenaOS — Testing Strategy

Governing rule (ADR-0005): **a subsystem exists only when a test would fail
if it were faked.** "Printing success" is not a test. Tests assert observable
machine effects: bytes that came back out of a UART in loopback mode, register
values read back after a GDT load, firmware structures parsed from real UEFI
memory, privilege-level changes, VM exit behavior.

**Phase/milestone commit gate:** run `tools/run_tests.sh`, then build the
final ESP (`tools/build.sh --image`) and run `tools/stability_loop.sh 100`
against that exact image **before committing completion**. Require 100/100
with no failures and compare `build/stability-receipt.txt` to the
SHA-256 of `build/arena-boot.efi`. A rebuilt/different kernel invalidates
the receipt. This is now a commit discipline, not only a release gate;
release staging/publishing retains its own full-suite and receipt check.

**Phase 8.4 signed-package staging completion gate (ADR-0053):** the
pinned exact-source/no_std guest verifier, static stack bound and signed
canonical bytes are tested by `tools/test_m84_stage.py` (durable root/
subordinate stages and policy, cap/marker refusals, eight authentic signed
revocations, downgrade/conflict, full table/disk, degraded reboot and held-
Process-cap manager restart), `tools/test_m84_crash.py` (nine STAGE and seven
POLICY actual SIGKILL AFS1-prefix boundaries), and controlled guest
`test_m84_red_control.py` and `test_m84_wrong_root.py` (mutant images
restored byte-exact afterward). A ninth distinct APOL v1 digest cannot be
encoded in its eight slots; the user approved an issuer-side typed
pre-signing `NoSpace` proof instead of a fictitious guest-ninth artifact.
The final full suite passed **47/47**, final EFI SHA-256
`5fd9101725a955e7846204b16df6bc8fc87cea93eb359e04a535bac38bc66508`
passed an artifact-bound **100/100**, and the extracted
`phase84-complete` bundle booted from its bundled firmware/AFS1 platter.
Old OVMF-only pre-entry timeouts were recorded as failures of earlier runs,
not counted as part of the successful final 100/100; their exact firmware
cause remains unknown. This milestone is **staging only**, not an installer,
activation path or a production signing/release ceremony. Full proof and
qualification receipt are in `docs/adr/0053-gate-evidence.md`.

**Phase 8.3 linked-library completion gate (ADR-0052, reopened for reply-cap disposal):**
`test_m83_returned_cap.py` exercises the *production* linked `Syscall`
client in two real ring-3 faulttest instances. The isolated faultd
server deliberately returns an inert notification cap on 40 calls per
client. The client requires typed `ReturnedCap`, an undescribable
would-be landing slot, exact 2/32 cap occupancy after **each** reply,
more calls than the fixed table can hold, and a normal no-cap PING after
the storm; the original M6 death/restart proof must still pass. The
new regression was run on a deliberately removed cleanup and failed
on a still-describable slot and kernel halt; it passed after restoring
the cleanup. A failed `SYS_CAP_DESTROY` is reported as a transport
failure, never as contained authority. The FS helpers bound outgoing
requests only; successful server handles, byte counts and cursors are
caller-validated, not certified by the library. The original 8.3
42-suite/100-boot receipt remains a prior image, not proof for the
corrected binary. The corrective suite passed **43/43**, a fresh rebuilt
EFI received **100/100**, and the extracted corrected archive booted
with both 40-reply proofs, real-wire `netlib`, clean disk audit, and
Power-cap shutdown; exact hashes are in ADR-0052.

`userspace/arena-lib` is a separately compiled no_std rlib, not a module
included only by its original test client. `tools/run_tests.sh` host-builds
and tests its syscall/IPC and filesystem contracts, then compiles it for
`x86_64-unknown-none`; the retained native network API fake-transport
suite checks bounded UDP/TCP/DNS wire bytes and bearer lifecycle.
`test_m83_libraries.py` boots two independent linked real FS consumers
(`fstest` and the permission broker) against fsd's actual AFS1 device and
two independent linked network clients (`arptest` and shell `netlib`)
against real slirp ARP; the shell also checks wrong-kind Image refusal and
an absent-stack boot honestly SKIPs. The unchanged fstest 34 disk-operation
contract, durable ALLOW/REVOKE, prior FS crash tests, client cap audits
and real-wire regression tests remain mandatory. An exact-resource
lifecycle fixture must wait for the concurrent permission app's **real
manager-owned Process-cap reap** before taking its baseline: the first
full 8.3 attempt exposed a legitimate live-app baseline (12 processes)
versus settled 11, not a leak; the tests now use an event conjunction,
never a loosened resource count. An unrelated pre-kernel 87-byte OVMF
startup stall also timed out; the harness preserves serial/QMP diagnostics,
and no successful QEMU exit or retry is counted as proof of that failed
boot. Final suite/receipt/archive digests live in ADR-0052.

The **Phase 8.2 completion gate** retains all old suites and adds
`test_m82_permission.py` (single receiver, request/approval/bearer
separation, ordering and exact-file mediated read),
`test_m82_policy_restart.py` (same physical platter across DENY/ALLOW/
REVOKE reboot, real broker reaps, held-endpoint reacquisition, old token
refusal), `test_m82_policy_crash.py` (all **three** ALLOW/DENY/REVOKE
transitions killed at seven actual CREATE/WRITE/CLOSE/rescan/ack
boundaries each, independent AFS1 platter audit before same-disk recovery),
`test_m82_policy_refusal.py` (eighth/ninth immutable generation, real
allocator exhaustion with typed NO_SPACE then DEGRADED, visible malformed
newest record cannot fall back), and `test_m82_policy_negative.py`
(forgery, independent bearer copy, capacity-four refusal, missing rngd,
shutdown of the **real production fsd** through its receiver-verified
marker, kernel-root reap/endpoint orphaning, no stale issuance and no
false restart READY, same-platter reboot restoration).

The bounded no_std `userspace/permission.rs` codec validates the built-in
versioned 8-byte READ request separately from the 512-byte durable
ALLOW/DENY record: malformed version/scope, zero/duplicate/unknown
operations, overbroad rights and nonzero reserved/decision bytes refuse.
The manager resolves actual inventory with the already host-tested
no_std manifest (missing authority, wrong kind, duplicate/overbroad
rights); the broker independently refuses four malformed requests from
a real mediator-endpoint holder before considering an ALLOW. Host Rust
and independent Python record vectors run with a bare-metal
`x86_64-unknown-none` no_std compilation gate.
No claim is made for arbitrary commit-sector corruption, malicious disk
rollback, full no-disk boot, or identity-based permission controls.

The first full pass encountered two unexplained 120-second timeouts, one
with only 87 bytes of pre-kernel OVMF serial output. Neither was scored a
pass; `mtest` now retains the serial tail and QMP CPU registers on any
future timeout for diagnosis rather than hiding it. Subsequent full runs,
including the **final request-codec image**, passed **40/40**. This evidence
does not explain or claim to have eliminated the earlier pre-kernel
intermittent stalls. The final EFI-bound 100/100 (zero failures) receipt
and independently booted, checksum-verified extracted archive are
recorded in ADR-0048's completion section.

The earlier Phase 8.2 **volatile integration checkpoint** adds
`test_m82_permission.py` (receiver-side missing/wrong/genuine marker,
separate request vs approval vs issued 128-bit bearer, real mediated
`arena.txt` read, independent bearer copy and endpoint delegation,
receiver-side revoke and same-disk reboot default DENY) and
`test_m82_ipc_caller.py` (wrong/bounded-stall and deliberately killed
blocked `SYS_IPC_CALL` caller, exact resources, original endpoint reuse,
no false READY). M4's in-guest four-state abandoned-call fixture also
checks staged-cap disposal, typed late-reply refusal and frame cleanup.
The shared QEMU harness refuses rc=0 kernel halts and missing historical
PASS markers; the focused test checks both negative cases with synthetic
serial input. The fresh **35/35** historical suite, final-EFI-bound
**100/100** boots and independently booted, checksum-verified extracted
archive passed for EFI SHA-256
`98331c626c4febed721682fdddc6ee9c3df5a7aa48067cc9690b8e64620bbc15`.
This is **not** 8.2 completion: policy decisions are volatile. Persistent
record/restart/crash-model and serialization gates require a new image,
fresh historical suite and a new 100/100 receipt.

The earlier Phase 8.2 **cap-space foundation** adds `test_m82_capspace.py`: a
real QEMU boot checks the new fixed 32-slot M3 last-slot/overfull
capacity proof, exactly bounded Power snapshots after three production
restart cycles and clean shutdown. `tools/probe_capspace_layout.py`
compiles extracted production structs for both host and bare-metal
target; it measures static table sizes, NOT live frame use. Historical
8.1 skip/commit/no-op/eight-generation tests remain in the full suite.
No app approval or service-mediated grant is claimed by this slice.
The full suite has 33 suites. This *partial code checkpoint* passed
33/33 and a fresh final-EFI-bound 100/100; its extracted archive also
passed checksums and a real QEMU boot. Later 8.2 code checkpoints must
repeat those gates on their own final images.

The Phase 8.1 **completed checkpoint** retains the transactional-core tests:
`test_m81_update.py`
(separate service-issued updater marker; exact guest-committed value;
seven observed CREATE/WRITE/CLOSE/reply SIGKILL gates, committed platter
audit BEFORE retry and exact same-disk recovery; visible malformed
predecessor and full disk return typed refusals) plus
`test_m81_update_table.py` (eight live guest updates, all prior values
byte-exact, typed ninth refusal). Host-prepared allocator exhaustion
marks sectors used in AFS1's committed bitmap; the host audit permits
this as honest unreachable allocation, not a simulated device error.
`mtest.boot` supports a boot-time kill gate with no shell feed; its
historical command-gated kill semantics remain unchanged. Each boot's
Power-gated `stackstress` snapshot checks exact boot-relative free-frame
consumption, live spawn records and occupied process slots across skip,
commit, no-op, all eight generations and the ninth refusal; subtracting
each boot's post-EBS free-frame baseline accounts for genuine OVMF map
variation rather than tolerating a guest leak. A second marked SET after
real disk allocation failure must return DEGRADED on the same boot;
preflight table exhaustion stays NO_SPACE and exact old-value READ
survives. Full suite is 31 suites; the fresh final-image 100/100 and
extracted-bundle boot are mandatory and complete for 8.1.

The Phase 8.1 **read-boundary checkpoint** is not Phase 8.1 completion.
`tools/test_m81_read.py` boots the real guest repeatedly, including the
same platter after the trusted shell writes a malformed reserved name,
a separate offline host-provisioned valid 512-byte record (the guest
reads the exact payload), a corrupted newest checksum after an older
valid record (no fallback), and an optional-device-absent boot. Its
ordinary reader lacks fsd and update-marker rights and checks 20
receiver-side missing/wrong-kind refusals before repeating READ; host
`afs1.audit` checks each disk. Offline host provisioning is NOT a guest
transaction. Authorized SET, crash points, full-table and trusted-updater
proofs are still outstanding. `tools/stability_loop.sh 100` checks the
ordinary reader boundary on every final-image boot as well as the 8.0
stack path. The complete historical suite and fresh 100/100 must precede
any checkpoint bundle or commit.

Phase 8.0's early manifest resolver, caller-cap inventory and readiness
badge gate have host fail-closed tests in
`userspace/servicemgr/src/{manifest,inventory,readiness}.rs` (ADR-0037/0038).
The M4 shell QEMU fixture additionally uses the real describe/finish
syscalls for two spawn/reap cycles and bad-kind/stale-cap refusals.
`test_m8_bootstrap.py` now verifies a real manager boot and its live
caller-cap query against the kernel's installed-cap audit. Separate
full-network and no-network fixtures check genuine post-DRIVER_OK
readiness and fail-closed OFFLINE with no partial authority. ADR-0040
adds a manager-private restart timer notification (full fixture 11/11 after the separate admin stop channel,
twelfth refused): unlike the driver-ready event and stack backoff
objects, neither driver nor child can forge its timeout. The 100-boot
gate requires that same bootstrap on the full fixture. These
checks now include one actual manager-owned production `netstackd`
spawn, an explicit ready badge and an independent kernel audit of all
four installed child caps. `test_m8_restart.py` then types the Power-
holding shell's explicit `stacktest` command: genuine ARP wire traffic,
an rngd-backed UDP bearer held across production netstackd's orderly
exit, Process-cap reap, timer backoff, second cap-audited child on the
same endpoint, SERVICE_GONE while dead, refused old bearer and a new
ARP request on the wire. An absent dependency grants no shell client
cap. The 100-boot gate exercises this command on the final image, not
just idle boot. The subsequent negative-space and active probe suites
close 8.0 as a suite-level qualification, not a fabricated `m8: RESULT`
line on an unattended boot. `test_m8_stress.py`
checks the full restart budget on *production* children and reads a
Power-gated atomic kernel snapshot after each recovery: free frames,
spawn records and process slots must all match the live baseline
exactly. Its fourth exit must leave OFFLINE without a fifth child.
`test_m8_fault.py` separately triggers an actual CPL3 #UD inside the
production stack while its caller awaits IPC. It checks the kernel's
in-flight SERVICE_GONE sweep, manager Process-cap reap, new wire work
through the same endpoint, bearer revocation and exact accounting;
an absent dependency yields SKIP. The qualified bundle is extracted and
boots the opt-in fixture for the current checkpoint; ordinary
interactive boots do not auto-kill the stack. `test_m8_stop.py`
checks a forgery of the shared wake hint, a private admin request,
the kernel's mode-1 Process-cap operation on a still-live production
child, an in-flight caller failure, an independently audited restart
and flat accounting; no-device boots SKIP. The stop request passes no
Process handle to the shell or driver. Each new commit carries a bundled
boot image and running instructions under `releases/checkpoints/`
(ADR-0039), gated by full suite and artifact-bound 100/100 boots.
`test_m8_lifecycle.py` (ADR-0044) checks actual root-issued Process
references in the Power-holding shell against independent boot and
manager-child identities. Both finish modes refuse self, the kernel-owned
manager/drivers, and a read-only manager-owned child; guessed/empty/
wrong-kind slots and rights amplification fail. An ordinary user child
is positively reaped, resource counts stay flat, and the same production
endpoint serves fresh wire traffic. Both absent-device fixtures SKIP.
The shell receives narrowly scoped diagnostic references only from the
trusted boot root on a full fixture; neither a driver nor an ordinary
client receives them. `test_m8_dependencies.py` (ADR-0045) proves two
independently cap-audited active workers before the first stack spawn
and after its forced live stop: netd MAC, real rngd 64-byte device DMA,
private success and exit, Process-cap reap, then production readiness
and new wire. Two destructive negative fixtures distinguish a real
rngd #UD (driver IPC failure, deferred kernel-owned process teardown,
supervised driver restart but NO new stack) from an indefinitely blocked
rngd GET (manager timer deadline and authorized worker stop, NO new
stack). Both absent-device fixtures SKIP without worker authority. All
27 suites and 100/100 final-image boots qualify the four-area 8.0 exit
and ADR-0047's service-side diagnostic-authority repair;
normal boot does not inject failures or claim the destructive fixtures
ran on its own. `test_m8_diagnostic.py` exercises distinct receiving-
service refusals on the ordinary endpoint (missing and wrong marker),
then the real boot-granted authority for orderly exit, genuine #UD,
rngd #UD and a stalled GET in separate disposable VMs. The test worker
also tries 20 attenuated wrong markers with no DESTROY right before
using the legitimate proof: these must be discarded as IPC-landed
references so one untrusted caller cannot exhaust the driver's 16 cap
slots. M5–M7 legacy poison/hang test clients now prove missing and
wrong-object refusal before the authorized, clean test-only exit.
Production workers test netd/rngd SHUTDOWN refusals before both active
protocol probes; input and console service fixtures retain their own
no-marker refusal and successful diagnostic shutdown proofs.

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

**It happened again in M7.2, so this is now a rule rather than an
anecdote.** netstackd's receive parser gained a self-test that feeds
it seven frames the wire will never send (options headers, fragments,
truncated datagrams, bad checksums, an echo reply from the wrong
host) and asserts each is refused. It passed 7/7 — and then passed
7/7 again with the *old permissive parser deliberately restored*,
because the options frame carried a 20-byte checksum and was being
rejected for the wrong reason entirely.

The rule: **when a test of a failure path passes, break the thing it
tests and watch it fail before believing it.** Every synthetic frame
is now valid except for the single thing under test, and the hardened
self-test reports 6/7 against the injected regression.

### DNS and UDP negative space (M7.5, ADR-0034)

The M7.4 test proved a DNS query went out and a response with its
transaction id came back; it did NOT parse a name into an address.
M7.5 now does, and the proof deliberately spans two boundaries: the
client drains the real 61-byte response with UDP_OP_RECV_CHUNK under
the same bearer handle (a forged handle and invalid offset fail),
while DNS_OP_LOOKUP asks the stack itself to parse a fresh answer and
return an IPv4 A record. CLOSE/rebind then refuses the OLD bearer, so
revocation remains structural across slot reuse. The network test
checks these as distinct properties, not one PASS standing in for all.

The wire will not spontaneously provide a bad checksum or malicious
compression pointer. The guest self-test synthesizes a valid odd-length
UDP packet, damages ONLY a payload byte, requires the checksum failure
to be counted, and proves the zero-checksum IPv4 exception; outgoing
checksum is independently checked. `tools/run_tests.sh` also compiles
`userspace/netstackd/src/dns.rs` with host `rustc --test` to check a
compressed answer and malformed responses/queries, including a pointer
cycle and truncation. Changing the matching logic must break one of
these gates. A real external reply still proves the happy path.

### TCP burst and two-sided wire proof (M7.6, ADR-0035)

The guest OPEN must return a bearer before the handshake completes;
POLL drives the real three-way handshake with an independent Linux
socket through QEMU slirp. The host fixture binds before QEMU starts
and independently checks the exact request, sends a 200-byte
position-dependent response and observes guest EOF. The guest checks
every response byte across multiple IPC reads and confirms FIN was
acknowledged, the peer closed and the handle was revoked. A host EOF
alone is *not* sufficient: shutting down QEMU after a failed guest
boot also closes the host socket. `test_m7.py` requires both proofs;
`test_m7_no_tcp.py` boots without a peer and demands an explicit SKIP,
not a fabricated TCP success. The artifact-bound stability loop binds
a fresh host peer and checks guest completion **and** host evidence on
each of its 100 boots. Host parser tests cover malformed TCP headers,
wrong pseudo-header checksum, SYN MSS and retry timing.

This also caught a mechanism, not just a flake: netd's old RX/TX
notification badges were overlapping integers interpreted as bit masks
by `SYS_WAIT`. A merged RX notification could falsely satisfy a TX
wait and leave RECV parked with a completed frame. The fix uses
disjoint bits and a bounded completed-frame FIFO, with compile-time
badge-disjointness checks (ADR-0035).

### Native network API (M7.7, ADR-0036)

The actual no_std API module is compiled into host tests with an exact
IPC-call fake that checks operation, packed arguments and inline bytes.
The guest test also uses that same module on real slirp traffic:
ARP/ICMP/DNS/TCP plus a second UDP query, including multi-message
receive and bearer revocation. `test_m7.py` and each 100-boot
qualification require its real-wire marker alongside the independent
host TCP peer evidence; a fake transport success cannot satisfy these
boot gates. Explicit `close`/`release` must be observed by the caller.

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

### Phase 8.1 design gate (NOT a guest milestone)

The same `userspace/config.rs` module intended for configd is tested via
`rustc --test --edition 2024 -D warnings userspace/config.rs` and compiled
as a `no_std` bare-metal library through `tools/config_no_std.rs`. Its
byte vectors match the independent `python3 tools/test_config_record.py`
reference: 512-byte records, reserved namespace, pending empty files,
sequence exhaustion and visible corruption refusal. The scanner remains
poisoned if a caller ignores an ingestion error. The separate
`python3 tools/probe_config_commit_fallback.py` exposes the older-commit
fallback under *arbitrary* AFS1 commit-sector corruption; that model is
explicitly outside the accepted ordered-write/atomic-sector contract.
All four checks form one host-side suite in `tools/run_tests.sh`. This
proves neither filesystem IPC nor receiving-service authority: `configd`
does not exist yet; 8.1 remains incomplete and the 8.0 archive is still
the deployable image. A cold source-identical rebuild produced an EFI with
12 different bytes: the PE COFF timestamps and linked RSDS/PDB identifier.
Do not transfer a 100/100 receipt between those hashes; the archive's
own EFI and matching receipt remain the qualified 8.0 build.

## Phase 8.5 rebuilt dynamic Image and offline-signed update gates

Accepted ADR-0054/0055 fix the exact AINS/AACT format, one system-wide
unretired dynamic child, the existing lifecycle-admin bearer, and 18-slot
Notification table. `docs/PHASE85-WIP.md` identifies all current targeted
real guest observations and their scope. The automatic historical suite
`tools/run_tests.sh` includes `test_m85_install.py`, `test_m85_select.py`,
`test_m85_upgrade.py`, `test_m85_live_cutover.py`,
`test_m85_resources.py`, `test_m85_maximal.py`, both AINS/AACT crash
matrices, the omitted-production-hook RED/GREEN control, provisional and
live-child manager-death controls and the direct production destroy guard.
The signed fixed `app.test` commands are **private test fixtures**; they
are not a production package picker.

A clean result requires the complete `ALL TESTS PASSED (N test suites)`
line from one nonoverlapping run, then a **new**
`tools/build.sh --image; tools/stability_loop.sh 100` run with the latter's
receipt SHA-256 equal to the exact final `build/arena-boot.efi`. If there
is an OVMF-only stall or any semantic fatal/error, count it as a failed
boot; a firmware exit code of zero alone never qualifies the guest.
Run `python3 tools/checkpoint_bundle.py phase85-complete
build/phase85-qualified-suite.log` only after those gates: it rejects a
missing/stale receipt, verifies internal archive checksums, independently
extracts and boots bundled ESP/firmware/formatted disk.

## Earlier Phase 9 partial graphics gates (historical checkpoint, superseded by final qualification)

ADR-0056, ADR-0057 and ADR-0058 describe the authority and remaining exit criteria.
The earlier transport-only checkpoint already had a standalone `no_std`
toolkit with bounded clipping/text and a linked 5×7 owned glyph renderer. Displayd stages frames in a checked
SharedRegion RAM mapping: on a validated modern virtio-gpu BAR it runs a
one-outstanding polled 2D command chain; otherwise GOP fallback copies
pixels to its exclusive MMIO window with volatile stores. At that point
neither path was yet a compositor, two-window demo or graphical keyboard
route. The **72/72** historical-plus-partial-graphics suite passed after exact
shared-unmap integration and its omitted-pin guest RED control. This is a
kernel/shared-lifecycle checkpoint, **not** the final multi-client graphics
qualification or a 100/100 EFI-bound receipt.

Run `source tools/dev-env/env.sh`, then `python3 tools/test_m9_gfxkit.py`,
`python3 tools/test_m9_compositor_model.py` (host-only pure state and versioned-wire model,
**not** a compositor service), `python3 tools/test_m9_gpu2d_wire.py`
(host/bare-metal 2D wire codec only, **not** a virtio-gpu device proof),
`python3 tools/test_m9_gpu2d_red.py` (host-only omitted device ACK check
RED/restored GREEN), `python3 tools/test_m9_bar_gate_red.py` (real guest
truncated-BAR MMIO-cap refusal plus omitted-coverage RED),
`python3 tools/test_m9_gop_handoff.py`,
`python3 tools/test_m9_gpu_pixels.py` (GPU-only real-device QMP pixels,
strict ACK chain and oversized-device-mode refusal),
`python3 tools/test_m9_gpu_fallback_red.py` (forced pre-command GPU feature
rejection, actual GOP QMP pixels, byte-exact restored GPU QMP green),
`python3 tools/test_m9_font_red.py`,
`python3 tools/test_m9_shared_ref_hook.py` and
`python3 tools/test_m9_shared_info_red.py` and
`python3 tools/test_m9_shared_unmap_red.py` (omitted pin-removal guest PANIC,
exact restored GOP QMP GREEN). The GOP check captures an
actual QMP PPM and checks width, full byte length, distant colored pixels
and both a bitmap glyph foreground/background sample. The font RED test
alters an actual shipping glyph before building/booting, requires the
**real QMP pixel** assertion to reject it, restores source/EFI/ESP and
boots the green image. The SharedRegion hook control omits one real
production retirement credit and requires an independent guest
conservation halt, then restores/boots green. The INFO control omits
only the actual size-query READ-cap check, requires the ring-3
WRITE-only probe to halt, then restores byte-exact EFI and checks QMP
pixels on green. The headless GOP absence
fixture must boot cleanly without any false graphical PASS claim.

`tools/stability_loop.sh N` now captures and validates QMP pixels on every
boot; it is not a completion receipt until **after** the final code change,
all historical plus Phase-9 targeted suites pass, and a fresh exact-EFI
100/100 run exercises the actual compositor, font, both clients and real
injected input. A green GOP-only stability loop is merely a precursor.
The `phase9-complete` archive must independently extract and boot with a
freshly captured, asserted display image. Never reuse a Phase-8.5 receipt,
a pre-final Phase-9 hash or a synthetic PPM as this qualification.

### ADR-0059: M7 controlled host UDP peer (Phase-9 integration, not qualification)

Historical M7 raw IPC/native API/stack DNS still sends real virtio-net UDP
packets, now to the prebound host test actor at `10.0.2.2:1053`. A successful
network-equipped boot requires exactly three independent `DNS_FIXTURE_QUERY`
receipts in `build/udp-dns-<label>.log`. `tools/stability_loop.sh` starts one
TCP+UDP host coprocess and checks each boot's three DNS queries. The 61-byte
response exercises the inline/continuation boundary. This fixture does **not**
prove external public DNS. A standalone QEMU boot with the network attached
requires that host peer; archive boot instructions must package or document
that requirement. `tools/test_m9_host_dns_red.py` proves wrong-TXID guest RED
and unchanged-EFI restored GREEN. A prior 72/73 and stopped 31/74 were
failures, not reclassified PASS; a new full run is mandatory.

Targeted ADR-0059 recheck: `python3 tools/test_m7.py` PASS with three host
query receipts, and `python3 tools/test_m9_host_dns_red.py` wrong-response-ID
guest RED / restored same-EFI GREEN PASS. The initial red-control GREEN
attempt had no shell feeder and timed out at `arena>`; fixing that test
feeder produced the passing run. No full-suite or final-image 100/100 claim
is implied by these targeted results.

ADR-0059 complete historical/early-graphics recheck: `tools/run_tests.sh`
passed **73/73** current suites (the live compositor, two clients and
injected graphical input are not yet integrated or covered). The first
provisional two-boot stability run **failed boot 2** due to an orphaned UDP
host actor: Bash had killed a coprocess wrapper rather than its Python child.
After ownership correction (`exec python3` in the one Bash coprocess, plus
same correction in the interactive launcher), the owned orphan was stopped
and a fresh `tools/stability_loop.sh 2` passed **2/2** with real QMP pixels
and three per-boot DNS receipts. Neither this preliminary EFI nor 2/2 is
Phase-9 milestone qualification. Restore/add renderer and held-cap graphics
negative tests to the final suite before re-running all targeted gates.

### ADR-0060 live compositor / input integration (provisional, NOT Phase-9 closure)

`python3 tools/test_m9_compositor_input.py` builds the EFI and independently
captures QMP PPMs before/after a `q` key sent through QEMU's real virtio-input
path. Two separately linked ring-3 programs own disjoint 19-page SharedRegion
windows; before pixels assert each color, the later z-order in their overlap,
the bounded bitmap-font title foreground, and unchanged display pixels.
After client B takes its focused key from the compositor's bounded queue and
issues DAMAGE, screenshot pixel (200,190) changes from RGB `(61,207,122)`
to `(255,187,17)` while distant pixels remain exact. Client A sends forty
cap-bearing forged KEY requests without the input proof token and receives
forty refusals. This passes on the integrated EFI. `tools/test_m82_capspace.py`
passes with actual post-EBS guest `(frames,records,processes)=(1672,16,16)`:
three new static graphics residents over the old 13/13 display baseline;
no increase in the one-unretired dynamic-child system-wide bound. Historical
`tools/test_m7.py` and both GPU display/pixel and default-oversize refusal
checks remain passing after integration.

A fresh *provisional* `bash tools/stability_loop.sh 2` passed 2/2 on one
integrated EFI. Per boot, its actor saves independently hashed **two QMP
captures**, injects the actual keyboard key, checks owned pixels and bitmap
font/z-order before and the focused window's changed pixel after, along with
all three real host DNS receipts and historical manager restart probes. The
actor's `INPUT before-sha after-sha` receipt differs across the two captured
frames; it does not count a serial PASS as a pixel. Neither 2/2 nor the
nonfinal EFI is the required final 100/100. Forced client/service deaths,
full clean historical suite, RED/GREEN lifecycle mutation, final resource
peaks and extracted-archive pixel boot remain open; ADR-0060 records the
current limitation of cleanup only on a following compositor IPC request.

The first post-integration full-suite attempt was STOPPED after
`tools/test_m6_typing.py` failed: each accepted keyboard key caused the
compositor to print a per-key serial diagnostic, interleaving with the
historical byte-exact echoed `echo Hello-From-The-Keyboard` line. This is a
real regression, not a flaky test. Removed the per-key diagnostic and gated
the historical keyboard command on the two *actual* one-time client startup
markers, preserving its byte-exact echo/output assertions. Both
`tools/test_m6_typing.py` and the live compositor/QMP test then passed on the
corrected image. The stopped run is not a full-suite PASS.

The next complete diagnostic run reached **73/74**, failing only
`tools/test_m9_compositor_model.py`'s pre-existing `cargo fmt --check` gate:
the new GRAPHICS v1 `Poll` arm had not yet been rustfmt-formatted. It was not
counted as a green full suite. All historical QEMU tests, including the full
M8.5 crash-prefix matrix, passed in that run; the new live compositor/QMP
input test passed. The formatter failure is mechanical and will be fixed
before a fresh complete run.

ADR-0061 targeted live-death tests (not a final suite):
`python3 tools/test_m9_client_death.py` PASS (forced original B exit with
no DESTROY, compositor presents uncovered base, root reaps full Process and
region refs/maps); `python3 tools/test_m9_client_death_red.py` PASS (omit
Process-witness check, stale QMP pixel + real bounded fail-stop RED and
exact-restored source/EFI uncovered-pixel GREEN); and
`python3 tools/test_m9_service_death_red.py` PASS (mutant service dies during
production inputd's call, real IPC sweep answers `STATUS_SERVICE_GONE`,
root ERROR halts, exact-restored QMP injected-key pixel GREEN).
`python3 tools/test_m9_resources.py` PASS: Power-only guest snapshots
`(frames,records,processes)=(114445,16,16)->(114482,15,15)` across real
child exit, with shared runs/pages/maps `3/507/6 -> 2/488/4`, compositor cap
occupancy 10 then 7 or 8 (one legitimate transient A IPC landing). This is
bounded live evidence, not a source-derived capacity estimate. The final
full historical suite, exact final-EFI 100/100 and extracted graphical
archive must still run after the last code change.

Phase-9 GPU-only compositor integration: `python3 tools/test_m9_gpu_compositor_input.py` passed with `-vga none` and a real
virtio-gpu command chain, two owned overlapping windows and QMP-before/after
800×600 pixel assertions after an injected focused keyboard event. Receipt:
`build/phase9-gpu-compositor-input.log`. A previous 78-test full-suite
attempt was interrupted deliberately after historical tests through M8.1 to
add this missing GPU-only integration control. **Do not count that interrupted
run as a complete historical/graphics PASS.** The required fresh 79-test
suite subsequently passed; see the final qualification below.

## Phase 9 final graphics qualification — 2026-10-02

- `bash tools/run_tests.sh` **79/79** host, guest, historical and Phase-9
  suites, including the real GOP fallback and GPU-only two-client QMP
  before/after injected-key pixels, malformed/capacity/authority refusals,
  owner-death stale-pixel RED/restored GREEN, service-death in-flight IPC
  `STATUS_SERVICE_GONE` RED/restored GREEN, all Phase-8.5 crash prefixes and
  guest Power resource measurements. Full receipt:
  `build/phase9-full-suite-after-resume.log`.
- After the suite and last image code change, `bash tools/build.sh --image`
  produced EFI SHA-256
  `a1cb60cb8749b5cc355e146a6ea4b768dd27367959338cdd2d795811c5a8f925`.
  `bash tools/stability_loop.sh 100` on **that** EFI passed **100/100** with
  zero failures in 773 seconds. Every boot captured/checked two actual QMP
  frames, bitmap glyph, clipped owned windows, preserved base and a focused
  keyboard-driven RGB change, plus historical service/IPC/wire checks.
  `build/phase9-stability100.log`, `build/stability-receipt.txt`, and 100
  `build/stability-pixels-boot-*.txt` are the receipts; before/after pixel
  SHA-256 for each boot are retained, with full PPM samples for boots 1/100.
- `python3 tools/checkpoint_bundle.py phase9-complete
  build/phase9-full-suite-after-resume.log` checked the exact ESP EFI,
  suite markers and receipt; extracted the hashed
  `releases/checkpoints/phase9-complete/arenaos-phase9-complete-qemu-x86_64.tar.gz`;
  and independently booted the extracted image with genuine QMP pixel/input
  assertions. The archive SHA-256 is
  `ff4bb21ee7a8b31560404bc0e5c7a3232fa7f0d5f21900b2c9b15d1382c0db83`.
  Separately extracting into `build/phase9-independent-extract`, running
  `sha256sum -c sha256sums.txt` and `python3 phase9_archive_boot.py` from
  that directory **also passed**, using only packaged scripts and QEMU;
  receipt: `build/phase9-independent-boot.log`. The pixel hashes matched the
  100-boot receipt: before
  `f7b5cf77eb9dd4f5a378c1231b8953446d9609193a820157dfabcbc1e5f9b3c3`,
  after `cda78402690ce87b888f07251b264d60dd997c3841acc758896fc326303991d6`.
- `cargo fmt --check` passed for kernel, phase9-work, inputd, compositor,
  gfxkit and gpu2d; Python bytecode compilation, Bash syntax and
  `git diff --check` passed. Strict `cargo clippy -- -D warnings` passed on
  phase9-work, inputd, compositor, gfxkit and gpu2d; kernel bare-metal
  `cargo check` passed, as did non-strict kernel clippy (55 baseline
  warnings, `build/phase9-final-kernel-clippy-warnings.log`). Repository-wide
  strict kernel clippy **did not pass** and is not claimed as a green gate.
  Bare-metal builds are exercised by the full suite. No false
  release claim of pointer routing, compositor restart, public DNS or
  production signing is made.
