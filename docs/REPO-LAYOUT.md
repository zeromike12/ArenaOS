# ArenaOS — Repository Layout

```
ArenaOS/
├── README.md               ← what this is, status board, quickstart
├── docs/
│   ├── VISION.md           ← product/engineering vision
│   ├── ARCHITECTURE.md     ← system architecture (living document)
│   ├── ROADMAP.md          ← milestones, exit criteria, scope firewall
│   ├── RISKS.md            ← risk register with mitigations
│   ├── DEV-ENV.md          ← toolchain, bootstrap, build/boot/debug how-to
│   ├── TESTING.md          ← testing philosophy, marker grammar, determinism
│   ├── CODING-CONVENTIONS.md
│   ├── REPO-LAYOUT.md      ← this file
│   └── adr/                ← architecture decision records (+ index, template)
├── kernel/                 ← all ring-0 + boot code (Rust cargo workspace)
│   ├── Cargo.toml          ← workspace root (members grow per milestone)
│   ├── .cargo/config.toml  ← default target = x86_64-unknown-uefi
│   ├── libs/               ← machine-independent kernel logic (host-testable)
│   │   ├── heap/           ← arena-heap: allocator core + host suite (M2.5)
│   │   └── sync/           ← arena-sync: spinlock + owner tracking, host threads (M2.6)
│   └── boot/               ← Milestone 1: boot stage = UEFI application
│       └── src/
│           ├── main.rs     ← efi_main entry, boot orchestration
│           ├── log.rs      ← structured serial logging
│           ├── panic.rs    ← panic handler (diagnostics + halt)
│           ├── uefi.rs     ← hand-written UEFI ABI bindings (ADR-0004)
│           ├── bootinfo.rs ← memory-map capture/classification
│           ├── m1.rs       ← Milestone-1 self-tests + runner
│           ├── arch/x86_64/← CPU state, cpuid, port I/O, GDT (arch quarantine)
│           └── drivers/serial.rs ← 16550 UART driver
├── tools/                  ← dev tooling (Python/bash — never in the OS image)
│   ├── build-sysroot.sh    ← offline rebuild of core/alloc for bare-metal targets
│   ├── build.sh            ← cargo build + ESP image assembly
│   ├── espimg.py           ← FAT16 ESP image builder (fresh-image discipline)
│   ├── qemu_env.py         ← QEMU/OVMF/musl-loader resolution (sandbox + normal hosts)
│   ├── run.sh              ← interactive boot (serial on stdio)
│   ├── run_tests.sh        ← all milestone regression tests
│   ├── test_m1.py          ← Milestone 1 automated boot test
│   └── dev-env/
│       ├── bootstrap.sh    ← idempotent full environment bootstrap
│       └── env.sh          ← shell environment (PATH etc.)
├── tests/                  ← cross-milestone test assets (fixtures, logs of record)
├── build/                  ← generated artifacts (gitignored)
└── .gitignore
```

Layout rules:

- **`kernel/`** is a cargo workspace; future crates: `kernel/core/` (the
  kernel proper, M2.7 split), `kernel/arch-x86_64/` if shared arch code
  outgrows in-crate modules. Every crate in it builds for a bare-metal
  target and contains zero external dependencies (ADR-0004).
- **`userspace/`** is live since M4.1 with its first crate,
  `userspace/payload` (the ADR-0016 test image: a static ELF64 built for
  `x86_64-unknown-none` with its own linker script, embedded into the
  kernel by `tools/build.sh`). Since M4.6 it holds a second crate,
  `userspace/shell` (the minimal shell, ADR-0020: same toolchain
  contract, its own address window at 0x400000, embedded as
  spawn-registry image 1 and spawned at boot as the initial service).
  Servers and apps join here in later phases.
- Future top-level directories, added only when their phase begins:
  `libs/` (shared userspace libraries), `drivers/` (userspace driver
  servers, Phase 6). Not created empty — directories appear with code.
- `tools/` may use external packages (pip/npm); it is explicitly outside the
  OS image. `tests/` holds assets consumed by `tools/test_*.py`.
- Docs are peers of code: a change that alters an architectural fact must
  touch the corresponding doc in the same commit.
