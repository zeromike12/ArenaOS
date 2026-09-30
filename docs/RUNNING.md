# Running ArenaOS in your own QEMU

Every completed milestone checkpoint ships a deployable, qualified QEMU
archive with its exact verified UEFI firmware pair. Tagged GitHub releases
are separate publication events. This page explains how to boot locally.

## Phase 8.2 cap-space foundation checkpoint (8.2 incomplete)

This commit's QEMU archive has the **new fixed 32-slot kernel cap
space**, with guest-tested last-slot COPY/MOVE, full-table refusal,
out-of-range refusal and frame-exact teardown. It is still Phase 8.1's
transactional store at runtime: **no permission service, CLI approval or
revocable grant exists yet**. ADR-0048 is accepted as a bounded design;
8.2 remains in progress. Download this commit's qualified image:

```sh
curl -fL -o arenaos-phase82-capspace-foundation-qemu-x86_64.tar.gz \
  https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/01a0e95e-arenaos/releases/checkpoints/phase82-capspace-foundation/arenaos-phase82-capspace-foundation-qemu-x86_64.tar.gz
curl -fL -o arenaos-phase82-capspace-foundation-qemu-x86_64.tar.gz.sha256 \
  https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/01a0e95e-arenaos/releases/checkpoints/phase82-capspace-foundation/arenaos-phase82-capspace-foundation-qemu-x86_64.tar.gz.sha256
sha256sum -c arenaos-phase82-capspace-foundation-qemu-x86_64.tar.gz.sha256
tar xzf arenaos-phase82-capspace-foundation-qemu-x86_64.tar.gz
sha256sum -c sha256sums.txt
cp ovmf-vars-template.img ovmf-vars.img
cp scratch-template.img scratch.img  # only on first boot; retain thereafter
```

Then use the QEMU command below. The prior ADR-only review archives
remain historical and run the **unchanged 8.1 binary**.

## Historical Phase 8.2 revised proposed-ADR review (no 8.2 runtime yet)

The revised, still-Proposed ADR-0048 commit carries its own deployable
archive at `releases/checkpoints/phase82-adr-reacquisition-review/`.
It reuses the exact qualified Phase 8.1 bytes and adds only a host
cap-space layout probe; see that archive's README for boot and checksum
steps. No 8.2 milestone qualification or permission service is claimed.

## Historical Phase 8.2 proposed-ADR review commit (no 8.2 code yet)

The initial proposed ADR-0048 review commit includes its own deployable archive at
`releases/checkpoints/phase82-adr-proposal/`. It is **byte-for-byte the
same archive** as the completed, qualified 8.1 image below; only the
outer filename differs. Thus it boots the same 8.1 system, and the
original 31-suite/100-boot/extracted-archive qualification applies to
those *unchanged bytes*. This is not a new 8.2 milestone, a fresh 8.2
qualification, or evidence that a permission UI exists. Read the
checkpoint's README for checksum and extraction instructions, then use
the QEMU command below.

## Phase 8.1 complete: exact per-commit QEMU build

This bootable image includes the marker-authorized immutable update,
exact post-commit rescan, crash recovery, bounded resource accounting
through all eight generations and a same-boot DEGRADED barrier after
real disk-full refusal. Phase 8.1 is **complete within the AFS1
ordered-write/atomic-sector crash model**: visible malformed config
records are rejected. Arbitrary commit-sector corruption and disk
rollback are not covered. The ordinary reader has no update marker.
Full historical suite: 31/31; fresh final-EFI-bound boots: 100/100.
Download this checkpoint:

```sh
curl -fL -o arenaos-phase81-complete-qemu-x86_64.tar.gz \
  https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/01a0e95e-arenaos/releases/checkpoints/phase81-complete/arenaos-phase81-complete-qemu-x86_64.tar.gz
curl -fL -o arenaos-phase81-complete-qemu-x86_64.tar.gz.sha256 \
  https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/01a0e95e-arenaos/releases/checkpoints/phase81-complete/arenaos-phase81-complete-qemu-x86_64.tar.gz.sha256
sha256sum -c arenaos-phase81-complete-qemu-x86_64.tar.gz.sha256
tar xzf arenaos-phase81-complete-qemu-x86_64.tar.gz
sha256sum -c sha256sums.txt
cp ovmf-vars-template.img ovmf-vars.img
cp scratch-template.img scratch.img  # FIRST boot only; retain thereafter
```

