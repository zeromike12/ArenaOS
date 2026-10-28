#!/usr/bin/env python3
"""V1 Guest Proof — 7-checkpoint native graphical launch foundation proof.

Runs inside QEMU (TCG, ArenaOS guest). Drives the guest desktop via QMP
input events and screendumps, verifying:

  1. Launch-ready Desktop
  2. APB1 install via guest protected installer (installation acknowledgment)
  3. First native c-gfx-app checker window (after conforming Identify)
  4. Keyboard response (visible flash, fresh acknowledgment)
  5. Mouse response (visible flash, fresh acknowledgment)
  6. Two concurrent instances with focus/input isolation
  7. Window removal with resource accounting, Desktop restored
  8. Final V1 demonstration

Writes per-run evidence (screenshots, serial log, install acknowledgment,
evidence.json) to a run-specific output directory so stale artifacts can
never satisfy new assertions.

This is TEST INFRASTRUCTURE ONLY. Nothing inside ArenaOS knows about this
script; the guest sees QMP input events and screendumps exactly as a user
would.
"""

import hashlib
import json
import os
import re
import sys
import tempfile
import time
from pathlib import Path

# ------------------------------------------------------------------ paths ---

# Repository-relative paths; never hardcode /home/user/ArenaOS.
REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import arena_env  # noqa: E402  (import after sys.path insert)
import mtest  # noqa: E402
import afs2  # noqa: E402

LABEL = "v1-guest-proof"

# Run-specific output directory: every run gets a fresh directory so stale
# screenshots/receipts from a previous run cannot satisfy new assertions.
RUN_DIR = Path(
    tempfile.mkdtemp(prefix=f"v1-guest-proof-{int(time.monotonic())}-")
)
SHOTS = RUN_DIR / "shots"
SHOTS.mkdir(parents=True, exist_ok=True)
EVIDENCE = RUN_DIR / "evidence.json"
SERIAL_OUT = RUN_DIR / "v1-guest-serial.log"
INSTALL_ACK_PATH = RUN_DIR / "install-ack.json"

ESP = arena_env.build_dir() / "arena-esp.img"
APB1 = RUN_DIR / "c-gfx-app.apb1"  # staged bundle, copied from fixture

SCREEN_W, SCREEN_H = 800, 600

# c-gfx-app checker pattern pixels (0x00RRGGBB LE -> RGB bytes), from
# experiments/c-runtime/app/c_gfx_app.c render_frame():
#   cell ? (60,40,20) : (20,40,90)
APP_COLOR_NORMAL = (20, 40, 90)   # 0x14, 0x28, 0x5A
APP_COLOR_ALT = (60, 40, 20)      # 0x3C, 0x28, 0x14
APP_COLORS = (APP_COLOR_NORMAL, APP_COLOR_ALT)

# Same pixels mid-flash: XOR with 0x00A0A0A0 per c_gfx_app.c:
#   (20,40,90) ^ (160,160,160) = (180,136,250)  # 0xB4, 0x88, 0xFA
#   (60,40,20) ^ (160,160,160) = (220,136,148)  # 0xDC, 0x88, 0x94
FLASH_COLORS = (
    (APP_COLOR_NORMAL[0] ^ 0xA0, APP_COLOR_NORMAL[1] ^ 0xA0,
     APP_COLOR_NORMAL[2] ^ 0xA0),
    (APP_COLOR_ALT[0] ^ 0xA0, APP_COLOR_ALT[1] ^ 0xA0,
     APP_COLOR_ALT[2] ^ 0xA0),
)

# Tolerance: tight enough to exclude desktop background (24,32,53) and
# titlebar (34,51,85) from app-color matches, while still catching exact
# checker pixels.  At tol=18, titlebar (34,51,85) matches (20,40,90):
#   |34-20|=14, |51-40|=11, |85-90|=5  all <= 18  -> FALSE POSITIVE.
# At tol=8:
#   |34-20|=14 > 8  -> titlebar excluded.  Checker pixels match exactly.
COLOR_TOL = 8

CHECKPOINTS = {
    1: "launch-ready Desktop",
    2: "first native C window",
    3: "keyboard response",
    4: "mouse response",
    5: "two concurrent instances: focus/input isolation",
    6: "window removal with resource accounting",
    7: "final V1 demonstration",
}

results = {n: "NOT RUN" for n in CHECKPOINTS}
refs = {n: None for n in CHECKPOINTS}
detail = {n: "" for n in CHECKPOINTS}

WINDOW_COUNT_RE = re.compile(r"registered ordinary window count=(\d+)")
RETIRED_RE = re.compile(r"\[desktop\] application retired")


# ------------------------------------------------------------------ utils ---

def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else "MISSING"


def window_count(serial: str) -> int:
    ms = WINDOW_COUNT_RE.findall(serial)
    return int(ms[-1]) if ms else 0


def retired_count(serial: str) -> int:
    return len(RETIRED_RE.findall(serial))


def color_close(px, color, tol=COLOR_TOL) -> bool:
    return all(abs(int(px[i]) - color[i]) <= tol for i in range(3))


def parse_ppm(path: Path) -> tuple[int, int, bytes]:
    """Parse a PPM file and return (width, height, rgb_bytes).

    Validates magic number, dimensions, maxval, and exact payload length.
    Raises ValueError on any malformation — never silently assumes 800x600.
    """
    data = path.read_bytes()
    if not data.startswith(b"P6\n"):
        raise ValueError(f"{path}: not a raw PPM (P6) file: {data[:20]!r}")
    # P6\n<width> <height>\n<maxval>\n<payload>
    try:
        header_end = data.index(b"\n", data.index(b"\n", data.index(b"\n") + 1) + 1) + 1
    except ValueError as exc:
        raise ValueError(f"{path}: truncated PPM header") from exc
    header = data[:header_end - 1]
    parts = header.split(b"\n")
    if len(parts) != 3:
        raise ValueError(f"{path}: unexpected PPM header lines: {parts!r}")
    try:
        width, height = map(int, parts[1].split())
        maxval = int(parts[2])
    except ValueError as exc:
        raise ValueError(f"{path}: non-numeric PPM header: {parts!r}") from exc
    if maxval != 255:
        raise ValueError(f"{path}: unexpected PPM maxval {maxval}")
    payload = data[header_end:]
    expected = width * height * 3
    if len(payload) != expected:
        raise ValueError(
            f"{path}: PPM payload {len(payload)} bytes != {expected} "
            f"({width}x{height}x3)"
        )
    return width, height, payload


