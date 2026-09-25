# ArenaOS — Major Technical Risks

Honest register, reviewed at every phase gate. L = likelihood, I = impact
(1–5). Mitigations are actions we take *now*, not hopes.

| # | Risk | L | I | Mitigation |
|---|------|---|---|------------|
| R1 | **Scope collapse** — the project grows faster than it stabilizes; classic OS-project death | 4 | 5 | Roadmap with a scope firewall (ROADMAP "NOT building yet"); smallest-useful-version rule per milestone; one milestone at a time; phase gates require tests, not vibes |
| R2 | **IPC performance** — microkernel control paths too slow for storage/net workloads | 3 | 4 | Design commitments made up-front (rendezvous + badge notifications + shared-memory rings, ADR-0002); benchmark harness is an M4 exit criterion; batching call reserved in the ABI |
| R3 | **Memory-map / paging transition bugs** — ExitBootServices + new page tables is the most crash-prone moment in any UEFI kernel | 4 | 4 | Boot-info contract isolates firmware data (ADR-0003); M2.4 is its own milestone with fault-injection tests; boot stage keeps serial alive until hand-off; 100-clean-boot loop test |
| R4 | **Offline toolchain fragility** — pinned wheels/npm tarballs/GitHub mirrors could disappear; sysroot bootstrap is hand-maintained | 3 | 4 | All dev-env inputs pinned + documented with sources in DEV-ENV.md; bootstrap script is idempotent and re-runnable; sysroot build verified by compile+boot smoke test; toolchain lives outside the repo but is 100% reproducible from documented steps |
| R5 | **QEMU-only reality distortion** — firmware/hardware quirks absent in OVMF/TCG (timing, SMP, real UARTs, real GPUs) | 3 | 3 | VirtIO-first driver strategy (matches QEMU's strongest surface); Phase 8 hardening milestone on real firmware; interfaces keep hardware assumptions explicit and documented |
| R6 | **UEFI spec drift in hand-written bindings** — wrong offsets/sizes corrupt memory silently | 2 | 4 | Static size/offset assertions in every `#[repr(C)]` struct; spec-section citations in comments (ADR-0004); M1 test boots real EDK2 every run — silent corruption fails loudly |
| R7 | **Single-maintainer bus factor** — knowledge trapped in one head | 3 | 5 | ADR discipline (decisions never live only in chat); docs are deliverables of each milestone; SAFETY comments; repo hygiene rules; tests as executable documentation |
| R8 | **Capability model ergonomics** — rights masks and delegation get designed wrong and are painful to change post-M4 | 3 | 4 | Start minimal (4 object types in M4); grow with concrete use cases; ABI versioning (ADR-0006) allows a v2 before 1.0 freeze |
| R9 | **Determinism of tests under TCG** — timing-sensitive tests flake | 2 | 3 | No wall-clock assertions; TSC used only as a monotonic counter; scheduler has an explicit deterministic RR test mode (M3.2) |
| R10 | **x86 lock-in creep** — arch assumptions leak out of `arch/` modules, ARM64 becomes rewrite | 3 | 2 | Coding convention: anything touching registers/paging/interrupts lives in `arch/`; review checklist item; boot-info record arch-neutral by contract |
| R11 | **Rust `no_std` friction** — missing `core` pieces (no async, limited float, allocator plumbing) slow subsystems down | 2 | 2 | Accepted in ADR-0001; we own allocators anyway; float use in kernel is banned by convention except where unavoidable (soft-float target) |
| R12 | **Debuggability of userspace services** — cross-process failures hard to trace | 3 | 3 | Structured logging from day one (every service, same serial grammar); kernel introspection objects (capability-gated); supervisor records restart history |
| R13 | **Burnout / motivation** — 20-year project, solo start | 4 | 4 | Milestones sized to days-to-two-weeks, each ending in a *booting, demonstrable* system; visible status board in README; no death marches on untestable features |

## Risk review ritual

At each phase gate: re-score this table, close dead risks, add new ones with
mitigations, and record material changes in the gate's commit message.
Risks without mitigations are not allowed to stay open silently.
