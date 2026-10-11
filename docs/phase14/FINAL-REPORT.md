# ArenaOS Phase 14 final report

## Release identity

- Branch: `arena/phase14-native-pie-aslr`
- Starting Phase-13 release tip: `74ace4f9e9898fa8f567d2d666c2a84c6b333bc9`
- Qualified implementation source commit: `e24f3fb2c83613bd4d60c08e8f7d5555c6b43bef`
- Phase-14 ADR: [ADR-0110](../adr/0110-static-native-pie-and-kernel-placement.md), accepted and qualified
- Full historical plus Phase-14 runner: 119/119 groups passed
- Production EFI SHA-256: `418c63acbf9eee0eadb6dd8de21e860bddd5e868d612028d7e93cb45e8d1ff2f`
- ESP SHA-256: `22fd51a49d4235cc20acd90eff92e75ad6e3f85fb2d0a14b51c37de27dfc4ce9`
- Fresh exact-artifact stability: 100/100 boots, zero failures, 946 seconds
- Stability receipt: `build/stability-receipt.txt`
- phase14-complete archive SHA-256: `b15c6c7cc09ab80d6d54f901802e1ce5807024752182c36aa95e36cf2b861f29`

The new branch was created directly from the exact Phase-13 release tip. The
Phase-13 branch and release tip remain unchanged. The final 119-group run
records a clean source tree and the same source commit at its beginning and
end. The final desktop-profile EFI was rebuilt from that source after the
suite, matched the already qualified T2/T3 EFI byte-for-byte, and passed a
fresh 100-boot run with the exact EFI hash above.

## Accepted native executable profile

The production ELF validator retains the existing fixed-address static
`ET_EXEC` contract and adds one bounded ELF64 `ET_DYN` profile:

- ELF64 little-endian x86-64, System V ABI version zero, current ELF version,
  zero reserved identification bytes and flags, and the standard 64-byte ELF
  and 56-byte program headers.
- At most 8 program headers and 8 `PT_LOAD` segments. The existing 256-KiB
  Image limit applies; the image span and mapped load pages are each at most
  128 4-KiB pages.
- `PT_LOAD`, one bounded `PT_DYNAMIC`, and optionally one empty non-executable
  `PT_GNU_STACK` only. No interpreter, TLS, PHDR, RELRO, dependencies, PLT,
  text relocations, GNU hash, unknown tags, or other dynamic-linker features.
- Load segments are page-aligned and non-overlapping at byte and page
  granularity. Only R, RW, or RX segment layouts are accepted. The entry must
  be inside executable load memory. W+X is refused.
- Dynamic metadata and relocation metadata are bounded to 16 KiB total;
  RELA data is at most 12 KiB, each record is 24 bytes, and at most 512 records
  are accepted. The symbol, string, and SysV hash tables have only the
  null-symbol shape required by the recorded rust-lld profile.
- The sole relocation is x86-64 `R_X86_64_RELATIVE` (type 8), symbol index
  zero. Targets are unique, 8-byte aligned, fully inside writable non-X load
  memory; nonnegative addends resolve inside the image. The kernel applies
  `*(B + r_offset) = B + r_addend` with checked arithmetic.

This is not general third-party ELF compatibility. No dynamic linker, symbol
resolution, shared libraries, Linux ABI, glibc, musl, `dlopen`, TLS, or PLT
binding was added.

## Fixture, package, and launch proof

The reproducible Rust 1.97.0 `rust-lld` fixture is a genuine `ET_DYN` binary,
not a relabeled `ET_EXEC`. It has three load segments (RX, R, RW), executable
code, read-only data, initialized writable data, BSS, one bounded dynamic
table, and 30 symbol-zero `R_X86_64_RELATIVE` records. Its ELF SHA-256 is
`d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`.
The signed APB1 is 24,040 bytes and has SHA-256
`04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`.
Headers and records are in [PIE-FIXTURE.md](PIE-FIXTURE.md).

The positive guest used the unchanged verified path: APB1 signature and
filesd verification, packaged policy, protected AFS2 installed record, fresh
Image capability, kernel spawn, and native Startup ABI v2. Desktop discovered
the installed application in All Applications and launched the same immutable
Image twice. Both runs validated the actual Startup ABI base and entry,
relocated function pointer and data, read-only constant, writable data, BSS,
and successful exit status 42. The kernel logs verified RX/R/RW final PTEs,
W^X, and absent image and stack guard mappings.

Observed bases from the canonical full-suite guest were
`0x5cea57600000` and `0x48d0b8a00000`. Both are distinct, 2-MiB aligned,
within the 64–96-TiB arena, and selected from 16,777,215 valid candidate
slots for this image. Startup ABI values matched the kernel placement logs.
Each launch returned process, Image, capability, region, page, and map counts
to the measured steady state. The first launch may warm three empty parent
page-table frames under the pre-existing SharedRegion unmap reuse contract;
they are not child-owned mappings or leaked Image/process resources. The
second launch returned to that same steady state.

## Entropy, placement, and security limits

