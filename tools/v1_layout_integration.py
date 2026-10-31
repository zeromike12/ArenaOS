#!/usr/bin/env python3
"""Real-grant 301/300-page integration proof for the ordinary C client.

The signed-layout fixture IDs are handled by the trusted Desktop's existing
POOL-backed SharedRegion allocation. No capability, syscall, or launch
authority is added: each app is installed by the guest's protected installer,
launched through the normal verified catalog path, and receives the same
manifest flags and delegated capabilities as the regular C graphics client.

V1_CC=gcc|clang V1_EVIDENCE_DIR=docs/v1/evidence/<run>/layout-integration-gcc \
    python3 tools/v1_layout_integration.py
"""

import hashlib
import json
import os
import re
import shutil
import sys
import time
import traceback
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import afs1  # noqa: E402
import apb1_format  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
import v1_guest_proof as proof  # noqa: E402

LABEL = "v1-layout-integration"
OUT = Path(os.environ.get("V1_EVIDENCE_DIR") or
           Path(mtest.arena_env.build_dir()) / "v1-layout-integration-run")
ARTIFACTS = OUT / "artifacts"
SHOTS = OUT / "shots"
EVIDENCE = OUT / "integration-evidence.json"
FLAGS = 1 | 8 | 16  # MULTI_INSTANCE | STANDARD_STREAMS | NATIVE_SYNC
REQUIRED_PAGES = 301
PIXEL_WORDS = 640 * 480
PAGE_BYTES = 4096

RE_ALLOCATED = re.compile(
    r"\[desktop\] layout fixture SharedRegion allocated app="
    r"com\.arena\.layout(?P<pages>300|301) pages=(?P<got>\d+) "
    r"surface=(?P<surface>\d+)"
)
RE_PRECREATE = re.compile(
    r"\[desktop\] layout fixture console precreation suppressed "
    r"app=com\.arena\.layout(?P<pages>300|301)"
)
RE_CLIENT_PAGES = re.compile(
    r"\[gfx\] surface pages=(?P<got>-?\d+) required=(?P<required>\d+)"
)
RE_PIXELS = re.compile(
    r"\[desktop\] layout fixture pixels surface=(?P<surface>\d+) "
    r"pages=(?P<pages>\d+) words=(?P<words>\d+) "
    r"nonzero=(?P<nonzero>\d+)"
)
RE_IDENTIFY = re.compile(r"\[gfx\] identify ok surface=(?P<surface>\d+)")
RE_GFX_CREATE = re.compile(
    r"\[gfx\] window created handle=(?P<handle>\d+) "
    r"surface=(?P<surface>\d+)"
)
RE_CHILD_EXIT_FOR_SURFACE = re.compile(
    r"\[desktop\] child Process-cap exit surface=(?P<surface>\d+) "
    r"status=(?P<status>\d+)"
)
RE_RETIRE_FOR_SURFACE = re.compile(
    r"\[desktop\] application retired: kind=6 surface=(?P<surface>\d+)"
)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_identity(path: Path) -> dict:
    return {
        "path": str(path),
        "size": path.stat().st_size if path.is_file() else None,
        "sha256": proof.sha256_file(path),
    }


def _build_apb1(elf: Path, compiler: str, pages: int) -> tuple[Path, dict]:
    app_id = f"com.arena.layout{pages}".encode("ascii")
    name = f"Layout Boundary {pages}".encode("ascii")
    manifest = apb1_format.make_manifest(
        app_id=app_id,
        package_id=b"org.arena.layout-boundary",
        display_name=name,
        version=1,
        flags=FLAGS,
        requested=0,
        entry=b"bin/cgfxapp",
        icon=b"",
        width=640,
        height=480,
        associations=(),
    )
    bundle = apb1_format.build_bundle(
        [(1, b"bin/cgfxapp", elf.read_bytes())], manifest=manifest)
    parsed = apb1_format.parse_bundle(bundle)
    if [name for _kind, name, _size, _digest in parsed.files] != [b"bin/cgfxapp"]:
        raise RuntimeError("APB1 round-trip changed the graphics payload list")
    out = ARTIFACTS / f"layout-{pages}-{compiler}.apb1"
    out.write_bytes(bundle)
    return out, {
        "application_id": app_id.decode(),
        "package_id": "org.arena.layout-boundary",
        "display_name": name.decode(),
        "version": 1,
        "manifest_flags": FLAGS,
        "requested_capabilities": 0,
        "preferred_window": [640, 480],
        "entry": "bin/cgfxapp",
        "elf": file_identity(elf),
        "apb1": file_identity(out),
    }