Use the QEMU command below. No update runs on an ordinary fresh boot:
`configup: SKIP` appears before the shell. To exercise the narrowly
opt-in, **trusted raw-FS test fixture** (not an 8.2 permission UI):

1. At `arena>`, type `write cfg-intent-one x`, then `shutdown`.
2. Reboot using the **same** `scratch.img` and a fresh vars copy. Before
   the prompt, the separate updater transfers its marker, writes the
   complete 512-byte `cfg8-01`, rescans, and reads `guest-v1` back.
3. For a second distinct value, type `rm cfg-intent-one` followed by
   `write cfg-intent-two x`, then reboot the same disk; `cfg8-02` is a
   separate immutable record holding `guest-v2`.

The intent file chooses test input only. It does not authorize SET;
configd checks the transferred, service-issued marker at the receiving
boundary. Neither the ordinary reader nor shell holds that marker.
There is no general configuration or permission-grant CLI yet.

## Historical Phase 8.1 transactional-core checkpoint (8.1 remained incomplete)

The earlier partial archive remains available at
`releases/checkpoints/phase81-transactional-core/`. It proved SET, crash
recovery and table/disk refusal but did **not** yet prove numeric resource
accounting or the second same-boot DEGRADED refusal. Use its own archive
and checksum, not this completed checkpoint's receipt.

## Historical Phase 8.1 read-boundary checkpoint (8.1 remained incomplete)

That earlier checkpoint bundles a bootable resident `configd` and separate
ordinary reader (ADR-0046). It reads real AFS1 records and refuses SET
without genuine transferred authority, but **does not perform authorized
SET, atomic update or crash recovery yet**. The host-provisioned record
proof is not a guest writer. For that earlier checkpoint, fetch its archive instead of the Phase 8.0
archive below:

```sh
curl -fL -o arenaos-phase81-read-boundary-qemu-x86_64.tar.gz \
  https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/01a0e95e-arenaos/releases/checkpoints/phase81-read-boundary/arenaos-phase81-read-boundary-qemu-x86_64.tar.gz
curl -fL -o arenaos-phase81-read-boundary-qemu-x86_64.tar.gz.sha256 \
  https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/01a0e95e-arenaos/releases/checkpoints/phase81-read-boundary/arenaos-phase81-read-boundary-qemu-x86_64.tar.gz.sha256
sha256sum -c arenaos-phase81-read-boundary-qemu-x86_64.tar.gz.sha256
tar xzf arenaos-phase81-read-boundary-qemu-x86_64.tar.gz
sha256sum -c sha256sums.txt
cp ovmf-vars-template.img ovmf-vars.img
cp scratch-template.img scratch.img  # only on first boot; retain thereafter
```

Then use the QEMU command below (with both virtio-net and virtio-rng).
A fresh scratch disk prints `configread: READ UNSET` and
`configread: ORDINARY READ BOUNDARY PASS`, before the shell prompt.
The ordinary reader is reaped before shell startup. Existing shell commands
remain available; there is deliberately no configuration write command.

## Phase 8 checkpoint: boot the exact per-commit build

Phase 8.0 was **complete after the ADR-0047 receiving-service authority
closure**; 8.1 is now also complete in the bounded ADR-0046 scope. Earlier 8.0 checkpoint images are historical,
not substitutes for this corrected build. Every Phase 8 checkpoint commit
includes a qualified, self-contained QEMU archive under
`releases/checkpoints/` (ADR-0039). For the **completed 8.0 service-manager**
checkpoint on this branch, download the archive directly from the
commit's repository tree (or clone and use its local path):

```sh
curl -fL -o arenaos-phase8-service-authority-qemu-x86_64.tar.gz \
  https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/01a0e95e-arenaos/releases/checkpoints/phase8-service-authority/arenaos-phase8-service-authority-qemu-x86_64.tar.gz
curl -fL -o arenaos-phase8-service-authority-qemu-x86_64.tar.gz.sha256 \
  https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/01a0e95e-arenaos/releases/checkpoints/phase8-service-authority/arenaos-phase8-service-authority-qemu-x86_64.tar.gz.sha256
sha256sum -c arenaos-phase8-service-authority-qemu-x86_64.tar.gz.sha256
tar xzf arenaos-phase8-service-authority-qemu-x86_64.tar.gz
sha256sum -c sha256sums.txt
cp ovmf-vars-template.img ovmf-vars.img   # fresh copy each boot
cp scratch-template.img scratch.img       # FIRST boot only; keep it thereafter
```

