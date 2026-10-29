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
import traceback
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import arena_env  # noqa: E402
import mtest  # noqa: E402
import v1_guest_proof as proof  # noqa: E402
import afs1  # noqa: E402

LABEL = "v1-cycles"
CYCLES = int(os.environ.get("V1_CYCLES", "10"))

# Filled in by main() from the per-invocation directory, so evidence never
# lands in a fixed path that a later run silently overwrites.
RUN_DIR = None
SHOTS = None
SERIAL_OUT = None
EVIDENCE = None


def cycles(n: int) -> bytes:
    global SHOTS
    d = proof.V1Desktop(LABEL)
    proof.SHOTS = SHOTS
    SHOTS.mkdir(parents=True, exist_ok=True)
    records = []
    baseline = None
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

        # Install the bundle through the guest's protected installer first:
        # the catalog only lists installed applications.
        cursor0 = d.cursor()
        d.click(52, 58)
        time.sleep(0.1)
        d.click(52, 58)
        d.click(780, 560)
        if not d.wait_serial(proof.RE_INSTALL_ACK.pattern,
                             "protected installer receipt", 60):
            # A reused image may already hold this exact signed version; the
            # protected installer then refuses a duplicate (PKG_CONFLICT) and
            # the application is already in the catalog. Only an application
            # that is genuinely absent is a failure.
            already = d.wait_serial(r"APPLICATION VERSION ALREADY INSTALLED|"
                                    r"installed application registry",
                                    5, start=cursor0)
            if not already:
                records.append({"cycle": 0, "result": "FAIL",
                                "detail": "no install receipt and no "
                                          "already-installed refusal"})
                return b"shutdown\r"
            print("[v1-cycles] WARN: bundle already installed in this image",
                  flush=True)
        print("[v1-cycles] APB1 installed by the guest protected installer",
              flush=True)

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

            # Authoritative geometry from the Desktop's placement receipt:
            # the title bar occupies its top TITLE_H rows, which is where this
            # window's own close button lives. A pixel bounding box would be
            # the content area and would point at the wrong row.
            placed = None
            end = time.monotonic() + 10.0
            while time.monotonic() < end and placed is None:
                placed = proof.placement_for(d.serial(), surface)
                if placed is None:
                    time.sleep(0.1)
            if placed is None:
                entry["detail"] = "no placement receipt for the window"
                records.append(entry)
                break
            rect = placed["rect"]
            entry["rect"] = rect
            # A placement receipt is emitted when the window manager adopts
            # the window, which is immediately BEFORE the application's first
            # present. Poll for the frame instead of assuming one screenshot
            # already contains it.
            content = (rect[0], rect[1] + proof.TITLE_H,
                       rect[2], rect[3] - proof.TITLE_H)
            rendered = False
            end = time.monotonic() + 10.0
            while time.monotonic() < end:
                if proof.has_app_color(d.shot(f"cycle{cycle:02d}-window"),
                                       content):
                    rendered = True
                    break
                time.sleep(0.15)
            if not rendered:
                entry["detail"] = "placed window never rendered"
                records.append(entry)
                break

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
    except Exception as exc:  # noqa: BLE001 - reported, never swallowed
        # A workflow exception (a raised wait, a missing fixture file) must
        # reach the transcript with its traceback; otherwise the only symptom
        # is a boot that ends early with a partial record list.
        traceback.print_exc()
        records.append({"cycle": 0, "result": "FAIL",
                        "detail": f"workflow error: {type(exc).__name__}: {exc}"})
    finally:
        try:
            d.q.command("system_powerdown")
        except Exception:
            pass
        # Evidence is written on EVERY exit path, including early failures:
        # a failed run must leave its records behind, not a stale file from a
        # previous run.
        RUN_DIR.mkdir(parents=True, exist_ok=True)
        EVIDENCE.write_text(json.dumps({
            "label": LABEL,
            "cycles_requested": n,
            "baseline": baseline,
            "records": records,
            "pass": (len(records) == n
                     and all(r["result"] == "PASS" for r in records)),
        }, indent=2, default=str))
    return b"shutdown\r"


def main() -> int:
    global RUN_DIR, SHOTS, SERIAL_OUT, EVIDENCE
    RUN_DIR = Path(tempfile_dirname())
    RUN_DIR.mkdir(parents=True, exist_ok=True)
    SHOTS = RUN_DIR / "shots"
    SERIAL_OUT = RUN_DIR / "v1-cycles-serial.log"
    EVIDENCE = RUN_DIR / "cycles-evidence.json"
    print(f"[v1-cycles] run directory: {RUN_DIR}", flush=True)
    disk = RUN_DIR / "arena-disk.img"
    if not disk.exists():
        tmp8 = RUN_DIR / "arena-disk-8m.img"
        tmp8.write_bytes(b"\x00" * (8 * 1024 * 1024))
        afs1.mkfs(tmp8, 8 * 1024 * 1024 // afs1.SECTOR)
        data8 = tmp8.read_bytes()
        with open(disk, "wb") as f:
            f.write(data8)
            f.write(b"\x00" * (72 * 1024 * 1024 - 8 * 1024 * 1024))
        tmp8.unlink()
    apb1 = RUN_DIR / "c-gfx-app.apb1"
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
    """Fresh per-invocation directory.

    Reusing one directory meant a later run inherited the previous run's disk
    image, in which the APB1 bundle was already installed — the protected
    installer then (correctly) refused the second install and the run had no
    application to launch. Every invocation therefore gets its own directory
    and therefore its own formatted disk.
    """
    stamp = os.environ.get("V1_RUN_STAMP") or time.strftime("%Y%m%dT%H%M%S")
    return str(Path(mtest.arena_env.build_dir()) / f"v1-cycles-run-{stamp}")


if __name__ == "__main__":
    sys.exit(main())
