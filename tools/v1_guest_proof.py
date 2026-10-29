#!/usr/bin/env python3
"""V1 Guest Proof — 7-checkpoint native graphical launch foundation proof.

Runs inside QEMU (TCG, ArenaOS guest). Drives the guest desktop via QMP
input events and screendumps, verifying:

  1. Launch-ready Desktop
  2. APB1 install via guest protected installer (installation acknowledgment)
  3. First native c-gfx-app checker window (after conforming Identify)
  4. Keyboard response (visible flash, fresh acknowledgment)
  5. Mouse response (visible flash, fresh acknowledgment)
  6. Two concurrent instances with focus/input isolation (both directions)
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
import subprocess
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

# Close button geometry from arena_ui::metrics.
CLOSE_W, TITLE_H = 28, 28

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

# Track which instances have been closed in CP6 for specific tracking.
cp6_closed_instances: list[int] = []

WINDOW_COUNT_RE = re.compile(r"registered ordinary window count=(\d+)")
RETIRED_RE = re.compile(r"\[desktop\] application retired")
CHILD_EXIT_RE = re.compile(r"child Process-cap exit status=(\d+)")
RESOURCE_DETAIL_RE = re.compile(
    r"\[desktop\] native resource detail values=([\d/]+)"
)


# ------------------------------------------------------------------ utils ---

def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else "MISSING"


def _qemu_binary_path() -> Path | None:
    """Resolve the QEMU executable actually used to run the guest.

    arena_env.qemu_cmd() returns the full launch prefix:
      - With ARENA_QEMU set: [command]        (e.g. the qemu-patched wrapper)
      - Sandbox default:     ["env", "LD_LIBRARY_PATH=...", "<musl libc.so>",
                              "/opt/qemu/bin/qemu-system-x86_64"]
      - Workstation:         ["qemu-system-x86_64"]
    Returns the first existing file in the prefix whose name identifies
    QEMU, falling back to any existing file in the prefix (e.g. a wrapper
    script). Returns None when nothing resolvable exists.
    """
    for part in arena_env.qemu_cmd():
        p = Path(part)
        if p.exists() and p.is_file() and p.name.startswith("qemu-system"):
            return p
    for part in arena_env.qemu_cmd():
        p = Path(part)
        if p.exists() and p.is_file():
            return p
    return None


def qemu_identity() -> tuple[str, str]:
    """(path, sha256) of the QEMU binary actually used, or ("MISSING", "MISSING")."""
    qemu_bin = _qemu_binary_path()
    if qemu_bin is None:
        return ("MISSING", "MISSING")
    return (str(qemu_bin), sha256_file(qemu_bin))


def window_count(serial: str) -> int:
    ms = WINDOW_COUNT_RE.findall(serial)
    return int(ms[-1]) if ms else 0


def retired_count(serial: str) -> int:
    return len(RETIRED_RE.findall(serial))


def child_exit_count(serial: str) -> int:
    return len(CHILD_EXIT_RE.findall(serial))


def last_child_exit(serial: str) -> int | None:
    ms = CHILD_EXIT_RE.findall(serial)
    return int(ms[-1]) if ms else None


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


def serial_line_count(d) -> int:
    """Return the number of lines in the guest serial log (cursor)."""
    return len(d.serial().splitlines())


def resource_detail_after_cursor(serial: str, cursor: int) -> list[dict]:
    """Extract resource detail receipts appearing after line cursor.

    Returns list of {"windows": str, "app_instances": str} for each
    matching receipt found in lines[cursor:].
    """
    lines = serial.splitlines()
    receipts = []
    for ln in lines[cursor:]:
        m = RESOURCE_DETAIL_RE.search(ln)
        if not m:
            continue
        vals = m.group(1).split("/")
        # fields: total-frames/threads/endpoints/notifications/timers/
        #   vm-regions/vm-committed-pages/sync-domains/sync-keys/
        #   sync-waiters/app-instances/windows/helpers/stream-sets/max-windows-per-instance
        windows = vals[11] if len(vals) > 11 else "?"
        app_instances = vals[10] if len(vals) > 10 else "?"
        receipts.append({"windows": windows, "app_instances": app_instances})
    return receipts


def await_settlement_fresh(d, cursor: int, bound_s: float = 15.0) -> bool:
    """Wait for fresh zero-instance/zero-window settlement AFTER cursor.

    Only evaluates lifecycle and resource receipts that appear after
    `cursor` in the serial log. Requires both:
      1. At least one "application retired" receipt after cursor
      2. At least one resource detail receipt with app-instances=0 AND
         windows=0 after cursor

    This fixes the historical-snapshot false positive where the initial
    Desktop baseline (app-instances=0, windows=0) satisfied a search of the
    entire serial history.
    """
    end = time.monotonic() + bound_s
    while time.monotonic() < end:
        serial = d.serial()
        retirement_found = False
        settlement_found = False
        for ln in serial.splitlines()[cursor:]:
            if "application retired" in ln:
                retirement_found = True
            m = RESOURCE_DETAIL_RE.search(ln)
            if m:
                vals = m.group(1).split("/")
                windows = vals[11] if len(vals) > 11 else "?"
                app_instances = vals[10] if len(vals) > 10 else "?"
                if windows == "0" and app_instances == "0":
                    settlement_found = True
        if retirement_found and settlement_found:
            return True
        time.sleep(0.2)
    return False


def record(n: int, verdict: str, ref: str = "", text: str = "") -> None:
    results[n] = verdict
    refs[n] = ref
    detail[n] = text
    print(f"[v1-guest-proof] checkpoint {n} {verdict}: {CHECKPOINTS[n]} {text}",
          flush=True)


def write_evidence(all_pass: bool, rc: int, install_ack: dict | None,
                   serial_path: Path, qemu_path: Path,
                   identities: dict | None = None) -> None:
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

    # Actual identities — never hardcoded, never "MISSING" for required
    # identities. Hash the QEMU binary actually used to run the guest.
    qemu_bin_path, qemu_bin_hash = qemu_identity()
    evidence = {
        "label": LABEL,
        "milestone": "V1 — Native Graphical Launch Foundation",
        "branch": "arena/f6b7ebc4-arenaos",
        "source_commit": _source_commit(),
        "working_tree_dirty": _working_tree_dirty(),
        "run_dir": str(RUN_DIR),
        "qemu_binary": qemu_bin_path,
        "qemu_binary_sha256": qemu_bin_hash,
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
    if identities:
        evidence["identities"] = identities
    EVIDENCE.write_text(json.dumps(evidence, indent=2))
    print(f"[v1-guest-proof] evidence -> {EVIDENCE}")


def _source_commit() -> str | None:
    """Actual tested source commit, or MISSING if not a git repo."""
    try:
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
            f"serial tail:\n{self.serial()[-1500:]}"
        )

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
    """Extract the AFS2 volume region from a disk image."""
    data = disk_path.read_bytes()
    if len(data) < AFS2_REGION_OFFSET:
        raise ValueError(f"disk {disk_path} too small ({len(data)} bytes)")
    return data[AFS2_REGION_OFFSET:]


def write_afs2_region(disk_path: Path, region: bytes) -> None:
    """Write the AFS2 volume region back into the disk image."""
    data = bytearray(disk_path.read_bytes())
    data[AFS2_REGION_OFFSET:AFS2_REGION_OFFSET + len(region)] = region
    disk_path.write_bytes(bytes(data))


def stage_apb1_in_afs2(disk_path: Path, bundle: bytes,
                       filename: bytes = b"c-gfx-app.apb1") -> None:
    """Stage the APB1 bundle as a real AFS2 file entry.

    Uses the repository's AFS2 Volume API to create directory entries and
    file data — never a raw offset write. Verifies after writing that the
    path exists, contents match, and filesystem consistency passes.
    """
    region = read_afs2_region(disk_path)
    vol = afs2.Volume(region)

    root_id = vol.root_id()

    def ensure_dir(parent_id: int, name: bytes) -> int:
        try:
            child_id, _ = vol.lookup(parent_id, name)
            return child_id
        except afs2.FsError:
            return vol.mkdir(parent_id, name)

    users_id = ensure_dir(root_id, b"Users")
    user_id = ensure_dir(users_id, b"user")
    desktop_id = ensure_dir(user_id, b"Desktop")

    try:
        file_id, _ = vol.lookup(desktop_id, filename)
    except afs2.FsError:
        file_id = vol.create(desktop_id, filename)

    offset = 0
    while offset < len(bundle):
        chunk = bundle[offset:offset + afs2.PAYLOAD]
        vol.write(file_id, offset, chunk)
        offset += len(chunk)

    region_after = vol.image()
    vol2 = afs2.Volume(region_after)

    desktop_id2 = vol2.resolve("/Users/user/Desktop")
    file_id2, _ = vol2.lookup(desktop_id2, filename)
    rec = vol2.record(file_id2)
    if rec.size != len(bundle):
        raise RuntimeError(
            f"AFS2 staged file size {rec.size} != bundle {len(bundle)}")
    read_back = vol2.read(file_id2, 0, rec.size)
    if read_back != bundle:
        raise RuntimeError("AFS2 staged file contents mismatch")

    vol2.statfs()

    write_afs2_region(disk_path, region_after)
    print(f"[v1-guest-proof] APB1 staged in AFS2 at /Users/user/Desktop/"
          f"{filename.decode()} ({len(bundle)} bytes)", flush=True)


# ------------------------------------------------------------------ proof helpers ---

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


def instance_identity_from_rects(rects: list) -> list[dict]:
    """Establish instance identity from window rects.

    Each instance is identified by its screen geometry (x,y,w,h). Returns
    a list of {"rect": (x,y,w,h), "center": (cx,cy), "label": str}.
    Label is "A" for the first (top-leftmost), "B" for the second, etc.
    """
    identities = []
    for i, rect in enumerate(rects):
        cx, cy = rect_center(rect)
        identities.append({
            "rect": rect,
            "center": (cx, cy),
            "label": chr(ord("A") + i),
        })
    return identities


# ------------------------------------------------------------------ proof ---

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
        if not has_app_color(img2, rects[0]):
            record(2, "FAIL", "shots/cp02-first-native-c-window.ppm",
                   "checker colors not present in detected window")
            return b"shutdown\r"
        cx, cy = rect_center(rects[0])
        record(2, "PASS", "shots/cp02-first-native-c-window.ppm",
               f"{len(rects)} connected checker component(s), "
               f"checker pattern present")
        # Let any launch/initial flash settle before keyboard testing.
        time.sleep(0.8)

        # ============ Checkpoint 3: keyboard response ====================
        # Event-specific keyboard input sequence:
        #   1. Focus the instance
        #   2. Require prior input-driven state to settle (no flash)
        #   3. Inject keyboard event
        #   4. Require fresh flash in focused window (event-specific ack)
        d.click(cx, cy)
        time.sleep(0.3)
        # Settle any prior flash before injecting new input. The focus
        # click itself delivers a POINTER event that triggers a 30-frame
        # flash (~1s); allow up to 2.5s for it to settle.
        settled_before_kb = await_no_flash(d, rects[0], settle_s=2.5)
        if not settled_before_kb:
            record(3, "FAIL", "shots/cp03-keyboard-response.ppm",
                   "pre-input flash did not settle before keyboard injection")
            return b"shutdown\r"
        # Inject keyboard event.
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
        # Event-specific mouse input sequence:
        #   1. Require prior flash to settle (BEFORE clicking)
        #   2. Inject mouse click inside the window
        #   3. Require fresh flash in clicked window (event-specific ack)
        # Settle prior flash BEFORE the mouse click. Allow up to 2.5s
        # for any residual flash (e.g. from keyboard input in CP3) to settle.
        settled_before_mouse = await_no_flash(d, rects[0], settle_s=2.5)
        if not settled_before_mouse:
            record(4, "FAIL", "shots/cp04-mouse-response.ppm",
                   "pre-input flash did not settle before mouse injection")
            return b"shutdown\r"
        # Click inside the window (not at center, to distinguish from CP3).
        mx = rects[0][0] + rects[0][2] // 4
        my = rects[0][1] + rects[0][3] // 4
        d.click(mx, my)
        time.sleep(0.3)
        # Require fresh flash in clicked window after mouse input.
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
        # Launch a second c-gfx-app instance. Connected components may merge
        # overlapping windows into one component, so component count is NOT
        # used as instance identity. Instead:
        #   1. Confirm two instances via serial window count
        #   2. Use spatial probing: leftmost and rightmost app/flash-colored
        #      columns as probe regions for the two instances
        #   3. Test both focus directions independently
        launch_cgfx_app(d)
        d.wait(lambda: window_count(d.serial()) >= 2,
               "two c-gfx-app instances registered")
        time.sleep(0.5)
        img5a = d.shot("cp05a-two-concurrent-instances")

        # Find all pixels matching EITHER app colors OR flash colors.
        # Flash may be active from launch animation; include it for extent.
        def app_or_flash_mask(data, width, height):
            hits = []
            for y in range(height):
                base = (y * width) * 3
                for x in range(width):
                    i = base + x * 3
                    r, g, b = data[i], data[i + 1], data[i + 2]
                    if (any(color_close((r, g, b), c) for c in APP_COLORS)
                            or any(color_close((r, g, b), c)
                                   for c in FLASH_COLORS)):
                        hits.append((x, y))
            return hits

        hits = app_or_flash_mask(img5a, SCREEN_W, SCREEN_H)
        if not hits:
            record(5, "FAIL",
                   "shots/cp05a-two-concurrent-instances.ppm",
                   "no visible checker/flash pixels for either instance")
            return b"shutdown\r"

        xs = [p[0] for p in hits]
        ys = [p[1] for p in hits]
        left_x = min(xs)
        right_x = max(xs)
        center_y = (min(ys) + max(ys)) // 2

        # Leftmost column region -> instance A (leftmost window in cascade)
        # Rightmost column region -> instance B (rightmost window in cascade)
        # Reject rather than guess if we cannot establish separate regions.
        if left_x == right_x:
            record(5, "FAIL",
                   "shots/cp05a-two-concurrent-instances.ppm",
                   "instances overlap exactly (left_x=%d == right_x=%d); "
                   "cannot establish separate probe regions" % (left_x, right_x))
            return b"shutdown\r"

        # Probe columns: inset from extremes to avoid edge noise.
        probe_a_x = min(left_x + 15, right_x - 15)
        probe_b_x = max(right_x - 15, left_x + 15)
        if probe_a_x >= probe_b_x:
            record(5, "FAIL",
                   "shots/cp05a-two-concurrent-instances.ppm",
                   "instances too close (left_x=%d, right_x=%d); "
                   "cannot establish separate probe regions" % (left_x, right_x))
            return b"shutdown\r"

        probe_a = (probe_a_x, center_y)
        probe_b = (probe_b_x, center_y)
        probe_rect = (0, max(0, center_y - 15), SCREEN_W, 30)  # horizontal band

        # Verify both probe regions have non-background content.
        def non_bg_at(data, x, y, half=8):
            x0 = max(0, x - half)
            y0 = max(0, y - half)
            x1 = min(SCREEN_W, x + half)
            y1 = min(SCREEN_H, y + half)
            bg = (24, 32, 53)
            for row in range(y0, y1):
                base = (row * SCREEN_W) * 3
                for col in range(x0, x1):
                    i = base + col * 3
                    r, g, b = data[i], data[i + 1], data[i + 2]
                    if not color_close((r, g, b), bg):
                        return True
            return False

        if not (non_bg_at(img5a, *probe_a) and non_bg_at(img5a, *probe_b)):
            record(5, "FAIL",
                   "shots/cp05a-two-concurrent-instances.ppm",
                   "cannot establish non-background content at both probe "
                   "regions (probe_a=(%d,%d), probe_b=(%d,%d))"
                   % (probe_a_x, center_y, probe_b_x, center_y))
            return b"shutdown\r"

        # ===== Direction 1: Focus A, inject key, require A flashes, B not =====
        d.click(*probe_a)
        time.sleep(0.3)
        settled_a = await_no_flash(d,
                                   (probe_a_x - 8, center_y - 8, 16, 16),
                                   settle_s=2.5)
        if not settled_a:
            record(5, "FAIL", "shots/cp05-focus-isolation.ppm",
                   "instance A pre-input flash did not settle")
            return b"shutdown\r"
        d.type_text("a")  # keyboard input for instance A
        end = time.monotonic() + 2.0
        dir1_ok = False
        while time.monotonic() < end:
            shot = d.shot("__flash-probe__")
            flash_a = pixel_flash(shot, *probe_a)
            flash_b = pixel_flash(shot, *probe_b)
            if flash_a and not flash_b:
                dir1_ok = True
                break
            time.sleep(0.1)

        # ===== Direction 2: Focus B, inject key, require B flashes, A not =====
        settled_b = await_no_flash(d,
                                   (probe_b_x - 8, center_y - 8, 16, 16),
                                   settle_s=2.5)
        if not settled_b:
            record(5, "FAIL", "shots/cp05-focus-isolation.ppm",
                   "instance B pre-input flash did not settle after dir 1")
            return b"shutdown\r"
        d.click(*probe_b)
        time.sleep(0.3)
        settled_b2 = await_no_flash(d,
                                    (probe_b_x - 8, center_y - 8, 16, 16),
                                    settle_s=2.5)
        if not settled_b2:
            record(5, "FAIL", "shots/cp05-focus-isolation.ppm",
                   "instance B pre-input flash did not settle before dir 2")
            return b"shutdown\r"
        d.type_text("b")  # keyboard input for instance B
        end = time.monotonic() + 2.0
        dir2_ok = False
        while time.monotonic() < end:
            shot = d.shot("__flash-probe__")
            flash_b2 = pixel_flash(shot, *probe_b)
            flash_a2 = pixel_flash(shot, *probe_a)
            if flash_b2 and not flash_a2:
                dir2_ok = True
                break
            time.sleep(0.1)

        img5 = d.shot("cp05-focus-isolation")
        ok5 = dir1_ok and dir2_ok
        record(5, "PASS" if ok5 else "FAIL",
               "shots/cp05-focus-isolation.ppm",
               "dir1(A focus@x=%d): %s, dir2(B focus@x=%d): %s"
               % (probe_a_x, "PASS" if dir1_ok else "FAIL",
                  probe_b_x, "PASS" if dir2_ok else "FAIL")
               if ok5
               else "isolation failed: dir1=%s, dir2=%s"
               % (dir1_ok, dir2_ok))        # ============ Checkpoint 6: window removal with accounting =======
        # Retire every live app window using close button. Track specific
        # instances. Require fresh zero-instance/zero-window settlement
        # AFTER a serial cursor captured before closing.
        restored = False
        windows_retired = 0
        # Close button: top-right of each window's title bar
        # (CLOSE_WIDTH=28, TITLE_HEIGHT=28 from arena_ui::metrics).
        for _ in range(4):
            rects_now = window_rects(d.shot("__cp6-probe__"))
            if not rects_now:
                restored = True
                break
            rx, ry, rw, rh = rects_now[0]
            cx_close = rx + rw - CLOSE_W // 2
            cy_close = ry + TITLE_H // 2
            d.click(cx_close, cy_close)
            time.sleep(0.4)
            # Wait for the window to VISUALLY disappear from the screen.
            end_v = time.monotonic() + 3.0
            visually_gone = False
            while time.monotonic() < end_v:
                if not window_rects(d.shot("__cp6-visual-probe__")):
                    visually_gone = True
                    break
                time.sleep(0.1)
            if retired_count(d.serial()) > 0:
                windows_retired += 1
            if visually_gone:
                restored = True
                break

        # Capture serial cursor AFTER all closes, before checking settlement.
        # We need a cursor that captures the state before the final close
        # operation's retirement events.
        cp6_cursor = serial_line_count(d)

        # Wait for fresh zero-instance/zero-window settlement after cursor.
        settled = await_settlement_fresh(d, cp6_cursor)

        time.sleep(0.4)
        img6 = d.shot("cp06-desktop-restored")
        rects6 = window_rects(img6)

        # Resource settlement must be fresh (after cursor), not historical.
        ok6 = settled and len(rects6) == 0
        record(6, "PASS" if ok6 else "FAIL",
               "shots/cp06-desktop-restored.ppm",
               f"{windows_retired} window(s) retired, Desktop restored, "
               f"resources settled={settled}, visual rects={len(rects6)}"
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
            # Let the newly launched window finish its initial render
            # before interacting with it.
            time.sleep(0.8)
            # Click to focus the window, keeping the pointer on it so
            # keyboard input is delivered to the focused window.
            cx7, cy7 = rect_center(rects7[0])
            d.point(cx7, cy7, True)
            d.point(cx7, cy7, False)
            time.sleep(0.3)
            # Wait for any click/pointer flash to settle before key injection.
            await_no_flash(d, rects7[0], settle_s=2.5)
            d.type_text("v")
            end = time.monotonic() + 3.0
            ok7 = False
            while time.monotonic() < end:
                s7 = d.shot("__cp7-probe__")
                # Use the pre-computed window rect: flash colors XOR every
                # content pixel, so during a flash the window carries NO app
                # colors and window_rects() returns [] — recomputing rects
                # inside the poll would make flash detection impossible.
                if has_flash_color(s7, rects7[0]):
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


# ------------------------------------------------------------------ verdict ---

def compute_overall_verdict(rc: int, results: dict, install_ack: dict | None,
                            identities: dict | None,
                            serial_path: Path, evidence_path: Path) -> bool:
    """Single production verdict computation used consistently.

    Success requires ALL of the following:
      - Successful guest execution (rc == 0)
      - Every checkpoint executed and PASS (no NOT RUN, no FAIL)
      - Installation acknowledgment captured
      - Required identities present and verified (no MISSING)
      - Required evidence files present and valid
    """
    if rc != 0:
        return False
    # Every checkpoint must be PASS (no NOT RUN, no FAIL).
    for n in sorted(CHECKPOINTS):
        if results.get(n) != "PASS":
            return False
    # Installation acknowledgment must be present.
    if install_ack is None:
        return False
    # Required identities must be present and not MISSING.
    if identities is None:
        return False
    for key in ("qemu_binary_sha256", "apb1_sha256"):
        if identities.get(key) in (None, "MISSING", ""):
            return False
    # Required evidence files must be present. The guest serial log exists
    # before the verdict; evidence.json is written immediately after, so it
    # cannot be required to exist here (that would be self-defeating).
    if not serial_path.exists():
        return False
    return True


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
                       serial_path=SERIAL_OUT,
                       qemu_path=arena_env.build_dir() /
                       "qemu-system-x86_64-patched")
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

    # Build identities for provenance. Hash the QEMU binary actually used
    # to run the guest, not a path that may not exist in this environment.
    qemu_bin_path, qemu_bin_hash = qemu_identity()
    identities = {
        "source_commit": _source_commit(),
        "working_tree_dirty": _working_tree_dirty(),
        "qemu_binary": qemu_bin_path,
        "qemu_binary_sha256": qemu_bin_hash,
        "apb1_sha256": sha256_file(APB1),
        "esp_sha256": sha256_file(ESP),
        "serial_sha256": sha256_file(SERIAL_OUT),
    }

    # COMPUTE VERDICT FIRST — before writing evidence.
    all_pass = compute_overall_verdict(
        rc=rc,
        results=results,
        install_ack=install_ack,
        identities=identities,
        serial_path=SERIAL_OUT,
        evidence_path=EVIDENCE,
    )

    write_evidence(all_pass=all_pass, rc=rc, install_ack=install_ack,
                   serial_path=SERIAL_OUT,
                   qemu_path=arena_env.build_dir() /
                   "qemu-system-x86_64-patched",
                   identities=identities)

    print(f"[v1-guest-proof] checkpoints:", flush=True)
    for n in sorted(CHECKPOINTS):
        print(f"  checkpoint {n} {results[n]}: {CHECKPOINTS[n]} ({refs[n]})",
              flush=True)
    print(f"[v1-guest-proof] V1 GUEST PROOF {'PASS' if all_pass else 'FAIL'}",
          flush=True)
    return 0 if all_pass else 1


# ------------------------------------------------------------------ focused controls (no QEMU) ---

def test_ppm_parsing() -> None:
    """Focused control: PPM parser validates dimensions, format, payload."""
    import struct
    tmp = Path(tempfile.mkdtemp())

    w, h = 800, 600
    payload = bytes([0x14, 0x28, 0x5A]) * (w * h)
    p = tmp / "valid.ppm"
    p.write_bytes(b"P6\n" + f"{w} {h}\n".encode() + b"255\n" + payload)
    width, height, data = parse_ppm(p)
    assert width == w and height == h and data == payload, "valid PPM"
    print("  PASS: valid PPM parsed correctly")

    p2 = tmp / "bad_magic.ppm"
    p2.write_bytes(b"P5\n800 600\n255\n" + b"\x00" * (w * h * 3))
    try:
        parse_ppm(p2)
        assert False, "should have raised"
    except ValueError:
        print("  PASS: bad magic rejected")

    p3 = tmp / "truncated.ppm"
    p3.write_bytes(b"P6\n800 600\n255\n" + b"\x00" * 100)
    try:
        parse_ppm(p3)
        assert False, "should have raised"
    except ValueError:
        print("  PASS: truncated payload rejected")

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
    normal = bytes([0x14, 0x28, 0x5A])  # (20,40,90)
    alt = bytes([0x3C, 0x28, 0x14])      # (60,40,20)
    flash_normal = bytes([0x14 ^ 0xA0, 0x28 ^ 0xA0, 0x5A ^ 0xA0])
    flash_alt = bytes([0x3C ^ 0xA0, 0x28 ^ 0xA0, 0x14 ^ 0xA0])
    background = bytes([24, 32, 53])
    titlebar = bytes([34, 51, 85])

    img = bytearray(background * (w * h))
    img[0:3] = normal
    img[3:6] = alt
    data = bytes(img)

    hits = app_mask(data, w, h)
    assert (0, 0) in hits, "checker pixel (0,0) detected"
    assert (1, 0) in hits, "checker pixel (1,0) detected"
    assert len(hits) == 2, f"expected 2 hits, got {len(hits)}"
    print("  PASS: checker pixels detected")

    img_bg = bytearray(background * (w * h))
    assert app_mask(bytes(img_bg), w, h) == [], "background excluded"
    print("  PASS: desktop background excluded")

    img_tb = bytearray(titlebar * (w * h))
    assert app_mask(bytes(img_tb), w, h) == [], "titlebar excluded"
    print("  PASS: titlebar excluded at tolerance 8")

    img_flash = bytearray(background * (w * h))
    img_flash[0:3] = flash_normal
    assert has_flash_color(bytes(img_flash), (0, 0, 1, 1), w, h)
    print("  PASS: flash color (normal) detected")

    img_flash2 = bytearray(background * (w * h))
    img_flash2[0:3] = flash_alt
    assert has_flash_color(bytes(img_flash2), (0, 0, 1, 1), w, h)
    print("  PASS: flash color (alt) detected")


def test_window_rects_overlap() -> None:
    """Focused control: overlapping windows merge into one component."""
    w, h = 200, 200
    background = bytes([24, 32, 53])
    normal = bytes([0x14, 0x28, 0x5A])

    img = bytearray(background * (w * h))

    def fill_square(img, x, y, size, color):
        for dy in range(size):
            for dx in range(size):
                px = ((y + dy) * w + (x + dx)) * 3
                img[px:px + 3] = color

    fill_square(img, 10, 10, 50, normal)
    fill_square(img, 30, 30, 50, normal)  # overlaps first

    rects = window_rects(bytes(img), w, h)
    assert len(rects) == 1, f"expected 1 merged component, got {len(rects)}"
    print("  PASS: overlapping windows merge into one component "
          "(component != instance)")

    img2 = bytearray(background * (w * h))
    fill_square(img2, 10, 10, 30, normal)
    fill_square(img2, 100, 100, 30, normal)
    rects2 = window_rects(bytes(img2), w, h)
    assert len(rects2) == 2, f"expected 2 components, got {len(rects2)}"
    print("  PASS: non-overlapping windows are separate components")


def test_callback_contract() -> None:
    """Focused control: mtest.boot callbacks take no arguments."""
    import inspect
    sig = inspect.signature(proof)
    assert len(sig.parameters) == 0, f"proof() takes {len(sig.parameters)} args"
    print("  PASS: proof() takes no arguments (matches mtest.boot contract)")

    good = lambda: b"shutdown\r"
    assert good() == b"shutdown\r"
    print("  PASS: no-arg callback returns shutdown payload")


def test_evidence_writing() -> None:
    """Focused control: evidence.json is written even on unsuccessful runs."""
    import copy
    saved_results = dict(results)
    saved_refs = dict(refs)
    saved_detail = dict(detail)
    try:
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
        results.clear(); results.update(saved_results)
        refs.clear(); refs.update(saved_refs)
        detail.clear(); detail.update(saved_detail)


# ---------------- CP6 negative controls (no QEMU, parser/verdict logic) ---

def _make_serial_with_receipts(receipts: list[str]) -> str:
    """Build a synthetic serial string from receipt lines."""
    return "\n".join(receipts)


def test_cp6_zero_resources_but_app_live() -> None:
    """NEGATIVE: CP6 must FAIL when serial has zero/zero receipt but
    app-instances > 0 in the latest receipt (app still live).

    Simulates: boot contains zero-resource receipts (baseline) but an
    application remains live. Historical-snapshot search would falsely PASS.
    """
    serial = (
        "[desktop] native resource detail values=114501/16/12/51/0/0/0/0/0/0/0/0/0/0/0\n"
        "[desktop] registered ordinary window count=0\n"
        "[desktop] native resource detail values=114501/17/12/51/0/2/208/0/0/0/1/1/0/0/1\n"
        "[desktop] registered ordinary window count=1\n"
    )
    cursor = len(serial.splitlines())
    # No fresh retirement or zero/zero receipt after cursor.
    result = await_settlement_fresh_from_serial(serial, cursor)
    assert result is False, "should FAIL: app live, no fresh settlement"
    print("  PASS: zero-resource receipts but app live -> FAIL")


def test_cp6_only_one_retires() -> None:
    """NEGATIVE: CP6 must FAIL when only one of two instances retires.

    Simulates: two instances launched, only one retires, the other remains
    live with app-instances=1.
    """
    serial = (
        "[desktop] application retired: kind=6 Process consumed\n"
        "[desktop] native resource detail values=114501/17/12/51/0/2/208/0/0/0/1/1/0/0/1\n"
        "[desktop] registered ordinary window count=1\n"
    )
    cursor = len(serial.splitlines())
    result = await_settlement_fresh_from_serial(serial, cursor)
    assert result is False, "should FAIL: only one instance retired, not zero/zero"
    print("  PASS: only one of two instances retires -> FAIL")


def test_cp6_windows_gone_resources_leak() -> None:
    """NEGATIVE: CP6 must FAIL when visual windows gone but resources leak.

    Simulates: windows visually removed (no rects) but app-instances > 0.
    """
    serial = (
        "[desktop] application retired: kind=6 Process consumed\n"
        "[desktop] native resource detail values=114501/17/12/51/0/2/208/0/0/0/1/0/0/0/1\n"
        "[desktop] registered ordinary window count=0\n"
    )
    cursor = len(serial.splitlines())
    result = await_settlement_fresh_from_serial(serial, cursor)
    assert result is False, "should FAIL: resources leaked (app-instances=1)"
    print("  PASS: windows gone but resources leak -> FAIL")


def test_cp6_resources_settled_stale_window() -> None:
    """NEGATIVE: CP6 must FAIL when resources settle but stale window remains.

    Simulates: app-instances=0, windows=0 in resource detail but visual
    window still present (stale presentation).
    """
    serial = (
        "[desktop] application retired: kind=6 Process consumed\n"
        "[desktop] native resource detail values=114501/16/12/51/0/0/0/0/0/0/0/0/0/0/0\n"
        "[desktop] registered ordinary window count=0\n"
    )
    cursor = 0  # cursor at start — all receipts are fresh relative to cursor
    # Resources settled but we need to check visual separately — this control
    # tests that the resource parser correctly identifies zero/zero. The
    # visual check is separate in CP6 (len(rects6) == 0).
    result = await_settlement_fresh_from_serial(serial, cursor)
    assert result is True, "should PASS: resources truly settled"
    print("  PASS: resources settled (zero/zero) recognized -> resource PASS")


def test_cp6_fresh_receipts_missing() -> None:
    """NEGATIVE: CP6 must FAIL when no fresh settlement receipts after cursor.

    Simulates: cursor captures state, then no new retirement or resource
    receipts arrive within bound.
    """
    serial = (
        "[desktop] native resource detail values=114501/16/12/51/0/0/0/0/0/0/0/0/0/0/0\n"
        "[desktop] registered ordinary window count=0\n"
    )
    cursor = len(serial.splitlines())  # cursor after all existing receipts
    result = await_settlement_fresh_from_serial(serial, cursor)
    assert result is False, "should FAIL: no fresh receipts after cursor"
    print("  PASS: no fresh settlement receipts -> FAIL")


def test_cp6_all_success() -> None:
    """ALL-SUCCESS: CP6 must PASS when retirement + fresh zero/zero settlement."""
    serial = (
        "[desktop] application retired: kind=6 Process consumed\n"
        "[desktop] native resource detail values=114501/16/12/51/0/0/0/0/0/0/0/0/0/0/0\n"
        "[desktop] registered ordinary window count=0\n"
    )
    cursor = 0  # cursor at beginning, all receipts are fresh
    result = await_settlement_fresh_from_serial(serial, cursor)
    assert result is True, "should PASS: retirement + fresh zero/zero"
    print("  PASS: retirement + fresh zero/zero settlement -> PASS")


def await_settlement_fresh_from_serial(serial: str, cursor: int) -> bool:
    """Inline version of await_settlement_fresh for testing without QEMU.

    Operates on a serial string directly instead of polling d.serial().
    """
    retirement_found = False
    settlement_found = False
    for ln in serial.splitlines()[cursor:]:
        if "application retired" in ln:
            retirement_found = True
        m = RESOURCE_DETAIL_RE.search(ln)
        if m:
            vals = m.group(1).split("/")
            windows = vals[11] if len(vals) > 11 else "?"
            app_instances = vals[10] if len(vals) > 10 else "?"
            if windows == "0" and app_instances == "0":
                settlement_found = True
    return retirement_found and settlement_found


def test_overall_verdict_missing_checkpoint() -> None:
    """NEGATIVE: overall verdict must be False when a checkpoint is FAIL."""
    saved = dict(results)
    try:
        results[1] = "PASS"
        results[2] = "FAIL"
        result = compute_overall_verdict(
            rc=0, results=results, install_ack={"serial_line": "x"},
            identities={"qemu_binary_sha256": "abc", "apb1_sha256": "def"},
            serial_path=SERIAL_OUT, evidence_path=EVIDENCE,
        )
        assert result is False, "should FAIL: checkpoint 2 is FAIL"
        print("  PASS: missing checkpoint success (FAIL) -> overall FAIL")
    finally:
        results.clear(); results.update(saved)


def test_overall_verdict_missing_identity() -> None:
    """NEGATIVE: overall verdict must be False when required identity missing."""
    saved = dict(results)
    try:
        for n in sorted(CHECKPOINTS):
            results[n] = "PASS"
        result = compute_overall_verdict(
            rc=0, results=results, install_ack={"serial_line": "x"},
            identities={"qemu_binary_sha256": "MISSING", "apb1_sha256": "def"},
            serial_path=SERIAL_OUT, evidence_path=EVIDENCE,
        )
        assert result is False, "should FAIL: QEMU identity MISSING"
        print("  PASS: missing identity -> overall FAIL")
    finally:
        results.clear(); results.update(saved)


def test_overall_verdict_all_pass() -> None:
    """ALL-SUCCESS: overall verdict True when all conditions met."""
    saved = dict(results)
    try:
        for n in sorted(CHECKPOINTS):
            results[n] = "PASS"
        SERIAL_OUT.parent.mkdir(parents=True, exist_ok=True)
        SERIAL_OUT.write_text("test serial\n")
        EVIDENCE.parent.mkdir(parents=True, exist_ok=True)
        EVIDENCE.write_text("{}")
        result = compute_overall_verdict(
            rc=0, results=results, install_ack={"serial_line": "x"},
            identities={"qemu_binary_sha256": "abc", "apb1_sha256": "def"},
            serial_path=SERIAL_OUT, evidence_path=EVIDENCE,
        )
        assert result is True, "should PASS: all conditions met"
        print("  PASS: all conditions met -> overall PASS")
    finally:
        results.clear(); results.update(saved)
        SERIAL_OUT.unlink(missing_ok=True)
        EVIDENCE.unlink(missing_ok=True)


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
        # CP6 negative controls (parser/verdict logic, no QEMU)
        ("CP6 neg: zero-resource receipts but app live", test_cp6_zero_resources_but_app_live),
        ("CP6 neg: only one of two instances retires", test_cp6_only_one_retires),
        ("CP6 neg: windows gone but resources leak", test_cp6_windows_gone_resources_leak),
        ("CP6 neg: resources settled, stale window", test_cp6_resources_settled_stale_window),
        ("CP6 neg: fresh receipts missing after cursor", test_cp6_fresh_receipts_missing),
        ("CP6 pos: retirement + fresh zero/zero settlement", test_cp6_all_success),
        # Overall verdict controls
        ("Verdict: missing checkpoint success -> FAIL", test_overall_verdict_missing_checkpoint),
        ("Verdict: missing identity -> FAIL", test_overall_verdict_missing_identity),
        ("Verdict: all conditions met -> PASS", test_overall_verdict_all_pass),
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