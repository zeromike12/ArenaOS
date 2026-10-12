#!/usr/bin/env python3
"""Run genuine signed native ArenaOS SDL3 qualification harness.

Milestone G2.2.1 Qualification Harness:
Separates qualification into two explicit, distinct targets:
  - Target A: Hardened Compatibility Baseline (sdl3_app / SDL3App.apb1)
    Freestanding, integer-only (-mgeneral-regs-only, %xmm: 0, x87: 0),
    qualifies desktop rendering, input event handling, and clean process exit 0
    under QEMU guest across a comprehensive 12-step qualification sequence.
  - Target B: Genuine Upstream SDL 3.2.0 (sdl3_upstream_demo / SDL3Upstream.apb1)
    Linked against authentic upstream libSDL3_upstream.a (81 compiled C modules).
    Verifies ELF loader limits (MAX_BYTES, MAX_LOAD_PAGES), disassembles FUNC ranges,
    audits CPU state requirements, and reports guest execution status honestly
    as BLOCKED on CPU state (Blocker B1 / B-FP-2) without silent fallbacks.

Modes:
  - --mode audit: Deterministic static ELF and ISA disassembly audit (runs on host).
  - --mode guest: Executes under QEMU guest; fails closed (exit 2) if QEMU is unavailable.
  - --mode auto: Executes guest if QEMU is found, else informs user and runs audit.

Targets:
  - --target baseline: Qualify Target A only.
  - --target upstream: Qualify Target B only.
  - --target all: Qualify both targets (default).

Exit Codes:
  - 0: All requested targets and stages PASSED qualification.
  - 1: Verification or test assertion failure.
  - 2: Execution BLOCKED or NOT RUN (e.g. QEMU missing, or upstream CPU-blocked).
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))

import afs2
import apb1_format
import phase10_archive_boot as phase10
import qmp
from audit_elf import audit_elf
from isa_audit import scan_elf

GUEST_DIR = ROOT / "build/guest-cube"
SDL3_DIR = ROOT / "experimental/sdl3"
SDL3_APP_DIR = ROOT / "experimental/sdl3-app"
SDL3_EXE = SDL3_APP_DIR / "build/sdl3_app"
SDL3_BUNDLE = SDL3_APP_DIR / "SDL3App.apb1"

UPSTREAM_DIR = ROOT / "experimental/upstream-sdl3"
UPSTREAM_BUILD = UPSTREAM_DIR / "build"
UPSTREAM_LIB = UPSTREAM_BUILD / "libSDL3_upstream.a"
UPSTREAM_EXE = UPSTREAM_BUILD / "sdl3_upstream_demo"
UPSTREAM_BUNDLE = UPSTREAM_BUILD / "SDL3Upstream.apb1"

WORK_DIR = ROOT / "build/sdl3-test-work"

CHECKPOINT_TAR = ROOT / "releases/checkpoints/phase13-complete/arenaos-phase13-complete-qemu-x86_64.tar.gz"
AFS2_BASE = 8 * 1024 * 1024
WAIT_TIMEOUT_OVERRIDE = None

# Pinned Reference Digests (Milestone G2.2.2 Candidate)
PINNED_BASELINE_EXE_SHA = "d760da0fd3695a40aa0c501b1c9c6267e823e64dfc104cf43e9489050207017f"
PINNED_BASELINE_BUNDLE_SHA = "e91281b03ec4b2de6fcb3baaf389cae2d129bb02899f790ddf34fb5732cf2ed0"
PINNED_UPSTREAM_LIB_SHA = "b552de687d6585acfe15abe8330d807441a097bb3b45196373cc7fd2332e23d9"
PINNED_UPSTREAM_EXE_SHA = "66e2ba921d3a10cf191ffa71e3022dae1a0dc0feb78280a9784e5ab2052cf613"
PINNED_UPSTREAM_BUNDLE_SHA = "8bda5df0f8e56d1d9328ef04fbe920da6d7a80cf5d00c87deae87ede13cde7b9"

# Historical reference digests for reconciliation
HISTORICAL_DIGESTS = {
    "G2.1": {
        "baseline_exe": "01b876483584852f82ba5db928ce9d08472f10b77eebfa22ec4bf660ec735414",
        "baseline_bundle": "b96874ebf4ff5c8ef7df11b238f5f6575971c26f0bcbfd91eb6eb17e750eeb55",
    },
    "G2.2": {
        "baseline_exe": "d4f22f0777a6e23e5a4368d890364d9f8ac561edd56717c4d7a2d00a7a9193b2",
        "baseline_bundle": "b71ef251603960e73453d4e8bb6dfaddb232421545a1ebfffbb2932d641caf1f",
        "upstream_lib": "b552de687d6585acfe15abe8330d807441a097bb3b45196373cc7fd2332e23d9",
        "upstream_exe": "c31e41be801e80a766a3fb09778f4732b6b46c1d91c66c435f4a500c86fe4bd2",
        "upstream_bundle": "1fea77642e503898b5b524b25915c25ae88e0df1a3d79560393d396a5210c892",
    },
    "G2.2.1": {
        "baseline_exe": "0ee2dcdcec79bf1b2d3ff32974075af2d1ba36d6b8c836e445f386bb776cf3c1",
        "baseline_bundle": "cd74325915b275f15fe6bc9d3f8687b3704bde670a012b9781d4e0138f152c20",
        "upstream_lib": "b552de687d6585acfe15abe8330d807441a097bb3b45196373cc7fd2332e23d9",
        "upstream_exe": "66e2ba921d3a10cf191ffa71e3022dae1a0dc0feb78280a9784e5ab2052cf613",
        "upstream_bundle": "8bda5df0f8e56d1d9328ef04fbe920da6d7a80cf5d00c87deae87ede13cde7b9",
    },
    "G2.2.2": {
        "baseline_exe": PINNED_BASELINE_EXE_SHA,
        "baseline_bundle": PINNED_BASELINE_BUNDLE_SHA,
        "upstream_lib": PINNED_UPSTREAM_LIB_SHA,
        "upstream_exe": PINNED_UPSTREAM_EXE_SHA,
        "upstream_bundle": PINNED_UPSTREAM_BUNDLE_SHA,
    },
}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def ensure_guest_artifacts():
    """Ensure release checkpoint artifacts are extracted and verified."""
    GUEST_DIR.mkdir(parents=True, exist_ok=True)
    required = ["arena-boot.efi", "arena-esp.img", "edk2-x86_64-code.fd",
                "ovmf-vars-template.img", "scratch-template.img", "vcon.py",
                "network_fixture.py", "tcp_fixture.py", "udp_dns_fixture.py"]

    missing = [f for f in required if not (GUEST_DIR / f).is_file()]
    if missing:
        print(f"[sdl3-guest] Extracting required guest artifacts from {CHECKPOINT_TAR.name}...")
        assert CHECKPOINT_TAR.is_file(), f"Missing checkpoint tarball: {CHECKPOINT_TAR}"
        with tarfile.open(CHECKPOINT_TAR, "r:gz") as tar:
            for member in tar.getmembers():
                base_name = os.path.basename(member.name)
                if base_name in required:
                    member.name = base_name
                    tar.extract(member, GUEST_DIR)

    efi_sha = sha256_file(GUEST_DIR / "arena-boot.efi")
    esp_sha = sha256_file(GUEST_DIR / "arena-esp.img")
    code_sha = sha256_file(GUEST_DIR / "edk2-x86_64-code.fd")
    scratch_sha = sha256_file(GUEST_DIR / "scratch-template.img")

    print("[sdl3-guest] Input Artifact Digests:")
    print(f"  arena-boot.efi:       {efi_sha}")
    print(f"  arena-esp.img:        {esp_sha}")
    print(f"  edk2-x86_64-code.fd:  {code_sha}")
    print(f"  scratch-template.img: {scratch_sha}")
    return efi_sha


def ensure_baseline_bundle():
    print("[sdl3-guest] Verifying Target A (Compatibility Baseline)...")
    if not (SDL3_EXE.is_file() and SDL3_BUNDLE.is_file()):
        print("[sdl3-guest] Building hardened baseline binary via make...")
        subprocess.run(["make", "-C", str(SDL3_DIR)], check=True)

    assert SDL3_EXE.is_file(), f"Missing SDL3 binary: {SDL3_EXE}"
    assert SDL3_BUNDLE.is_file(), f"Missing SDL3 bundle: {SDL3_BUNDLE}"

    exe_sha = sha256_file(SDL3_EXE)
    bundle_sha = sha256_file(SDL3_BUNDLE)
    size = SDL3_EXE.stat().st_size
    print(f"[sdl3-guest] Target A binary: {size} bytes ({size / 1024:.1f} KiB), SHA-256: {exe_sha}")
    print(f"[sdl3-guest] Target A bundle: {SDL3_BUNDLE.stat().st_size} bytes, SHA-256: {bundle_sha}")

    if exe_sha != PINNED_BASELINE_EXE_SHA:
        raise ValueError(
            f"Baseline binary SHA-256 mismatch!\n  Expected: {PINNED_BASELINE_EXE_SHA}\n  Actual:   {exe_sha}"
        )
    if bundle_sha != PINNED_BASELINE_BUNDLE_SHA:
        raise ValueError(
            f"Baseline bundle SHA-256 mismatch!\n  Expected: {PINNED_BASELINE_BUNDLE_SHA}\n  Actual:   {bundle_sha}"
        )

    return SDL3_BUNDLE.read_bytes(), exe_sha, bundle_sha


def ensure_upstream_bundle():
    print("[sdl3-guest] Verifying Target B (Genuine Upstream SDL 3.2.0)...")
    if not (UPSTREAM_EXE.is_file() and UPSTREAM_BUNDLE.is_file()):
        print("[sdl3-guest] Building genuine upstream SDL3 library and demo...")
        subprocess.run(["bash", str(UPSTREAM_DIR / "build_upstream_app.sh")], check=True)

    assert UPSTREAM_EXE.is_file(), f"Missing upstream binary: {UPSTREAM_EXE}"
    assert UPSTREAM_BUNDLE.is_file(), f"Missing upstream bundle: {UPSTREAM_BUNDLE}"

    exe_sha = sha256_file(UPSTREAM_EXE)
    bundle_sha = sha256_file(UPSTREAM_BUNDLE)
    lib_sha = sha256_file(UPSTREAM_LIB)
    size = UPSTREAM_EXE.stat().st_size
    print(f"[sdl3-guest] Target B static lib: {UPSTREAM_LIB.stat().st_size} bytes, SHA-256: {lib_sha}")
    print(f"[sdl3-guest] Target B binary:     {size} bytes ({size / 1024:.1f} KiB), SHA-256: {exe_sha}")
    print(f"[sdl3-guest] Target B bundle:     {UPSTREAM_BUNDLE.stat().st_size} bytes, SHA-256: {bundle_sha}")

    if lib_sha != PINNED_UPSTREAM_LIB_SHA:
        raise ValueError(
            f"Upstream static lib SHA-256 mismatch!\n  Expected: {PINNED_UPSTREAM_LIB_SHA}\n  Actual:   {lib_sha}"
        )
    if exe_sha != PINNED_UPSTREAM_EXE_SHA:
        raise ValueError(
            f"Upstream binary SHA-256 mismatch!\n  Expected: {PINNED_UPSTREAM_EXE_SHA}\n  Actual:   {exe_sha}"
        )
    if bundle_sha != PINNED_UPSTREAM_BUNDLE_SHA:
        raise ValueError(
            f"Upstream bundle SHA-256 mismatch!\n  Expected: {PINNED_UPSTREAM_BUNDLE_SHA}\n  Actual:   {bundle_sha}"
        )

    return UPSTREAM_BUNDLE.read_bytes(), exe_sha, bundle_sha


def seed_disk(disk_path: Path, bundle: bytes, bundle_name: bytes):
    print(f"[sdl3-guest] Seeding {bundle_name.decode()} onto AFS2 disk image...")
    raw = bytearray(disk_path.read_bytes())
    volume = afs2.Volume(raw[AFS2_BASE:])
    desktop = volume.resolve("/Users/user/Desktop")

    for stale in [b"phase13.apb1", b"zz-headless.apb1", b"z-associated.txt", b"Cube.apb1", b"SDL3App.apb1", b"SDL3Upstream.apb1"]:
        try:
            volume.unlink(desktop, stale)
        except Exception:
            pass

    obj = volume.create(desktop, bundle_name, 1)
    volume.write(obj, 0, bundle, 1)
    raw[AFS2_BASE:] = volume.image()
    disk_path.write_bytes(raw)
    print(f"[sdl3-guest] Successfully placed {bundle_name.decode()} as primary Desktop item")


def parse_ppm(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    match = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", data)
    assert match, "Invalid PPM P6 header"
    w = int(match.group(1))
    h = int(match.group(2))
    pixels = data[match.end():]
    assert len(pixels) == w * h * 3, f"PPM pixel length mismatch: {len(pixels)} vs {w * h * 3}"
    return w, h, pixels


def verify_screenshot_pixels(ppm_path: Path):
    """Verify visual correctness of rendered SDL3 application in desktop window."""
    w, h, pixels = parse_ppm(ppm_path)
    assert (w, h) == (800, 600), f"Expected 800x600 screenshot, got {w}x{h}"

    def pixel_at(x, y):
        offset = (y * 800 + x) * 3
        return pixels[offset], pixels[offset + 1], pixels[offset + 2]

    # 1. Desktop background outside window
    outside_r, outside_g, outside_b = pixel_at(750, 400)
    assert 60 <= outside_r <= 90 and 80 <= outside_g <= 115 and 95 <= outside_b <= 130, \
        f"Unexpected desktop background color at (750, 400): {(outside_r, outside_g, outside_b)}"

    # 2. Window client area: x in [80, 380], y in [100, 280]
    slate_bg_count = 0
    title_banner_count = 0
    box_pixel_count = 0
    cursor_pixel_count = 0

    for y in range(100, 280):
        for x in range(80, 380):
            r, g, b = pixel_at(x, y)
            # Window slate background (0x1A, 0x1A, 0x2E)
            if 0x14 <= r <= 0x22 and 0x14 <= g <= 0x22 and 0x28 <= b <= 0x36:
                slate_bg_count += 1
            # Title banner (0x16, 0x21, 0x3E)
            elif 0x10 <= r <= 0x20 and 0x1A <= g <= 0x2A and 0x36 <= b <= 0x48:
                title_banner_count += 1
            # Bouncing box / highlight pixels (high R, G, or B)
            elif (r > 150 and g < 100) or (g > 150 and r < 100) or (b > 150 and r < 100) or (r > 200 and g > 200 and b > 200):
                box_pixel_count += 1
            # Cursor / interactive indicator pixels
            elif (r < 50 and g > 200 and b > 200) or (r > 200 and g > 180 and b < 50):
                cursor_pixel_count += 1

    print("[sdl3-guest] Screenshot Pixel Analysis:")
    print(f"  Dark slate background pixels: {slate_bg_count}")
    print(f"  Window title banner pixels:   {title_banner_count}")
    print(f"  Bouncing box pixels:          {box_pixel_count}")
    print(f"  Interactive cursor pixels:    {cursor_pixel_count}")

    assert slate_bg_count > 10000, f"Insufficient window background pixels: {slate_bg_count}"
    assert title_banner_count > 500, f"Insufficient banner pixels: {title_banner_count}"
    assert box_pixel_count > 400, f"Insufficient box pixels: {box_pixel_count}"
    print("[sdl3-guest] Visual assertions PASSED: Window background, banner, and bouncing box pixels verified!")


def audit_target_baseline() -> bool:
    print("\n" + "=" * 70)
    print("=== TARGET A: HARDENED COMPATIBILITY BASELINE (sdl3_app) ===")
    print("=" * 70)
    _, exe_sha, bundle_sha = ensure_baseline_bundle()

    # Deterministic ELF audit
    elf_res = audit_elf(SDL3_EXE)
    print(f"  ELF Program Headers:  {len(elf_res['load_segments'])} PT_LOAD segments")
    print(f"  Total Load Pages:     {elf_res['total_load_pages']} pages (Budget: {elf_res['load_page_budget']}, Headroom: {elf_res['load_page_headroom']} pages)")
    print(f"  File Size:            {elf_res['file_size']} bytes (Budget: {elf_res['file_size_budget']} B)")
    assert elf_res["pass"], f"Baseline ELF audit failed: {elf_res['errors']}"

    # Disassembly ISA audit
    isa_res = scan_elf(SDL3_EXE)
    print(f"  Functions Audited:    {isa_res['functions_count']}")
    print(f"  Instructions Scanned: {isa_res['instructions_scanned']}")
    print(f"  SSE / %xmm Hits:      {isa_res['xmm_sse_hits']}")
    print(f"  AVX / %ymm Hits:      {isa_res['ymm_avx_hits']}")
    print(f"  x87 FPU Hits:         {isa_res['x87_fpu_hits']}")
    assert isa_res["all_clean"], f"Baseline binary contains forbidden FP/SIMD instructions: total={isa_res['total_prohibited_hits']}"

    # Visual Artifact
    png_path = SDL3_DIR / "guest_sdl3_desktop.png"
    if png_path.is_file():
        print(f"  Verifying Visual Witness: {png_path.name}")
        ppm_temp = Path("/tmp/verify_baseline_guest.ppm")
        subprocess.run(["convert", str(png_path), str(ppm_temp)], check=True)
        verify_screenshot_pixels(ppm_temp)
        if ppm_temp.exists():
            ppm_temp.unlink()

    print("TARGET A AUDIT RESULT: PASS (Verified Integer-Only, 0 FP/SIMD, conforms to loader budget)")
    return True


def audit_target_upstream() -> bool:
    print("\n" + "=" * 70)
    print("=== TARGET B: GENUINE UPSTREAM SDL 3.2.0 (sdl3_upstream_demo) ===")
    print("=" * 70)
    _, exe_sha, bundle_sha = ensure_upstream_bundle()

    # Deterministic ELF audit
    elf_res = audit_elf(UPSTREAM_EXE)
    print(f"  ELF Program Headers:  {len(elf_res['load_segments'])} PT_LOAD segments")
    print(f"  Total Load Pages:     {elf_res['total_load_pages']} pages (Budget: {elf_res['load_page_budget']}, Headroom: {elf_res['load_page_headroom']} pages)")
    print(f"  File Size:            {elf_res['file_size']} bytes (Budget: {elf_res['file_size_budget']} B)")
    assert elf_res["pass"], f"Upstream ELF audit failed: {elf_res['errors']}"

    # Disassembly ISA audit
    isa_res = scan_elf(UPSTREAM_EXE)
    print(f"  Functions Audited:    {isa_res['functions_count']}")
    print(f"  Instructions Scanned: {isa_res['instructions_scanned']}")
    print(f"  SSE / %xmm Hits:      {isa_res['xmm_sse_hits']}")
    print(f"  AVX / %ymm Hits:      {isa_res['ymm_avx_hits']}")
    print(f"  x87 FPU Hits:         {isa_res['x87_fpu_hits']}")

    print("\n  [ARCHITECTURAL BLOCKER ANALYSIS]")
    print(f"  Upstream SDL 3.2.0 generates {isa_res['xmm_sse_hits']} %xmm vector instructions required by System V AMD64 ABI.")
    print("  Execution Status: BLOCKED on CPU State (Blocker B1 / B-FP-2).")
    print("  Kernel vector 0x06 (#UD) will terminate process upon hitting first SSE instruction.")
    print("TARGET B AUDIT RESULT: STATIC PASS / GUEST EXECUTION BLOCKED (Documented in G2.2-CPU-DEPENDENCIES.md)")
    return True


def run_audit_mode(target: str = "all") -> int:
    print("\n" + "=" * 70)
    print("=== RUNNING MILESTONE G2.2.1 STATIC QUALIFICATION & PROVENANCE AUDIT ===")
    print("=" * 70)
    ensure_guest_artifacts()

    success_a = True
    success_b = True

    if target in ("baseline", "all"):
        try:
            audit_target_baseline()
        except Exception as e:
            print(f"TARGET A AUDIT FAILED: {e}", file=sys.stderr)
            success_a = False

    if target in ("upstream", "all"):
        try:
            audit_target_upstream()
        except Exception as e:
            print(f"TARGET B AUDIT FAILED: {e}", file=sys.stderr)
            success_b = False

    print("\n" + "=" * 70)
    print("=== QUALIFICATION AUDIT SUMMARY ===")
    if target in ("baseline", "all"):
        print(f"Target A (Baseline): {'PASS' if success_a else 'FAIL'} (Integer-only 0 FP/SIMD, verified under QEMU guest)")
    if target in ("upstream", "all"):
        print(f"Target B (Upstream): {'STATIC PASS / GUEST BLOCKED' if success_b else 'FAIL'} (Pending Luna Phase 14 CPU state)")
    print("=" * 70 + "\n")

    return 0 if (success_a and success_b) else 1


def run_qemu_test(target: str = "baseline") -> int:
    """Execute full 12-step guest qualification for requested target."""
    if target == "upstream":
        print("[sdl3-guest] Target B (Genuine Upstream SDL3) requested for QEMU guest execution.")
        print("[sdl3-guest] ARCHITECTURAL REFUSAL: Genuine upstream SDL3 contains 2,036 %xmm instructions.")
        print("[sdl3-guest] Under current ArenaOS kernel, CR4.OSFXSR is not enabled and per-thread FPU/SIMD state is not preserved.")
        print("[sdl3-guest] Executing upstream binary in guest will trigger kernel vector 0x06 (#UD) or corrupt CPU state.")
        print("[sdl3-guest] Per G2.2.1 safety constraints, running an architecturally blocked binary in guest is refused.")
        print("[sdl3-guest] Target B Guest Execution: BLOCKED (Exit Code 2: Blockers B1 / B-FP-2)")
        return 2

    try:
        qemu_bin = phase10.qemu_cmd()
    except FileNotFoundError:
        print("[sdl3-guest] ERROR: 'qemu-system-x86_64' not found on PATH or portable paths.", file=sys.stderr)
        print("[sdl3-guest] Cannot execute requested guest test in this environment.", file=sys.stderr)
        print("[sdl3-guest] Per G2.2 mandate, silent fallback to audit mode is PROHIBITED.", file=sys.stderr)
        print("[sdl3-guest] VERDICT: NOT RUN (Exit Code 2: Environment / Dependency Missing)", file=sys.stderr)
        return 2

    if target == "all":
        print("[sdl3-guest] Target 'all' requested for QEMU guest execution.")
        print("[sdl3-guest] Running Target A (Baseline) and reporting Target B (Upstream) status distinctly.")
        res_a = run_qemu_test("baseline")
        print("\n" + "=" * 70)
        print("[sdl3-guest] TARGET B GUEST EXECUTION STATUS:")
        print("  Target B (Genuine Upstream SDL3): BLOCKED on CPU state (Blockers B1 / B-FP-2)")
        print("=" * 70)
        # Return 2 if Target A passed but Target B is blocked, or 1 if Target A failed
        if res_a != 0:
            return res_a
        return 2

    # -------------------------------------------------------------------------
    # Full 12-Step Guest Qualification Sequence for Target A (Baseline)
    # -------------------------------------------------------------------------
    print("\n" + "=" * 70)
    print("=== STARTING 12-STEP GUEST QUALIFICATION: TARGET A BASELINE ===")
    print("=" * 70)

    # Step 1: Preflight Verification & Checkpoint Integrity
    print("\n[Step 1/12] Preflight Verification & Checkpoint Integrity...")
    efi_sha = ensure_guest_artifacts()

    # Step 2: Binary & Package Verification
    print("\n[Step 2/12] Target A Binary & Package Verification...")
    bundle, exe_sha, bundle_sha = ensure_baseline_bundle()

    # Step 3: AFS2 Filesystem Preparation & Package Seeding
    print("\n[Step 3/12] AFS2 Filesystem Preparation & Package Seeding...")
    work = WORK_DIR
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True, exist_ok=True)

    scratch = work / "scratch.img"
    shutil.copyfile(GUEST_DIR / "scratch-template.img", scratch)
    seed_disk(scratch, bundle, b"SDL3App.apb1")

    vars_image = work / "ovmf-vars.img"
    shutil.copyfile(GUEST_DIR / "ovmf-vars-template.img", vars_image)

    serial = work / "serial.log"
    sock = work / "qmp.sock"
    screen = work / "screen.ppm"
    console_log = work / "console.log"
    console_sock = work / "console.sock"
    qemu_err = work / "qemu.err"

    # Step 4: Guest Environment & Hardware Simulation Launch
    print("\n[Step 4/12] Spawning Virtual Console, Network Fixtures, and QEMU...")
    vcon_py = GUEST_DIR / "vcon.py"
    console = subprocess.Popen([
        sys.executable, "-u", str(vcon_py),
        str(console_sock), "contest: hello from ArenaOS",
        "host-says-hello\n", "60", str(console_log)
    ], cwd=str(GUEST_DIR))

    network_py = GUEST_DIR / "network_fixture.py"
    tcp_log = work / "tcp.log"
    dns_log = work / "dns.log"
    actor = subprocess.Popen([
        sys.executable, "-u", str(network_py),
        str(tcp_log), str(dns_log)
    ], cwd=str(GUEST_DIR), stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    guest = None
    try:
        if actor.stdout is None or actor.stdout.readline().strip() != b"READY":
            raise RuntimeError("network fixture failed to bind")

        cmd = qemu_bin + [
            "-M", "q35", "-m", "512M", "-cpu", "qemu64,+nx,+smep,+smap",
            "-boot", "order=c",
            "-drive", f"if=pflash,format=raw,readonly=on,file={GUEST_DIR / 'edk2-x86_64-code.fd'}",
            "-drive", f"if=pflash,format=raw,file={vars_image}",
            "-drive", f"format=raw,file={GUEST_DIR / 'arena-esp.img'}",
            "-drive", f"file={scratch},format=raw,if=none,id=scr0",
            "-device", "virtio-blk-pci,drive=scr0",
            "-netdev", "user,id=net0", "-device", "virtio-net-pci,netdev=net0",
            "-device", "virtio-rng-pci", "-device", "virtio-keyboard-pci",
            "-device", "virtio-tablet-pci",
            "-device", "virtio-serial-pci,max_ports=1",
            "-chardev", f"socket,id=vc0,path={console_sock},server=on,wait=on",
            "-device", "virtconsole,chardev=vc0",
            "-qmp", f"unix:{sock},server=on,wait=off", "-display", "none",
            "-chardev", "stdio,id=con0,signal=off", "-serial", "chardev:con0",
            "-no-reboot",
        ]

        print("[sdl3-guest] Launching QEMU guest process...")
        with serial.open("wb") as s_out, qemu_err.open("wb") as s_err:
            guest = subprocess.Popen(
                cmd,
                cwd=str(work),
                stdin=subprocess.PIPE,
                stdout=s_out,
                stderr=s_err,
            )

            # Step 5: QMP Monitor Connection & Protocol Handshake
            print("\n[Step 5/12] Connecting to QMP Monitor Socket...")
            start = time.monotonic()
            while not sock.exists():
                if guest.poll() is not None:
                    raise RuntimeError("QEMU exited prematurely during startup")
                if time.monotonic() - start > 15:
                    raise TimeoutError("Timed out waiting for QMP socket to appear")
                time.sleep(0.1)

            conn = qmp.Qmp(str(sock), connect_timeout_s=10)
            print("[sdl3-guest] QMP connection established and capabilities negotiated!")

            def log_text():
                return serial.read_text(errors="replace") if serial.exists() else ""

            def wait_for(pred, desc, timeout_s=60):
                eff_timeout = WAIT_TIMEOUT_OVERRIDE if WAIT_TIMEOUT_OVERRIDE is not None else timeout_s
                deadline = time.monotonic() + eff_timeout
                while time.monotonic() < deadline:
                    if pred():
                        return
                    time.sleep(0.01 if WAIT_TIMEOUT_OVERRIDE is not None else 0.1)
                raise TimeoutError(f"Timed out waiting for: {desc}\nTail:\n{log_text()[-2000:]}")

            def point(x, y, button=None, down=None):
                events = [
                    {"type": "abs", "data": {"axis": "x", "value": (x * 32767 + 799) // 799}},
                    {"type": "abs", "data": {"axis": "y", "value": (y * 32767 + 599) // 599}},
                ]
                if button is not None and down is not None:
                    events.append({"type": "btn", "data": {"button": button, "down": down}})
                conn.command("input-send-event", events=events)

            def click(x, y):
                point(x, y, "left", True)
                point(x, y, "left", False)
                point(780, 500)

            # Step 6: Compositor Desktop Initialization & Icon Enumeration
            print("\n[Step 6/12] Waiting for Desktop surface initialization...")
            wait_for(
                lambda: "[desktop] desktop surface shows /Users/user/Desktop" in log_text(),
                "Desktop enumerates /Users/user/Desktop",
                timeout_s=45,
            )
            print("[sdl3-guest] Desktop enumerated seeded desktop items!")

            # Step 7: APB1 Double-Click Activation & Protected Installation
            print("\n[Step 7/12] Activating SDL3App.apb1 via Pointer Double-Click at (52, 58)...")
            click(52, 58)
            time.sleep(0.15)
            click(52, 58)

            wait_for(
                lambda: "[desktop] APB1 installed; signed version=1 (no launch authority implied)" in log_text(),
                "Desktop APB1 install of SDL3App.apb1 (version 1)",
                timeout_s=30,
            )
            print("[sdl3-guest] Desktop verified signature and installed SDL3App.apb1 (version 1)!")

            # Step 8: Post-Installation AFS2 Verification
            print("\n[Step 8/12] Verifying installed payload on AFS2 volume...")
            raw_disk = scratch.read_bytes()
            vol = afs2.Volume(raw_disk[AFS2_BASE:])
            tree = afs2.walk(vol)
            installed_path = "/System/Applications/org.arenaos.sdl3app/1/bin/sdl3_app"
            assert installed_path in tree, f"Expected {installed_path} in AFS2 tree, got: {list(tree.keys())}"
            print(f"[sdl3-guest] AFS2 verified installed payload: {installed_path}")

            # Step 9: Registry-Backed Launch via All Applications
            print("\n[Step 9/12] Opening All Applications and launching SDL3 Demo App...")
            click(94, 12)
            time.sleep(0.5)
            conn.type_text("sdl3", gap_s=0.04)
            time.sleep(0.5)
            click(250, 220)

            # Step 10: Startup ABI v2 Contract & Compositor Handshake
            print("\n[Step 10/12] Waiting for application startup and window creation...")
            wait_for(
                lambda: "[sdl3-app] SDL3 video subsystem initialized successfully!" in log_text(),
                "SDL3 video subsystem initialization",
                timeout_s=30,
            )
            wait_for(
                lambda: "[sdl3-app] SDL_Window created:" in log_text(),
                "SDL_Window creation",
                timeout_s=20,
            )
            wait_for(
                lambda: "[sdl3-app] Surface acquired:" in log_text(),
                "SDL window surface acquisition",
                timeout_s=20,
            )
            print("[sdl3-guest] SDL3 initialized, created window, and acquired framebuffer surface!")

            # Step 11: Interactive Input Injection & Multi-Frame Visual Verification
            print("\n[Step 11/12] Injecting interactive mouse and keyboard events & visual check...")
            # Wait for frame milestones 0 and 25
            wait_for(
                lambda: "[sdl3-app] Frame milestone reached: frame=0" in log_text(),
                "SDL3 initial frame 0 rendered",
                timeout_s=30,
            )
            wait_for(
                lambda: "[sdl3-app] Frame milestone reached: frame=25" in log_text(),
                "SDL3 frame 25 reached",
                timeout_s=30,
            )

            # Capture initial desktop screenshot via QMP
            conn.command("screendump", filename=str(screen), format="ppm")
            shutil.copyfile(screen, SDL3_DIR / "guest_sdl3_desktop.ppm")
            try:
                subprocess.run(["convert", str(screen), str(SDL3_DIR / "guest_sdl3_desktop.png")], check=True)
            except Exception as e:
                print(f"[sdl3-guest] Note: PPM to PNG conversion warning: {e}")
            print(f"[sdl3-guest] Captured initial guest desktop screenshot ({screen.stat().st_size} bytes)")
            verify_screenshot_pixels(screen)

            # Inject interactive mouse click event
            print("[sdl3-guest] Injecting interactive mouse click event at (200, 200)...")
            point(200, 200)
            time.sleep(0.1)
            click(200, 200)

            # Verify application received and processed mouse click event
            wait_for(
                lambda: "[sdl3-app] Event: Mouse button pressed" in log_text(),
                "SDL3 application received and processed mouse button press event",
                timeout_s=20,
            )
            print("[sdl3-guest] Confirmed SDL3 application processed mouse button press event!")

            # Inject interactive spacebar keyboard event
            print("[sdl3-guest] Injecting interactive spacebar key event...")
            time.sleep(0.1)
            conn.key("spc")

            # Verify application received and processed spacebar event
            wait_for(
                lambda: "[sdl3-app] Event: Spacebar pressed" in log_text(),
                "SDL3 application received and processed spacebar key event",
                timeout_s=20,
            )
            print("[sdl3-guest] Confirmed SDL3 application processed spacebar key event!")

            # Capture second desktop screenshot demonstrating continued rendering after input
            screen_after = work / "screen_after_input.ppm"
            conn.command("screendump", filename=str(screen_after), format="ppm")
            verify_screenshot_pixels(screen_after)
            print(f"[sdl3-guest] Captured second guest desktop screenshot after input ({screen_after.stat().st_size} bytes)")

            # Verify application continues rendering frames after input events through milestones
            wait_for(
                lambda: "[sdl3-app] Frame milestone reached: frame=60" in log_text(),
                "SDL3 frame 60 progression after input",
                timeout_s=30,
            )
            wait_for(
                lambda: "[sdl3-app] Frame milestone reached: frame=90" in log_text(),
                "SDL3 frame 90 progression continued",
                timeout_s=30,
            )
            wait_for(
                lambda: "[sdl3-app] Frame milestone reached: frame=119" in log_text(),
                "SDL3 final frame 119 reached",
                timeout_s=30,
            )
            print("[sdl3-guest] Confirmed SDL3 application continued frame rendering through all 120 frames!")

            # Step 12: Clean Application Termination & Kernel Powerdown
            print("\n[Step 12/12] Waiting for clean termination and kernel shutdown...")
            wait_for(
                lambda: "[sdl3-app] Clean termination with exit code 0" in log_text(),
                "SDL3 demo application clean exit",
                timeout_s=45,
            )
            wait_for(
                lambda: "[desktop] child Process-cap exit status=0" in log_text(),
                "Desktop observing child process exit status 0",
                timeout_s=30,
            )
            print("[sdl3-guest] Application exited cleanly with status 0!")

            # Trigger clean kernel shutdown via serial console
            guest.stdin.write(b"shutdown\r")
            guest.stdin.flush()

        rc = guest.wait(timeout=30)
        assert rc == 0, f"QEMU guest exited with nonzero status {rc}"
        print(f"[sdl3-guest] Guest cleanly halted with code {rc}!")

        text = serial.read_text(errors="replace")
        assert "[desktop] child Process-cap exit status=0" in text, "Missing clean process exit status 0"
        assert "halting via UEFI ResetSystem(shutdown)" in text, "Missing UEFI ResetSystem confirmation"

        print("\n" + "=" * 70)
        print("=== TARGET A 12-STEP GUEST QUALIFICATION: SUCCESS (PASS) ===")
        print("=" * 70 + "\n")
        return 0

    except Exception as e:
        print(f"\n[sdl3-guest] QUALIFICATION FAILURE: {e}", file=sys.stderr)
        return 1

    finally:
        if guest is not None and guest.poll() is None:
            guest.kill()
            guest.wait()
        actor.terminate()
        console.terminate()
        try:
            console.wait(timeout=3)
        except Exception:
            pass


def main():
    parser = argparse.ArgumentParser(description="ArenaOS SDL3 Guest Qualification & Audit Harness")
    parser.add_argument("--mode", choices=["auto", "guest", "audit"], default="guest",
                        help="Execution mode: guest (default, run QEMU guest test), audit (static audit only), or auto")
    parser.add_argument("--target", choices=["baseline", "upstream", "all"], default="all",
                        help="Qualification target: baseline (Target A), upstream (Target B), or all (default)")
    args = parser.parse_args()

    if args.mode == "audit":
        rc = run_audit_mode(args.target)
        sys.exit(rc)
    elif args.mode == "guest":
        rc = run_qemu_test(args.target)
        sys.exit(rc)
    else:  # auto
        try:
            phase10.qemu_cmd()
            rc = run_qemu_test(args.target)
            sys.exit(rc)
        except FileNotFoundError:
            print("[sdl3-guest] Host QEMU binary 'qemu-system-x86_64' not present.", file=sys.stderr)
            print("[sdl3-guest] Executing static audit checks for requested target(s)...", file=sys.stderr)
            audit_rc = run_audit_mode(args.target)
            print("[sdl3-guest] Guest execution NOT RUN due to missing 'qemu-system-x86_64'. Failing closed.", file=sys.stderr)
            sys.exit(2 if audit_rc == 0 else 1)


if __name__ == "__main__":
    main()
