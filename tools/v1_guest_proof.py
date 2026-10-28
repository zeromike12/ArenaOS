#!/usr/bin/env python3
"""V1 Guest Proof — 7-checkpoint native graphical launch foundation proof.

Runs inside QEMU (TCG, ArenaOS guest). Drives the guest desktop via QMP
input events and screendumps, verifying:
  1. Launch-ready Desktop
  2. APB1 install via guest protected installer (signed receipt)
  3. First native c-gfx-app checker window
  4. Keyboard response (visible flash)
  5. Mouse response (visible flash)
  6. Two concurrent instances with focus/input isolation
  7. Window removal, Desktop restored
  8. Final V1 demonstration

Writes evidence.json + v1-guest-serial.log to /home/user/v1-boot/.
"""

import hashlib
import json
import sys
import time
from pathlib import Path

# ------------------------------------------------------------------ paths ---

LABEL = "v1-guest-proof"
ESP = Path("/home/user/ArenaOS/build/arena-esp.img")
APB1 = Path("/home/user/v1-boot/c-gfx-app.apb1")
ROOT = Path("/home/user/v1-boot")
SHOTS = ROOT / "shots"
SHOTS.mkdir(parents=True, exist_ok=True)
EVIDENCE = ROOT / "evidence.json"
SERIAL_OUT = ROOT / "v1-guest-serial.log"
INSTALL_RECEIPT_PATH = ROOT / "install-receipt.json"

SCREEN_W, SCREEN_H = 800, 600

# c-gfx-app checker pattern pixels (0x00RRGGBB LE -> RGB bytes).
APP_COLORS = ((20, 40, 90), (60, 40, 20))
# Same pixels mid-flash (0x00RRGGBB ^ 0x00A0A0A0).
FLASH_COLORS = ((0x20 ^ 0xA0, 0x28 ^ 0xA0, 0x5A ^ 0xA0),
                (0x3C ^ 0xA0, 0x28 ^ 0xA0, 0x14 ^ 0xA0))

# Tolerance must be tight enough to exclude desktop background (24,32,53),
# titlebar (34,51,85), etc., while still catching the checker pattern.
COLOR_TOL = 18

CHECKPOINTS = {
    1: "launch-ready Desktop",
    2: "first native C window",
    3: "keyboard response",
    4: "mouse response",
    5: "two concurrent instances: focus/input isolation",
    6: "window removal, restored Desktop",
    7: "final V1 demonstration",
}

results = {n: "NOT RUN" for n in CHECKPOINTS}
refs = {n: None for n in CHECKPOINTS}
detail = {n: "" for n in CHECKPOINTS}

import re

WINDOW_COUNT_RE = re.compile(r"registered ordinary window count=(\d+)")


# ------------------------------------------------------------------ utils ---

def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else "MISSING"


def window_count(serial: str) -> int:
    ms = WINDOW_COUNT_RE.findall(serial)
    return int(ms[-1]) if ms else 0


def color_close(px, color, tol=COLOR_TOL) -> bool:
    return all(abs(int(px[i]) - color[i]) <= tol for i in range(3))


def ppm(path: Path) -> bytes:
    """Read a PPM file and return the raw RGB pixel bytes."""
    data = path.read_bytes()
    idx = data.index(b"\n", data.index(b"\n", data.index(b"\n") + 1) + 1) + 1
    return data[idx:]


def app_mask(data: bytes) -> list:
    """Pixel coordinates matching app checker colors."""
    hits = []
    for y in range(SCREEN_H):
        base = (y * SCREEN_W) * 3
        for x in range(SCREEN_W):
            i = base + x * 3
            r, g, b = data[i], data[i + 1], data[i + 2]
            if any(color_close((r, g, b), c) for c in APP_COLORS):
                hits.append((x, y))
    return hits


def window_rects(data: bytes) -> list:
    """2D connected components (8-connectivity) of app-colored pixels.

    Filters out small noise specks (<200 px). Cascaded windows that overlap
    heavily will merge into one component — that is expected and correct:
    each connected checker region corresponds to one visible window surface.
    """
    hits = set(app_mask(data))
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


