# Phase-14 native static PIE fixture

This crate is a small native ArenaOS application linked as ELF64 `ET_DYN`.
It uses `arena-runtime` Startup ABI v2, checks the kernel-provided entry and
image base, reads initialized data, verifies a read-only constant and BSS, calls
a function through a stored function pointer, prints the observed addresses,
and exits with status 42.

Build it from the repository root with:

```sh
tools/build_phase14_pie.sh
```

The crate-local `.cargo/config.toml` selects `x86_64-unknown-none`, PIC code,
the checked-in `pie.ld`, and Rust 1.97.0's `rust-lld` PIE mode with no dynamic
linker or undefined symbols. `--hash-style=sysv` makes the only symbol metadata
the null-only dynamic symbol table and its one-bucket System V hash table.
No C runtime or loader is linked. The script copies the linker-produced binary
to `fixture.elf`; this checked-in ELF is also used by kernel validator tests.
Rebuilding with the recorded toolchain must reproduce the fixture hash in
`docs/phase14/PIE-FIXTURE.md`.

The fixture intentionally exercises relocations generated throughout the Rust
startup runtime. The kernel must apply every supported `R_X86_64_RELATIVE`
record, not just the one application-visible function pointer.