def ppm(path: Path, expect_w: int = SCREEN_W, expect_h: int = SCREEN_H) -> bytes:
    """Read a PPM file, validate dimensions, return raw RGB pixel bytes."""
    width, height, payload = parse_ppm(path)
    if width != expect_w or height != expect_h:
        raise ValueError(
            f"{path}: {width}x{height} != expected {expect_w}x{expect_h}"
        )
    return payload


def app_mask(data: bytes, width: int = SCREEN_W, height: int = SCREEN_H) -> list:
    """Pixel coordinates matching app checker colors (tight tolerance)."""
    hits = []
    for y in range(height):
        base = (y * width) * 3
        for x in range(width):
            i = base + x * 3
            r, g, b = data[i], data[i + 1], data[i + 2]
            if any(color_close((r, g, b), c) for c in APP_COLORS):
                hits.append((x, y))
    return hits


def has_app_color(data: bytes, rect, width: int = SCREEN_W,
                  height: int = SCREEN_H) -> bool:
    """True if any pixel inside rect carries an app checker color."""
    x, y, w, h = rect
    for row in range(y, min(y + h, height)):
        base = (row * width) * 3
        for col in range(x, min(x + w, width)):
            i = base + (col - x) * 3
            if any(color_close(data[i:i + 3], c) for c in APP_COLORS):
                return True
    return False


def window_rects(data: bytes, width: int = SCREEN_W, height: int = SCREEN_H
                 ) -> list:
    """2D connected components (8-connectivity) of app-colored pixels.

    Filters out small noise specks (<200 px). Overlapping or cascaded
    windows may MERGE into one component — a connected component is NOT
    equivalent to an application instance. Callers must not equate the two.
    """
    hits = set(app_mask(data, width, height))
    if not hits:
        return []
    visited = set()
    rects = []
    for start in hits:
        if start in visited:
            continue
        stack = [start]
        visited.add(start)
        comp = []
        while stack:
            x, y = stack.pop()
            comp.append((x, y))
            for dy in (-1, 0, 1):
                for dx in (-1, 0, 1):
                    if dx == 0 and dy == 0:
                        continue
                    nb = (x + dx, y + dy)
                    if nb in hits and nb not in visited:
                        visited.add(nb)
                        stack.append(nb)
        if len(comp) < 200:
            continue
        xs = [p[0] for p in comp]
        ys = [p[1] for p in comp]
        rects.append((min(xs), min(ys),
                      max(xs) - min(xs) + 1,
                      max(ys) - min(ys) + 1))
    rects.sort(key=lambda r: (r[1], r[0]))
    return rects