From the extracted directory, start QEMU (8.0+). This configuration
attaches the **network and entropy devices** needed by the production
stack. It deliberately omits a virtual keyboard/console-port actor, so
those optional boot tests honestly SKIP; type at the bidirectional
serial console. The absent host TCP fixture also reports an honest
SKIP, never a fake TCP pass:

```sh
qemu-system-x86_64 -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
  -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
  -drive if=pflash,format=raw,file=ovmf-vars.img \
  -drive format=raw,file=arena-esp.img \
  -drive file=scratch.img,format=raw,if=none,id=scr0 \
  -device virtio-blk-pci,drive=scr0 \
  -netdev user,id=net0 -device virtio-net-pci,netdev=net0 \
  -device virtio-rng-pci \
  -display none -serial mon:stdio -no-reboot
```

Look for `servicemgr: production netstackd READY pid ...` and the
kernel's five-cap inherited child audit, then the `arena> ` prompt. Type
`stacktest` for the **opt-in**, privileged orderly restart proof: the
same shell-held client endpoint survives the child's exit/reap/backoff,
its old rngd-backed UDP bearer is rejected, and its new instance
resolves 10.0.2.2 over real ARP. This spends one of the manager's three
restart attempts; normal boots do not intentionally restart it. Type
`help`, `ps`, or `shutdown` to exit cleanly (`Ctrl-A X` quits QEMU).
For a **destructive test VM**, `stackstress` instead repeats the real
wire/bearer restart three times with byte-exact free-frame, spawn-record
and process-slot snapshots, then proves the fourth exit exhausts the
three-restart budget and leaves the service OFFLINE. Reboot to restore
it; this is never run automatically on an ordinary boot.
`stackfault` is a separate **opt-in** crash test: a real ring-3 #UD
occurs in the production stack with an IPC call still unanswered. The
Power-holding shell observes `STATUS_SERVICE_GONE`, then proves a fresh
instance accepts the original endpoint but rejects the old bearer,
sends new ARP onto the wire, and returns to exact frame/record/process
accounting. Do not use this against a service whose state you need.
`stackstop` is the separate **opt-in** live-stop proof. A forged
shared manager-event wake leaves the first child alive. Only a second,
private admin request lets the manager use its held Process/DESTROY cap
to force-stop that live child. The stop request never delegates the
manager's Process cap to the shell. An in-flight call is failed; the
same client endpoint survives; a fresh
child rejects the old bearer and sends new ARP, and kernel resource
counts return to baseline. This spends one restart attempt.
`lifetest` is a separate **opt-in** lifecycle-authority proof
(ADR-0044). On a full device fixture, the boot root gives the existing
Power-holding shell diagnostic Process references to itself, the manager
and two drivers, plus a READ-only reference to the independently audited
manager-owned child. Both finish modes refuse these protected/read-only
targets, a forged pid, empty/wrong-kind and stale slots, and attempted
rights amplification. A normal shell child is reaped by its real held
Process cap; the production child then serves new ARP wire traffic and
resource counts stay flat. No ordinary client or driver receives these
references. Without the network or entropy device, `lifetest` SKIPs.
Destructive IPC opcodes are not enabled merely by possession of an
ordinary service endpoint. The receiver verifies a transferred,
boot-granted Notification reference; the shell carries a separate
explicit marker for `stacktest`, `stackstress` and `stackfault`.
These commands first verify missing/wrong-marker refusal in the
service. Normal clients cannot mint or infer that capability from a
numeric opcode. The manager alone delegates the rngd proof to its
opt-in worker; production storaged, fsd, netd, inputd and consoled
have no poison marker. ADR-0047 documents the legacy test-service
shutdown fixtures and malformed-cap cleanup.

