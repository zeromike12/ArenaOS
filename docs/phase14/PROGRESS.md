# ArenaOS Phase 14 progress

## Checkpoint

- Branch: `arena/phase14-native-pie-aslr`
- Parent: `74ace4f9e9898fa8f567d2d666c2a84c6b333bc9` (Phase-13 release tip)
- Phase-13 implementation checkpoint: `c4ad723a3e66b8b72a020589cd9ab6eab73de34d`
- Starting tree: clean at the exact release tip before Phase-14 documentation.
- Milestone 14.0: audit complete; bounded implementation contract recorded in ADR-0110.

## Audit findings

- `kernel/kernel/src/elf.rs` is the shared production validator/loader. It
  currently accepts the established `ET_EXEC` subset and rejects `ET_DYN`.
- `image_registry.rs` copies immutable verified Image bytes and accounts for
  pins/references. Randomized placement must remain per-spawn state.
- `spawn.rs` owns preparation and rollback. It pins the Image before loading,
  creates the process and stack, and reclaims prepared state on refusal.
- Process-owned mappings and VM use the lower user half. The VM arena begins at
  32 TiB; ordinary mmap begins at 1 GiB and is bounded below 1 TiB. The planned
  PIE arena is disjoint from those established ranges.
- Startup ABI v2 already carries entry and image base at byte offsets 104 and
  112. The existing startup cap-descriptor checks and transport remain intact.
- `rngd` is started before the service manager and signals readiness only after
  the virtio RNG is usable. PIE launch will require readiness; fixed `ET_EXEC`
  startup remains independent of it.
- The APB1 → filesd verification → packaged policy → fresh Image capability →
  kernel spawn route is already present and is the required Phase-14 proof path.
- Host setup is verified without root: Rust 1.97.0 / rustfmt /
  `x86_64-unknown-none` from official rustup; QEMU 10.0.13 and OVMF 2025.02
  extracted from packages authenticated by the official Debian Trixie archive.
- Milestone 14.1 has a real `rust-lld` PIE build. It is 23,256 bytes, has five
  program headers, three non-overlapping RX/R/RW load segments, no interpreter,
  one bounded `PT_DYNAMIC`, and 30 symbol-zero `R_X86_64_RELATIVE` records.
  `docs/phase14/PIE-FIXTURE.md` records the exact ELF and relocation metadata.

## Current state

The production validator now accepts only the ADR-0110 static PIE profile and
retains the prior fixed-address `ET_EXEC` path. The real fixture passes the same
validator used by Image registration. PIE loading builds private RW/NX pages,
applies checked `R_X86_64_RELATIVE` relocations, enforces and verifies final
RX/R/RW page flags, and maps no image/stack guards. M4 guest coverage verifies
all 30 relocation results, the zero-filled writable tail, exact PTE flags,
unmapped guard pages, frame-exact teardown, and refusal of a pre-existing
process-region or mapped guard-page collision by the production placement
preflight.

The kernel CSPRNG is a ChaCha20 generator with a boot known-answer check and
unbiased bounded slot selection. Production `rngd` obtains 32 bytes from the
virtio RNG after `DRIVER_OK` and submits them through the exact write-only,
non-copyable `KernelEntropySeed` capability. The service-manager readiness
signal follows that seed operation. PIE spawn checks and launches return a
typed refusal when the CSPRNG is not ready; fixed `ET_EXEC` starts do not wait
on it.

T0 so far: full `tools/build.sh` completed, and the M4 QEMU guest passed 9/9,
including the Phase-14 parser mutation corpus and positive relocation/mapping
checks. The same guest also passed M1, M2, M3, M5, M6, M7, M11, and M12. The
first M6 attempt found that the new `rngd` seed capability probes used syscall
wrappers with unspecified unused registers; those probes now use the explicit
six-argument ABI wrapper, and the rerun passed M6 6/6. The host needed
`pyfatfs` plus setuptools 80.10.2 in `/tmp` for ESP image generation, and its
QEMU package ROM search path required an explicit `-L` directory assembled
from the official Debian `share/qemu` and `share/seabios` package contents.

T1 positive guest proof passed against real QEMU/OVMF with virtio block, user
networking, RNG, keyboard, tablet, and console devices. Desktop installed the
validly signed APB1 through filesd and packaged, the AFS2 record and exact ELF
bytes were checked, and All Applications launched the same immutable Image
twice. The fixture verified Startup ABI v2's actual base and entry, relocated
data and function pointer, read-only constant, BSS, and exit 42. Kernel-selected
bases were `0x526a33200000` and `0x552f9b800000`; both were 2-MiB aligned in the
64–96 TiB arena, with 16,777,215 valid placement candidates. Both launches
restored the same steady process/Image/cap/map/resource receipt. The first
launch warmed three empty parent page-table frames retained by the existing
SharedRegion unmap contract; the second launch returned exactly to that steady
state.

T1 signed APB1 negative guest coverage passed for invalid ELF magic,
unsupported relocation, relocation destination outside the image, overflowing
relocation span, invalid segment alignment, and W+X layout. Each package was
installed and retained its exact signed record and malformed executable bytes;
the production Image validator refused it at installed-image resolution. A
separately signed and otherwise valid image with a link-time layout outside the
placement arena passed Image validation but was refused by kernel spawn with
`STATUS_NO_SPACE` before any placement was logged. A temporary red mutation
withheld rngd's device bytes from the kernel while preserving the manager-ready
path; signed installation still completed and PIE spawn preflight returned
`STATUS_NO_ENTROPY`. That script restored the exact rngd source and production
EFI/ESP bytes. The checked-in signed APB1 fixture is 24,040 bytes with
SHA-256 `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`.
The no-entropy mutant is test-only and is not present in the release image.

