#!/usr/bin/env python3
"""Package and verify a V1 qualification evidence directory.

    python3 tools/v1_evidence.py package docs/v1/evidence/v2-2026-10-29
    python3 tools/v1_evidence.py verify docs/v1/evidence/v2-2026-10-29.tar.gz

`package` writes `<dir>/MANIFEST.json` (every file, its size and SHA-256),
`<dir>/README.md` (what the run is and how to check it), and a
`<dir>.tar.gz` archive that a reader can fetch later.

`verify` extracts the archive to a fresh temporary directory — NOT the
original workspace — and re-checks every digest. It fails if any file is
missing, has a different size or digest, or is present in the archive but
absent from the manifest (an unlisted file is unverifiable evidence).

The point is not the archive format. It is that a reader with only the
tarball can independently confirm that the serial logs, screenshots,
receipts and hashes are the ones the verdict was computed from.
"""

import hashlib
import json
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def _git(*args: str) -> str:
    try:
        out = subprocess.run(["git", *args], cwd=REPO_ROOT, capture_output=True,
                             text=True, check=False, timeout=30)
        return out.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return "unavailable"


# Independent restatement of the checkpoint images a guest run must publish.
#
# It is deliberately duplicated here instead of imported from the harness: a
# producer that silently stops publishing a checkpoint must not be able to
# relax the packaging rule that notices. The names cover the four input
# checkpoints (keyboard, mouse, per-instance isolation focus) and the final
# demonstration, which is exactly what M's round-2 requirement #2 asks for.
REQUIRED_GUEST_FRAMES = (
    "shots/cp01-launch-ready-desktop.png",
    "shots/cp03-first-native-c-window.png",
    "shots/cp04-keyboard-response.png",
    "shots/cp05-mouse-response.png",
    "shots/cp06a-two-instances.png",
    "shots/cp06a-focused.png",
    "shots/cp06b-focused.png",
    "shots/cp07-desktop-restored.png",
    "shots/cp08-final-demonstration.png",
)


def png_raster(data: bytes) -> tuple:
    """Decode a harness PNG (8-bit RGB, filter 0) to a bounded raster.

    Implemented here rather than shared with the producer so packaging checks
    the bytes that are actually published. Malformed/truncated chunks and
    oversized decompressions are reported as ValueError, never parser crashes.
    """
    import struct
    import zlib

    if not isinstance(data, bytes) or not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("not a PNG")
    pos = 8
    width = height = None
    depth = colour = compression = filter_method = interlace = None
    idat = []
    seen_ihdr = False
    seen_idat = False
    seen_iend = False
    while pos < len(data):
        if pos + 12 > len(data):
            raise ValueError("truncated PNG chunk header")
        (length,) = struct.unpack_from(">I", data, pos)
        tag = data[pos + 4:pos + 8]
        chunk_end = pos + 12 + length
        if chunk_end > len(data):
            raise ValueError(f"truncated PNG chunk {tag!r}")
        payload = data[pos + 8:pos + 8 + length]
        (crc,) = struct.unpack_from(">I", data, pos + 8 + length)
        if zlib.crc32(tag + payload) & 0xFFFFFFFF != crc:
            raise ValueError(f"PNG chunk {tag!r} CRC mismatch")
        if tag == b"IHDR":
            if seen_ihdr or pos != 8 or length != 13:
                raise ValueError("invalid PNG IHDR")
            (width, height, depth, colour, compression,
             filter_method, interlace) = struct.unpack(">IIBBBBB", payload)
            seen_ihdr = True
        elif tag == b"IDAT":
            if not seen_ihdr or seen_iend:
                raise ValueError("PNG IDAT is out of order")
            idat.append(payload)
            seen_idat = True
        elif tag == b"IEND":
            if length != 0 or not seen_idat:
                raise ValueError("invalid PNG IEND")
            seen_iend = True
            pos = chunk_end
            break
        pos = chunk_end
    if not seen_ihdr or not seen_iend or pos != len(data):
        raise ValueError("PNG is missing IHDR/IEND or has trailing bytes")
    if (depth, colour, compression, filter_method, interlace) != (8, 2, 0, 0, 0):
        raise ValueError("unexpected PNG depth/colour/compression/interlace")
    if not width or not height or width > 4096 or height > 4096 \
            or width * height > 16 * 1024 * 1024:
        raise ValueError("PNG dimensions are invalid or exceed the decode bound")
    stride = width * 3
    expected = (stride + 1) * height
    decoder = zlib.decompressobj()
    try:
        raw = decoder.decompress(b"".join(idat), expected + 1)
        if decoder.unconsumed_tail or len(raw) > expected:
            raise ValueError("PNG raster exceeds its declared dimensions")
        raw += decoder.flush()
    except zlib.error as exc:
        raise ValueError(f"invalid PNG deflate stream: {exc}") from exc
    if len(raw) != expected or not decoder.eof or decoder.unused_data:
        raise ValueError("PNG raster length/deflate termination is invalid")
    out = bytearray()
    for y in range(height):
        i = y * (stride + 1)
        if raw[i] != 0:
            raise ValueError(f"unexpected PNG filter {raw[i]} on row {y}")
        out += raw[i + 1:i + 1 + stride]
    return width, height, bytes(out)