def _build_artifacts(compiler: str) -> tuple[dict, dict]:
    sys.path.insert(0, str(REPO_ROOT / "experiments" / "c-runtime"))
    import run as crun  # noqa: E402

    elfs = crun.build_guest(compiler)
    elf = elfs["c-gfx-app"]
    audit = crun.audit_elf(elf)
    if audit.get("loader_failures"):
        raise RuntimeError(f"c-gfx-app loader audit failed: {audit}")
    disasm = crun.disassemble_check(elf)
    if disasm.get("simd_or_x87_count"):
        raise RuntimeError(f"c-gfx-app emits SIMD/x87: {disasm}")
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    stored_elf = ARTIFACTS / f"c-gfx-app-{compiler}.elf"
    shutil.copyfile(elf, stored_elf)
    fixtures = {}
    for pages in (301, 300):
        apb1, identity = _build_apb1(elf, compiler, pages)
        identity["elf"] = file_identity(stored_elf)
        identity["build_elf_path"] = str(elf)
        identity["compiler"] = compiler
        identity["compiler_disassembly"] = disasm
        identity["elf_audit"] = audit
        fixtures[pages] = {"bundle_path": str(apb1), **identity}
    return fixtures, {
        "gcc": proof.compiler_identities()["gcc"],
        "clang_via_zig": proof.compiler_identities()["clang_via_zig"],
        "zig": proof.compiler_identities()["zig"],
        "rustc": proof.compiler_identities()["rustc"],
        "guest_compiler_under_test": compiler,
    }


