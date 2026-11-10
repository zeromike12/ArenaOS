#!/usr/bin/env python3
"""Strict integration-evidence contract controls for the qualification packager."""

import hashlib
import json
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import v1_evidence  # noqa: E402

WIDTH, HEIGHT = 800, 600
PIE_SHA = "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"
APB1_SHA = "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def png_bytes(width: int, height: int, raster: bytes) -> bytes:
    def chunk(tag: bytes, body: bytes) -> bytes:
        return (struct.pack(">I", len(body)) + tag + body
                + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF))

    rows = b"".join(b"\x00" + raster[y * width * 3:(y + 1) * width * 3]
                    for y in range(height))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))


def rendered_raster(include_bottom: bool = True) -> bytes:
    raster = bytearray(WIDTH * HEIGHT * 3)
    # Placement [70,60,640,480], with the production 28-pixel title bar.
    # One checker pixel in each content band is sufficient for the same
    # existential color predicate used by the producer.
    points = [(100, 100), (100, 300)]
    if include_bottom:
        points.append((100, 500))
    for x, y in points:
        i = (y * WIDTH + x) * 3
        raster[i:i + 3] = bytes((20, 40, 90))
    return bytes(raster)


def make_run(root: Path) -> dict:
    root.mkdir(parents=True, exist_ok=True)
    (root / "artifacts").mkdir()
    (root / "shots").mkdir()

    efi = b"captured-BOOTX64-efi"
    (root / "artifacts/BOOTX64.EFI").write_bytes(efi)
    elf = b"guest-c-graphics-elf"
    (root / "artifacts/c-gfx-app-gcc.elf").write_bytes(elf)
    page_artifacts = {}
    fixtures = {}
    for pages in (301, 300):
        apb1 = f"signed-layout-{pages}-fixture".encode()
        apb_name = f"layout-{pages}-gcc.apb1"
        (root / "artifacts" / apb_name).write_bytes(apb1)
        page_artifacts[str(pages)] = {
            "application_id": f"com.arena.layout{pages}",
            "manifest_flags": 25,
            "requested_capabilities": 0,
            "elf": {
                "path": f"artifacts/c-gfx-app-gcc.elf",
                "size": len(elf),
                "sha256": sha(elf),
            },
            "apb1": {
                "path": f"artifacts/{apb_name}",
                "size": len(apb1),
                "sha256": sha(apb1),
            },
        }

        frame_names = ["prelaunch", "launcher-selection", "after-cleanup"]
        if pages == 301:
            frame_names.append("rendered")
        frames = {}
        for name in frame_names:
            raster = rendered_raster() if pages == 301 and name == "rendered" else bytes(WIDTH * HEIGHT * 3)
            png = png_bytes(WIDTH, HEIGHT, raster)
            rel = f"shots/layout-gcc-{pages}-{name}.png"
            (root / rel).write_bytes(png)
            frames[name] = {
                "png": rel,
                "png_sha256": sha(png),
                "raster_sha256": sha(raster),
                "width": WIDTH,
                "height": HEIGHT,
            }
        if pages == 301:
            # Deliberately obsolete prose: structure + pixels, not prose, is
            # the validator contract.
            frames["rendered"]["assertion"] = (
                "production checkerboard color present in the placed window content")

        serial_dir = root / f"layout-gcc-{pages}"
        serial_dir.mkdir()
        if pages == 301:
            serial_lines = [
                "[desktop] real desktop frame presented",
                "filesd: AFS2 mounted",
                "arena>",
                "[gfx] identify ok surface=77",
                "[desktop] layout fixture SharedRegion allocated app=com.arena.layout301 pages=301 surface=77",
                "[desktop] ordinary window placed handle=1 surface=77 x=70 y=60 w=640 h=480",
            ]
        else:
            serial_lines = ["[gfx] surface pages=300 required=301",
                            "[gfx] size-check refused before map/window/damage; exit=7"]
        serial = ("\n".join(serial_lines) + "\n").encode()
        serial_path = serial_dir / "serial.log"
        serial_path.write_bytes(serial)

        fixture = {
            "label": f"layout-gcc-{pages}",
            "compiler": "gcc",
            "expected_pages": pages,
            "application_id": f"com.arena.layout{pages}",
            "manifest_flags": 25,
            "requested_capabilities": 0,
            "apb1_sha256": sha(apb1),
            "elf_sha256": sha(elf),
            "pass": True,
            "allocated_pages": pages,
            "client_reported_pages": pages,
            "retired_pages": pages,
            "client_exit_status": 0 if pages == 301 else 7,
            "pixel_words": 307200 if pages == 301 else 306176,
            "pixel_nonzero_words": 307200 if pages == 301 else 0,
            "window_created": pages == 301,
            "identify_surface_object_id": 77 if pages == 301 else None,
            "surface_object_id": 77 if pages == 301 else 88,
            "no_map_write_window_damage_before_refusal": True,
            "frames": frames,
            "serial": {
                "path": f"layout-gcc-{pages}/serial.log",
                "sha256": sha(serial),
            },
        }
        if pages == 301:
            fixture.update({
                "rendered_visual_assertion": True,
                "rendered_visual_bands": {
                    "top": True, "middle": True, "bottom": True,
                },
                "placement": {
                    "line": 5, "handle": 1, "surface": 77,
                    "rect": [70, 60, 640, 480],
                },
            })
        fixtures[str(pages)] = fixture

    qemu = {
        "identity_ok": True,
        "elf_ok": True,
        "wrapper_path": "/pinned/qemu-patched",
        "wrapper_sha256": "1" * 64,
        "elf_path": "/pinned/qemu-elf",
        "elf_sha256": "2" * 64,
        "loader_path": "/pinned/loader",
        "loader_sha256": "3" * 64,
        "firmware": {
            "code_path": "/pinned/OVMF_CODE.fd",
            "code_sha256": "4" * 64,
            "vars_path": "/pinned/OVMF_VARS.fd",
            "vars_sha256": "5" * 64,
        },
        "runtime_libs": [{
            "path": "/pinned/libc.so.6", "sha256": "6" * 64,
        }],
    }
    compiler_ids = {
        "gcc": "gcc 12.2.0",
        "clang_via_zig": "clang 22.1.8",
        "zig": "0.17.0",
        "rustc": "rustc 1.97.0",
        "guest_compiler_under_test": "gcc",
    }
    payload = {
        "kind": "real_shared_region_layout_integration",
        "pass": True,
        "rc": 0,
        "source_commit": "a" * 40,
        "guest_compiler_under_test": "gcc",
        "compiler_identities": compiler_ids,
        "identities": {
            "source_commit": "a" * 40,
            "boot_efi": {
                "path": "/build/arena-boot.efi", "size": len(efi), "sha256": sha(efi),
            },
            "esp": {"path": "/build/arena-esp.img", "size": 8, "sha256": "7" * 64},
            "extracted_efi": {
                "path": "artifacts/BOOTX64.EFI", "size": len(efi), "sha256": sha(efi),
            },
            "efi_embedded_matches_build": True,
            "protected_phase14_pie": {
                "path": "/workspace/userspace/phase14-pie/fixture.elf",
                "size": 23256, "sha256": PIE_SHA,
            },
            "protected_signed_apb1": {
                "path": "/workspace/userspace/phase14-pie/phase14-pie.apb1",
                "size": 24040, "sha256": APB1_SHA,
            },
            "qemu": qemu,
            "compilers": compiler_ids,
        },
        "fixture_contract": {
            "required_pages": 301,
            "pixel_offset_bytes": 4096,
            "pixel_bytes": 640 * 480 * 4,
            "flags": 25,
            "requested_capabilities": 0,
            "new_kernel_syscalls": [],
            "added_child_capabilities": [],
        },
        "artifacts": page_artifacts,
        "fixtures": fixtures,
    }
    path = root / "integration-evidence.json"
    path.write_text(json.dumps(payload, indent=2) + "\n")
    return payload