def check_frames(root: Path) -> list:
    """Require every checkpoint image, and that it still shows its frame.

    A run that records a verdict but publishes no image for a checkpoint has
    not shown the frame the assertion was made on. Each published PNG must
    decode back to the raster digest the harness recorded at capture time, so
    a re-encoded, resized or substituted image is caught here rather than
    merely checksummed.
    """
    problems = []
    for evidence_path in sorted(root.rglob("evidence.json")):
        run_dir = evidence_path.parent
        rel_dir = str(run_dir.relative_to(root)) or "."
        try:
            payload = json.loads(evidence_path.read_text())
        except (OSError, ValueError) as exc:
            problems.append(f"{rel_dir}: evidence.json unreadable: {exc}")
            continue
        frames = payload.get("frames")
        if not isinstance(frames, dict) or not frames:
            problems.append(f"{rel_dir}: evidence.json records no checkpoint "
                            "frames")
            continue
        required = payload.get("required_frames") or list(REQUIRED_GUEST_FRAMES)
        missing = [n for n in REQUIRED_GUEST_FRAMES if n not in frames]
        if missing:
            problems.append(f"{rel_dir}: evidence.json omits checkpoint "
                            f"frames: {', '.join(missing)}")
        for name in required:
            entry = frames.get(name)
            if entry is None:
                problems.append(f"{rel_dir}: no digest entry for {name}")
                continue
            path = run_dir / name
            if not path.is_file() or path.stat().st_size == 0:
                problems.append(f"{rel_dir}: required checkpoint image "
                                f"{name} is missing or empty")
                continue
            data = path.read_bytes()
            if sha256_bytes(data) != entry.get("published_png_sha256"):
                problems.append(f"{rel_dir}: {name} does not match its "
                                "recorded published digest")
                continue
            try:
                w, h, raster = png_raster(data)
            except (ValueError, OSError) as exc:
                problems.append(f"{rel_dir}: {name} cannot be decoded: {exc}")
                continue
            if sha256_bytes(raster) != entry.get("raster_sha256"):
                problems.append(f"{rel_dir}: {name} decodes to a different "
                                "raster than the tested frame")
            if (w, h) != (entry.get("width"), entry.get("height")):
                problems.append(f"{rel_dir}: {name} is {w}x{h}, recorded "
                                f"{entry.get('width')}x{entry.get('height')}")
    return problems


def _is_sha256(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-fA-F]{64}", value) is not None


def _integration_resolve_file(run_dir: Path, raw: object) -> Path | None:
    """Resolve a recorded path only when the file remains inside this run."""
    if not isinstance(raw, str) or not raw.strip():
        return None
    candidate = Path(raw)
    choices = [candidate] if candidate.is_absolute() else [
        run_dir / candidate,
        REPO_ROOT / candidate,
    ]
    for choice in choices:
        try:
            resolved = choice.resolve()
            resolved.relative_to(run_dir.resolve())
        except (OSError, ValueError):
            continue
        if resolved.is_file():
            return resolved
    return None


def _integration_artifact_file(run_dir: Path, identity: dict) -> Path | None:
    raw = identity.get("path")
    if not isinstance(raw, str) or not raw.strip():
        return None
    name = Path(raw).name
    if name in ("", ".", ".."):
        return None
    candidate = _integration_resolve_file(run_dir, str(run_dir / "artifacts" / name))
    if candidate is not None:
        return candidate
    return _integration_resolve_file(run_dir, raw)


def _raster_has_app_color(raster: bytes, rect: tuple[int, int, int, int],
                          width: int, height: int) -> bool:
    """Independent equivalent of the production checker-color scan."""
    if len(raster) != width * height * 3:
        return False
    x, y, w, h = rect
    colors = ((20, 40, 90), (60, 40, 20))
    for row in range(y, y + h):
        offset = (row * width + x) * 3
        for col in range(w):
            index = offset + col * 3
            pixel = raster[index:index + 3]
            if any(all(abs(pixel[channel] - color[channel]) <= 8
                       for channel in range(3)) for color in colors):
                return True
    return False


