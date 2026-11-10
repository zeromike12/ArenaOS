# Surface-layout boundary correction evidence — 2026-10-30

This is a new, separate evidence record. The historical `v2-2026-10-29-r3` tree and archive were not edited. Its earlier surface-layout result is not used as proof of the 300-page production refusal.

## Result

The production page-count predicate now lives in `experiments/c-runtime/app/c_gfx_app.c` and is called by the real client path immediately after `ARENA_SYS_SHARED_PAGES`, before mapping the surface or creating/publishing a window. The host test and the guest refusal client call that same predicate; the guest's synthetic 301/300 values are explicitly classified as `CLIENT-POLICY` inputs, not kernel grants.

In the final staged-source guest run (`development-final/`), both compiler cells completed all 24 refusal cases:

| Evidence | 301 pages | 300 pages | Real Desktop grant | Matrix |
|---|---:|---:|---:|---|
| GCC | accepted, status 0 | refused, status 7 | 471 pages | 24/24 PASS |
| Clang via pinned Zig 0.17.0 | accepted, status 0 | refused, status 7 | 471 pages | 24/24 PASS |

`layout-pages-minimum` is the separate real kernel query. The 301/300 policy inputs are not presented as forged or kernel-created `SharedRegion` grants. `layout-required-pages` reports exactly 301, and `layout-pixel-window-fits` passes. See each compiler's `refusals-evidence.json`, serial log, APB1 and screenshots under `development-final/`.

The hosted GCC sanitizer/differential suite passed **1,767 checks**; both guest ELFs passed static loader and instruction audits (zero loader failures and zero SIMD/x87 instructions). The 35 focused guest-proof controls and 10 aggregate negative controls also passed; raw logs and the summarized audit record are in `environment/`.

## Qualification boundary — deliberately not claimed

The guest tests used the explicitly **development-only, unqualified** image produced by `bash tools/build_phase16_dev.sh --image`. Its EFI SHA-256 is `a522f293c7685aa13ba5f15c8cbdbfa2b6f42acb5105a8b80900c1fb7e430e27`; its ESP SHA-256 is `430039e099f7d3e8cc887dbbd42c8697a6da2c1d46099633227f57115764bd12`, and the ESP was independently checked to embed that EFI. The build script completed its kernel ISA audit and clearly labels the artifacts development-only. The protected canonical `tools/build.sh --image` build was not run: its documented `/workspace/ArenaOS` build root is absent here, and its protected Phase 14 guard was not weakened or bypassed.

Accordingly, both final refusal-stage receipts set `qualified: false`. The case-level 5e evidence validates for GCC and Clang, but the requirement-aware aggregate correctly reports requirement **5e as `BLOCKED`**, not `PASS`, because the boot image is development-only. `development-final/requirements-5e-validation.json` records the case validation, source/EFI identity checks and expected blocked status. This is a targeted 5e check, not a full V1 gate or qualification run; no historical artifact qualification is transferred to the changed source.

The pinned backend is the repository's locally modified QEMU 11.1.2 TCG package, not upstream-QEMU or hardware-backed. The archive sidecar and all 2,319 package file hashes were verified. The wrapper uses the isolated glibc 2.40 loader and explicit `-accel tcg`; the QMP initialization smoke passed. QEMU, wrapper, loader, runtime-library, ROM and OVMF hashes are recorded in `environment/environment.json` and `environment/qemu-tcg-smoke.json`.

## Preserved attempts and receipts

- `refusals-gcc/` and `gate/receipts/refusals-gcc.json` preserve the first attempted boot with the older, immutable Phase 15 package EFI/ESP. That Desktop boot reached the refusal launch, but the client exited with status 14 (Identify failed) before the matrix; it is a failure record, not acceptance evidence.
- `development/` preserves the successful preliminary GCC/Clang development run from before the final aggregate evidence-binding controls were added. Its receipts are historical to that earlier code fingerprint; do not use them for the final tree.
- `development-final/` contains the final staged-source GCC/Clang evidence and receipts. These receipts match the current staged source fingerprint, EFI hash and guest evidence, while remaining unqualified as described above. Raw matrix PPMs are retained beside the PNGs; `screenshot_ppm_sha256` hashes each PPM's RGB raster payload (excluding its header).
- Serial transcripts are preserved byte-for-byte; a few guest `displayd` lines end with spaces. A broad `git diff --check` therefore reports those raw evidence lines, while the source-only whitespace check is clean.
- No generated 72 MiB scratch disks are retained. The two reconstructed Rust target `.tar.xz` files remain untracked local recovery inputs and are intentionally excluded from this evidence/commit.

The guest implementation did not add syscalls, weaken policy, or use a privileged launch bypass. The evidence is limited to the boundary correction and its requirement-aware reporting.