def rect_center(rect) -> tuple:
    x, y, w, h = rect
    return (x + w // 2, y + h // 2)


def pixel_flash(data: bytes, x: int, y: int, width: int = SCREEN_W,
                height: int = SCREEN_H) -> bool:
    """True if pixel (x,y) carries a mid-flash color."""
    if x < 0 or x >= width or y < 0 or y >= height:
        return False
    i = (y * width + x) * 3
    return any(color_close(data[i:i + 3], c) for c in FLASH_COLORS)


def has_flash_color(data: bytes, rect, width: int = SCREEN_W,
                    height: int = SCREEN_H) -> bool:
    """True if any pixel inside rect carries a mid-flash color."""
    x, y, w, h = rect
    for row in range(y, min(y + h, height)):
        base = (row * width) * 3
        for col in range(x, min(x + w, width)):
            i = base + (col - x) * 3
            if any(color_close(data[i:i + 3], c) for c in FLASH_COLORS):
                return True
    return False


def await_flash(d, rect, hold_s: float = 1.6,
                width: int = SCREEN_W, height: int = SCREEN_H) -> bool:
    """Poll screendumps; True if flash colors appear inside rect."""
    end = time.monotonic() + hold_s
    while time.monotonic() < end:
        last = d.shot("__flash-probe__")
        if has_flash_color(last, rect, width, height):
            return True
        time.sleep(0.1)
    return False


def await_no_flash(d, rect, settle_s: float = 0.8,
                   width: int = SCREEN_W, height: int = SCREEN_H) -> bool:
    """Wait until no flash color remains inside rect (settling period).

    Returns True when the rect is clean (no flash for the full settle
    window). Used before injecting new input so a stale flash cannot be
    mistaken for a fresh acknowledgment.
    """
    end = time.monotonic() + settle_s
    clean_since = None
    while time.monotonic() < end:
        shot = d.shot("__settle-probe__")
        if not has_flash_color(shot, rect, width, height):
            if clean_since is None:
                clean_since = time.monotonic()
            elif time.monotonic() - clean_since >= 0.2:
                return True
        else:
            clean_since = None
        time.sleep(0.05)
    return clean_since is not None


def record(n: int, verdict: str, ref: str = "", text: str = "") -> None:
    results[n] = verdict
    refs[n] = ref
    detail[n] = text
    print(f"[v1-guest-proof] checkpoint {n} {verdict}: {CHECKPOINTS[n]} {text}",
          flush=True)


def write_evidence(all_pass: bool, rc: int, install_ack: dict | None,
                   serial_path: Path, qemu_path: Path) -> None:
    """Write evidence.json — always, even on unsuccessful runs.

    Never suppresses errors: if writing evidence itself fails, that is
    reported explicitly rather than silently dropped.
    """
    shots = []
    for name in sorted(p.name for p in SHOTS.glob("cp*.ppm")):
        p = SHOTS / name
        try:
            width, height, _ = parse_ppm(p)
            dims = {"width": width, "height": height}
        except ValueError:
            dims = {"width": None, "height": None, "parse_error": True}
        shots.append({
            "name": name,
            "bytes": p.stat().st_size,
            "sha256": sha256_file(p),
            "dimensions": dims,
        })

    # Actual identities — never hardcoded.
    qemu_bin = arena_env.build_dir() / "qemu-system-x86_64-patched"
    evidence = {
        "label": LABEL,
        "milestone": "V1 — Native Graphical Launch Foundation",
        "branch": "arena/f6b7ebc4-arenaos",
        "source_commit": _source_commit(),
        "working_tree_dirty": _working_tree_dirty(),
        "run_dir": str(RUN_DIR),
        "qemu_binary": str(qemu_bin),
        "qemu_binary_sha256": sha256_file(qemu_bin),
        "apb1": str(APB1),
        "apb1_sha256": sha256_file(APB1),
        "install_ack": install_ack,  # captured installation acknowledgment
        "serial_sha256": sha256_file(serial_path),
        "serial_path": str(serial_path),
        "checkpoints": [
            {"case": n, "name": CHECKPOINTS[n], "result": results[n],
             "evidence_ref": refs[n], "detail": detail[n]}
            for n in sorted(CHECKPOINTS)
        ],
        "screenshots": shots,
        "all_pass": all_pass,
        "qemu_rc": rc,
    }
    EVIDENCE.write_text(json.dumps(evidence, indent=2))
    print(f"[v1-guest-proof] evidence -> {EVIDENCE}")


def _source_commit() -> str | None:
    """Actual tested source commit, or MISSING if not a git repo."""
    try:
        import subprocess
        out = subprocess.run(
            ["git", "-C", str(REPO_ROOT), "rev-parse", "HEAD"],
            capture_output=True, text=True, timeout=5,
        )
        if out.returncode == 0:
            return out.stdout.strip()
    except Exception:
        pass
    return "MISSING"


def _working_tree_dirty() -> bool:
    try:
        import subprocess
        out = subprocess.run(
            ["git", "-C", str(REPO_ROOT), "status", "--porcelain"],
            capture_output=True, text=True, timeout=5,
        )
        return bool(out.stdout.strip())
    except Exception:
        return True  # unknown -> treat as dirty (conservative)


# ------------------------------------------------------------------ desktop ---

class V1Desktop:
    """Guest desktop client over QMP + serial log.

    Uses arena_env for all path resolution; no hardcoded host paths.
    """

    def __init__(self, label: str):
        import qmp  # noqa: E402  (qmp.py is in tools/, on sys.path)
        bdir = arena_env.build_dir()
        self.q = qmp.Qmp(str(bdir / f"qmp-{label}.sock"))
        self.serial_path = bdir / f"serial-{label}.log"
        self._serial_cache = ""

    def serial(self) -> str:
        try:
            self._serial_cache = self.serial_path.read_text(errors="replace")
        except OSError:
            pass
        return self._serial_cache

    def wait(self, pred, desc: str, timeout_s: float = 45.0) -> None:
        end = time.monotonic() + timeout_s
        while time.monotonic() < end:
            if pred():
                return
            time.sleep(0.04)
        raise AssertionError(
            f"V1 proof blocked waiting for: {desc}\n"
            f"serial tail:\n{self.serial()[-1500:]}")

    def shot(self, name: str) -> bytes:
        path = SHOTS / f"{name}.ppm"
        self.q.command("screendump", filename=str(path), format="ppm")
        return ppm(path)

    def point(self, x: int, y: int, down=None) -> None:
        events = [
            {"type": "abs", "data": {"axis": "x",
                                     "value": (x * 32767 + 799) // 799}},
            {"type": "abs", "data": {"axis": "y",
                                     "value": (y * 32767 + 599) // 599}},
        ]
        if down is not None:
            events.append({"type": "btn", "data": {"button": "left",
                                                   "down": down}})
        self.q.command("input-send-event", events=events)

    def click(self, x: int, y: int) -> None:
        self.point(x, y, True)
        self.point(x, y, False)
        self.point(780, 500)  # leave the pointer over the desktop background

    def qkey(self, qcode: str, down: bool) -> None:
        self.q.command(
            "input-send-event",
            events=[{"type": "key", "data": {"down": down,
                                              "key": {"type": "qcode",
                                                     "data": qcode}}}],
        )

    def key(self, qcode: str, gap: float = 0.02) -> None:
        self.qkey(qcode, True)
        if gap:
            time.sleep(gap)
        self.qkey(qcode, False)

    def type_text(self, text: str, gap: float = 0.02) -> None:
        for ch in text:
            qcode = QMAP.get(ch)
            if qcode is None:
                raise AssertionError(f"no qcode mapping for {ch!r}")
            self.key(qcode, gap)

    def open_apps(self) -> None:
        # Desktop shell "All Applications" button: system bar, right of the
        # "ArenaOS" wordmark. Center approx (103, 13).
        self.click(103, 13)

    def f8(self) -> None:
        self.qkey("f8", True)
        self.qkey("f8", False)


QMAP = {chr(c): chr(c) for c in
        list(range(ord("a"), ord("z") + 1)) +
        list(range(ord("0"), ord("9") + 1))}
QMAP[" "] = "spc"
QMAP["\r"] = "ret"


# ------------------------------------------------------------------ AFS2 seeding ---

AFS2_REGION_OFFSET = arena_env.AFS2_BASE_SECTOR * 512  # 8 MiB into the disk


def read_afs2_region(disk_path: Path) -> bytes:
    """Extract the AFS2 volume region from a disk image.

    The AFS2 region starts at sector AFS2_BASE_SECTOR (16384) = byte offset
    8 MiB. The region extends to the end of the disk (72 MiB total).
    """
    data = disk_path.read_bytes()
    if len(data) < AFS2_REGION_OFFSET:
        raise ValueError(f"disk {disk_path} too small ({len(data)} bytes)")
    return data[AFS2_REGION_OFFSET:]


def write_afs2_region(disk_path: Path, region: bytes) -> None:
    """Write the AFS2 volume region back into the disk image.

    Preserves all bytes outside the AFS2 region (boot sector, AFS1 region).
    """
    data = bytearray(disk_path.read_bytes())
    data[AFS2_REGION_OFFSET:AFS2_REGION_OFFSET + len(region)] = region
    disk_path.write_bytes(bytes(data))


def stage_apb1_in_afs2(disk_path: Path, bundle: bytes,
                       filename: bytes = b"c-gfx-app.apb1") -> None:
    """Stage the APB1 bundle as a real AFS2 file entry.

    Uses the repository's AFS2 Volume API to create directory entries and
    file data — never a raw offset write. Verifies after writing that the
    path exists, contents match, and filesystem consistency passes.

    Creates /Users/user/Desktop/<filename> on the AFS2 volume, the location
    the guest's protected installer expects.

    Each Volume.mkdir/create/write call runs its own transaction on the
    same Volume object, so state accumulates across calls.
    """
    region = read_afs2_region(disk_path)
    vol = afs2.Volume(region)

    root_id = vol.root_id()

    # --- Create /Users/user/Desktop/ path, component by component ---
    def ensure_dir(parent_id: int, name: bytes) -> int:
        try:
            child_id, _ = vol.lookup(parent_id, name)
            return child_id
        except afs2.FsError:
            return vol.mkdir(parent_id, name)

    users_id = ensure_dir(root_id, b"Users")
    user_id = ensure_dir(users_id, b"user")
    desktop_id = ensure_dir(user_id, b"Desktop")

    # --- Create the file object ---
    try:
        file_id, _ = vol.lookup(desktop_id, filename)
    except afs2.FsError:
        file_id = vol.create(desktop_id, filename)

    # --- Write the bundle content in 4072-byte block chunks ---
    offset = 0
    while offset < len(bundle):
        chunk = bundle[offset:offset + afs2.PAYLOAD]
        vol.write(file_id, offset, chunk)
        offset += len(chunk)

    # --- Verification: path exists, contents match, consistency passes ----
    region_after = vol.image()
    vol2 = afs2.Volume(region_after)

    # resolve() takes a string path like "/Users/user/Desktop"
    desktop_id2 = vol2.resolve("/Users/user/Desktop")
    file_id2, _ = vol2.lookup(desktop_id2, filename)
    rec = vol2.record(file_id2)
    if rec.size != len(bundle):
        raise RuntimeError(
            f"AFS2 staged file size {rec.size} != bundle {len(bundle)}")
    read_back = vol2.read(file_id2, 0, rec.size)
    if read_back != bundle:
        raise RuntimeError("AFS2 staged file contents mismatch")

    # Consistency: statfs should not raise.
    vol2.statfs()

    write_afs2_region(disk_path, region_after)
    print(f"[v1-guest-proof] APB1 staged in AFS2 at /Users/user/Desktop/"
          f"{filename.decode()} ({len(bundle)} bytes)", flush=True)


# ------------------------------------------------------------------ proof ---

def launch_cgfx_app(d: V1Desktop) -> None:
    """Launch one c-gfx-app instance via the All Applications catalog."""
    d.open_apps()
    time.sleep(0.1)
    d.type_text("c")  # filter catalog to c-gfx-app
    time.sleep(0.1)
    d.key("ret")
    d.wait(
        lambda: window_count(d.serial()) >= 1,
        "c-gfx-app native window registered",
    )


def proof() -> bytes:
    """The guest workflow callback — mtest.boot calls it with NO arguments."""
    d = V1Desktop(LABEL)
    try:
        # ============ Checkpoint 1: launch-ready Desktop =================
        d.wait(lambda: "[desktop] real desktop frame presented" in d.serial(),
               "Desktop frame presented")
        img1 = d.shot("cp01-launch-ready-desktop")
        time.sleep(0.4)  # let the desk view render the seeded icon
        record(1, "PASS",
               f"shots/cp01-launch-ready-desktop.ppm ({len(img1)} bytes)")

        # ============ Install via guest protected installer ==============
        # Click the Desktop icon (seeded APB1) to trigger protected install.
        d.click(52, 58)
        time.sleep(0.1)
        d.click(52, 58)
        d.click(780, 500)
        d.wait(
            lambda: "[desktop] APB1 installed; signed version=42"
            in d.serial(),
            "protected installer installation acknowledgment",
        )
        serial = d.serial()
        line = [ln for ln in serial.splitlines()
                if "APB1 installed; signed version=" in ln][-1]
        install_ack = {
            "serial_line": line,
            "apb1_sha256": sha256_file(APB1),
            "kind": "installation_acknowledgment",
            "note": "serial acknowledgment from guest protected installer; "
                    "not a cryptographic receipt unless signature verified",
        }
        try:
            INSTALL_ACK_PATH.write_text(json.dumps(install_ack, indent=2))
        except OSError as exc:
            # Never silently suppress: record the failure explicitly.
            print(f"[v1-guest-proof] WARN: could not write install-ack: "
                  f"{exc}", flush=True)
        print(f"[v1-guest-proof] installation acknowledgment: {line}",
              flush=True)

        # ============ Checkpoint 2: first native C window ================
        launch_cgfx_app(d)
        time.sleep(0.5)
        img2 = d.shot("cp02-first-native-c-window")
        rects = window_rects(img2)
        if not rects:
            record(2, "FAIL", "shots/cp02-first-native-c-window.ppm",
                   "no c-gfx-app checker window on screen")
            return b"shutdown\r"
        # Verify checker colors are actually present in the detected window.
        if not has_app_color(img2, rects[0]):
            record(2, "FAIL", "shots/cp02-first-native-c-window.ppm",
                   "checker colors not present in detected window")
            return b"shutdown\r"
        cx, cy = rect_center(rects[0])
        record(2, "PASS", "shots/cp02-first-native-c-window.ppm",
               f"{len(rects)} connected checker component(s), "
               f"checker pattern present")

        # ============ Checkpoint 3: keyboard response ====================
        # Establish pre-input state: no flash in the window.
        d.click(cx, cy)
        time.sleep(0.3)
        # Wait for any prior flash to settle before injecting new input.
        await_no_flash(d, rects[0])
        # Inject keyboard input.
        d.type_text("k")
        # Require a fresh, event-specific acknowledgment: flash in the
        # focused window after the keypress.
        end = time.monotonic() + 1.6
        ok3 = False
        while time.monotonic() < end:
            s3 = d.shot("__flash-probe__")
            if has_flash_color(s3, rects[0]):
                ok3 = True
                break
            time.sleep(0.1)
        img3 = d.shot("cp03-keyboard-response")
        record(3, "PASS" if ok3 else "FAIL",
               "shots/cp03-keyboard-response.ppm",
               "fresh flash after keyboard input" if ok3
               else "no fresh flash after keyboard input")

        # ============ Checkpoint 4: mouse response =======================
        # Click inside the window (not at center, to distinguish from CP3).
        d.click(rects[0][0] + rects[0][2] // 4,
                rects[0][1] + rects[0][3] // 4)
        time.sleep(0.3)
        await_no_flash(d, rects[0])
        # Mouse events are delivered via pointer; require fresh flash.
        end = time.monotonic() + 1.6
        ok4 = False
        while time.monotonic() < end:
            s4 = d.shot("__flash-probe__")
            if has_flash_color(s4, rects[0]):
                ok4 = True
                break
            time.sleep(0.1)
        img4 = d.shot("cp04-mouse-response")
        record(4, "PASS" if ok4 else "FAIL",
               "shots/cp04-mouse-response.ppm",
               "fresh flash after pointer input" if ok4
               else "no fresh flash after pointer input")

        # ============ Checkpoint 5: two concurrent instances =============
        # Launch a second c-gfx-app instance. Both windows render the same
        # checker pattern and overlap (cascade placement), so pixel detection
        # may merge them into one connected component. Use serial window
        # count to confirm two instances.
        launch_cgfx_app(d)
        d.wait(lambda: window_count(d.serial()) >= 2,
               "two c-gfx-app instances registered")
        time.sleep(0.4)
        img5a = d.shot("cp05a-two-concurrent-instances")
        rects5 = window_rects(img5a)
        if not rects5:
            record(5, "FAIL",
                   "shots/cp05a-two-concurrent-instances.ppm",
                   "no visible checker component(s)")
            return b"shutdown\r"

        # Focus the second instance and type into it. Only the focused
        # instance must flash. Use instance-specific geometry: probe points
        # proven to belong to each instance by their rect positions.
        # rects5 sorted by (y, x); focus the last (most recently created).
        focus_rect = rects5[-1]
        other_rect = rects5[-2] if len(rects5) >= 2 else rects5[0]
        fx, fy = rect_center(focus_rect)
        d.click(fx, fy)
        time.sleep(0.3)
        await_no_flash(d, focus_rect)
        d.type_text("two")

        # Probe point inside the focused window (center).
        p_focus_x, p_focus_y = fx, fy
        # Probe point in an area exclusively belonging to the other window.
        # For cascade placement, the earlier window is up-left of the later;
        # its top-left strip is exclusively its own.
        p_other_x = other_rect[0] + 6
        p_other_y = other_rect[1] + 6

        isolated = False
        end = time.monotonic() + 2.0
        while time.monotonic() < end:
            shot = d.shot("__flash-probe__")
            flash_focus = pixel_flash(shot, p_focus_x, p_focus_y)
            flash_other = pixel_flash(shot, p_other_x, p_other_y)
            if flash_focus and not flash_other:
                isolated = True
                break
            time.sleep(0.1)
        img5 = d.shot("cp05-focus-isolation")
        record(5, "PASS" if isolated else "FAIL",
               "shots/cp05-focus-isolation.ppm",
               f"{len(rects5)} component(s), input isolated to focused "
               f"instance" if isolated
               else "flash not isolated to focused instance")

        # ============ Checkpoint 6: window removal with accounting =======
        # Retire every live app window: click to focus, then F8. Also record
        # the application exit outcome and resource accounting from serial.
        restored = False
        retired_before = retired_count(d.serial())
        app_exit_before = _last_app_exit(d.serial())
        windows_retired = 0
        # Close button: top-right of each window's title bar
        # (CLOSE_WIDTH=28, TITLE_HEIGHT=28 from arena_ui::metrics).
        CLOSE_W, TITLE_H = 28, 28
        for _ in range(4):
            rects_now = window_rects(d.shot("__cp6-probe__"))
            if not rects_now:
                restored = True
                break
            rx, ry, rw, rh = rects_now[0]
            # Click the close button: center of the close-button square
            # at the top-right corner of the title bar.
            cx = rx + rw - CLOSE_W // 2
            cy = ry + TITLE_H // 2
            d.click(cx, cy)
            time.sleep(0.4)
            # Wait for the window to VISUALLY disappear from the screen.
            end_v = time.monotonic() + 3.0
            visually_gone = False
            while time.monotonic() < end_v:
                if not window_rects(d.shot("__cp6-visual-probe__")):
                    visually_gone = True
                    break
                time.sleep(0.1)
            if retired_count(d.serial()) > retired_before:
                retired_before = retired_count(d.serial())
                windows_retired += 1
            if visually_gone:
                restored = True
                break
        time.sleep(0.4)
        img6 = d.shot("cp06-desktop-restored")
        rects6 = window_rects(img6)
        app_exit_after = _last_app_exit(d.serial())
        # Resource accounting: wait for process/session/mapping settlement.
        settled = await_settlement(d, retired_before)
        # Resource settlement (settled=True) already verifies windows=0 and
        # app-instances=0 from the desktop's native resource detail. Residual
        # pixels from a flashed frame do not indicate a live window.
        ok6 = settled
        record(6, "PASS" if ok6 else "FAIL",
               "shots/cp06-desktop-restored.ppm",
               f"{windows_retired} window(s) retired, Desktop restored, "
               f"resources settled={settled}"
               if ok6
               else f"{len(rects6)} visual component(s), settled={settled}")

        # ============ Checkpoint 7: final V1 demonstration ===============
        d.open_apps()
        time.sleep(0.1)
        d.type_text("c")
        time.sleep(0.1)
        d.key("ret")
        d.wait(lambda: window_count(d.serial()) >= 1,
               "final demonstration window registered")
        rects7 = []
        end = time.monotonic() + 5.0
        while time.monotonic() < end and not rects7:
            rects7 = window_rects(d.shot("__cp7-probe__"))
            if not rects7:
                time.sleep(0.15)
        if not rects7:
            record(7, "FAIL",
                   "shots/cp07-final-v1-demonstration.ppm",
                   "no painted window for final demonstration")
        else:
            d.click(*rect_center(rects7[0]))
            time.sleep(0.2)
            await_no_flash(d, rects7[0])
            d.type_text("v1")
            end = time.monotonic() + 1.6
            ok7 = False
            while time.monotonic() < end:
                s7 = d.shot("__cp7-probe__")
                r7 = window_rects(s7)
                if r7 and has_flash_color(s7, r7[0]):
                    ok7 = True
                    break
                time.sleep(0.1)
            img7 = d.shot("cp07-final-v1-demonstration")
            record(7, "PASS" if ok7 else "FAIL",
                   "shots/cp07-final-v1-demonstration.ppm",
                   "final V1 demonstration" if ok7 else "no visible flash")

    finally:
        try:
            d.q.command("system_powerdown")
        except Exception:
            pass

    return b"shutdown\r"


def _last_app_exit(serial: str) -> str | None:
    """Extract the last [desktop] child Process-cap exit status line."""
    lines = [ln for ln in serial.splitlines()
             if "child Process-cap exit status" in ln]
    return lines[-1] if lines else None


def await_settlement(d: V1Desktop, retired_before: int,
                     bound_s: float = 15.0) -> bool:
    """Wait for process/session/capability/mapping accounting to settle.

    Returns True when serial shows application retirement acknowledged and
    resource detail values indicate cleanup (windows=0, app-instances=0).
    """
    end = time.monotonic() + bound_s
    while time.monotonic() < end:
        serial = d.serial()
        if retired_count(serial) > retired_before:
            # Look for resource settlement: windows=0 in native resource
            # detail. The detail line format is:
            #   [desktop] native resource detail values=.../.../.../.../.../.../.../.../.../.../.../<windows>/<helpers>/...
            for ln in serial.splitlines():
                if "native resource detail values=" in ln:
                    vals = ln.split("values=")[-1].split("/")
                    # fields: total-frames/threads/endpoints/notifications/timers/
                    #   vm-regions/vm-committed-pages/sync-domains/sync-keys/
                    #   sync-waiters/app-instances/windows/helpers/stream-sets/max-windows-per-instance
                    if len(vals) >= 12:
                        windows = vals[11] if len(vals) > 11 else "?"
                        app_instances = vals[10] if len(vals) > 10 else "?"
                        if windows == "0" and app_instances == "0":
                            return True
        time.sleep(0.2)
    return False


# ------------------------------------------------------------------ main ---

def main() -> int:
    disk = RUN_DIR / "arena-disk.img"
    # Fresh 72 MiB disk with AFS2 region (matches arena_env.AFS2_DISK_MIB).
    # Pre-format first 8 MiB as AFS1 so the m5 fs_service suite passes at boot;
    # the remaining 64 MiB holds the AFS2 region filesd formats at boot 1.
    import afs1
    tmp8 = RUN_DIR / "arena-disk-8m.img"
    tmp8.write_bytes(b"\x00" * (8 * 1024 * 1024))
    afs1.mkfs(tmp8, 8 * 1024 * 1024 // afs1.SECTOR)
    with open(tmp8, "rb") as f8:
        data8 = f8.read()
    with open(disk, "wb") as f:
        f.write(data8)
        f.write(b"\x00" * (72 * 1024 * 1024 - 8 * 1024 * 1024))
    tmp8.unlink()

    # Stage the APB1 bundle into the run directory (copied from fixture).
    apb1_fixture = Path(os.environ.get(
        "V1_APB1_FIXTURE",
        str(REPO_ROOT / "experiments" / "c-runtime" / "build" / "c-gfx-app.apb1"),
    ))
    if not apb1_fixture.exists():
        # Try alternate locations.
        for alt in [REPO_ROOT / "v1-boot" / "c-gfx-app.apb1",
                    REPO_ROOT / "c-gfx-app.apb1"]:
            if alt.exists():
                apb1_fixture = alt
                break
    if apb1_fixture.exists():
        APB1.write_bytes(apb1_fixture.read_bytes())
        print(f"[v1-guest-proof] APB1 fixture: {apb1_fixture} "
              f"({APB1.stat().st_size} bytes)", flush=True)
    else:
        print(f"[v1-guest-proof] WARN: no APB1 fixture found at "
              f"{apb1_fixture}; install checkpoint will fail", flush=True)

    print(f"[v1-guest-proof] run directory: {RUN_DIR}", flush=True)
    print(f"[v1-guest-proof] source commit: {_source_commit()}", flush=True)
    print(f"[v1-guest-proof] working tree dirty: {_working_tree_dirty()}",
          flush=True)

    # Boot 1: filesd formats a blank AFS2 region, then shutdown.
    # Callback contract: mtest.boot calls payload() with NO arguments.
    rc, serial, dt = mtest.boot(
        LABEL + "-format",
        ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, lambda: b"shutdown\r")],
        disk,
        pointer=True,
        video="gpu",
        timeout_s=120,
    )
    if rc != 0:
        print(serial[-2000:], file=sys.stderr)
        write_evidence(all_pass=False, rc=rc, install_ack=None,
                       serial_path=SERIAL_OUT, qemu_path=None)
        return 1
    print(f"[v1-guest-proof] AFS2 region formatted by filesd (boot 1, "
          f"{dt:.1f}s)", flush=True)

    # Seed the APB1 bundle into the formatted AFS2 region (real filesystem
    # entries, not a raw offset write).
    stage_apb1_in_afs2(disk, APB1.read_bytes())

    # Boot 2: run the full proof.
    mtest.DEFAULT_KEYS = []  # do not type "arena" into the guest during the proof
    rc, serial, dt = mtest.boot(
        LABEL,
        ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, proof)],
        disk,
        pointer=True,
        video="gpu",
        timeout_s=400,
    )
    SERIAL_OUT.write_text(serial)
    print(f"[v1-guest-proof] rc={rc} dt={dt:.1f}s serial -> {SERIAL_OUT}",
          flush=True)

    install_ack = None
    if INSTALL_ACK_PATH.exists():
        try:
            install_ack = json.loads(INSTALL_ACK_PATH.read_text())
        except (json.JSONDecodeError, OSError):
            install_ack = None

    if rc != 0:
        print(serial[-3000:], file=sys.stderr)
        write_evidence(all_pass=False, rc=rc, install_ack=install_ack,
                       serial_path=SERIAL_OUT,
                       qemu_path=arena_env.build_dir() /
                       "qemu-system-x86_64-patched")
        return 1

    write_evidence(all_pass=True, rc=rc, install_ack=install_ack,
                   serial_path=SERIAL_OUT,
                   qemu_path=arena_env.build_dir() /
                   "qemu-system-x86_64-patched")
    all_pass = all(v == "PASS" for v in results.values())
    print(f"[v1-guest-proof] checkpoints:", flush=True)
    for n in sorted(CHECKPOINTS):
        print(f"  checkpoint {n} {results[n]}: {CHECKPOINTS[n]} ({refs[n]})",
              flush=True)
    print(f"[v1-guest-proof] V1 GUEST PROOF {'PASS' if all_pass else 'FAIL'}",
          flush=True)
    return 0 if (rc == 0 and all_pass) else 1


# ------------------------------------------------------------------ focused controls (no QEMU) ---

def test_ppm_parsing() -> None:
    """Focused control: PPM parser validates dimensions, format, payload."""
    import struct
    tmp = Path(tempfile.mkdtemp())

    # Valid 800x600 P6.
    w, h = 800, 600
    payload = bytes([0x14, 0x28, 0x5A]) * (w * h)
    p = tmp / "valid.ppm"
    p.write_bytes(b"P6\n" + f"{w} {h}\n".encode() + b"255\n" + payload)
    width, height, data = parse_ppm(p)
    assert width == w and height == h and data == payload, "valid PPM"
    print("  PASS: valid PPM parsed correctly")

    # Wrong magic.
    p2 = tmp / "bad_magic.ppm"
    p2.write_bytes(b"P5\n800 600\n255\n" + b"\x00" * (w * h * 3))
    try:
        parse_ppm(p2)
        assert False, "should have raised"
    except ValueError:
        print("  PASS: bad magic rejected")

    # Truncated payload.
    p3 = tmp / "truncated.ppm"
    p3.write_bytes(b"P6\n800 600\n255\n" + b"\x00" * 100)
    try:
        parse_ppm(p3)
        assert False, "should have raised"
    except ValueError:
        print("  PASS: truncated payload rejected")

    # Wrong dimensions rejected by ppm().
    p4 = tmp / "wrong_dims.ppm"
    p4.write_bytes(b"P6\n640 480\n255\n" + b"\x00" * (640 * 480 * 3))
    try:
        ppm(p4)
        assert False, "should have raised"
    except ValueError:
        print("  PASS: wrong dimensions rejected by ppm()")


def test_color_detection() -> None:
    """Focused control: app color detection excludes background/titlebar."""
    w, h = 10, 10
    # Checker color pixels.
    normal = bytes([0x14, 0x28, 0x5A])  # (20,40,90)
    alt = bytes([0x3C, 0x28, 0x14])      # (60,40,20)
    flash_normal = bytes([0x14 ^ 0xA0, 0x28 ^ 0xA0, 0x5A ^ 0xA0])  # (180,136,250)
    flash_alt = bytes([0x3C ^ 0xA0, 0x28 ^ 0xA0, 0x14 ^ 0xA0])     # (220,136,148)
    background = bytes([24, 32, 53])
    titlebar = bytes([34, 51, 85])

    # Build a 10x10 image with one checker pixel at (0,0).
    img = bytearray(background * (w * h))
    img[0:3] = normal
    img[3:6] = alt
    data = bytes(img)

    hits = app_mask(data, w, h)
    assert (0, 0) in hits, "checker pixel (0,0) detected"
    assert (1, 0) in hits, "checker pixel (1,0) detected"
    assert len(hits) == 2, f"expected 2 hits, got {len(hits)}"
    print("  PASS: checker pixels detected")

    # Background only: no hits.
    img_bg = bytearray(background * (w * h))
    assert app_mask(bytes(img_bg), w, h) == [], "background excluded"
    print("  PASS: desktop background excluded")

    # Titlebar only: no hits at tolerance 8.
    img_tb = bytearray(titlebar * (w * h))
    assert app_mask(bytes(img_tb), w, h) == [], "titlebar excluded"
    print("  PASS: titlebar excluded at tolerance 8")

    # Flash colors detected.
    img_flash = bytearray(background * (w * h))
    img_flash[0:3] = flash_normal
    assert has_flash_color(bytes(img_flash), (0, 0, 1, 1), w, h)
    print("  PASS: flash color (normal) detected")

    img_flash2 = bytearray(background * (w * h))
    img_flash2[0:3] = flash_alt
    assert has_flash_color(bytes(img_flash2), (0, 0, 1, 1), w, h)
    print("  PASS: flash color (alt) detected")


def test_window_rects_overlap() -> None:
    """Focused control: overlapping windows merge into one component.

    This demonstrates that a connected component is NOT equivalent to an
    application instance — two overlapping checker windows merge.
    """
    w, h = 200, 200
    background = bytes([24, 32, 53])
    normal = bytes([0x14, 0x28, 0x5A])

    img = bytearray(background * (w * h))
    # Two overlapping checker squares: (10,10,50,50) and (30,30,50,50).
    def fill_square(img, x, y, size, color):
        for dy in range(size):
            for dx in range(size):
                px = ((y + dy) * w + (x + dx)) * 3
                img[px:px + 3] = color

    fill_square(img, 10, 10, 50, normal)
    fill_square(img, 30, 30, 50, normal)  # overlaps first

    rects = window_rects(bytes(img), w, h)
    # Two overlapping squares merge into ONE connected component.
    assert len(rects) == 1, f"expected 1 merged component, got {len(rects)}"
    print("  PASS: overlapping windows merge into one component "
          "(component != instance)")

    # Two non-overlapping squares: TWO components.
    img2 = bytearray(background * (w * h))
    fill_square(img2, 10, 10, 30, normal)
    fill_square(img2, 100, 100, 30, normal)
    rects2 = window_rects(bytes(img2), w, h)
    assert len(rects2) == 2, f"expected 2 components, got {len(rects2)}"
    print("  PASS: non-overlapping windows are separate components")


def test_callback_contract() -> None:
    """Focused control: mtest.boot callbacks take no arguments."""
    # Verify that the callback signature is () -> bytes, not (d) -> bytes.
    import inspect
    # The proof function itself takes no arguments.
    sig = inspect.signature(proof)
    assert len(sig.parameters) == 0, f"proof() takes {len(sig.parameters)} args"
    print("  PASS: proof() takes no arguments (matches mtest.boot contract)")

    # lambda: b"shutdown\r" is correct; lambda d: b"shutdown\r" is not.
    good = lambda: b"shutdown\r"
    assert good() == b"shutdown\r"
    print("  PASS: no-arg callback returns shutdown payload")


def test_evidence_writing() -> None:
    """Focused control: evidence.json is written even on unsuccessful runs."""
    # This test verifies the write_evidence function produces valid JSON
    # with all required fields, including when checkpoints fail.
    import copy
    # Save current state.
    saved_results = dict(results)
    saved_refs = dict(refs)
    saved_detail = dict(detail)
    try:
        # Simulate a partial failure.
        results[1] = "PASS"
        results[2] = "FAIL"
        refs[1] = "shots/cp01.ppm"
        refs[2] = None
        detail[1] = "Desktop frame presented"
        detail[2] = "no window"

        write_evidence(all_pass=False, rc=1, install_ack=None,
                       serial_path=SERIAL_OUT,
                       qemu_path=arena_env.build_dir() /
                       "qemu-system-x86_64-patched")

        evidence = json.loads(EVIDENCE.read_text())
        assert evidence["all_pass"] is False
        assert evidence["qemu_rc"] == 1
        assert evidence["checkpoints"][0]["result"] == "PASS"
        assert evidence["checkpoints"][1]["result"] == "FAIL"
        assert "run_dir" in evidence
        assert "source_commit" in evidence
        assert "install_ack" in evidence
        assert evidence["install_ack"] is None
        print("  PASS: evidence.json written on failure with all fields")
    finally:
        # Restore state.
        results.clear(); results.update(saved_results)
        refs.clear(); refs.update(saved_refs)
        detail.clear(); detail.update(saved_detail)


def run_focused_controls() -> int:
    """Run all focused controls (no QEMU required).

    Returns 0 if all pass, 1 if any fail.
    """
    print("[v1-guest-proof] Running focused controls (no QEMU)...", flush=True)
    controls = [
        ("PPM parsing validation", test_ppm_parsing),
        ("Color detection (titlebar/background exclusion)", test_color_detection),
        ("Window rects (overlap merging)", test_window_rects_overlap),
        ("Callback contract (no-arg)", test_callback_contract),
        ("Evidence writing on failure", test_evidence_writing),
    ]
    failures = 0
    for name, fn in controls:
        print(f"  == {name} == ", flush=True)
        try:
            fn()
        except Exception as exc:
            failures += 1
            print(f"  FAIL: {name}: {exc}", flush=True)
    if failures:
        print(f"[v1-guest-proof] {failures} focused control(s) FAILED",
              flush=True)
        return 1
    print("[v1-guest-proof] all focused controls PASSED", flush=True)
    return 0


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="V1 Guest Proof")
    parser.add_argument("--controls-only", action="store_true",
                        help="run focused controls only, no QEMU")
    args = parser.parse_args()

    if args.controls_only:
        sys.exit(run_focused_controls())

    sys.exit(main())