def check_integration_frames(root: Path) -> list:
    """Strictly validate real-grant records, identities, geometry and captures.

    This is the qualification packager's independent check. It intentionally
    ignores human-readable assertion prose: the machine-readable visual
    assertion, all three bands, the placed geometry, the identified surface,
    the actual QEMU raster and its recorded digests are the contract.
    """
    problems = []
    placement_pattern = re.compile(
        r"\[desktop\] ordinary window placed handle=(\d+) surface=(\d+) "
        r"x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)"
    )
    for evidence_path in sorted(root.rglob("integration-evidence.json")):
        run_dir = evidence_path.parent.resolve()
        rel_dir = str(evidence_path.parent.relative_to(root)) or "."

        def problem(message: str) -> None:
            problems.append(f"{rel_dir}: {message}")

        try:
            payload = json.loads(evidence_path.read_text())
        except (OSError, ValueError) as exc:
            problem(f"integration-evidence.json unreadable: {exc}")
            continue
        if not isinstance(payload, dict):
            problem("integration-evidence.json root is not an object")
            continue
        if payload.get("kind") != "real_shared_region_layout_integration":
            problem("evidence kind is not real SharedRegion integration")
        if (payload.get("pass") is not True
                or type(payload.get("rc")) is not int
                or payload.get("rc") != 0):
            problem("qualification integration run does not report PASS/exit 0")
        compiler = payload.get("guest_compiler_under_test")
        if compiler not in ("gcc", "clang"):
            problem("guest compiler identity is missing or invalid")
        source_commit = payload.get("source_commit")
        if not isinstance(source_commit, str) or re.fullmatch(r"[0-9a-fA-F]{40}", source_commit) is None:
            problem("source commit identity is not a full 40-hex commit")

        identities = payload.get("identities")
        if not isinstance(identities, dict):
            problem("candidate identities are missing")
            identities = {}
        if identities.get("source_commit") != source_commit:
            problem("nested source commit identity does not match the run")

        def check_identity(identity: object, name: str) -> dict:
            if not isinstance(identity, dict):
                problem(f"{name} identity is missing")
                return {}
            if not isinstance(identity.get("path"), str) or not identity["path"].strip():
                problem(f"{name} path identity is missing")
            if type(identity.get("size")) is not int or identity["size"] <= 0:
                problem(f"{name} size identity is invalid")
            if not _is_sha256(identity.get("sha256")):
                problem(f"{name} SHA-256 identity is invalid")
            return identity

        boot_efi = check_identity(identities.get("boot_efi"), "boot EFI")
        esp = check_identity(identities.get("esp"), "ESP")
        extracted_efi = check_identity(identities.get("extracted_efi"), "extracted EFI")
        if identities.get("efi_embedded_matches_build") is not True \
                or boot_efi.get("sha256") != extracted_efi.get("sha256") \
                or boot_efi.get("size") != extracted_efi.get("size"):
            problem("extracted EFI is not byte-identical to the built EFI")
        efi_copy = _integration_resolve_file(run_dir, "artifacts/BOOTX64.EFI")
        if efi_copy is None:
            problem("captured artifacts/BOOTX64.EFI is missing or escapes its run")
        elif (sha256_file(efi_copy) != extracted_efi.get("sha256")
              or efi_copy.stat().st_size != extracted_efi.get("size")):
            problem("captured artifacts/BOOTX64.EFI does not match extracted EFI identity")

        protected = (
            ("protected_phase14_pie",
             "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"),
            ("protected_signed_apb1",
             "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa"),
        )
        for name, expected in protected:
            record = check_identity(identities.get(name), name)
            if record.get("sha256") != expected:
                problem(f"protected artifact {name} identity changed")

        qemu = identities.get("qemu")
        if not isinstance(qemu, dict):
            qemu = {}
            problem("QEMU identity chain is missing")
        if qemu.get("identity_ok") is not True or qemu.get("elf_ok") is not True:
            problem("QEMU identity chain is incomplete")
        for key in ("wrapper_path", "elf_path", "loader_path"):
            if not isinstance(qemu.get(key), str) or not qemu[key].strip():
                problem(f"QEMU {key} identity is missing")
        for key in ("wrapper_sha256", "elf_sha256", "loader_sha256"):
            if not _is_sha256(qemu.get(key)):
                problem(f"QEMU {key} is not a valid SHA-256")
        firmware = qemu.get("firmware")
        if not isinstance(firmware, dict):
            firmware = {}
            problem("QEMU firmware identities are missing")
        for key in ("code_path", "vars_path"):
            if not isinstance(firmware.get(key), str) or not firmware[key].strip():
                problem(f"QEMU firmware {key} identity is missing")
        for key in ("code_sha256", "vars_sha256"):
            if not _is_sha256(firmware.get(key)):
                problem(f"QEMU firmware {key} is not a valid SHA-256")
        runtime_libs = qemu.get("runtime_libs")
        if not isinstance(runtime_libs, list) or not any(
                isinstance(lib, dict)
                and str(lib.get("path", "")).endswith("/libc.so.6")
                and _is_sha256(lib.get("sha256"))
                for lib in runtime_libs):
            problem("isolated QEMU libc identity is missing")

        for compiler_source, where, selected_required in (
                (identities.get("compilers"), "nested", False),
                (payload.get("compiler_identities"), "run", True)):
            if not isinstance(compiler_source, dict):
                problem(f"{where} compiler identities are missing")
                continue
            for key in ("gcc", "clang_via_zig", "zig", "rustc"):
                value = compiler_source.get(key)
                if not isinstance(value, str) or not value.strip() or "MISSING" in value:
                    problem(f"{where} compiler identity {key} is missing")
            selected_compiler = compiler_source.get("guest_compiler_under_test")
            if selected_required and selected_compiler not in ("gcc", "clang"):
                problem(f"{where} selected guest compiler identity is missing")
            elif selected_compiler is not None and selected_compiler != compiler:
                problem(f"{where} guest compiler identity does not match the run")

        contract = payload.get("fixture_contract")
        if not isinstance(contract, dict):
            contract = {}
            problem("fixture contract is missing")
        contract_page_fields = ("required_pages", "pixel_offset_bytes", "pixel_bytes")
        contract_expected = (301, 4096, 640 * 480 * 4)
        if any(type(contract.get(field)) is not int for field in contract_page_fields) \
                or tuple(contract.get(field) for field in contract_page_fields) != contract_expected:
            problem("fixture page/layout contract is inconsistent")
        if (type(contract.get("flags")) is not int or contract.get("flags") != 25
                or type(contract.get("requested_capabilities")) is not int
                or contract.get("requested_capabilities") != 0):
            problem("fixture flags or requested capabilities changed")
        if contract.get("new_kernel_syscalls") != [] \
                or contract.get("added_child_capabilities") != []:
            problem("fixture claims additional kernel syscalls or child capabilities")

        artifacts = payload.get("artifacts")
        fixtures = payload.get("fixtures")
        if not isinstance(artifacts, dict):
            artifacts = {}
            problem("per-page artifact identities are missing")
        if not isinstance(fixtures, dict):
            fixtures = {}
            problem("per-page integration records are missing")

        decoded: dict[tuple[int, str], tuple[int, int, bytes]] = {}
        for pages in (301, 300):
            key = str(pages)
            fixture = fixtures.get(key)
            artifact = artifacts.get(key)
            if not isinstance(fixture, dict):
                problem(f"{pages}-page fixture record is missing")
                continue
            if not isinstance(artifact, dict):
                problem(f"{pages}-page artifact identity is missing")
                artifact = {}
            if artifact.get("application_id") != f"com.arena.layout{pages}":
                problem(f"{pages}-page artifact application identity is invalid")
            if (type(artifact.get("manifest_flags")) is not int
                    or artifact.get("manifest_flags") != 25
                    or type(artifact.get("requested_capabilities")) is not int
                    or artifact.get("requested_capabilities") != 0):
                problem(f"{pages}-page artifact permissions/flags changed")
            if fixture.get("pass") is not True:
                problem(f"{pages}-page fixture does not report PASS")
            page_fields = ("expected_pages", "allocated_pages",
                           "client_reported_pages", "retired_pages")
            if any(type(fixture.get(field)) is not int for field in page_fields) \
                    or any(fixture.get(field) != pages for field in page_fields):
                problem(f"{pages}-page case is not bound to an exact real grant")
            if fixture.get("compiler") != compiler:
                problem(f"{pages}-page fixture compiler identity is inconsistent")
            if fixture.get("application_id") != f"com.arena.layout{pages}":
                problem(f"{pages}-page fixture application identity is invalid")
            if (type(fixture.get("manifest_flags")) is not int
                    or fixture.get("manifest_flags") != 25
                    or type(fixture.get("requested_capabilities")) is not int
                    or fixture.get("requested_capabilities") != 0):
                problem(f"{pages}-page fixture permissions/flags changed")

            artifact_elf = check_identity(artifact.get("elf"), f"{pages}-page ELF")
            artifact_apb1 = check_identity(artifact.get("apb1"), f"{pages}-page APB1")
            for name, identity in (("ELF", artifact_elf), ("APB1", artifact_apb1)):
                file_path = _integration_artifact_file(run_dir, identity)
                if file_path is None:
                    problem(f"{pages}-page {name} capture is missing from artifacts/")
                elif (sha256_file(file_path) != identity.get("sha256")
                      or file_path.stat().st_size != identity.get("size")):
                    problem(f"{pages}-page {name} capture does not match its identity")
            if artifact_apb1.get("sha256") != fixture.get("apb1_sha256"):
                problem(f"{pages}-page APB1 digest differs between fixture and artifact")
            if artifact_elf.get("sha256") != fixture.get("elf_sha256"):
                problem(f"{pages}-page ELF digest differs between fixture and artifact")

            frames = fixture.get("frames")
            if not isinstance(frames, dict):
                problem(f"{pages}-page frame map is missing")
                frames = {}
            required = {"prelaunch", "launcher-selection", "after-cleanup"}
            if pages == 301:
                required.add("rendered")
            missing = sorted(required - set(frames))
            if missing:
                problem(f"{pages}-page run omits screenshots {', '.join(missing)}")
            for name, frame in frames.items():
                if not isinstance(frame, dict):
                    problem(f"{pages}-page frame {name} has no identity")
                    continue
                raw = frame.get("png")
                png = _integration_resolve_file(run_dir, raw)
                if png is None:
                    problem(f"{pages}-page frame {name} is missing or escapes its run directory")
                    continue
                if not _is_sha256(frame.get("png_sha256")):
                    problem(f"{pages}-page frame {name} PNG SHA-256 is invalid")
                    continue
                data = png.read_bytes()
                if sha256_bytes(data) != frame.get("png_sha256"):
                    problem(f"{pages}-page frame {name} PNG hash changed")
                    continue
                if not _is_sha256(frame.get("raster_sha256")):
                    problem(f"{pages}-page frame {name} raster SHA-256 is invalid")
                try:
                    width, height, raster = png_raster(data)
                except (ValueError, OSError, IndexError, TypeError) as exc:
                    problem(f"{pages}-page frame {name} cannot decode: {exc}")
                    continue
                if sha256_bytes(raster) != frame.get("raster_sha256"):
                    problem(f"{pages}-page frame {name} raster hash changed")
                if (width, height) != (800, 600) \
                        or frame.get("width") != width \
                        or frame.get("height") != height:
                    problem(f"{pages}-page frame {name} has invalid dimensions")
                decoded[(pages, name)] = (width, height, raster)

            serial_entry = fixture.get("serial")
            if not isinstance(serial_entry, dict):
                problem(f"{pages}-page serial identity is missing")
                serial_text = ""
            else:
                serial_path = _integration_resolve_file(run_dir, serial_entry.get("path"))
                if serial_path is None:
                    problem(f"{pages}-page serial log is missing or outside the run")
                    serial_text = ""
                elif not _is_sha256(serial_entry.get("sha256")) \
                        or sha256_file(serial_path) != serial_entry.get("sha256"):
                    problem(f"{pages}-page serial log does not match its identity")
                    serial_text = serial_path.read_text(errors="replace")
                else:
                    serial_text = serial_path.read_text(errors="replace")

            if pages == 301:
                surface = fixture.get("surface_object_id")
                identify_surface = fixture.get("identify_surface_object_id")
                if (type(surface) is not int or surface <= 0
                        or type(identify_surface) is not int
                        or identify_surface != surface):
                    problem("301-page Identify identity is missing or mismatched")
                placement = fixture.get("placement")
                if not isinstance(placement, dict):
                    problem("301-page placement geometry is missing")
                    placement = {}
                rect = placement.get("rect")
                rect_ok = (isinstance(rect, (list, tuple)) and len(rect) == 4
                           and all(type(value) is int for value in rect))
                if rect_ok:
                    x, y, width, height = rect
                    rect_ok = (x >= 0 and y >= 0 and width > 0 and height > 28
                               and x + width <= 800 and y + height <= 600)
                if not rect_ok:
                    problem("301-page placement rectangle is invalid or off-screen")
                if type(placement.get("handle")) is not int or placement["handle"] <= 0:
                    problem("301-page placement handle identity is invalid")
                if type(placement.get("line")) is not int or placement["line"] < 0:
                    problem("301-page placement serial line identity is invalid")
                if (type(placement.get("surface")) is not int
                        or placement.get("surface") != surface):
                    problem("301-page placed surface does not match Identify")
                if (fixture.get("window_created") is not True
                        or type(fixture.get("client_exit_status")) is not int
                        or fixture.get("client_exit_status") != 0):
                    problem("301-page successful window/exit assertion is missing")
                if (type(fixture.get("pixel_words")) is not int
                        or fixture.get("pixel_words") != 307200
                        or type(fixture.get("pixel_nonzero_words")) is not int
                        or fixture.get("pixel_nonzero_words") != 307200):
                    problem("301-page rendered pixel receipt is incomplete")

                bands = fixture.get("rendered_visual_bands")
                if fixture.get("rendered_visual_assertion") is not True:
                    problem("301-page rendered visual assertion is missing")
                if not isinstance(bands, dict) or any(
                        bands.get(name) is not True
                        for name in ("top", "middle", "bottom")):
                    problem("301-page top/middle/bottom visual-band assertions are incomplete")

                allocation_matches = re.findall(
                    r"\[desktop\] layout fixture SharedRegion allocated "
                    r"app=com\.arena\.layout301 pages=(\d+) surface=(\d+)",
                    serial_text)
                if len(allocation_matches) != 1 \
                        or allocation_matches[0][0] != "301" \
                        or type(surface) is not int \
                        or int(allocation_matches[0][1]) != surface:
                    problem("301-page serial does not bind its grant to the identified surface")
                placement_matches = []
                for line_no, line in enumerate(serial_text.splitlines()):
                    match = placement_pattern.search(line)
                    if match and type(surface) is int and int(match.group(2)) == surface:
                        placement_matches.append((line_no, match.groups()))
                if len(placement_matches) != 1:
                    problem("301-page serial does not contain one exact placement receipt")
                elif rect_ok:
                    line_no, values = placement_matches[0]
                    handle, serial_surface, px, py, pw, ph = map(int, values)
                    if (line_no != placement.get("line")
                            or handle != placement.get("handle")
                            or serial_surface != surface
                            or [px, py, pw, ph] != list(rect)):
                        problem("301-page placement geometry/identity differs from serial")

                frame = frames.get("rendered")
                visual_frame = decoded.get((301, "rendered"))
                if isinstance(frame, dict) and visual_frame is not None and rect_ok:
                    _width, _height, raster = visual_frame
                    x, y, width, height = rect
                    content_y = y + 28
                    content_height = height - 28
                    band_height = content_height // 3
                    visual_bands = {
                        "top": (x, content_y, width, band_height),
                        "middle": (x, content_y + band_height, width, band_height),
                        "bottom": (x, content_y + 2 * band_height, width,
                                   content_height - 2 * band_height),
                    }
                    for band_name, band_rect in visual_bands.items():
                        if not _raster_has_app_color(raster, band_rect, 800, 600):
                            problem(f"301-page rendered capture has no checker color in {band_name} band")
            else:
                if (fixture.get("identify_surface_object_id") is not None
                        or fixture.get("window_created") is not False
                        or type(fixture.get("client_exit_status")) is not int
                        or fixture.get("client_exit_status") != 7):
                    problem("300-page refusal must remain pre-Identify/no-window with exit 7")
                if (fixture.get("no_map_write_window_damage_before_refusal") is not True
                        or type(fixture.get("pixel_words")) is not int
                        or fixture.get("pixel_words") != 306176
                        or type(fixture.get("pixel_nonzero_words")) is not int
                        or fixture.get("pixel_nonzero_words") != 0):
                    problem("300-page refusal/write assertions are incomplete")

    return problems