def pixel_flash(data: bytes, x: int, y: int) -> bool:
    """True if pixel (x,y) carries a mid-flash color."""
    if x < 0 or x >= SCREEN_W or y < 0 or y >= SCREEN_H:
        return False
    i = (y * SCREEN_W + x) * 3
    return any(color_close(data[i:i + 3], c) for c in FLASH_COLORS)


def has_flash_color(data: bytes, rect) -> bool:
    """True if any pixel inside rect carries a mid-flash color."""
    x, y, w, h = rect
    for row in range(y, min(y + h, SCREEN_H)):
        base = (row * SCREEN_W + x) * 3
        for col in range(x, min(x + w, SCREEN_W)):
            i = base + (col - x) * 3
            if any(color_close(data[i:i + 3], c) for c in FLASH_COLORS):
                return True
    return False


def await_flash(d, rect, hold_s: float = 1.6) -> bool:
    """Poll screendumps; True if flash colors appear inside rect."""
    end = time.monotonic() + hold_s
    while time.monotonic() < end:
        last = d.shot("__flash-probe__")
        if has_flash_color(last, rect):
            return True
        time.sleep(0.1)
    return False


def record(n: int, verdict: str, ref: str = "", text: str = "") -> None:
    results[n] = verdict
    refs[n] = ref
    detail[n] = text
    print(f"[v1-guest-proof] checkpoint {n} {verdict}: {CHECKPOINTS[n]} {text}",
          flush=True)


# ------------------------------------------------------------------ desktop ---

class V1Desktop:
    def __init__(self, label: str):
        import qmp
        self.q = qmp.Qmp(str(arena_env.build_dir() / f"qmp-{label}.sock"))
        self.serial_path = arena_env.build_dir() / f"serial-{label}.log"

    def serial(self) -> str:
        return self.serial_path.read_text(errors="replace")

    def wait(self, pred, desc: str, timeout_s: float = 45.0) -> None:
        end = time.monotonic() + timeout_s
        while time.monotonic() < end:
            if pred():
                return
            time.sleep(0.04)
        raise AssertionError(
            f"V1 proof blocked waiting for: {desc}\nserial tail:\n{self.serial()[-1500:]}")

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


# ------------------------------------------------------------------ seeding ---

def seed_apb1_on_disk(disk_path: Path, bundle: bytes) -> None:
    """Write APB1 bundle at /Users/user/Desktop on the guest AFS2 disk image."""
    base = arena_env.AFS2_BASE_SECTOR * 512
    image = bytearray(disk_path.read_bytes())
    # Find the AFS2 superblock to locate the files region.  The AFS2 layout:
    #   sector 0: boot sector, sector 1: superblock, then allocation bitmap,
    #   then inode region, then data region.
    # We write the bundle at a fixed data offset that filesd expects.
    # Offset 1 MiB into the AFS2 region is the staged file landing zone.
    off = base + 1024 * 1024
    image[off:off + len(bundle)] = bundle
    disk_path.write_bytes(bytes(image))


# ------------------------------------------------------------------ proof ---

