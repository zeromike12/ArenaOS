#!/usr/bin/env python3
"""V1 lifecycle cycles — repeated launch/close with fresh, attributed receipts.

Qualification item: "Repeated launch/close, including the previously specified
minimum of ten cycles" and "Bounded lifecycle and resource settlement".

One QEMU boot drives N cycles (default 10). Each cycle must produce, from
scratch:

  1. a NEW Identify receipt naming a surface object ID not currently live;
  2. a rendered native window on screen;
  3. a close through the window's own close button;
  4. an instance-specific `application retired: ... surface=S` receipt and a
     `child Process-cap exit surface=S status=0` receipt, both AFTER that
     cycle's cursor;
  5. visual removal of that instance's window;
  6. resource settlement back to the idle baseline recorded before cycle 1 —
     compared with the same `evaluate_cleanup` implementation the main proof
     uses (`tools/v1_guest_proof.py`), not a reimplementation.

Nothing here is allowed to pass on a global counter or a historical receipt:
each cycle captures its own cursor and fails on missing telemetry.
"""

import json
import os
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import arena_env  # noqa: E402
import mtest  # noqa: E402
import v1_guest_proof as proof  # noqa: E402
import afs1  # noqa: E402

LABEL = "v1-cycles"
CYCLES = int(os.environ.get("V1_CYCLES", "10"))

RUN_DIR = Path(mtest.arena_env.build_dir()) / "v1-cycles-run"
SHOTS = RUN_DIR / "shots"
SERIAL_OUT = RUN_DIR / "v1-cycles-serial.log"
EVIDENCE = RUN_DIR / "cycles-evidence.json"


def cycles(n: int) -> bytes:
    d = proof.V1Desktop(LABEL)
    proof.SHOTS = SHOTS
    SHOTS.mkdir(parents=True, exist_ok=True)
    records = []
    try:
        d.wait(lambda: "[desktop] real desktop frame presented" in d.serial(),
               "Desktop frame presented")
        baseline = proof.last_detail(d.serial())
        if baseline is None:
            records.append({"cycle": 0, "result": "FAIL",
                            "detail": "no idle resource baseline"})
            return b"shutdown\r"
        print(f"[v1-cycles] baseline app-instances={baseline['app-instances']} "
              f"windows={baseline['windows']}", flush=True)

        for cycle in range(1, n + 1):
            cursor = d.cursor()
            d.open_apps()
            time.sleep(0.1)
            d.type_text("c")
            time.sleep(0.1)
            d.key("ret")
            identified = d.wait_serial(proof.RE_IDENTIFY_OK.pattern,
                                       f"cycle {cycle} Identify", 40,
                                       start=cursor)
            entry = {"cycle": cycle, "result": "FAIL", "detail": ""}
            if not identified:
                entry["detail"] = "no Identify receipt"
                records.append(entry)
                break
            surface = [r["surface"] for r in
                       proof.parse_gfx_identities(d.serial())
                       if r["line"] >= cursor][-1]
            entry["surface"] = surface
            entry["identify_line"] = [r["line"] for r in
                                      proof.parse_gfx_identities(d.serial())
                                      if r["line"] >= cursor][-1]

            rects = []
            end = time.monotonic() + 8.0
            while time.monotonic() < end and not rects:
                rects = proof.window_rects(d.shot(f"cycle{cycle:02d}-open"))
                if not rects:
                    time.sleep(0.15)
            if not rects:
                entry["detail"] = "no rendered window"
                records.append(entry)
                break
            rect = rects[-1]
            entry["rect"] = rect
            d.shot(f"cycle{cycle:02d}-window")

            close_cursor = d.cursor()
            cx = rect[0] + rect[2] - proof.CLOSE_W // 2
            cy = rect[1] + proof.TITLE_H // 2
            d.click(cx, cy)
            end = time.monotonic() + 20.0
            cleanup = proof.evaluate_cleanup(d.serial(), close_cursor,
                                             [surface], baseline)
            while time.monotonic() < end and not cleanup["ok"]:
                time.sleep(0.25)
                cleanup = proof.evaluate_cleanup(d.serial(), close_cursor,
                                                 [surface], baseline)
            gone = False
            end = time.monotonic() + 4.0
            while time.monotonic() < end:
                if not proof.has_app_color(d.shot(f"cycle{cycle:02d}-closed"),
                                           rect):
                    gone = True
                    break
                time.sleep(0.1)
            d.shot(f"cycle{cycle:02d}-settled")
            entry["cleanup"] = {
                "retired": cleanup["per_instance"].get(str(surface), {}),
                "settlement": cleanup["settlement"],
                "matches_baseline": cleanup["settlement_matches_baseline"],
            }
            entry["visual_gone"] = gone
            entry["result"] = "PASS" if (cleanup["ok"] and gone) else "FAIL"
            entry["detail"] = (
                f"surface={surface} exited="
                f"{cleanup['per_instance'].get(str(surface), {}).get('exit_status')} "
                f"settled={cleanup['settlement']['app-instances'] if cleanup['settlement'] else None}/"
                f"{cleanup['settlement']['windows'] if cleanup['settlement'] else None} "
                f"visual_gone={gone}")
            records.append(entry)
            print(f"[v1-cycles] cycle {cycle}: {entry['result']} {entry['detail']}",
                  flush=True)
            if entry["result"] != "PASS":
                break
            time.sleep(0.4)
    finally:
        try:
            d.q.command("system_powerdown")
        except Exception:
            pass
    RUN_DIR.mkdir(parents=True, exist_ok=True)
    EVIDENCE.write_text(json.dumps({
        "label": LABEL,
        "cycles_requested": n,
        "baseline": baseline,
        "records": records,
        "pass": all(r["result"] == "PASS" for r in records) and len(records) == n,
    }, indent=2, default=str))
    return b"shutdown\r"


