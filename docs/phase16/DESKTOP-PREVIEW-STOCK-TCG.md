# Phase 16 diagnostic desktop preview — XF NOT RUN

This is a demo, not release, qualification, strict m15 PASS, or a milestone acceptance. The separate ZIP was built from mainline checkout `d2bfd660040ddface1cd6e791301fb8ea73d181c` using the existing `ARENA_GRAPHICS_FIXTURE=desktop ARENA_PHASE15_HISTORICAL_DIAGNOSTIC=1 tools/build.sh --image` option. No kernel source or protected guard was modified. The original Phase 14 PIE (`d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`) and signed APB1 (`04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`) checks passed, as did the kernel ISA audit. Strict artifacts were not replaced.

- ZIP `ArenaOS-Phase16-Desktop-Preview.zip`: SHA-256 `dfea2d77cace90ef5a50439920f871f984c3d476294cbdb0a5b87190ad3495a5`
- Diagnostic EFI: `fdad949615de9361fe933b32c33764000c1794c799b77129ab31ad0b4e845fda`
- Diagnostic ESP: `182a230dc3568a1f2db6d8db85736a324860e4d82ab34281394767bca8772795`

On Linux, unpatched QEMU 11.0.2 with explicit `-accel tcg` booted the extracted package's ESP, firmware and fresh prepared disk. Evidence bundled in the ZIP consists of the raw serial log, genuine QMP framebuffer PNG, and TCP/DNS peer captures. It records `m15: DIAGNOSTIC PASS (5/5; XF NOT RUN)`, package service and installed registry readiness, Desktop presentation, a QMP tablet dock click that spawned the gallery and changed framebuffer pixels, and guest-authorized UEFI shutdown with QEMU exit 0. A second boot from the final extracted ZIP also passed these checks. The first capture's raw serial SHA-256 is `d7280eaf566b07112dda692ef436df3ba1cc5549bb28eec1f566d964312b9a57`; the screenshot SHA-256 is `cc5df1d5aba3927de3b74333e85844fd180a3282c5f6151f39689b298248829a`.

Linux used the repository's existing Python fixture peers and headless/QMP instrumentation; it did not exercise the Windows PowerShell peers or launcher. **Windows testing has not been performed.** The package includes its own PowerShell TCP/UDP peers and launch instructions and requires installed Windows QEMU, not compilers or Python. Sources for the Windows launcher and the Linux verification harness are in `tools/phase16-desktop-preview/`.