After virtio RNG reaches `DRIVER_OK`, trusted `rngd` obtains 32 device bytes
and seeds the kernel ChaCha20 CSPRNG through a narrow write-only,
non-copyable `KernelEntropySeed` capability. A boot known-answer test checks
the generator. Kernel placement uses rejection-sampled 2-MiB slots; an
application, filename, Image metadata, or ELF header cannot choose the load
bias. PIE checks and starts fail with typed `STATUS_NO_ENTROPY` if seeding is
unavailable. Fixed-address `ET_EXEC` boot services do not wait for rngd. No
counter, fixed-seed, wall-clock, or other fallback claims ASLR.

The placement interval is `[0x400000000000, 0x600000000000)`, giving at most
24 bits of slot choice before image-boundary and collision exclusions. The
complete image envelope and unmapped lower/upper guards must fit; the stack
uses its established guard. Process-owned mappings and page tables are
preflighted before population. Private pages are relocated before final
protections and publication. The entropy guarantee depends on the virtio RNG
device, QEMU/backend and trusted rngd/kernel boundary; this is placement
randomization, not a claim against a compromised entropy source or memory
disclosure.

## Negative and preservation evidence

Real signed APB1 guest coverage refused malformed ELF magic, unsupported
relocation kinds, relocation targets outside the image, relocation-span
arithmetic overflow, invalid segment alignment, and forbidden W+X. A signed
otherwise valid ELF outside the placement arena was refused with
`STATUS_NO_SPACE` before placement. Separate guests withheld entropy and
revoked the Image authority; the kernel returned `STATUS_NO_ENTROPY` and
`STATUS_BAD_ARG` respectively. Production preflight rejected collisions with
process-owned ranges and mapped guard pages. These failures did not execute
the image and restored process/Image/capability/mapping ownership to the
expected baseline. The M8.5 Image-cap limits and exhaustion cases also passed.

The complete qualification runner passed 119 groups: all 115 Phase-13
historical groups plus four Phase-14 groups. This includes the affected M4
ELF parser/loading/spawn corpus; M8.5 Image lifecycle; M9 SharedRegion and
resource accounting; M10/M11 Desktop lifecycle; M12 installed package and
Startup ABI authority; and M13 native VM, heap, thread, synchronization, and
mixed workload regressions. The historical fixed-address `ET_EXEC` path
passed unchanged. T2 clean boots passed 10/10 in 98 seconds and T3 passed
25/25 in 249 seconds. T4 passed 100/100 fresh boots in 946 seconds. All three
receipts qualify the same EFI SHA listed above. The full-suite log is
`build/qualification-phase14-full-suite.log`.

## Toolchain and release reproduction

Qualification ran on Debian GNU/Linux 13.6 (Trixie), x86-64, without root or
sudo. Rust and guest targets came from official rustup; QEMU and OVMF packages
were authenticated against the official Debian Trixie archive and extracted
unprivileged under `/tmp`.

- rustc 1.97.0 (`2d8144b78`, 2026-07-07); Cargo 1.97.0; rustfmt 1.9.0-stable
- LLVM 22.1.6; targets `x86_64-unknown-none` and `x86_64-unknown-uefi`
- QEMU 10.0.13 (Debian `1:10.0.13+ds-0+deb13u1`)
- OVMF `edk2-ovmf 2025.02-8+deb13u1`, 4-MiB CODE/VARS

The tested QEMU profile uses UEFI/OVMF, virtio block, virtio networking with
QEMU user networking, virtio RNG, virtio keyboard, virtio tablet, and
virtio-console where the test needs it. The exact EFI, ESP, disk template,
OVMF files, signed fixtures, test scripts, and evidence are in the
`phase14-complete` release archive. The independent extracted witness is
`python3 phase14_archive_boot.py`; it validates archive checksums and boots
only the extracted files, first checking Phase-13 `ET_EXEC` behavior and then
installing and running the signed PIE APB1 twice. It does not read the source
checkout, build tree, hidden fixtures, or a private signing key.

The final independently extracted run returned exit status zero. It verified
the archive manifest and the exact 100/100 EFI receipt, passed the Phase-13
fixed-address application/runtime and boot-profile gates, and completed the
signed Phase-14 Desktop/filesd/packaged installation. The kernel selected
bases `0x51a925800000` and `0x46c558600000`; both PIE launches checked Startup
ABI v2, relocation state, exit 42, and exact teardown. The guest passed its
network and graphics checks and shut down through UEFI ResetSystem.

For graphical Windows QEMU use, follow [MANUAL-WINDOWS.md](MANUAL-WINDOWS.md).
The command uses UEFI/OVMF, virtio block/network/RNG/keyboard/tablet/console,
and a distinct persistent data disk at `D:\ArenaOS\phase14-data.img`.

## Phase-15 recommendation

Keep Phase 14's static native profile stable. Any dynamic linking or broader
ELF compatibility needs a new bounded ADR and independently justified loader
and authority contracts. Useful follow-up work is additional boundary
mutation coverage, and entropy reseeding/health evidence if a narrowly scoped
trusted source is introduced. Do not expose raw entropy to applications or
make fixed `ET_EXEC` startup depend on rngd.