def main() -> int:
    RUND = Path(tempfile_dirname())
    RUND.mkdir(parents=True, exist_ok=True)
    disk = RUND / "arena-disk.img"
    if not disk.exists():
        tmp8 = RUND / "arena-disk-8m.img"
        tmp8.write_bytes(b"\x00" * (8 * 1024 * 1024))
        afs1.mkfs(tmp8, 8 * 1024 * 1024 // afs1.SECTOR)
        data8 = tmp8.read_bytes()
        with open(disk, "wb") as f:
            f.write(data8)
            f.write(b"\x00" * (72 * 1024 * 1024 - 8 * 1024 * 1024))
        tmp8.unlink()
    apb1 = RUND / "c-gfx-app.apb1"
    proof.APB1 = apb1
    proof._build_apb1(apb1)

    rc, serial, _dt = mtest.boot(
        LABEL + "-format", proof.ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, lambda: b"shutdown\r")],
        disk, pointer=True, video="gpu", timeout_s=180)
    if rc != 0:
        print(serial[-2000:], file=sys.stderr)
        return 1
    proof.stage_apb1_in_afs2(disk, apb1.read_bytes())

    mtest.DEFAULT_KEYS = []
    rc, serial, dt = mtest.boot(
        LABEL, proof.ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, lambda: cycles(CYCLES))],
        disk, pointer=True, video="gpu", timeout_s=900)
    SERIAL_OUT.write_text(serial)
    print(f"[v1-cycles] rc={rc} dt={dt:.1f}s serial -> {SERIAL_OUT}", flush=True)
    verdict = json.loads(EVIDENCE.read_text())
    verdict["serial_sha256"] = proof.sha256_file(SERIAL_OUT)
    verdict["rc"] = rc
    EVIDENCE.write_text(json.dumps(verdict, indent=2, default=str))
    passed = rc == 0 and verdict.get("pass")
    print(f"[v1-cycles] {len([r for r in verdict['records'] if r['result'] == 'PASS'])}"
          f"/{CYCLES} cycles PASS -> {'PASS' if passed else 'FAIL'}", flush=True)
    return 0 if passed else 1


def tempfile_dirname() -> str:
    return str(Path(mtest.arena_env.build_dir()) / "v1-cycles-run")


if __name__ == "__main__":
    sys.exit(main())