A separate signed APB1 stale-authority guest revokes the freshly registered
Image before IPC transfer. The test-only Desktop client mutation uses the
known link-time entry/base solely to reach production `SYS_SPAWN_CHECK`; the
kernel refuses the stale Image with `STATUS_BAD_ARG`, the PIE never runs, and
ownership/resource receipts return to baseline. The first version of this
mutation was rejected one stage earlier by Desktop's live Image-info check.
That was correct behavior; the adjusted test bypasses only this descriptive
check so it can directly prove the kernel authority gate. Packaged/Desktop
sources and the exact production EFI/ESP are restored byte-for-byte by the
test.

T0/T1 results: `tools/build.sh --image` and M4's 9/9 guest passed after the
collision-preflight addition; M1, M2, M3, M5, M6, M7, M11, and M12 also passed
in that same boot. T2 and T3 preservation checkpoints are now complete. The
final complete historical suite, fresh 100/100 exact-artifact stability, and
independent release-archive boot remain pending.

## Preservation follow-up

The first 119-group Phase-14 full-suite run on source checkpoint
`f6443caaf5ff2f7ac32378b8bdac9892c9559386` completed 115 groups and exposed
four failures. The negative guest missed one All Applications row after using
its full title as a QMP search string; it now uses short unique filters, and
all seven signed negative PIE package cases pass in
`build/phase14-negative-afterfix.log`.

The M11 Desktop boot failure was a measured TSC calibration disagreement:
20-ms PIT windows gave a 2.74-GHz estimate while independent windows measured
about 2.596 GHz after TCG host descheduling. The kernel boot calibration and
M2 remeasurement now use 100-ms windows; the existing 5% agreement limit is
unchanged. `tools/test_m11_desk.py` passes after the change. The M11 window
resize test exposed a separate IPC client bug: a nonzero server status was
collapsed to `-2`, hiding `STATUS_RESIZE_SUPERSEDED`. The client now preserves
the status, and `tools/test_m11_wm.py` passes its resize race, 12-session
workload, and teardown checks.

The M12 scale run in the first full suite stopped during its M5 filesystem
setup after 14 virtio block completions, with all three service threads still
live and no later completion. The observed evidence does not identify the
cause, and none of the M12 filesystem or storage implementation was changed.
The full targeted M12 guest was rerun and passed: its filesystem setup
completed, all 32 Desktop sessions started, session 33 was refused without
resource mutation, 16 sessions were closed and reused, and final counts
returned exactly to baseline. It took 259.1 seconds and is recorded in
`build/m12-scale-afterfix.log` and `build/serial-m12-scale.log`. The full suite
must confirm this case again before release qualification. T2 then passed
10/10 clean boots in 98 seconds and T3 passed 25/25 in 249 seconds on EFI
SHA-256 `418c63acbf9eee0eadb6dd8de21e860bddd5e868d612028d7e93cb45e8d1ff2f`.
Each boot had the same complete nine-suite PASS set, orderly UEFI shutdown,
and the runner's Desktop pixel/input, network, virtio-console, and native
application lifecycle checks. The M5 filesystem self-test also completed on
each boot. Receipts and logs are `build/phase14-stability-t2-receipt.txt`,
`build/phase14-stability-t3-receipt.txt`, and their matching `*-t2.log` and
`*-t3.log` files.

After those changes, `tools/test_phase14_pie_guest.py` again installed and ran
the signed application twice and returned exit 42 with exact process/Image
teardown. Its observed bases were `0x53674ce00000` and `0x5d036e600000`; both
were 2-MiB aligned within the 64–96-TiB placement arena, out of 16,777,215
candidate slots. The relocated function pointer, initialized data, BSS,
Startup ABI v2 base/entry, final protections, and guard checks passed. The
receipt is `build/phase14-pie-afterfix.log` and the guest serial is
`build/serial-phase14-pie.log`.

## Evidence log

| Stage | Result |
| --- | --- |
| Source branch | Created from exact Phase-13 release tip; no Phase-13 files changed |
| Host | Debian GNU/Linux 13 (trixie), unprivileged user; `cargo`, `rustc`, `rustfmt`, QEMU, and OVMF were initially absent from PATH |
| Historical qualification | Phase-13 report records 115/115 suite groups and 100/100 clean boots; Phase-14 exact-artifact qualification remains pending |
| Native PIE | Rust 1.97.0/rust-lld fixture, genuine `ET_DYN`; SHA-256 `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e` |
| Positive APB1 guest | Two launches exited 42 at distinct bases; each verified relocated function/data, Startup ABI v2, RX/R/RW protections, guards, and reclamation |
| Signed negative APB1 guest | Six malformed/unsupported ELF classes refused at Image validation; valid out-of-arena image refused with `STATUS_NO_SPACE`; ownership counters returned to baseline |
| Missing entropy guest | Seed-withholding red mutation refused signed PIE with `STATUS_NO_ENTROPY`; original rngd source and EFI/ESP restored byte-for-byte |
| Stale Image authority guest | Revoked Image received through signed APB1 was refused by kernel `SYS_SPAWN_CHECK` with `STATUS_BAD_ARG`; process/Image ownership returned to baseline |
| Fresh PIE rerun | Bases `0x53674ce00000` and `0x5d036e600000`; both signed launches exited 42 with exact steady-state receipts |
| Historical follow-up | M11 Desktop and window-manager targeted reruns pass; M12 scale targeted rerun passes; T2 10/10 and T3 25/25 clean boot checkpoints pass; final full suite pending |