def _new_disk(run_dir: Path) -> Path:
    run_dir.mkdir(parents=True, exist_ok=True)
    disk = run_dir / "arena-disk.img"
    tmp8 = run_dir / "arena-disk-8m.img"
    tmp8.write_bytes(b"\0" * (8 * 1024 * 1024))
    afs1.mkfs(tmp8, 8 * 1024 * 1024 // afs1.SECTOR)
    with disk.open("wb") as stream:
        stream.write(tmp8.read_bytes())
        stream.write(b"\0" * (72 * 1024 * 1024 - 8 * 1024 * 1024))
    tmp8.unlink()
    return disk


def _save_frame(desktop: proof.V1Desktop, name: str) -> dict:
    raster = desktop.shot(name)
    png = proof.save_frame(name, raster)
    return {
        "png": str(png.relative_to(OUT)),
        "png_sha256": proof.sha256_file(png),
        "raster_sha256": sha256_bytes(raster),
        "width": proof.SCREEN_W,
        "height": proof.SCREEN_H,
    }


def _last_match(pattern: re.Pattern, serial: str):
    matches = list(pattern.finditer(serial))
    return matches[-1].groupdict() if matches else None


def _line_index(serial: str, marker: str, start: int = 0) -> int | None:
    for index, line in enumerate(serial.splitlines()):
        if index >= start and marker in line:
            return index
    return None


def _scenario(compiler: str, pages: int, fixture: dict) -> dict:
    label = f"layout-{compiler}-{pages}"
    run_dir = OUT / label
    disk = _new_disk(run_dir)
    bundle = Path(fixture["bundle_path"])
    record = {
        "label": label,
        "compiler": compiler,
        "expected_pages": pages,
        "application_id": fixture["application_id"],
        "package_id": fixture["package_id"],
        "manifest_flags": fixture["manifest_flags"],
        "requested_capabilities": fixture["requested_capabilities"],
        "apb1_sha256": fixture["apb1"]["sha256"],
        "elf_sha256": fixture["elf"]["sha256"],
        "authority_contract": {
            "install": "guest protected APB1 installer; signed-version receipt required",
            "launch": "verified installed catalog -> packaged fresh Image -> ordinary ABI-v2 launch",
            "child_flags": fixture["manifest_flags"],
            "added_application_capabilities": [],
            "identify_required_on_success": True,
            "stream_precreate_suppressed_for_boundary_ordering": True,
        },
        "frames": {},
    }

    def run() -> bytes:
        # mtest.boot invokes callable feed payloads with no arguments. Connect
        # to this boot's real QMP socket here, after the readiness markers have
        # fired, rather than declaring an argument the feeder cannot supply.
        desktop = proof.V1Desktop(label)
        try:
            desktop.wait(
                lambda: "[desktop] real desktop frame presented" in desktop.serial()
                and "filesd: AFS2 mounted" in desktop.serial()
                and "arena>" in desktop.serial(),
                "Desktop, AFS2, and command prompt ready",
                90,
            )
            initial = proof.settle_baseline(desktop, timeout_s=30)
            before_detail = initial["detail"]
            before_receipt = initial["receipt"]
            if before_detail is None or before_receipt is None:
                raise AssertionError("no settled prelaunch resource baseline")
            if before_detail["windows"] != 0 or before_detail["app-instances"] != 0:
                raise AssertionError(f"prelaunch Desktop is not clean: {before_detail}")
            record["before"] = {
                "detail": before_detail,
                "receipt": before_receipt,
            }
            record["frames"]["prelaunch"] = _save_frame(
                desktop, f"{label}-prelaunch")

            install_cursor = desktop.cursor()
            desktop.click(52, 58)
            time.sleep(0.1)
            desktop.click(52, 58)
            desktop.click(780, 560)
            if not desktop.wait_serial(
                    proof.RE_INSTALL_ACK.pattern,
                    "protected signed APB1 install receipt", 60,
                    start=install_cursor):
                raise AssertionError("protected installer did not sign/install the APB1")
            install_lines = [
                line for line in desktop.serial().splitlines()
                if proof.RE_INSTALL_ACK.search(line)
            ]
            record["install_receipt"] = install_lines[-1] if install_lines else None
            record["install_signed_version"] = (
                int(proof.RE_INSTALL_ACK.search(install_lines[-1]).group(1))
                if install_lines else None)

            launch_cursor = desktop.cursor()
            desktop.open_apps()
            time.sleep(0.1)
            desktop.type_text(f"layout {pages}")
            time.sleep(0.5)
            record["frames"]["launcher-selection"] = _save_frame(
                desktop, f"{label}-launcher-selection")
            desktop.key("ret")

            if not desktop.wait_serial(
                    r"\[gfx\] surface pages=-?\d+ required=301",
                    "ordinary C size check page query", 60,
                    start=launch_cursor):
                raise AssertionError("the normal C graphics client did not report its page query")
            if pages == 301:
                # Capture a genuine live QEMU frame before the ordinary
                # 1200-frame client loop exits and releases its resources.
                if not desktop.wait_serial(
                        r"\[gfx\] first damage published",
                        "first production damage after the C page guard and Identify",
                        30, start=launch_cursor):
                    raise AssertionError("301-page app never published its first damage")
                early_serial = desktop.serial()
                early_allocated = _last_match(RE_ALLOCATED, early_serial)
                early_identify = _last_match(RE_IDENTIFY, early_serial)
                early_created = _last_match(RE_GFX_CREATE, early_serial)
                if early_allocated is None or int(early_allocated["got"]) != 301:
                    raise AssertionError(f"301-page Desktop grant is not exact: {early_allocated}")
                if early_identify is None or int(early_identify["surface"]) != int(early_allocated["surface"]):
                    raise AssertionError("mandatory Identify did not bind the real 301-page surface")
                if early_created is None or int(early_created["surface"]) != int(early_allocated["surface"]):
                    raise AssertionError("C client did not create a window on the identified surface")
                placement = proof.placement_for(early_serial, int(early_allocated["surface"]))
                if placement is None:
                    desktop.wait(
                        lambda: proof.placement_for(
                            desktop.serial(), int(early_allocated["surface"])) is not None,
                        "window placement for the identified 301-page surface", 20)
                    early_serial = desktop.serial()
                    placement = proof.placement_for(
                        early_serial, int(early_allocated["surface"]))
                rect = placement["rect"]
                content = (rect[0], rect[1] + proof.TITLE_H,
                           rect[2], rect[3] - proof.TITLE_H)
                rendered = b""
                visual = False
                for _attempt in range(30):
                    rendered = desktop.shot(f"{label}-rendered")
                    visual = proof.has_app_color(rendered, content)
                    if visual:
                        break
                    time.sleep(0.1)
                record["frames"]["rendered"] = {
                    **_save_raster(f"{label}-rendered", rendered),
                    "assertion": "production checkerboard color present in the placed window content",
                }
                record["rendered_visual_assertion"] = visual
                record["placement"] = placement
                if not visual:
                    raise AssertionError("QEMU frame does not show the C client's checkerboard")
                sequence_markers = (
                    "[gfx] surface pages=301 required=301",
                    "[gfx] size-check accepted; map request next",
                    "[gfx] shared-map syscall",
                    "[gfx] surface mapped",
                    "[gfx] identify ok",
                    "[gfx] create-window request after Identify",
                    "[gfx] window created",
                    "[gfx] first frame pixels written=307200",
                    "[gfx] first damage published",
                )
                sequence = [_line_index(early_serial, marker, launch_cursor)
                            for marker in sequence_markers]
                if any(index is None for index in sequence) \
                        or sequence != sorted(sequence):
                    raise AssertionError(f"301 production path order is incomplete or wrong: {sequence}")
                record["first_frame_sequence"] = dict(zip(sequence_markers, sequence))
                if not desktop.wait_serial(
                        rf"\[desktop\] layout fixture pixels surface=\d+ pages=301 ",
                        "Desktop 301-page pixel receipt after process exit", 60,
                        start=launch_cursor):
                    raise AssertionError("Desktop did not report the 301-page pixel receipt")
                if not desktop.wait_serial(
                        rf"\[desktop\] child Process-cap exit surface=\d+ status=0",
                        "ordinary 301-page C app exit status 0", 60,
                        start=launch_cursor):
                    raise AssertionError("301-page client did not exit cleanly")
            else:
                if not desktop.wait_serial(
                        rf"\[desktop\] layout fixture pixels surface=\d+ pages=300 ",
                        "Desktop 300-page pixel receipt after process exit", 30,
                        start=launch_cursor):
                    raise AssertionError("Desktop did not report the 300-page pixel receipt")
                if not desktop.wait_serial(
                        r"\[desktop\] child Process-cap exit surface=\d+ status=7",
                        "ordinary C size-check exit status 7", 30,
                        start=launch_cursor):
                    raise AssertionError("300-page client did not exit with status 7")
            if not desktop.wait_serial(
                    r"\[desktop\] application retired: kind=6 surface=\d+",
                    "fixture session retirement and cleanup", 30,
                    start=launch_cursor):
                raise AssertionError("Desktop did not retire the fixture application session")

            serial_now = desktop.serial()
            allocated = _last_match(RE_ALLOCATED, serial_now)
            suppressed = _last_match(RE_PRECREATE, serial_now)
            client_page_match = _last_match(RE_CLIENT_PAGES, serial_now)
            pixels = _last_match(RE_PIXELS, serial_now)
            identify = _last_match(RE_IDENTIFY, serial_now)
            created = _last_match(RE_GFX_CREATE, serial_now)
            if allocated is None or int(allocated["pages"]) != pages \
                    or int(allocated["got"]) != pages:
                raise AssertionError(f"actual Desktop allocation is not exactly {pages}: {allocated}")
            if suppressed is None or int(suppressed["pages"]) != pages:
                raise AssertionError("Desktop did not suppress console pre-creation for the fixture")
            if client_page_match is None \
                    or int(client_page_match["got"]) != pages \
                    or int(client_page_match["required"]) != REQUIRED_PAGES:
                raise AssertionError(f"C client query mismatch: {client_page_match}")
            if pixels is None or int(pixels["pages"]) != pages \
                    or int(pixels["surface"]) != int(allocated["surface"]):
                raise AssertionError(f"Desktop pixel receipt mismatch: {pixels}")

            if pages == 301:
                if identify is None or int(identify["surface"]) != int(allocated["surface"]):
                    raise AssertionError("mandatory Identify did not bind the real surface")
                if created is None or int(created["surface"]) != int(allocated["surface"]):
                    raise AssertionError("C client did not create a window on the identified surface")
                if not all(marker in serial_now for marker in (
                        "[gfx] size-check accepted; map request next",
                        "[gfx] shared-map syscall",
                        "[gfx] surface mapped",
                        "[gfx] identify ok",
                        "[gfx] create-window request after Identify",
                        "[gfx] first frame pixels written=307200",
                        "[gfx] first damage published")):
                    raise AssertionError("301 path did not complete map/Identify/window/write/damage sequence")
                if int(pixels["words"]) != PIXEL_WORDS \
                        or int(pixels["nonzero"]) != PIXEL_WORDS:
                    raise AssertionError(f"301 raster was not fully written: {pixels}")
                if proof.placement_for(serial_now, int(allocated["surface"])) is None:
                    raise AssertionError("Desktop did not place the identified 301-page window")
            else:
                if identify is not None or created is not None:
                    raise AssertionError("300-page refusal reached Identify or window creation")
                forbidden = (
                    "[gfx] size-check accepted; map request next",
                    "[gfx] shared-map syscall",
                    "[gfx] surface mapped",
                    "[gfx] map failed",
                    "[gfx] identify ok",
                    "[gfx] create-window request after Identify",
                    "[gfx] window created",
                    "[gfx] first frame pixels written",
                    "[gfx] first damage published",
                )
                observed = [marker for marker in forbidden if marker in serial_now]
                if observed:
                    raise AssertionError(f"300-page refusal crossed the pre-map boundary: {observed}")
                if "[gfx] size-check refused before map/window/damage; exit=7" not in serial_now:
                    raise AssertionError("300-page refusal marker is missing")
                query_line = _line_index(
                    serial_now, "[gfx] surface pages=300 required=301", launch_cursor)
                refusal_line = _line_index(
                    serial_now, "[gfx] size-check refused before map/window/damage; exit=7",
                    launch_cursor)
                if query_line is None or refusal_line is None or query_line >= refusal_line:
                    raise AssertionError("300-page refusal did not follow the real client page query")
                record["refusal_sequence"] = {
                    "page_query_line": query_line,
                    "refusal_line": refusal_line,
                    "required_pages": REQUIRED_PAGES,
                }
                expected_words = (pages * PAGE_BYTES - PAGE_BYTES) // 4
                if int(pixels["words"]) != expected_words or int(pixels["nonzero"]) != 0:
                    raise AssertionError(f"300-page pixel backing changed before refusal: {pixels}")
                if proof.placement_for(serial_now, int(allocated["surface"])) is not None:
                    raise AssertionError("Desktop recorded a window for the refused surface")

            child_exit = _last_match(RE_CHILD_EXIT_FOR_SURFACE, serial_now)
            retire = _last_match(RE_RETIRE_FOR_SURFACE, serial_now)
            if child_exit is None or int(child_exit["status"]) != (0 if pages == 301 else 7):
                raise AssertionError(f"wrong child exit status for {pages} pages: {child_exit}")
            if retire is None or int(retire["surface"]) != int(allocated["surface"]):
                raise AssertionError("Desktop did not retire the exact integration surface")

            after_detail, after_receipt = proof.wait_settle_snapshot(
                desktop.serial, start=launch_cursor, timeout=30)
            record["after"] = {"detail": after_detail, "receipt": after_receipt}
            drift, warmup = proof.compare_resources(
                after_detail, after_receipt, before_detail, before_receipt)
            record["resource_drift"] = drift
            record["frame_warmup"] = warmup
            record["settled"] = bool(after_detail is not None
                                     and after_receipt is not None
                                     and drift == {})
            record["no_window_after_cleanup"] = bool(
                after_detail is not None
                and after_detail["windows"] == before_detail["windows"] == 0
                and after_detail["app-instances"] == before_detail["app-instances"] == 0)
            if not record["settled"] or not record["no_window_after_cleanup"]:
                raise AssertionError(
                    f"cleanup did not return to the boot baseline: drift={drift}, "
                    f"detail={after_detail}")

            record["frames"]["after-cleanup"] = _save_frame(
                desktop, f"{label}-after-cleanup")
            record["allocated_pages"] = int(allocated["got"])
            record["client_reported_pages"] = int(client_page_match["got"])
            record["retired_pages"] = int(pixels["pages"])
            record["pixel_words"] = int(pixels["words"])
            record["pixel_nonzero_words"] = int(pixels["nonzero"])
            record["surface_object_id"] = int(allocated["surface"])
            record["client_exit_status"] = int(child_exit["status"])
            record["identify_surface_object_id"] = int(identify["surface"]) if identify else None
            record["window_created"] = created is not None
            record["no_map_write_window_damage_before_refusal"] = (
                pages != 300 or not any(marker in serial_now for marker in (
                    "[gfx] size-check accepted; map request next",
                    "[gfx] shared-map syscall",
                    "[gfx] surface mapped",
                    "[gfx] map failed",
                    "[gfx] identify ok",
                    "[gfx] create-window request after Identify",
                    "[gfx] window created",
                    "[gfx] first frame pixels written",
                    "[gfx] first damage published")))
            record["precreation_suppressed"] = True
            record["launch_cursor"] = launch_cursor
            record["pass"] = True
        except Exception as exc:  # noqa: BLE001 - preserve a failing receipt
            record["pass"] = False
            record["fatal"] = f"{type(exc).__name__}: {exc}"
            traceback.print_exc()
        finally:
            serial_path = run_dir / "serial.log"
            serial_path.write_text(desktop.serial())
            record["serial"] = {
                "path": str(serial_path.relative_to(OUT)),
                "sha256": proof.sha256_file(serial_path),
                "bytes": serial_path.stat().st_size,
            }
            try:
                desktop.q.command("system_powerdown")
            except Exception:
                pass
            finally:
                desktop.q.close()
        return b"shutdown\r"

    ready = ((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
              b"arena>"), 1, run)
    mtest.DEFAULT_KEYS = []
    format_label = f"{label}-format"
    format_rc, format_serial, format_seconds = mtest.boot(
        format_label, proof.ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, lambda: b"shutdown\r")],
        disk, pointer=True, video="gpu", timeout_s=180)
    format_log = run_dir / "format-serial.log"
    format_log.write_text(format_serial)
    record["format_boot"] = {
        "label": format_label,
        "rc": format_rc,
        "seconds": format_seconds,
        "serial_path": str(format_log.relative_to(OUT)),
        "serial_sha256": proof.sha256_file(format_log),
    }
    if format_rc != 0:
        record["pass"] = False
        record["fatal"] = "AFS2 format boot failed"
        disk.unlink(missing_ok=True)
        return record

    proof.stage_apb1_in_afs2(disk, bundle.read_bytes(),
                              filename=f"layout-{pages}.apb1".encode())
    rc, serial, seconds = mtest.boot(
        label, proof.ESP, [ready], disk, pointer=True, video="gpu", timeout_s=480)
    record["qemu"] = {"label": label, "rc": rc, "seconds": seconds}
    serial_path = run_dir / "serial.log"
    serial_path.write_text(serial)
    record["serial"] = {
        "path": str(serial_path.relative_to(OUT)),
        "sha256": proof.sha256_file(serial_path),
        "bytes": serial_path.stat().st_size,
    }
    record["pass"] = bool(record.get("pass") and rc == 0)
    if rc != 0:
        record.setdefault("fatal", f"QEMU run exited {rc}")
    stderr_candidates = (
        arena_env.build_dir() / f"qemu-stderr-{label}.log",
        arena_env.build_dir() / f"qemu-stderr-{format_label}.log",
    )
    for source in stderr_candidates:
        if source.is_file():
            shutil.copyfile(source, run_dir / source.name)
    # The generated disk is only a test input; keep the signed APB1, serial,
    # screenshots, and manifests, not a 72-MiB scratch filesystem image.
    disk.unlink(missing_ok=True)
    return record