The manager also probes netd's real MAC and rngd's real device-entropy
GET **before each** stack spawn, with its own timer and a Process-cap-
reaped worker. `depdeny` is a separate destructive diagnostic: after
stopping the live stack it faults rngd on the next probe GET and the
manager refuses to launch another child even though the kernel
supervisor repairs rngd. `depstall` deliberately wedges rngd's next
GET; the manager's own deadline stops its blocked worker and leaves the
stack OFFLINE. Use these only in a disposable VM, then reboot. Missing
devices SKIP rather than granting partial authority. Phase 8.0 passed
the full historical suite and an exact-image 100/100 QEMU qualification;
8.1 transactional configuration storage is next.
Earlier per-commit bundles remain available at
[`3669743`](https://github.com/zeromike12/ArenaOS/commit/3669743ecabbda26110541b7eea4ebee4b722a7f),
[`30885a8`](https://github.com/zeromike12/ArenaOS/commit/30885a8e1e2e62867aecf80e816abdd7e478791e),
[`7e79547`](https://github.com/zeromike12/ArenaOS/commit/7e79547c649f07da67ecc54cc35a996109fd6658) and
[`f1bbafb`](https://github.com/zeromike12/ArenaOS/commit/f1bbafbe117d9a814df5a684fd45716286490487).

The archive includes this document, a formatted AFS1 scratch template and the
exact EDK2 firmware pair used by qualification. Build-from-source
interactive alternative: `tools/dev-env/bootstrap.sh && tools/run.sh`.

## What you need

* Any x86-64 host (Linux, macOS, Windows/WSL) with
  `qemu-system-x86_64` **8.0 or newer** installed (developed and tested
  against QEMU 11.0.2, TCG emulation — no KVM required).
  * Debian/Ubuntu: `sudo apt install qemu-system-x86`
  * Fedora: `sudo dnf install qemu-system-x86-core`
  * Arch: `sudo pacman -S qemu-full`
  * macOS: `brew install qemu`
* The release artifacts. Normally they are attached to the GitHub
  release itself:

```sh
gh release download v0.6.0 --repo zeromike12/ArenaOS
```

  The build environment cannot reach GitHub's asset-upload endpoint
  (`uploads.github.com`), so releases additionally ship the **identical
  bundle through the repository** under `releases/<tag>/` — download the
  tarball, check its sha256, extract, and you have the same six files:

```sh
curl -LO https://github.com/zeromike12/ArenaOS/raw/refs/heads/arena/01a0d6fd-arenaos/releases/v0.6.0/arenaos-v0.6.0-qemu-x86_64.tar.gz
sha256sum -c arenaos-v0.6.0-qemu-x86_64.tar.gz.sha256
tar xzf arenaos-v0.6.0-qemu-x86_64.tar.gz
```

  (Each release's notes link its own bundle; after a branch merge the
  same path works under `raw/refs/heads/main/...`.)

| Asset | What it is |
|---|---|
| `arena-esp.img` | The bootable disk: an EFI System Partition holding the ArenaOS boot stage + kernel (a UEFI application, ADR-0003) |
| `scratch-template.img` | A **formatted AFS1 data disk** template (8 MiB, ADR-0023) — copy it to `scratch.img`; everything you `write` in the VM lives there and survives reboots |
| `edk2-x86_64-code.fd` | EDK2/OVMF firmware **code** flash (read-only), the exact build the release was tested with |
| `ovmf-vars-template.img` | Blank firmware **NVRAM** template (writable copy required per boot) |
| `RUNNING.md` | This file |
| `sha256sums.txt` | Checksums for all of the above |

> EDK2 firmware is redistributed under its BSD-2-Clause-Patent license;
> QEMU itself is **not** redistributed — install it from your platform's
> package manager.

## Boot it

The vars flash is written by the firmware, so always boot from a **fresh
copy** of the template. The data disk is different: copy
`scratch-template.img` **once** — then keep reusing your `scratch.img`
across boots, because that is where your files live (copy the template
over it again whenever you want a factory-fresh volume):

```sh
cp ovmf-vars-template.img ovmf-vars.img
cp scratch-template.img scratch.img    # only for the FIRST boot (or a reset)

qemu-system-x86_64 \
    -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
    -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
    -drive if=pflash,format=raw,file=ovmf-vars.img \
    -drive format=raw,file=arena-esp.img \
    -drive file=scratch.img,format=raw,if=none,id=scr0 \
    -device virtio-blk-pci,drive=scr0 \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -device virtio-rng-pci \
    -device virtio-keyboard-pci \
    -chardev socket,id=vcon0,path=/tmp/arena-console.sock,server=on,wait=off \
    -device virtio-serial-pci,max_ports=1 \
    -device virtconsole,chardev=vcon0 \
    -display none -serial mon:stdio -no-reboot
```

**A second console on a socket (new in v0.9.0).** The last three lines
attach a virtio-console port whose host end is a unix socket. While the
VM runs, open another terminal and:

```sh
nc -U /tmp/arena-console.sock
```

You are now on the machine's console — the same one: what ArenaOS
prints appears in both places, and what you type in either drives the
same `arena>` prompt. That is `consoled` (M6.4, ADR-0027): the kernel
keeps one console and this driver attaches a second channel to it in
both directions. Leave the lines out and nothing changes; the boot
suite's console test reports an honest SKIP if nobody connects.

**Want to actually type on a keyboard?** Drop `-display none` and use
`-display gtk` (or `sdl`/`cocoa`, whatever your QEMU has) with
`-device virtio-keyboard-pci` attached: a QEMU window opens, and since
v0.8.0 (M6.3) your keystrokes in that window drive the `arena>` prompt
directly through the `inputd` driver. The serial console keeps working
at the same time — both feed the same line discipline, so you can type
in either and read the log in your terminal.

The scratch disk is **required** and must be **AFS1-formatted**: since
M5.3 the filesystem service (`fsd`, ring 3) mounts it during the boot
suite and again for the shell, and a zero-filled or foreign image is
refused by design (`fsd: sector 0 is not an AFS1 superblock`) — the
machine halts after the failed suite. Building from source? Format one
with the layout's source of truth: `python3 -c 'import sys;
sys.path.insert(0, "tools"); import afs1; afs1.mkfs("scratch.img",
16384)'`.

The two `-netdev`/`-device virtio-net-pci` lines (new in v0.6.0), the
`-device virtio-rng-pci` line (new in v0.7.0), and the
`-device virtio-keyboard-pci` line (new in v0.8.0), and the
virtio-console trio (new in v0.9.0) are all **optional**: they attach QEMU's built-in user-mode network, which the
`netd` driver (M6.1) proves with a real ARP round trip every boot;
QEMU's built-in entropy source, which the `rngd` driver (M6.2) proves
by drawing randomness straight into a client's pages; a virtual
keyboard, which the `inputd` driver (M6.3) turns into live typing at
the shell prompt; and a console port, which the `consoled` driver
(M6.4) turns into a second console you can reach with `nc -U`. Boot without any of them (e.g. an older saved
command) and the machine stays green — the corresponding test reports
an honest `SKIP`, the kernel logs the service as offline, and the
serial console remains a complete way to use the machine. No host setup or privileges are needed either way:
`virtio-rng-pci` with no backend uses QEMU's own `rng-builtin`
(the platform CSPRNG), which works on Linux, macOS, and Windows hosts
alike. On a museum-piece QEMU that predates that default (pre-4.1),
add `-object rng-random,filename=/dev/urandom,id=rng0` and write
`-device virtio-rng-pci,rng=rng0`.

Serial is *a* console — in **both directions**. Everything ArenaOS logs
goes there, and since Milestone 4.6 (ADR-0020) your keystrokes come
back in through the same port: after the boot-time test suites pass (a
few seconds), the kernel spawns the storage service, the filesystem
service, the network service (when the NIC is attached), the entropy
service, the keyboard service (when a keyboard is attached), and the
**shell**, and the machine waits for you at the `arena> ` prompt.
Type `help`. The VM stops only when you type `shutdown` (or kill QEMU
with `Ctrl-A X`) — a boot that ends by itself would mean the shell
never came up. Since v0.8.0 the prompt answers to a real keyboard as
well: `inputd` decodes the keycodes and feeds them into the SAME line
discipline, so echo, backspace, and the blocking read behave
identically whichever way you type.

### Using your distro's OVMF instead

The bundled firmware is optional — any recent OVMF/EDK2 x86-64 split
(code + vars) works. Point the two `pflash` drives at your distro's
files instead, e.g. on Debian/Ubuntu:

```sh
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE.fd \
    -drive if=pflash,format=raw,file=/usr/share/OVMF/OVMF_VARS.fd \
```

(copy `OVMF_VARS.fd` to a writable location first, as above).

## The shell

The initial service (a real userspace image, spawned through the M4.5
protocol with kernel-granted capabilities). What it understands:

| Command | What happens |
|---|---|
| `help` | lists the builtins |
| `ps` | live processes as `(pid, threads)` pairs — you will see the shell itself plus the resident `storaged` and `fsd` services (and `netd` when the NIC is attached) |
| `echo TEXT` | prints TEXT (the kernel line discipline echoes as you type; backspace works) |
| `ls` | lists the files on the AFS1 volume with their committed sizes |
| `cat NAME` | streams a file back through `fsd` + `storaged` (the device DMAs straight into the shell's own frame — zero-copy, ADR-0023) |
| `write NAME TXT` | creates a new file with TXT as its contents and **commits it to the disk** — it is still there next boot. v1 refuses to overwrite an existing file (no truncate yet): an honest refusal, never a silent clobber |
| `rm NAME` | deletes a file transactionally (its sectors return to the allocator two commits later — no valid old commit ever sees them reused) |
| `spawn` | `SYS_SPAWN`s registry image 0 — the untouched M4.3 test payload — as a child process: its pinned message lands mid-session, then its exit badge comes back through the shell's notification |
| `shutdown` | the Power-gated halt: the kernel logs the requesting pid and hands the machine to firmware's `ResetSystem` |

Anything else answers `unknown command: '…' — try 'help'`.

Try this — it is the whole storage stack, end to end, in ring 3:

```
arena> write note.txt hello from my own OS
  wrote 26 bytes to 'note.txt'
arena> shutdown
```

…then boot the same `scratch.img` again and `cat note.txt`.

## What a healthy boot looks like (current: Phase 6 in progress — v0.6.0)

The serial output is a boot stage log followed by kernel log lines. The
machine-checkable landmarks, in order:

1. Boot stage banner, memory-map capture, page-table install (ADR-0008)
2. `m1: RESULT PASS (8/8)` — Milestone 1 self-tests re-run every boot
3. `ebs_exited` — ExitBootServices survived (ADR-0011)
4. `timer chain reclaimed: ... ioapic pin 2 ...` — the kernel took the
   PIT→IOAPIC→LAPIC chain back from firmware
5. 21 `m2:test:<name>: PASS` lines, ending with `m2: RESULT PASS (21/21)`
6. 13 `m3:test:<name>: PASS` lines — kernel threads, preemption, ring 3
   + syscalls, processes as address spaces, capability spaces
   (ADR-0012…0015) — ending with `m3: RESULT PASS (13/13)`
7. 9 `m4:test:<name>: PASS` lines — the ELF validator and its rejection
   corpus, the image loader (ADR-0016), the syscall ABI v1 proven from
   ring 3 (ADR-0017), the first user process, the IPC v1 echo-server
   demo (ADR-0018), the spawn protocol's supervisor restart demo
   (ADR-0019), and the console input service (ADR-0020), ending with
   `m4: RESULT PASS (9/9)`
8. `console input armed: com1 rx -> ioapic pin 4 -> vector 33` — the
   input half of the console (right after the timer-chain line, step 4)
9. 6 `m5:test:<name>: PASS` lines — the PCI scan, the untyped-memory
   grants, the ring-3 MMIO window, the IRQ relay (ADR-0021), the
   resident block service driving the real virtio-blk device from
   ring 3 (ADR-0022), and `fs_service`: the AFS1 filesystem boundary —
   on a first-boot volume fstest creates, writes, and verifies a file
   (`PASS (fresh)`); on a volume that survived a reboot it re-finds and
   verifies the committed file with zero writes (`PASS (persisted)`) —
   ending with `m5: RESULT PASS (6/6)`
10. `m6:test:net_service: PASS` — the virtio-net link proof
    (ADR-0024): `netd` (the ring-3 NIC driver) plus `nettest`, which
    hand-builds a 42-byte ARP request for QEMU's built-in gateway
    10.0.2.2, sends it zero-copy through the transmit queue, and
    verifies the reply that arrives — by interrupt — on the receive
    queue, field by field at exact wire offsets — ending with
    `m6:test:rng_service: PASS` right after it
11. `m6:test:rng_service: PASS` — the entropy proof (ADR-0025):
    `rngd` (the ring-3 virtio-rng driver, built on the shared virtio
    core) plus `rngtest`, which draws two 4 KiB frames of randomness —
    the device DMAs them straight into the client's own pages — and
    checks that both are full-length, not all-zero, not one repeated
    byte, and different from each other, with the kernel counting
    exactly one interrupt per draw. The line
    `rngtest: draw A … vs draw B …` shows fresh bytes every boot.
    `rngtest: draw A … vs draw B …` shows fresh bytes every boot
12. `m6:test:input_service` — the input proof (ADR-0026): `inputd`
    (the ring-3 virtio-input keyboard driver) plus `inputtest`, which
    reads DECODED key bytes back through the service boundary. A
    keyboard is the one fixture that produces nothing unless someone
    uses it, so this test needs a typist: in an automated run the
    harness types `arena` on the virtual keyboard over QMP and the
    test PASSes. In YOUR run nobody is typing during the boot suite,
    so after about a second the suite calls the wait off and reports
    an honest **SKIP** — the machine carries on to the shell exactly
    as normal. (Want to see it pass? Type `arena` in the QEMU window
    while the suite is running.) Together these end with
    `m6: RESULT PASS (4/4)` — or a `RESULT SKIP` line naming whatever
    was missing, which is equally green: booting without the NIC,
    rng, keyboard, or console port changes nothing else
13. `m6:test:console_service` — the console-channel proof (ADR-0027):
    `consoled` plus `contest`, which sends a line out the port and
    reads the host's answer back in. Like the keyboard, this fixture
    needs somebody at the other end: connect `nc -U` to the socket
    before the suite runs and it PASSes; otherwise it SKIPs honestly
    after about a second and the boot carries on exactly as normal
14. `storaged spawned: pid …`, `fsd spawned: pid …`, then
    `fsd: mounted AFS1 — commit seq …`, and (with the fixtures)
    `netd spawned: pid …` + `netd: virtio-net ready — DRIVER_OK,
    mac …`, `rngd spawned: pid …` + `rngd: virtio-rng ready —
    DRIVER_OK …`, `inputd spawned: pid …` + `inputd: console mode
    — keystrokes feed the shell's line discipline`, and
    `consoled spawned: pid …` + `consoled: console mode — the port is
    a second console` — the production services come up on the same
    devices the suite just proved
    (ADR-0022/0023/0024/0025/0026/0027)
15. `milestones 5–6.4 complete … spawning the shell`, then
    `shell spawned: pid …` — the hand-off
16. `ArenaOS shell v0.9 …` and the `arena> ` prompt — the machine is
    now an interactive system with a real filesystem; type into it
    (see "The shell" above)
17. After `shutdown`: `shutdown requested by pid … through its Power
    cap` and `halting via UEFI ResetSystem(shutdown)` — the clean-halt
    declaration (the automated harnesses type `shutdown` for you,
    marker-paced)
18. QEMU exits on its own with status 0

If you see `PANIC`, a `FAIL` marker, or QEMU hangs instead, please open
an issue with the full serial output attached — the log is designed to
be a diagnostic artifact, not decoration.

## Useful variations

```sh
# Log serial to a file INSTEAD of the terminal — output only: the shell
# gets no input this way and the machine stays up until you kill QEMU
# (the automated suites feed it from a pipe; see tools/mtest.py):
    -serial file:serial.log

# Log to a file AND keep typing (the monitor multiplex also lands in
# the log — fine for humans, the harnesses use a clean chardev instead):
    -serial mon:stdio | tee serial.log

# Interrupt/CPU-reset trace for debugging (QEMU-side):
    -d int,cpu_reset -D qemu-int.log

# Faster on Linux hosts (untested configuration — the project's
# reference environment is TCG):
    -enable-kvm

# Boot WITHOUT the network (pre-v0.6.0 style — stays green: the m6
# suite reports an honest SKIP and netd is not spawned): drop the
# -netdev and -device virtio-net-pci lines
```

## Building from source instead

Releases are convenience artifacts; the repo builds everything itself:

```sh
./tools/dev-env/bootstrap.sh   # one-time: Rust, QEMU, EDK2 (see docs/DEV-ENV.md)
./tools/run_tests.sh           # build + ALL milestone harnesses (M1…M5,
                               # incl. two-boot persistence and the
                               # crash-consistency gate)
./tools/run.sh                 # interactive boot, serial on stdio
./tools/stability_loop.sh 100  # 100 clean boots (ADR-0011 gate)
```
