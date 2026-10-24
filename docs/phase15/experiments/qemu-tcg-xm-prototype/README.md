# Phase 15 modified-QEMU TCG qualification evidence

**Current Phase 15 status: ACCEPTED by M and C for this exact locally modified
QEMU TCG backend.** This prototype record preserves the experiment and its
pre-acceptance evidence; it is not an upstream-QEMU or hardware-backed result.
M accepted ADR-0113 on 2026-10-24. See [`../../ACCEPTANCE.md`](../../ACCEPTANCE.md)
and [the accepted ADR](../../../adr/0113-native-fpu-simd-state.md).

## Current results

The unchanged strict ArenaOS image passed the six Phase 15 assertions on the
pinned patched QEMU 11.1.2 binary under both CPU profiles:

- `max,+nx,+smep,+smap`
- `qemu64,+nx,+smep,+smap`

Both QEMU process exits were 0 and both semantic guest verdicts were PASS 6/6.
Raw serial records show the CPL3 instruction generated vector `0x13`, fault
status `Some(113)` (hex formatting, therefore `0x113`), fault-thread
termination, same-process sibling and independent peer survival, scheduler-slot
reclamation, restored resource baseline, continued kernel operation, and
declared clean shutdown.

The strict EFI SHA-256 is
`bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e`. Its
exact paired recovery ESP SHA-256 is
`631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`.
The runner verifies that the ESP embeds this EFI byte for byte. Both trials
explicitly selected `-accel tcg`. The patched binary advertised TCG only,
KVM was disabled at build time, and `/dev/kvm` is absent.

The strict EFI/ESP were built from ArenaOS commit
`13986b3dcf007b1721d85ef3509945131f8c6584`. The last reviewed source commit
before this work was `df4ad9d4622e2e7e7d2b1ce03a57048305dd902a`; the intervening
commit contains documentation changes only, with kernel and strict guest
sources byte-identical. The final runner source revision is recorded in each
machine-readable run receipt.

A same-source, same-configuration unpatched QEMU 11.1.2 control booted the
same exact EFI/ESP pair under both profiles. QEMU exited 0, while the guest
semantic exit was 97 and the strict result was FAIL 5/6 because the exception
was not delivered. The raw paired control is retained beside the patched
receipts.

## Added independent exception controls

The independent UEFI control source now checks saved RIP before editing either
exception frame, all 128 bits of the XMM destination, a packed SQRTPS operation
with a negative lane after a valid lane, and sticky MXCSR flags. The controls
continue after a failed case so that the source-matched unpatched build records
every actual result.

| Check | Patched local-TCG | Same-source unpatched |
|---|---|---|
| Valid SQRTSS with invalid unmasked | PASS | PASS |
| Masked invalid SQRTSS sets IE without vector 19 | PASS | PASS |
| Set IE, unmask IE, then valid SQRTSS preserves IE without a fault | PASS | PASS |
| Newly raised IE faults even when IE was already sticky | PASS | FAIL; no vector 19 |
| Exact saved RIP for six faulting instructions, checked before frame edit | PASS | FAIL; exception delivery absent |
| Full 128-bit destination preserved on unmasked scalar SQRTSS and DIVSS | PASS | FAIL with missing exception delivery |
| CR4.OSXMMEXCPT clear selects vector 6 | PASS | FAIL |
| Existing IE plus new unmasked ZE produces vector 19 and MXCSR `0x1d05` | PASS | FAIL |
| Packed SQRTPS exceptional lane commits no partial 128-bit destination | PASS | FAIL with missing exception delivery |

The patched control process exit was **33**, the expected `isa-debug-exit`
success token. The unpatched control process exit was **35**, its expected
semantic-failure token. These are distinct from the guest's printed verdicts.
The raw serial, separate QEMU stderr, exit receipts, and machine-readable
comparison are in `logs/`; `tools/verify_phase15_xm_controls.py` checks their
identities and results.

## Runner and verifier negative controls

The committed receipts in `logs/negative-controls/` exercise failure handling
without weakening the acceptance checks:

| Negative case | Observed result |
|---|---|
| Missing machine-readable receipt | `verify` rejected the empty receipt directory with exit 1. |
| Wrong strict EFI, wrong ESP, or unpatched QEMU binary | The runner rejected each mismatched SHA-256 with exit 1 before starting a guest. |
| QEMU exit 0 with the unpatched guest's semantic FAIL 5/6 and exit 97 | The per-boot receipt retained `qemu_exit_zero=true`, set `qualification_pass=false`, and exited 1. |
| Accelerator initialization failure | A labeled test wrapper reported the real patched QEMU version and TCG-only list, then changed only the preflight QMP smoke request to disabled KVM. QEMU exited 1; the runner saved `accelerator_initialized=false` and aborted before guest execution. |

The accelerator wrapper is a test fixture, not an emulator backend. It never
starts ArenaOS and does not alter the pinned qualification binary. A separate
archive-extraction negative control also checks missing evidence and wrong
package identities after the final archive is assembled.

## Patch scope and limits

