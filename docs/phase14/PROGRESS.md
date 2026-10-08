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

## Current state

The exact branch parent is confirmed. Required Phase-13 handoff and scope ADR,
ELF contract, Image, process VM, and Startup ABI references have been read. No
production implementation has started. The next checkpoint is a real toolchain
and linker-produced PIE fixture, followed by T0 validator work.

## Evidence log

| Stage | Result |
| --- | --- |
| Source branch | Created from exact Phase-13 release tip; no Phase-13 files changed |
| Host | Debian GNU/Linux 13 (trixie), unprivileged user; `cargo`, `rustc`, `rustfmt`, QEMU, and OVMF were initially absent from PATH |
| Historical qualification | Phase-13 report records 115/115 suite groups and 100/100 clean boots; Phase-14 exact-artifact qualification remains pending |
