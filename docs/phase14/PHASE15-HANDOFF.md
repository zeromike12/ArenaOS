# Phase-15 handoff

Phase 14 adds one qualified static x86-64 PIE profile to the existing native
Image capability and spawn path. The production loader accepts the checked-in
Rust `rust-lld` fixture profile and `R_X86_64_RELATIVE` only. It does not
provide symbol lookup, a dynamic linker, shared libraries, TLS, PLT binding,
Linux syscalls, or a POSIX process ABI. Existing fixed-address `ET_EXEC`
applications continue through their prior loader path.

The kernel selects each PIE bias from a 64–96 TiB, 2-MiB aligned arena using
the ChaCha20 CSPRNG seeded through the exact non-copyable rngd seed capability
with virtio-rng device bytes. The arena exposes at most 24 bits of placement
choice; image size, exclusions, collisions, and backend/device trust reduce
the effective guarantee. Launch fails when seed state is unavailable or no
valid placement remains. No fixed or time-derived fallback is present.

Recommended Phase-15 planning topics:

- Decide whether the next native executable milestone should retain a
  deliberately static profile or add a separately bounded dynamic-linking
  contract. Any loader work requires a new ADR before implementation.
- Improve entropy reseeding and health evidence only if the architecture gains
  a narrowly scoped trusted source; do not expose raw entropy to applications.
- Extend the PIE fixture/mutation matrix around page-boundary splits,
  page-rounded segment conflicts, dynamic-tag duplication, and relocation
  count/metadata limits without relaxing the Phase-14 profile.
- Keep per-launch placement separate from immutable Image metadata and retain
  the existing signed APB1, fresh Image-capability, process ownership, and
  Startup ABI v2 contracts.

Qualification evidence, artifact hashes, supported metadata, and current
limitations are in `FINAL-REPORT.md` and ADR-0110. Phase-14's exact extracted
archive boot is the release reproduction baseline.