The patch is separate from ArenaOS. It changes only QEMU's x86 TCG code in
`target/i386/cpu.h`, `target/i386/ops_sse.h`, and
`target/i386/tcg/fpu_helper.c`. It adds vector 19 and wraps the existing
`SSE_HELPER_S` arithmetic instantiations for add, subtract, multiply, divide,
min, and max (packed/single/double variants) plus `sqrtps`, `sqrtpd`,
`sqrtss`, and `sqrtsd`. The helper snapshots SoftFloat flags and the whole
destination register, merges new status flags with existing sticky flags,
applies MXCSR masks and CR4.OSXMMEXCPT, and restores the full destination before
raising #XM or #UD.

The tested independent subset is legacy 128-bit SQRTSS, SQRTPS, and DIVSS,
including valid, masked, unmasked, sticky-flag, precise-RIP, and full-destination
cases. The ArenaOS test executes its existing legacy CPL3 SQRTSS probe. No
AVX/XSTATE behavior is claimed; no ArenaOS kernel, strict fixture, assertion,
or exception path was modified. The patch is not a complete x86 SIMD
floating-point exception implementation, and other helper families remain
outside its documented scope.

## Frozen backend identity

- Official QEMU tag: `v11.1.2`, tag commit
  `ceaea6cb3e2a7d966f9ca97c7ebb5b88cd764dd0`, peeled source commit
  `4fc49f46dc95d4a27de2509e7fceb2931e91faeb`.
- Official source archive SHA-256:
  `fc3d810fc6b27f60df77eaf6748004ca76de2edc2c4ff3a9f9a9ada33066c242`.
- Patch SHA-256:
  `c2909ea974f4ccaa9526bd5b70aef5599971bc567dd87d6ba8d9b06b327c8dd2`.
- Pinned patched QEMU 11.1.2 binary SHA-256:
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`.
- Same-build unpatched control binary SHA-256:
  `443c81fc9482493297ae7a3f37300bef3a82a2873e428f72884cf52d16e27284`.
- Configure target: `x86_64-softmmu`; TCG enabled, KVM disabled, default optional features disabled, Pixman/slirp
  enabled, FDT/docs disabled, Werror disabled.
- Measured build: Debian GCC 14.2.0-19, Meson 1.6.1, Ninja
  1.11.1.git.kitware.jobserver-1, GLib 2.84.4, Pixman 0.44.0, libslirp 4.8.0.
- OVMF Code SHA-256:
  `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94`.
- OVMF Vars template SHA-256:
  `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e`.

The full identity manifest is
[`patched-tcg-qualification.json`](../../patched-tcg-qualification.json).
The exact build command, dependencies, runtime libraries, QEMU smoke evidence,
and image identities are recorded in `logs/build-and-control-identities.txt`.

A clean build using the committed rebuild script completed with the same QEMU
release, patch, dependency versions, and TCG-only accelerator configuration.
Its binary hash was
`2ff8e8896de96cbba3fc368a39e45e4394ff174495d45cbae2452ca192941450`, distinct
from the selected qualification binary. This proves the rebuild procedure is
runnable; the different binary identity was not substituted into any strict
or stability trial. Qualification continues to use the immutable pinned
binary `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`.

## Rebuild and run

Use Debian 13 or compatible x86-64 Linux with glibc 2.38 or newer. Install the
official development tools/libraries (GCC, Meson, Ninja, GLib, Pixman, libslirp,
zlib, and PCRE2); the tested versions are listed in the identity manifest. The
rebuild script downloads the QEMU release archive from the official QEMU GitLab
project, checks its SHA-256, and applies the patch:

```sh
tools/build_phase15_patched_qemu.sh patched /path/to/fresh-qemu-output
```

Build a new control EFI/ESP from the committed assembly using the pinned Rust
1.97 UEFI target:

```sh
ARENA_RUST_BIN=/path/to/rust-1.97/bin tools/build_phase15_xm_controls.sh /path/to/controls
```

Install the runner's pinned Python dependencies from the repository's
requirements file; it pins `pyfatfs==1.1.0` and compatible transitive versions:

```sh
python3 -m venv .venv-phase15
. .venv-phase15/bin/activate
python -m pip install -r tools/phase15_external/requirements.txt
```

The runner verifies the Python dependency version, emulator and packaged
runtime-library hashes, image/firmware hashes, the embedded EFI, advertised
accelerator set, and an actual QMP initialization using explicit TCG. Run the
pinned backend preflight, strict two-profile test, and official strict
stability loop with `tools/phase15_patched_tcg_qualification.py`.
The runnable package and concise commands are documented in
[`PATCHED-TCG-QUALIFICATION.md`](../../PATCHED-TCG-QUALIFICATION.md). A rebuild
that produces a different QEMU binary hash is a different backend identity;
do not substitute it without new focused validation and qualification runs.

## M/C review and acceptance

The strict two-profile runs, 121-suite registered regression, final-source
checks, exception controls, and strict T2/T3/T4 gates are complete. C's
independent review verified the immutable package and raw evidence, replayed the
strict test under both CPU profiles, and reran the expanded controls. M and C
accepted Phase 15 on 2026-10-24 for the exact pinned locally modified TCG
backend; see [`../../ACCEPTANCE.md`](../../ACCEPTANCE.md). The historic
120/120 diagnostic aggregate remains separate and does not substitute for the
strict gates. M accepted ADR-0113. PR
[#4](https://github.com/zeromike12/ArenaOS/pull/4) is open against `main` and
has not been merged; this evidence record does not publish a release.