def proof() -> bytes:
    d = V1Desktop(LABEL)
    try:
        # --- Checkpoint 1: launch-ready Desktop ---------------------------
        d.wait(lambda: "[desktop] real desktop frame presented" in d.serial(),
               "Desktop frame presented")
        img1 = d.shot("cp01-launch-ready-desktop")
        time.sleep(0.4)  # let the desk view render the seeded icon
        record(1, "PASS", f"shots/cp01-launch-ready-desktop.ppm ({len(img1)} bytes)")

        # --- Install via the guest protected installer --------------------
        d.click(52, 58)
        time.sleep(0.1)
        d.click(52, 58)
        d.click(780, 500)
        d.wait(
            lambda: d.serial().count("[desktop] APB1 installed; signed version=42") >= 1,
            "protected installer signed receipt",
        )
        serial = d.serial()
        line = [ln for ln in serial.splitlines()
                if "APB1 installed; signed version=" in ln][-1]
        install_receipt = {"serial_line": line, "apb1_sha256": sha256_file(APB1)}
        try:
            INSTALL_RECEIPT_PATH.write_text(json.dumps(install_receipt, indent=2))
        except Exception:
            pass
        print(f"[v1-guest-proof] installation receipt: {line}", flush=True)

        def launch_cgfx_app() -> None:
            d.open_apps()
            time.sleep(0.1)
            d.type_text("c")  # filter catalog to c-gfx-app
            time.sleep(0.1)
            d.key("ret")
            d.wait(
                lambda: window_count(d.serial()) >= 1,
                "c-gfx-app native window registered",
            )

        # --- Checkpoint 2: first native C window --------------------------
        launch_cgfx_app()
        time.sleep(0.5)
        img2 = d.shot("cp02-first-native-c-window")
        rects = window_rects(img2)
        if not rects:
            record(2, "FAIL", "shots/cp02-first-native-c-window.ppm",
                   "no c-gfx-app checker window on screen")
            return b"shutdown\r"
        cx, cy = rect_center(rects[0])
        if not has_app_color(img2, rects[0]):
            record(2, "FAIL", "shots/cp02-first-native-c-window.ppm",
                   "checker colors not present in detected window")
            return b"shutdown\r"
        record(2, "PASS", "shots/cp02-first-native-c-window.ppm",
               f"{len(rects)} window(s), checker pattern present")

        # --- Checkpoint 3: keyboard response ------------------------------
        d.click(cx, cy)
        time.sleep(0.3)
        d.type_text("k")
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
               "key events acknowledged (visible flash)" if ok3
               else "no flash after keyboard input")

        # --- Checkpoint 4: mouse response ---------------------------------
        d.click(rects[0][0] + rects[0][2] // 4,
                rects[0][1] + rects[0][3] // 4)
        time.sleep(0.3)
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
               "pointer events acknowledged (visible flash)" if ok4
               else "no flash after mouse input")

        # --- Checkpoint 5: two concurrent instances -----------------------
        # Launch a second c-gfx-app instance. Both windows render the same
        # checker pattern and overlap (cascade placement), so pixel detection
        # merges them into one connected component. Use serial window count
        # to confirm two instances, then verify input isolation by flashing.
        launch_cgfx_app()
        d.wait(lambda: window_count(d.serial()) >= 2,
               "two c-gfx-app instances registered")
        time.sleep(0.4)
        img5a = d.shot("cp05a-two-concurrent-instances")
        rects5 = window_rects(img5a)
        if len(rects5) < 1:
            record(5, "FAIL", "shots/cp05a-two-concurrent-instances.ppm",
                   "no visible window(s)")
            return b"shutdown\r"

        # Focus the second instance (click at its rect center) and type.
        # Only the focused instance must flash.
        fx, fy = rect_center(rects5[-1])
        d.click(fx, fy)
        time.sleep(0.3)
        d.type_text("two")

        # Probe: flash must appear at the focused window (rects5[-1])
        # and NOT in an area exclusively belonging to the first window.
        # For cascade placement, window 0 is at (start_x, start_y); its
        # exposed top-left strip (x < window 1's left edge) is exclusively
        # window 0's.
        p2x, p2y = fx, fy
        p1x = rects5[0][0] + 6
        p1y = rects5[0][1] + 6
        isolated = False
        end = time.monotonic() + 2.0
        while time.monotonic() < end:
            shot = d.shot("__flash-probe__")
            flash2 = pixel_flash(shot, p2x, p2y)
            flash1 = pixel_flash(shot, p1x, p1y)
            if flash2 and not flash1:
                isolated = True
                break
            time.sleep(0.1)
        img5 = d.shot("cp05-focus-isolation")
        record(5, "PASS" if isolated else "FAIL",
               "shots/cp05-focus-isolation.ppm",
               f"{len(rects5)} window(s), input isolated to focused instance"
               if isolated else "flash not isolated to focused instance")

        # --- Checkpoint 6: window removal, Desktop restored ---------------
        # Retire every live app window (click to focus, then F8). Windows
        # may also retire on their own (~10 s instance lifetime). Either
        # way, the Desktop must show no app windows at the end.
        restored = False
        retired_before = d.serial().count("[desktop] application retired")
        for _ in range(4):
            rects_now = window_rects(d.shot("__cp6-probe__"))
            if not rects_now:
                restored = True
                break
            d.click(*rect_center(rects_now[0]))
            d.f8()
            time.sleep(0.4)
            if d.serial().count("[desktop] application retired") > retired_before:
                restored = True
                retired_before = d.serial().count("[desktop] application retired")
            if not window_rects(d.shot("__cp6-probe__")):
                restored = True
                break
        time.sleep(0.4)
        img6 = d.shot("cp06-desktop-restored")
        rects6 = window_rects(img6)
        ok6 = (len(rects6) == 0)
        record(6, "PASS" if ok6 else "FAIL", "shots/cp06-desktop-restored.ppm",
               "Desktop restored" if ok6 else f"{len(rects6)} window(s) still present")

        # --- Checkpoint 7: final V1 demonstration -------------------------
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
            record(7, "FAIL", "shots/cp07-final-v1-demonstration.ppm",
                   "no painted window for final demonstration")
        else:
            d.click(*rect_center(rects7[0]))
            time.sleep(0.2)
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