def _save_raster(name: str, raster: bytes) -> dict:
    png = proof.save_frame(name, raster)
    return {
        "png": str(png.relative_to(OUT)),
        "png_sha256": proof.sha256_file(png),
        "raster_sha256": sha256_bytes(raster),
        "width": proof.SCREEN_W,
        "height": proof.SCREEN_H,
    }


def _candidate_identities() -> dict:
    build = arena_env.build_dir()
    efi = build / "arena-boot.efi"
    esp = build / "arena-esp.img"
    extracted = OUT / "artifacts" / "BOOTX64.EFI"
    extracted_ok = proof.extract_esp_file(esp, "/EFI/BOOT/BOOTX64.EFI", extracted)
    protected_pie = REPO_ROOT / "userspace/phase14-pie/fixture.elf"
    protected_apb1 = REPO_ROOT / "userspace/phase14-pie/phase14-pie.apb1"
    return {
        "source_commit": proof._source_commit(),
        "working_tree_dirty": proof._working_tree_dirty(),
        "dirty_paths_excluding_evidence": proof._source_dirty_paths(),
        "boot_efi": file_identity(efi),
        "esp": file_identity(esp),
        "extracted_efi": file_identity(extracted) if extracted_ok else {
            "path": str(extracted), "size": None, "sha256": "MISSING"},
        "efi_embedded_matches_build": bool(
            extracted_ok and extracted.read_bytes() == efi.read_bytes()),
        "protected_phase14_pie": file_identity(protected_pie),
        "protected_signed_apb1": file_identity(protected_apb1),
        "qemu": proof.qemu_identity(),
        "compilers": proof.compiler_identities(),
    }