def collect(root: Path) -> tuple:
    files = {}
    excluded = 0
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        rel = str(path.relative_to(root))
        if rel in ("MANIFEST.json", "README.md"):
            continue  # generated below, and named in their own manifest
        data = path.read_bytes()
        if rel.endswith(".ppm"):
            # A PPM screendump is a fixed 15-byte header followed by exactly
            # the raster of the published PNG, so it adds no evidence a reader
            # cannot already rebuild from the PNG plus the recorded digests --
            # it only makes the archive several megabytes larger per frame.
            excluded += 1
            continue
        files[rel] = {"sha256": sha256_bytes(data), "size": len(data)}
    return files, excluded


VERIFY_SNIPPET = """import hashlib, json, pathlib
root = pathlib.Path("%(root)s")
m = json.loads((root / "MANIFEST.json").read_text())
bad = []
for rel, want in m["files"].items():
    p = root / rel
    if not p.is_file():
        bad.append((rel, "missing"))
        continue
    got = hashlib.sha256(p.read_bytes()).hexdigest()
    if got != want["sha256"] or p.stat().st_size != want["size"]:
        bad.append((rel, got + " != " + want["sha256"]))
print("files:", len(m["files"]), "mismatches:", len(bad))
for b in bad:
    print(" ", b)
raise SystemExit(1 if bad else 0)
"""