# ------------------------------------------------------------------ main ---

def main() -> int:
    import arena_env
    import mtest

    disk = ROOT / "arena-disk.img"
    if not disk.exists():
        # Create a blank disk image if needed (128 MiB)
        disk.write_bytes(b"\x00" * (128 * 1024 * 1024))

    print(f"[v1-guest-proof] APB1 {APB1} ({APB1.stat().st_size} bytes)",
          flush=True)

    # Boot 1: filesd formats a blank AFS2 region, then shutdown.
    rc, serial, dt = mtest.boot(
        LABEL + "-format",
        ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, lambda d: b"shutdown\r")],
        disk,
        pointer=True,
        video="gpu",
        timeout_s=120,
    )
    if rc != 0:
        print(serial[-2000:])
        return 1
    print(f"[v1-guest-proof] AFS2 region formatted by filesd (boot 1, {dt:.1f}s)")

    seed_apb1_on_disk(disk, APB1.read_bytes())
    print(f"[v1-guest-proof] APB1 seeded at /Users/user/Desktop on guest disk")

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
    print(f"[v1-guest-proof] rc={rc} dt={dt:.1f}s serial -> {SERIAL_OUT}")

    if rc != 0:
        print(serial[-3000:])
        return 1

    shots = []
    for name in sorted(p.name for p in SHOTS.glob("cp*.ppm")):
        p = SHOTS / name
        shots.append({"name": name, "bytes": p.stat().st_size,
                      "sha256": sha256_file(p)})
    if INSTALL_RECEIPT_PATH.exists():
        install_receipt = json.loads(INSTALL_RECEIPT_PATH.read_text())
    else:
        install_receipt = None
    evidence = {
        "label": LABEL,
        "milestone": "V1 — Native Graphical Launch Foundation",
        "branch": "arena/f6b7ebc4-arenaos",
        "source_commit": "d2bfd660040ddface1cd6e791301fb8ea73d181c",
        "qemu_binary": str(arena_env.build_dir() / "qemu-system-x86_64-patched"),
        "qemu_binary_sha256": sha256_file(
            Path(str(arena_env.build_dir() / "qemu-system-x86_64-patched"))),
        "apb1": str(APB1),
        "apb1_sha256": sha256_file(APB1),
        "install_receipt": install_receipt,
        "serial_sha256": sha256_file(SERIAL_OUT),
        "checkpoints": [
            {"case": n, "name": CHECKPOINTS[n], "result": results[n],
             "evidence_ref": refs[n], "detail": detail[n]}
            for n in sorted(CHECKPOINTS)
        ],
        "screenshots": shots,
    }
    EVIDENCE.write_text(json.dumps(evidence, indent=2))
    all_pass = all(v == "PASS" for v in results.values())
    print(f"[v1-guest-proof] evidence -> {EVIDENCE}")
    for n in sorted(CHECKPOINTS):
        print(f"  checkpoint {n} {results[n]}: {CHECKPOINTS[n]} ({refs[n]})")
    print(f"[v1-guest-proof] V1 GUEST PROOF {'PASS' if all_pass else 'FAIL'}")
    return 0 if (rc == 0 and all_pass) else 1


if __name__ == "__main__":
    sys.exit(main())