def main() -> int:
    compiler = os.environ.get("V1_CC", "gcc")
    if compiler not in ("gcc", "clang"):
        raise SystemExit(f"V1_CC must be gcc or clang, not {compiler!r}")
    OUT.mkdir(parents=True, exist_ok=True)
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    SHOTS.mkdir(parents=True, exist_ok=True)
    proof.SHOTS = SHOTS
    command_log = OUT / "commands.txt"
    command_log.write_text(
        "# Canonical-path real-grant boundary run\n"
        "cd /workspace/ArenaOS\n"
        "source tools/dev-env/v1-env.sh\n"
        f"V1_CC={compiler} V1_EVIDENCE_DIR={OUT} "
        "python3 tools/v1_layout_integration.py\n"
        "# Per case: guest filesd formats AFS2; APB1 is staged into "
        "/Users/user/Desktop; protected installer signs; launcher resolves "
        "the installed image; QMP captures actual QEMU screendumps.\n"
    )
    (OUT / "harness.log").write_text(
        f"source_commit={proof._source_commit()}\n"
        f"guest_compiler={compiler}\n"
        f"working_tree_dirty={proof._working_tree_dirty()}\n")
    identities = _candidate_identities()
    record = {
        "label": LABEL,
        "kind": "real_shared_region_layout_integration",
        "pass": False,
        "rc": 1,
        "source_commit": identities["source_commit"],
        "working_tree_dirty": identities["working_tree_dirty"],
        "dirty_paths_excluding_evidence": identities["dirty_paths_excluding_evidence"],
        "guest_compiler_under_test": compiler,
        "identities": identities,
        "fixture_contract": {
            "required_pages": REQUIRED_PAGES,
            "pixel_offset_bytes": PAGE_BYTES,
            "pixel_bytes": PIXEL_WORDS * 4,
            "flags": FLAGS,
            "requested_capabilities": 0,
            "desktop_operation": "existing SYS_SHARED_CREATE from Desktop POOL; exact page count checked with existing SYS_SHARED_PAGES",
            "new_kernel_syscalls": [],
            "added_child_capabilities": [],
        },
        "fixtures": {},
    }
    try:
        if identities["boot_efi"]["sha256"] == "MISSING" \
                or identities["esp"]["sha256"] == "MISSING" \
                or not identities["efi_embedded_matches_build"]:
            raise RuntimeError("candidate EFI/ESP identity is missing or embedded EFI differs")
        fixtures, compiler_ids = _build_artifacts(compiler)
        record["compiler_identities"] = compiler_ids
        record["artifacts"] = fixtures
        for pages in (301, 300):
            scenario = _scenario(compiler, pages, fixtures[pages])
            record["fixtures"][str(pages)] = scenario
            if not scenario.get("pass"):
                raise RuntimeError(
                    f"{pages}-page real-grant scenario failed: "
                    f"{scenario.get('fatal', 'assertion failed')}")
        record["pass"] = True
        record["rc"] = 0
    except Exception as exc:  # noqa: BLE001 - preserve partial evidence
        record["fatal"] = f"{type(exc).__name__}: {exc}"
        record["pass"] = False
        record["rc"] = 1
        traceback.print_exc()
    finally:
        for scratch in OUT.glob("layout-*/arena-disk*.img"):
            scratch.unlink(missing_ok=True)
        record["identities"] = _candidate_identities()
        record["commands"] = {
            "path": str(command_log.relative_to(OUT)),
            "sha256": proof.sha256_file(command_log),
        }
        EVIDENCE.write_text(json.dumps(record, indent=2, default=str) + "\n")
        (OUT / "harness.log").write_text(
            f"source_commit={proof._source_commit()}\n"
            f"guest_compiler={compiler}\n"
            f"working_tree_dirty={proof._working_tree_dirty()}\n"
            f"pass={record['pass']} rc={record['rc']}\n"
            f"fatal={record.get('fatal', '')}\n")
    print(f"[v1-layout-integration] compiler={compiler} "
          f"pass={record['pass']} rc={record['rc']} evidence={EVIDENCE}",
          flush=True)
    return int(record["rc"])


if __name__ == "__main__":
    sys.exit(main())