def readme(root: Path, manifest: dict) -> str:
    files = manifest["files"]
    serials = [f for f in files if f.endswith(".log")]
    shots = [f for f in files if f.endswith((".ppm", ".png"))]
    snippet = VERIFY_SNIPPET % {"root": root.name}
    excluded = manifest.get("excluded_from_package", {}).get("count", 0)
    return f"""# {root.name} — V1 evidence package

Produced {manifest["created_utc"]} from commit
`{manifest["source_commit"]}` on branch `{manifest["branch"]}`
(working tree dirty: `{manifest["working_tree_dirty"]}`).

## What is here

This package records evidence and classifications; it does not declare V1
accepted or qualified. Read the current aggregate and status note before using
any result.

| Kind | Count | Notes |
|---|---|---|
| Serial logs | {len(serials)} | The guest's own output under QEMU TCG |
| Screenshots | {len(shots)} | The exact frames each checkpoint assertion was made on (PNG, losslessly re-encoded from the QEMU screendump raster) |
| Manifests and receipts | {len(files) - len(serials) - len(shots)} | JSON verdicts, identities, build receipts |

## How to check it

Extract the archive anywhere and verify every recorded digest from a fresh
directory (nothing is compared against the original workspace):

```sh
tar xzf {root.name}.tar.gz
cd {root.name} && python3 - <<'PY'
{snippet}PY
```

`tools/v1_evidence.py verify <archive>` performs exactly this check, plus a
scan for files that exist in the archive but not in the manifest.

`package` refuses to run unless every guest run in this directory publishes
all nine checkpoint images, each PNG still decoding to the raster digest the
harness recorded when that checkpoint passed. Real-grant integration PNGs are
independently decoded and checked against their recorded QEMU raster digest.
The 301-page rendered frame must carry a true visual assertion, all three
(top/middle/bottom) band assertions, valid on-screen geometry bound to the
identified surface and serial placement receipt, and actual checker pixels in
each band. Human-readable assertion prose is not part of the contract.

PPM working copies are not shipped ({excluded} of them): a PPM is a fixed
header plus the identical raster, so it is reproducible from the published
PNG, and each tested PPM's own SHA-256 is recorded in its run's
`evidence.json`.

## How the verdicts were produced

- `guest-gcc/`, `guest-clang/`: `tools/v1_guest_proof.py` — eight
  checkpoints; `evidence.json` is the run's own verdict, `harness-output.log`
  is the transcript, `v1-guest-serial.log` is the guest serial, `shots/` are
  the frames, `qemu-stderr.log` is the emulator's stderr.
- `phase15/`: `tools/test_m15_guest.py` — both final-ESP CPU profiles, with
  serial logs and QEMU stderr tied to the exact EFI/ESP identities in the gate
  receipts.
- `layout-integration-gcc/`, `layout-integration-clang/`:
  `tools/v1_layout_integration.py` — each cell records real Desktop grants of
  301 and 300 pages, the ordinary C client's map/Identify/window/write/damage
  order or pre-map refusal, retirement pixel scans, cleanup settlement, and
  QEMU screenshots tied to the recorded EFI/ESP, compiler, ELF, APB1, and
  emulator identities. These runs are distinct from the predicate-only
  `refusals-*` matrix.
- `lifecycle-cycles/`: `tools/v1_cycles.py` — ten launch/close cycles, each
  with its own cursor, instance-specific retirement and exit receipts,
  settlement against the pre-cycle baseline, and visual removal.
- `gate/`: the command transcript, stage receipts, and relevant host logs
  included before packaging; the `package`/`verify` receipts and final aggregate
  live in the sibling `*-packaging/` directory to avoid recursive self-reference.
- `env/`: environment recovery receipts (emulator identity, loader, TCG
  acceleration, toolchain versions, protected guards).

A missing serial log, a missing harness transcript, an absent identity or a
missing required artifact makes the run FAIL — see `docs/v1/V2-STATUS.md` §6.
"""


