#!/usr/bin/env python3
"""V1 guest refusal matrix — 24 classified cases executed inside ArenaOS.

The main proof (`tools/v1_guest_proof.py`) shows the positive path: a signed
application discovers its endpoint and surface, identifies itself, gets a
window, answers input and exits cleanly. This harness shows the *negative*
path the same way: a signed application, launched and installed through the
same protected installer and the same ordinary launch path, does everything
it can to be refused, and every refusal must happen for the documented
reason.

What runs in the guest (see experiments/c-runtime/app/c_gfx_refusals.c):

  11  descriptor-reordering independence   production discover() over
                                           reversed and rotated copies of the
                                           real verified record
  12  missing / ambiguous capabilities     production discover(), expecting
                                           its documented codes 2/3/4/5
  13  malformed frames, bad versions       the Desktop's production dispatch
                                           refusing magic/version/opcode/
                                           reserved-byte violations
  14  wrong rights                         production discover() with the
                                           endpoint's COPY bit and the
                                           surface's READ|WRITE rights removed
  15  stale / foreign identity             the production identity validator,
                                           fed a name one object ID off
  16  unexpected capability transfer       Identify carrying a real capability
                                           instead of CAP_NONE: the Desktop
                                           must refuse it AND dispose of it,
                                           and the client must still hold a
                                           working surface afterwards
  17  surface-layout boundaries            production client guard accepts
                                           exactly 301 pages and refuses 300;
                                           the actual kernel-granted region is
                                           checked separately, plus a real
                                           wrong-capability kernel refusal

Nothing privileged is added, no permission is weakened, and no product
syscall is introduced. A refusal is a *typed refusal*: the kernel delivered
the request and the Desktop answered with a non-zero status word — that is
distinguishable from a transport failure, and the client reports which one
it saw.

The verdict is the shared one in `v1_guest_proof.evaluate_refusal_matrix`,
so this file adds no second opinion about what a passing matrix means.

Usage:  python3 tools/v1_refusals.py
        V1_EVIDENCE_DIR=docs/v1/evidence/<label>/refusals python3 tools/v1_refusals.py
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
import afs1  # noqa: E402
import apb1_format  # noqa: E402
import mtest  # noqa: E402
import v1_guest_proof as proof  # noqa: E402

LABEL = "v1-refusals"
OUT = Path(os.environ.get("V1_EVIDENCE_DIR")
           or Path(mtest.arena_env.build_dir()) / "v1-refusals-run")
SERIAL_OUT = OUT / "refusals-serial.log"
EVIDENCE = OUT / "refusals-evidence.json"
APB1 = OUT / "c-gfx-refusals.apb1"


def _build_refusal_apb1(out: Path) -> Path:
    """Build the refusal client as a normal installable APB1 bundle."""
    sys.path.insert(0, str(REPO_ROOT / "experiments" / "c-runtime"))
    import run as crun

    cc = os.environ.get("V1_CC", "gcc")
    if cc not in ("gcc", "clang"):
        raise SystemExit(f"V1_CC must be gcc or clang, not {cc!r}")
    elfs = crun.build_guest(cc)
    # TEMPORARY diagnostic override (removed before the recorded run).
    # No override: the guest case must run the client the build system just
    # produced, otherwise a hand-placed ELF could stand in for a real launch.
    elf = elfs["c-gfx-refusals"]
    audit = crun.audit_elf(elf)
    if audit.get("loader_failures"):
        raise SystemExit(f"refusal client fails the loader audit: {audit}")
    disasm = crun.disassemble_check(elf)
    if disasm.get("simd_or_x87_count"):
        raise SystemExit(f"refusal client emits SIMD/x87: {disasm}")
    manifest = apb1_format.make_manifest(
        app_id=b"com.arena.refusals",
        package_id=b"org.arena.refusals",
        display_name=b"Refusal Matrix",
        version=7,
        # MULTI_INSTANCE | STANDARD_STREAMS | NATIVE_SYNC — the same flag set
        # the main V1 client runs with. The matrix needs a live session with
        # streams; nothing here depends on the single-instance behaviour.
        flags=1 | 8 | 16,
        requested=0,
        entry=b"bin/refusals",
        icon=b"",
        width=640,
        height=480,
        associations=(),
    )
    bundle = apb1_format.build_bundle(
        [(1, b"bin/refusals", elf.read_bytes())], manifest=manifest)
    parsed = apb1_format.parse_bundle(bundle)
    if [name for _k, name, _s, _d in parsed.files] != [b"bin/refusals"]:
        raise SystemExit("APB1 round-trip changed the payload list")
    out.write_bytes(bundle)
    print(f"[v1-refusals] built APB1 {out} ({len(bundle)} bytes, sha256 "
          f"{proof.sha256_file(out)}) with {cc} from ELF {elf} "
          f"(sha256 {proof.sha256_file(elf)})", flush=True)
    return out


REFUSAL_TIMEOUT_S = float(os.environ.get("V1_REFUSAL_TIMEOUT", "90"))


def refusals() -> bytes:
    d = proof.V1Desktop(LABEL)
    proof.SHOTS = OUT / "shots"
    proof.SHOTS.mkdir(parents=True, exist_ok=True)
    record = {"cases": {}, "notes": []}
    try:
        d.wait(lambda: "[desktop] real desktop frame presented" in d.serial(),
               "Desktop frame presented")
        before_detail, before_receipt = proof.wait_settle_snapshot(d.serial)
        record["before"] = {"detail": before_detail, "receipt": before_receipt}

        # Install through the guest's protected installer, exactly like the
        # main proof's application.
        d.click(52, 58)
        time.sleep(0.1)
        d.click(52, 58)
        d.click(780, 560)
        if not d.wait_serial(proof.RE_INSTALL_ACK.pattern,
                             "protected installer receipt", 60):
            record["fatal"] = "no install receipt for the refusal client"
            return b"shutdown\r"
        record["install_receipt"] = [
            ln for ln in d.serial().splitlines()
            if proof.RE_INSTALL_ACK.search(ln)][-1]

        cursor = d.cursor()
        d.open_apps()
        time.sleep(0.1)
        # The launcher filters by SUBSTRING and Enter launches the first
        # match, so a single "r" selects Terminal (it contains an "r") and
        # launches the wrong application. "Refusal" is unique to this app in
        # the installed catalog; the frame captured below is what proves the
        # right entry was highlighted before Enter.
        d.type_text("refusal")
        time.sleep(0.6)
        proof.save_frame("cp09-launcher-selection", d.shot("cp09-launcher-selection"))
        d.key("ret")
        if not d.wait_serial(r"\[refuse\] matrix complete",
                             "refusal matrix completion", REFUSAL_TIMEOUT_S,
                             start=cursor):
            record["fatal"] = (
                "refusal client never completed the matrix within "
                f"{REFUSAL_TIMEOUT_S:.0f}s of external supervision")
            return b"shutdown\r"
        time.sleep(0.3)
        record["screenshot"] = "shots/cp09-refusal-matrix.png"
        frame = d.shot("cp09-refusal-matrix")
        proof.save_frame("cp09-refusal-matrix", frame)
        record["screenshot_ppm_sha256"] = proof.hashlib.sha256(frame).hexdigest()

        verdict = proof.evaluate_refusal_matrix(d.serial(), cursor)
        record["cases"] = verdict["cases"]
        # Every case is published with the boundary it actually exercises, so
        # a mutated descriptor copy is never read as a kernel or IPC refusal.
        record["classes"] = verdict["classes"]
        record["unclassified"] = verdict["unclassified"]
        record["missing"] = verdict["missing"]
        record["failed"] = verdict["failed"]
        record["unexpected"] = verdict["unexpected"]

        # No unintended graphical state change: the client never creates a
        # window, so the Desktop's account of instances and windows must be
        # the same after it exits as before it started.
        d.wait_serial(r"\[desktop\] application retired", "refusal client exit",
                      30, start=cursor)
        time.sleep(0.6)
        after_detail, after_receipt = proof.wait_settle_snapshot(
            d.serial, cursor)
        record["after"] = {"detail": after_detail, "receipt": after_receipt}
        drift, warmup = proof.compare_resources(after_detail, after_receipt,
                                                before_detail, before_receipt)
        record["resource_drift"] = drift
        record["frame_warmup"] = warmup
        record["no_graphical_change"] = (
            after_detail is not None and after_detail["windows"] == 0
            and after_detail["app-instances"] == 0)
        record["settled"] = bool(drift == {})

        # Every refusal must have happened *before* the matrix completed, and
        # the client's own exit status is recorded by the Desktop.
        exits = [r for r in proof.parse_child_exits(d.serial())
                 if r["line"] >= cursor]
        record["exit_status"] = exits[-1]["status"] if exits else None
        record["verdict_ok"] = bool(
            verdict["ok"] and record["no_graphical_change"]
            and record["settled"] and record["exit_status"] == 0
            and not record["unclassified"])
    except Exception as exc:  # noqa: BLE001 - reported, never swallowed
        traceback.print_exc()
        record["fatal"] = f"{type(exc).__name__}: {exc}"
        record["verdict_ok"] = False
    finally:
        # Written here, not after the block: an early return for a failed
        # prerequisite must still leave the harness transcript and the
        # partial record on disk, or the failure has no evidence at all.
        record.setdefault("verdict_ok", False)
        OUT.mkdir(parents=True, exist_ok=True)
        SERIAL_OUT.write_text(d.serial())
        EVIDENCE.write_text(json.dumps({
            "label": LABEL,
            "kind": "guest_refusal_matrix",
            "pass": record.get("verdict_ok", False),
            "record": record,
            "source_commit": proof._source_commit(),
            "working_tree_dirty": proof._working_tree_dirty(),
            "dirty_paths_excluding_evidence": proof._source_dirty_paths(),
            "guest_compiler_under_test": os.environ.get("V1_CC", "gcc"),
            "apb1_sha256": proof.sha256_file(APB1) if APB1.exists() else "MISSING",
        }, indent=2, default=str))
        if record.get("fatal"):
            print(f"[v1-refusals] FATAL: {record['fatal']}", flush=True)
        try:
            d.q.command("system_powerdown")
        except Exception:
            pass
    return b"shutdown\r"


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    disk = OUT / "arena-disk.img"
    if not disk.exists():
        tmp8 = OUT / "arena-disk-8m.img"
        tmp8.write_bytes(b"\x00" * (8 * 1024 * 1024))
        afs1.mkfs(tmp8, 8 * 1024 * 1024 // afs1.SECTOR)
        data8 = tmp8.read_bytes()
        with open(disk, "wb") as f:
            f.write(data8)
            f.write(b"\x00" * (72 * 1024 * 1024 - 8 * 1024 * 1024))
        tmp8.unlink()

    _build_refusal_apb1(APB1)
    print(f"[v1-refusals] run directory: {OUT}", flush=True)

    # Boot 1 formats AFS2; boot 2 runs the matrix, exactly like the main
    # proof (the filesystem must be formatted by filesd before staging).
    rc, serial, _dt = mtest.boot(
        LABEL + "-format", proof.ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, lambda: b"shutdown\r")],
        disk, pointer=True, video="gpu", timeout_s=180)
    if rc != 0:
        print(serial[-2000:], file=sys.stderr)
        EVIDENCE.write_text(json.dumps({"label": LABEL, "pass": False,
                                        "fatal": "format boot failed"}))
        return 1
    proof.stage_apb1_in_afs2(disk, APB1.read_bytes())

    mtest.DEFAULT_KEYS = []
    rc, serial, dt = mtest.boot(
        LABEL, proof.ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, refusals)],
        disk, pointer=True, video="gpu", timeout_s=480)
    if not SERIAL_OUT.exists():
        SERIAL_OUT.write_text(serial)
    verdict = json.loads(EVIDENCE.read_text())
    verdict["serial_sha256"] = proof.sha256_file(SERIAL_OUT)
    verdict["rc"] = rc
    EVIDENCE.write_text(json.dumps(verdict, indent=2, default=str))
    passed = rc == 0 and verdict.get("pass")
    print(f"[v1-refusals] rc={rc} dt={dt:.1f}s cases="
          f"{len(verdict['record'].get('cases', {}))} -> "
          f"{'PASS' if passed else 'FAIL'}", flush=True)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
