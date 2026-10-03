# Phase 8.5 APKG v1 ELF payload-fit evidence (2026-09-30)

**Finding: a purpose-built, useful ELF64 ET_EXEC fits the existing signed APKG v1
payload.** This is a host-side format/size/design gate, **not** execution via a
disk-backed Image cap. No 8.5 kernel primitive, installer, boot registration or guest
activation exists or is claimed.

Reproduce with `source tools/dev-env/env.sh; python3 tools/test_phase85_elf_fit.py`
(requires the repository's Rust 1.97.0 bare-metal sysroot, OpenSSL and the existing
offline build tools). The script builds the unmodified Phase 8.4 OS only to provide
`elf.rs`'s existing embedded `include_bytes!` targets for its host adapter; it **does
not** execute the probe in QEMU. It builds `tools/phase85-elf-probe/` twice
offline/locked, the second time to a fresh independent target directory, then compiles
the **unmodified** `kernel/kernel/src/elf.rs` through
`tools/phase85-elf-probe/validate.rs` with host stubs for loader-only interfaces. The
host adapter **calls the real `elf::validate`** (not the stub loader) and checks W+X,
foreign-entry and truncated-file refusals. It signs the binary as canonical APKG v1 with
independent host OpenSSL and the already-public RFC 8032 **test** seed; it checks the
exact v1 domain, test-root verification, and tampered-message refusal. Temporary host
DER keys are deleted by `TemporaryDirectory`; none enters the guest or repository.

| Observation | Measured result |
|---|---|
| Built target | `x86_64-unknown-none`, no_std Rust, offline locked, `ET_EXEC` static `x86-64` |
| ELF file/payload | **648 bytes** (limit **4096**), two fresh builds byte-identical |
| `elf::validate` | **PASS**, entry `0x400078`, one `PT_LOAD`, file offset 0, `filesz=memsz=436`, `R|X` only, header inside segment; W+X, entry outside executable segment and truncation refused |
| ELF SHA-256 | `a591506a09d1e19ad24292f3608dbc9915b427380482332b6b84ab779b694d38` |
| Canonical root-signed `APKG` v1 | `app.test`, version 7, **840 bytes** full file (`192+648`), signed payload/source byte-exact; full-file SHA-256 `98cc82f0fb19d1c006c21f57853617cbd21a0f85993accbc20eb8b8b31cdfe97` |

The fixture is more than a print-only stub: with a single inherited Endpoint/WRITE cap
in slot zero it checks `SYS_CAP_DESCRIBE` for the exact kind and rights, makes a real
ABI-v1 `SYS_IPC_CALL` to `packaged` `QUERY app.test`, requires eligible version 7, a
nonzero returned full-file digest and **no returned cap**, then uses `SYS_DEBUG_WRITE`
and exits 42 only if the diagnostic byte count matches. Failure paths exit distinct
non-42 codes. The linker script's `PT_LOAD FILEHDR PHDRS` puts header and code in one RX
segment at file offset zero; all request/reply buffers are on the one-page stack and no
RW image segment is needed. This is a deliberately small purpose-built mechanism
fixture, **not** proof a general Rust application fits, not a new runtime format, and
not proof that the future installer grants only that one cap. The 8.5 guest gate must
later sign/seed this exact (or rebuilt/re-reviewed) artifact, install/activate/register
through the accepted path, spawn it and observe its real IPC, exit status, cap inventory
and resource teardown in QEMU. Any new compiler/toolchain/linker options or payload
changes require repeating this measurement. APKG v1's 4096-byte ceiling remains
unchanged; a future larger-app need requires its own explicitly versioned signed-wire
decision.