def package(root: Path) -> int:
    root = root.resolve()
    if not root.is_dir():
        print(f"[evidence] {root} is not a directory", file=sys.stderr)
        return 2
    frame_problems = check_frames(root) + check_integration_frames(root)
    if frame_problems:
        print("[evidence] refusing to package: checkpoint frames do not match "
              "their recorded identity")
        for problem in frame_problems:
            print(f"[evidence] PROBLEM: {problem}")
        return 1
    files, excluded_ppm = collect(root)
    # A single multi-megabyte blob in an evidence directory is almost always
    # scratch state (a formatted disk image, a copied boot image), and it
    # bloats the published package without giving a reader anything to check
    # that a recorded SHA-256 does not already give. Refuse rather than
    # quietly publish it.
    oversized = [rel for rel, meta in files.items() if meta["size"] > 32 * 1024 * 1024]
    if oversized:
        print("[evidence] refusing oversized artifacts (scratch, not evidence):")
        for rel in oversized:
            print(f"  {rel}: {files[rel]['size']} bytes")
        return 1
    manifest = {
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "source_commit": _git("rev-parse", "HEAD"),
        "branch": _git("rev-parse", "--abbrev-ref", "HEAD"),
        "working_tree_dirty": bool(_git("status", "--porcelain")),
        "files": files,
        "excluded_from_package": {
            "count": excluded_ppm,
            "pattern": "*.ppm",
            "reason": "PPM screendumps are a fixed header plus the exact "
                      "raster of the published PNG; their SHA-256 is "
                      "recorded in each run's evidence.json",
        },
        "checkpoint_frames": {
            "required": list(REQUIRED_GUEST_FRAMES),
            "checked": "each published PNG re-decoded and compared with the "
                       "raster digest the harness recorded when the checkpoint "
                       "assertion passed; integration PNGs are also decoded",
            "runs": sum(1 for _ in root.rglob("evidence.json")),
            "integration_runs": sum(1 for _ in root.rglob("integration-evidence.json")),
        },
        "total_bytes": sum(f["size"] for f in files.values()),
    }
    (root / "MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (root / "README.md").write_text(readme(root, manifest))
    archive = root.with_suffix(".tar.gz")
    # Archive exactly what the manifest lists (plus the manifest and README).
    # Adding the directory wholesale would ship the PPM working copies, which
    # `collect` deliberately leaves out -- and an unlisted file in the archive
    # is precisely what `verify` exists to reject.
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(root, arcname=root.name, recursive=False)
        for rel in sorted(files) + ["MANIFEST.json", "README.md"]:
            tar.add(root / rel, arcname=f"{root.name}/{rel}", recursive=False)
    print(f"[evidence] packaged {len(files)} files, "
          f"{manifest['total_bytes']} bytes -> {archive} "
          f"({archive.stat().st_size} bytes, sha256 {sha256_file(archive)})")
    sidecar = archive.parent / (archive.name + ".sha256")
    sidecar.write_text(f"{sha256_file(archive)}  {archive.name}\n")
    return 0


def verify(archive: Path) -> int:
    archive = archive.resolve()
    if not archive.is_file():
        print(f"[evidence] {archive} not found", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory(prefix="v1-evidence-verify-") as tmp:
        with tarfile.open(archive, "r:gz") as tar:
            # Reject absolute paths and ".." traversal before extracting;
            # `filter="data"` is not available on every Python version the
            # project supports, so the check is explicit.
            for member in tar.getmembers():
                name = member.name
                if name.startswith("/") or ".." in Path(name).parts:
                    print(f"[evidence] refusing unsafe member: {name}")
                    return 1
            try:
                tar.extractall(tmp, filter="data")
            except TypeError:
                tar.extractall(tmp)
        roots = [p for p in Path(tmp).iterdir() if p.is_dir()]
        if len(roots) != 1:
            print(f"[evidence] expected one root directory, found {len(roots)}")
            return 1
        root = roots[0]
        manifest_path = root / "MANIFEST.json"
        if not manifest_path.is_file():
            print("[evidence] MANIFEST.json missing from archive")
            return 1
        manifest = json.loads(manifest_path.read_text())
        problems = []
        for rel, want in manifest["files"].items():
            path = root / rel
            if not path.is_file():
                problems.append(f"{rel}: missing")
                continue
            data = path.read_bytes()
            if len(data) != want["size"]:
                problems.append(f"{rel}: size {len(data)} != {want['size']}")
            elif sha256_bytes(data) != want["sha256"]:
                problems.append(f"{rel}: sha256 mismatch")
        listed = set(manifest["files"])
        for path in root.rglob("*"):
            if not path.is_file():
                continue
            rel = str(path.relative_to(root))
            if rel in ("MANIFEST.json", "README.md"):
                continue
            if rel not in listed:
                problems.append(f"{rel}: present in archive, absent from manifest")
        print(f"[evidence] extracted to a fresh directory: {root}")
        print(f"[evidence] files checked: {len(manifest['files'])}, "
              f"problems: {len(problems)}")
        for p in problems:
            print(f"[evidence] PROBLEM: {p}")
        return 0 if not problems else 1


def seal(target: Path) -> int:
    """Package, verify from a fresh extraction, then write an EXTERNAL receipt.

    The archive cannot contain a receipt that describes itself: inserting one
    changes the archive, and the digest inside would then describe the
    previous generation. Round 2 shipped exactly that ambiguity. `seal`
    therefore writes `<archive>.receipt.txt` *next to* the archive, after the
    archive is final and verified, and never inside it.
    """
    rc = package(target)
    if rc != 0:
        return rc
    archive = target.with_suffix(".tar.gz")
    rc = verify(archive)
    if rc != 0:
        print("[evidence] seal refused: the package does not verify")
        return rc
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    manifest = json.loads((target / "MANIFEST.json").read_text())
    identities = {}
    gate = target / "gate" / "receipts" / "build.json"
    if gate.is_file():
        receipt = json.loads(gate.read_text())
        identities = {k: receipt.get(k) for k in
                      ("source_commit", "code_fingerprint", "efi_sha256",
                       "esp_sha256")}
    lines = [
        "ARENA V1 EVIDENCE — EXTERNAL ARCHIVE RECEIPT",
        "This file is written beside the archive, never inside it, so it",
        "describes the final bytes rather than an earlier generation.",
        "",
        f"archive            : {archive.name}",
        f"archive bytes      : {archive.stat().st_size}",
        f"archive sha256     : {digest}",
        f"verified files     : {len(manifest['files'])}",
        "verification       : python3 tools/v1_evidence.py verify "
        f"{archive}",
        "verification result: PASS — extracted to a fresh temporary "
        "directory, every digest re-checked, no unlisted file",
        "",
        "tested candidates (from the gate build receipt):",
    ]
    for k, v in identities.items():
        lines.append(f"  {k:<18}: {v}")
    lines += [
        "",
        f"sealed             : {time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}",
        "command            : python3 tools/v1_evidence.py seal "
        f"{target}",
    ]
    out = archive.with_suffix(archive.suffix + ".receipt.txt")
    out.write_text("\n".join(lines) + "\n")
    print(f"[evidence] external receipt -> {out}\n   {digest}  {len(manifest['files'])} files")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in ("package", "verify", "seal"):
        print(__doc__, file=sys.stderr)
        return 2
    target = Path(argv[2])
    if argv[1] == "seal":
        return seal(target)
    return package(target) if argv[1] == "package" else verify(target)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