class IntegrationEvidenceTests(unittest.TestCase):
    def run_validator(self, mutate=None):
        temp = tempfile.TemporaryDirectory(prefix="v1-integration-validator-")
        self.addCleanup(temp.cleanup)
        root = Path(temp.name) / "layout-integration-gcc"
        payload = make_run(root)
        if mutate:
            mutate(payload, root)
            (root / "integration-evidence.json").write_text(
                json.dumps(payload, indent=2) + "\n")
        return v1_evidence.check_integration_frames(root)

    def test_valid_structured_visual_contract_does_not_depend_on_prose(self):
        self.assertEqual([], self.run_validator())

    def test_visual_assertion_is_required(self):
        problems = self.run_validator(
            lambda payload, _root: payload["fixtures"]["301"].__setitem__(
                "rendered_visual_assertion", False))
        self.assertTrue(any("visual assertion is missing" in item for item in problems), problems)

    def test_all_three_band_assertions_are_required(self):
        problems = self.run_validator(
            lambda payload, _root: payload["fixtures"]["301"]["rendered_visual_bands"].__setitem__(
                "middle", False))
        self.assertTrue(any("top/middle/bottom" in item for item in problems), problems)

    def test_geometry_must_be_on_screen_and_bound_to_identify(self):
        def mutate(payload, _root):
            placement = payload["fixtures"]["301"]["placement"]
            placement["rect"] = [750, 550, 120, 80]
            placement["surface"] = 999
        problems = self.run_validator(mutate)
        self.assertTrue(any("rectangle is invalid or off-screen" in item for item in problems), problems)
        self.assertTrue(any("does not match Identify" in item for item in problems), problems)

    def test_capture_pixels_are_independently_checked_in_each_band(self):
        def mutate(payload, root):
            fixture = payload["fixtures"]["301"]
            frame = fixture["frames"]["rendered"]
            raster = rendered_raster(include_bottom=False)
            png = png_bytes(WIDTH, HEIGHT, raster)
            (root / frame["png"]).write_bytes(png)
            frame["png_sha256"] = sha(png)
            frame["raster_sha256"] = sha(raster)
        problems = self.run_validator(mutate)
        self.assertTrue(any("no checker color in bottom band" in item for item in problems), problems)

    def test_frame_hash_mismatch_is_rejected(self):
        def mutate(payload, root):
            path = root / payload["fixtures"]["301"]["frames"]["rendered"]["png"]
            path.write_bytes(path.read_bytes() + b"tampered")
        problems = self.run_validator(mutate)
        self.assertTrue(any("PNG hash changed" in item for item in problems), problems)

    def test_malformed_but_correctly_hashed_png_is_reported_not_raised(self):
        def mutate(payload, root):
            frame = payload["fixtures"]["301"]["frames"]["rendered"]
            malformed = b"\x89PNG\r\n\x1a\ntruncated"
            (root / frame["png"]).write_bytes(malformed)
            frame["png_sha256"] = sha(malformed)
        problems = self.run_validator(mutate)
        self.assertTrue(any("cannot decode" in item for item in problems), problems)

    def test_boolean_cannot_stand_in_for_zero_exit_or_capability_count(self):
        def mutate(payload, _root):
            payload["rc"] = False
            payload["fixture_contract"]["requested_capabilities"] = False
        problems = self.run_validator(mutate)
        self.assertTrue(any("PASS/exit 0" in item for item in problems), problems)
        self.assertTrue(any("fixture flags or requested capabilities" in item for item in problems), problems)

    def test_embedded_efi_identity_must_match_captured_efi(self):
        def mutate(payload, _root):
            payload["identities"]["extracted_efi"]["sha256"] = "f" * 64
        problems = self.run_validator(mutate)
        self.assertTrue(any("not byte-identical" in item for item in problems), problems)
        self.assertTrue(any("does not match extracted EFI" in item for item in problems), problems)

    def test_missing_300_fixture_is_rejected(self):
        problems = self.run_validator(lambda payload, _root: payload["fixtures"].pop("300"))
        self.assertTrue(any("300-page fixture record is missing" in item for item in problems), problems)

    def test_frame_path_cannot_escape_run_directory(self):
        def mutate(payload, _root):
            payload["fixtures"]["301"]["frames"]["rendered"]["png"] = "../../outside.png"
        problems = self.run_validator(mutate)
        self.assertTrue(any("escapes its run directory" in item for item in problems), problems)


if __name__ == "__main__":
    unittest.main(verbosity=2)
