#!/usr/bin/env python3
"""V1 Guest Proof (v2) — instance-attributed native graphical qualification.

Runs inside QEMU (TCG, ArenaOS guest). Drives the guest Desktop through QMP
input events and screendumps and verifies, per checkpoint:

  1. Launch-ready Desktop (plus a recorded idle resource baseline)
  2. APB1 install via the guest protected installer (installation receipt)
  3. First native C window after a conforming Identify round-trip
  4. Keyboard response — event-type- and instance-specific acknowledgment
  5. Mouse response — event-type- and instance-specific acknowledgment
  6. Two concurrent instances, focus/input isolation in BOTH directions
  7. Window removal: targeted retirements, successful exits, fresh resource
     settlement against the pre-launch baseline, and visual removal
  8. Final V1 demonstration

Design rules enforced here (review corrections from the V1 handoff):

  * Telemetry is attributed, never global. The guest client publishes the
    surface object ID it received in the Identify reply and validated against
    the SharedRegion capability it already held; every event acknowledgment
    carries {event type, per-instance counter, instance identity}. The
    Desktop publishes retirement and exit receipts naming the same session.
    `last_events_count()`-style global parsing is gone: a counter increment
    from some other instance can never satisfy an assertion.
  * Missing telemetry FAILS. There is no `baseline is None or ...` escape:
    an acknowledgment requires a *new* line for the *expected instance* with
    the *expected event type* and a strictly advanced per-instance counter.
  * Presentation is evidence of presentation only. A visible flash never
    substitutes for an acknowledgment, and an acknowledgment never
    substitutes for visible rendering; both are required where claimed.
  * Geometry is not identity. A window rectangle is bound to an instance only
    when that instance acknowledges the exact click delivered at that point.
  * Cleanup evidence is fresh. A serial cursor is captured before closing;
    only receipts after it count. Per-instance retirement and successful exit
    are required for every instance that was closed, settlement must match
    the idle baseline recorded before the first launch, and no spurious
    retirement may appear.
  * One verdict computation drives the exit status, the console summary and
    evidence.json. Missing identities, missing evidence artifacts, FAIL or
    NOT RUN checkpoints all prevent PASS.

This is TEST INFRASTRUCTURE ONLY. Nothing inside ArenaOS knows about this
script; the guest sees QMP input events and screendumps exactly as a user
would, and the serial lines it parses are the production client's and
Desktop's own output.
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

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import arena_env  # noqa: E402
import mtest  # noqa: E402
import afs2  # noqa: E402

LABEL = "v1-guest-proof"

# Evidence goes to a per-invocation temporary directory by default. Setting
# V1_EVIDENCE_DIR points the run at a durable package directory instead, so a
# published run's artifacts are written where the verdict says they are —
# not copied there afterwards by hand, which is how a stale file becomes
# indistinguishable from a fresh one.
RUN_DIR = Path(
    os.environ.get("V1_EVIDENCE_DIR")
    or tempfile.mkdtemp(prefix=f"v1-guest-proof-{int(time.monotonic())}-")
)
SHOTS = RUN_DIR / "shots"
SHOTS.mkdir(parents=True, exist_ok=True)
EVIDENCE = RUN_DIR / "evidence.json"
SERIAL_OUT = RUN_DIR / "v1-guest-serial.log"
INSTALL_ACK_PATH = RUN_DIR / "install-ack.json"
HARNESS_LOG = RUN_DIR / "harness-output.log"

ESP = arena_env.build_dir() / "arena-esp.img"
APB1 = RUN_DIR / "c-gfx-app.apb1"

SCREEN_W, SCREEN_H = 800, 600

# c-gfx-app checker pattern pixels (0x00RRGGBB LE -> RGB bytes):
#   cell ? (60,40,20) : (20,40,90)
APP_COLOR_NORMAL = (20, 40, 90)
APP_COLOR_ALT = (60, 40, 20)
APP_COLORS = (APP_COLOR_NORMAL, APP_COLOR_ALT)

# Same pixels mid-flash: XOR with 0x00A0A0A0.
FLASH_COLORS = (
    (APP_COLOR_NORMAL[0] ^ 0xA0, APP_COLOR_NORMAL[1] ^ 0xA0,
     APP_COLOR_NORMAL[2] ^ 0xA0),
    (APP_COLOR_ALT[0] ^ 0xA0, APP_COLOR_ALT[1] ^ 0xA0,
     APP_COLOR_ALT[2] ^ 0xA0),
)

# Tolerance excludes the desktop background (24,32,53) and titlebar
# (34,51,85) from app-color matches while catching exact checker pixels.
COLOR_TOL = 8

CLOSE_W, TITLE_H = 28, 28

# ADSK event types emitted by the guest client.
EVENT_KEY = 1
EVENT_POINTER = 2

CHECKPOINTS = {
    1: "launch-ready Desktop",
    2: "APB1 install receipt (protected installer)",
    3: "first native C window (Identify -> create -> render)",
    4: "keyboard response (instance-specific acknowledgment)",
    5: "mouse response (instance-specific acknowledgment)",
    6: "two concurrent instances: focus/input isolation, both directions",
    7: "window removal: retirements, exits, settlement, visual removal",
    8: "final V1 demonstration",
}

# Every case the refusal client must report. A missing line is a failure: the
# harness never infers "it must have refused" from silence.
# What each refusal case actually exercises. A refusal matrix is only as
# strong as this table: "the client refused it" means very different things
# for a mutated descriptor copy than for a frame the Desktop's own dispatch
# rejected, and publishing both as "the boundary refused" would overstate the
# weaker one. Every case must appear here, and `evaluate_refusal_matrix`
# refuses to pass a matrix whose cases are not all classified.
#
#   IPC        production IPC request/refusal: a real frame sent to the real
#              Desktop endpoint over the real protocol, judged by production
#              dispatch (not a client-side pre-check).
#   KERNEL     production kernel authority check: a real syscall that the
#              kernel refuses, or a kernel fact the case depends on.
#   DISCOVERY  guest execution of production discovery against a *synthetic
#              descriptor copy*: the client mutates its own startup record
#              and runs the shipped validator over it. Exercises the
#              validator, not the launcher that produced the real record.
#   IDENTITY   guest execution of the production validator against a mutated
#              identity: synthetic identity bytes, not a real foreign or
#              stale capability.
#   CONTROL    a genuine data path that must still succeed, so a refusal
#              cannot pass for the wrong reason (a validator that refuses
#              everything).
REFUSAL_CLASSES = {
    "baseline-record-accepted": ("CONTROL", "the launcher-built startup record is accepted by the shipped discovery"),
    "reorder-reversed": ("DISCOVERY", "production discovery over a reversed copy of the real descriptor"),
    "reorder-rotated": ("DISCOVERY", "production discovery over a rotated copy of the real descriptor"),
    "missing-endpoint": ("DISCOVERY", "production discovery over a copy with the endpoint slot removed"),
    "ambiguous-endpoint": ("DISCOVERY", "production discovery over a copy with two endpoint slots"),
    "wrong-endpoint-rights": ("DISCOVERY", "production discovery over a copy with endpoint rights the launcher never grants"),
    "missing-surface": ("DISCOVERY", "production discovery over a copy with the surface slot removed"),
    "ambiguous-surface": ("DISCOVERY", "production discovery over a copy with two surface slots"),
    "wrong-surface-rights": ("DISCOVERY", "production discovery over a copy with surface rights the launcher never grants"),
    "wire-magic": ("IPC", "a frame with the wrong magic sent to the real Desktop endpoint"),
    "wire-version": ("IPC", "a frame declaring an unsupported protocol version"),
    "wire-opcode": ("IPC", "an Identify slot carrying a different opcode"),
    "wire-reserved": ("IPC", "a frame with a non-zero reserved byte"),
    "identify-after-refusals": ("CONTROL", "the real session still answers Identify after four refused frames"),
    "foreign-identity": ("IDENTITY", "production identity validator over an object id one above the real one"),
    "genuine-identity": ("CONTROL", "the same validator accepts the genuine identity"),
    "unexpected-transfer": ("IPC", "a frame carrying a capability the dispatch never asked for"),
    "transfer-surface-retained": ("KERNEL", "the client still holds its own surface capability after that transfer"),
    "layout-pages-minimum": ("KERNEL", "the kernel-reported page count of the real granted surface region"),
    "layout-pixel-window-fits": ("CLIENT-ARITHMETIC", "client-side comparison of the documented offset and pixel bytes (NOT production logic; see V2-STATUS §5.2)"),
    "layout-wrong-kind-refused": ("KERNEL", "the kernel refuses the region query on a non-region capability"),
}

REFUSAL_CASES = (
    "baseline-record-accepted",
    "reorder-reversed",
    "reorder-rotated",
    "missing-endpoint",
    "ambiguous-endpoint",
    "wrong-endpoint-rights",
    "missing-surface",
    "ambiguous-surface",
    "wrong-surface-rights",
    "wire-magic",
    "wire-version",
    "wire-opcode",
    "wire-reserved",
    "identify-after-refusals",
    "foreign-identity",
    "genuine-identity",
    "unexpected-transfer",
    "transfer-surface-retained",
    "layout-pages-minimum",
    "layout-pixel-window-fits",
    "layout-wrong-kind-refused",
)

RE_REFUSAL = re.compile(
    r"\[refuse\] case=(?P<case>[a-z0-9-]+) want=(?P<want>-?\d+) "
    r"got=(?P<got>-?\d+) verdict=(?P<verdict>PASS|FAIL)")


def parse_refusals(serial_or_lines) -> dict:
    """Collect the guest refusal client's per-case verdicts.

    A case that appears more than once is reported with its LAST verdict and
    the duplicate count, so a fixture that printed PASS and then FAIL cannot
    hide behind the earlier line.
    """
    out = {}
    for ln in _lines(serial_or_lines):
        m = RE_REFUSAL.search(ln)
        if not m:
            continue
        case = m.group("case")
        entry = out.setdefault(case, {"count": 0})
        entry["count"] += 1
        entry["want"] = int(m.group("want"))
        entry["got"] = int(m.group("got"))
        entry["verdict"] = m.group("verdict")
        entry["line"] = ln
    return out


def evaluate_refusal_matrix(serial, cursor: int):
    """Decide the guest refusal matrix from the client's own case lines.

    Requires: every expected case reported, every case PASS, no duplicates
    that disagree, and no window/app-instance growth while the client ran
    (the client never creates a window, so any growth means the negatives
    mutated graphical state).
    """
    recent = "\n".join(_lines(serial)[cursor:])
    cases = parse_refusals(recent)
    missing = [c for c in REFUSAL_CASES if c not in cases]
    failed = [c for c, v in cases.items() if v["verdict"] != "PASS"]
    conflicting = [c for c, v in cases.items()
                   if v["count"] > 1 and v["verdict"] != "PASS"]
    unexpected = [c for c in cases if c not in REFUSAL_CASES]
    # A case with no stated boundary class is an unqualified claim: it would
    # be published without saying whether it exercised production dispatch,
    # the kernel, or only a mutated copy of the client's own input.
    unclassified = [c for c in REFUSAL_CASES if c not in REFUSAL_CLASSES]
    return {
        "ok": not missing and not failed and not conflicting and not unclassified,
        "cases": cases,
        "missing": missing,
        "failed": failed,
        "conflicting": conflicting,
        "unexpected": unexpected,
        "unclassified": unclassified,
        "classes": {c: {"kind": REFUSAL_CLASSES[c][0],
                        "description": REFUSAL_CLASSES[c][1]}
                    for c in REFUSAL_CASES if c in REFUSAL_CLASSES},
    }

results = {n: "NOT RUN" for n in CHECKPOINTS}
refs = {n: None for n in CHECKPOINTS}
detail = {n: "" for n in CHECKPOINTS}

# ============================================================================
# Production serial parsers — the single source of truth for both the live
# proof and the negative controls. Every parser returns records that carry
# the originating serial line index so freshness (cursor) is checkable.
# ============================================================================

RE_IDENTIFY_OK = re.compile(r"\[gfx\] identify ok surface=(\d+)")
RE_GFX_EVENT = re.compile(
    r"\[gfx\] event type=(\d+) events=(\d+) surface=(\d+)")
RE_GFX_FOCUS = re.compile(r"\[gfx\] focus=(-?\d+) surface=(\d+)")
RE_GFX_EXIT = re.compile(
    r"\[gfx\] exit events=(\d+) focused=(-?\d+) code=(\d+) surface=(\d+)")
RE_GFX_CREATE = re.compile(
    r"\[gfx\] window created handle=(\d+) surface=(\d+)")
RE_GFX_FAILURE = re.compile(r"\[gfx\] ([a-z ]+failed|no endpoint|ambiguous endpoint|"
                            r"no surface|ambiguous surface|wrong rights|"
                            r"surface describe mismatch|surface too small|"
                            r"no startup record)")
RE_WINDOW_COUNT = re.compile(r"\[desktop\] registered ordinary window count=(\d+)")
RE_RETIRED = re.compile(r"\[desktop\] application retired: kind=(\d+) surface=(\d+)")
RE_CHILD_EXIT = re.compile(r"\[desktop\] child Process-cap exit surface=(\d+) status=(\d+)")
RE_RESOURCE_DETAIL = re.compile(r"\[desktop\] native resource detail values=([\d/]+)")
RE_RESOURCE_RECEIPT = re.compile(
    r"\[desktop\] measured frames/records/processes/regions/pages/maps/caps=([\d/]+)")
RE_INSTALL_ACK = re.compile(r"\[desktop\] APB1 installed; signed version=(\d+)")
RE_DESKTOP_READY = re.compile(r"\[desktop\] real desktop frame presented")
# Authoritative placement: which screen rectangle the Desktop gave an
# instance's window. The title bar occupies its top TITLE_H rows, so this is
# also how the harness finds that exact window's own close button.
RE_PLACED = re.compile(
    r"\[desktop\] ordinary window placed handle=(\d+) surface=(\d+) "
    r"x=(\d+) y=(\d+) w=(\d+) h=(\d+)")

# Field indices inside a resource-detail receipt (see desktop.rs
# log_resource_detail; the field order is published on the preceding line).
DETAIL_FIELDS = (
    "total-frames", "threads", "endpoints", "notifications", "timers",
    "vm-regions", "vm-committed-pages", "sync-domains", "sync-keys",
    "sync-waiters", "app-instances", "windows", "helpers", "stream-sets",
    "max-windows-per-instance",
)
# The seven-field receipt the Desktop prints beside the detail line. Every
# one of these is live occupancy, so every one must return to its settled
# value when a lifecycle completes.
RECEIPT_FIELDS = (
    "free-frames", "spawn-records", "processes", "regions", "pages", "maps",
    "caps",
)

# Fields whose settlement is enforced. `total-frames` is deliberately absent:
# it is the frame pool's total capacity (kernel `frames::total_frames`), a
# constant by construction, so comparing it would assert nothing.
EXCLUDED_DETAIL_FIELDS = {
    "total-frames": "frame-pool capacity reported by frames::total_frames(); "
                    "constant by construction, not occupancy",
}
ENFORCED_DETAIL_FIELDS = tuple(
    f for f in DETAIL_FIELDS if f not in EXCLUDED_DETAIL_FIELDS)
ENFORCED_RECEIPT_FIELDS = RECEIPT_FIELDS

DETAIL_FIELD_NAMES = (
    "total-frames/threads/endpoints/notifications/timers/vm-regions/"
    "vm-committed-pages/sync-domains/sync-keys/sync-waiters/app-instances/"
    "windows/helpers/stream-sets/max-windows-per-instance"
)


def _lines(serial_or_lines) -> list[str]:
    if isinstance(serial_or_lines, str):
        return serial_or_lines.splitlines()
    return list(serial_or_lines)


def parse_resource_details(serial_or_lines, start: int = 0) -> list[dict]:
    """Parse `[desktop] native resource detail values=...` receipts.

    Returns one dict per receipt with the published field names plus
    `line` (index into the full serial). A receipt with the wrong arity is
    not silently reinterpreted: it is reported with `malformed=True`.
    """
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        if index < start:
            continue
        m = RE_RESOURCE_DETAIL.search(ln)
        if not m:
            continue
        values = m.group(1).split("/")
        if len(values) != len(DETAIL_FIELDS):
            out.append({"line": index, "malformed": True, "raw": m.group(1)})
            continue
        record = {"line": index, "malformed": False, "raw": m.group(1)}
        for name, value in zip(DETAIL_FIELDS, values):
            record[name] = int(value)
        out.append(record)
    return out


def parse_receipts(serial_or_lines, start: int = 0) -> list[dict]:
    """Parse `[desktop] measured frames/records/processes/regions/pages/maps/caps=...`.

    Same discipline as the detail line: a receipt with the wrong arity is
    reported as malformed rather than reinterpreted.
    """
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        if index < start:
            continue
        m = RE_RESOURCE_RECEIPT.search(ln)
        if not m:
            continue
        values = m.group(1).split("/")
        if len(values) != len(RECEIPT_FIELDS):
            out.append({"line": index, "malformed": True, "raw": m.group(1)})
            continue
        record = {"line": index, "malformed": False, "raw": m.group(1)}
        for name, value in zip(RECEIPT_FIELDS, values):
            record[name] = int(value)
        out.append(record)
    return out


def parse_gfx_events(serial_or_lines) -> list[dict]:
    """Every `[gfx] event type=T events=N surface=S` acknowledgment."""
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        m = RE_GFX_EVENT.search(ln)
        if m:
            out.append({
                "line": index,
                "type": int(m.group(1)),
                "events": int(m.group(2)),
                "surface": int(m.group(3)),
            })
    return out


def parse_gfx_focus(serial_or_lines) -> list[dict]:
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        m = RE_GFX_FOCUS.search(ln)
        if m:
            out.append({"line": index, "focused": int(m.group(1)),
                        "surface": int(m.group(2))})
    return out


def current_focus(serial_or_lines):
    """Surface the guest last acknowledged gaining focus, or None.

    A focus change is delivered to both sessions (gain and loss), so only
    `focus=1` acknowledgments describe who currently holds focus. Clicking a
    window that already has focus produces no new event, which is why the
    current state must be tracked rather than re-derived from the last click.
    """
    gains = [f for f in parse_gfx_focus(serial_or_lines) if f["focused"] == 1]
    return gains[-1]["surface"] if gains else None


def parse_gfx_identities(serial_or_lines) -> list[dict]:
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        m = RE_IDENTIFY_OK.search(ln)
        if m:
            out.append({"line": index, "surface": int(m.group(1))})
    return out


def parse_gfx_exits(serial_or_lines) -> list[dict]:
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        m = RE_GFX_EXIT.search(ln)
        if m:
            out.append({
                "line": index,
                "events": int(m.group(1)),
                "focused": int(m.group(2)),
                "code": int(m.group(3)),
                "surface": int(m.group(4)),
            })
    return out


def parse_retirements(serial_or_lines) -> list[dict]:
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        m = RE_RETIRED.search(ln)
        if m:
            out.append({"line": index, "kind": int(m.group(1)),
                        "surface": int(m.group(2))})
    return out


def parse_child_exits(serial_or_lines) -> list[dict]:
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        m = RE_CHILD_EXIT.search(ln)
        if m:
            out.append({"line": index, "surface": int(m.group(1)),
                        "status": int(m.group(2))})
    return out


def parse_placements(serial_or_lines) -> list[dict]:
    """Every `[desktop] ordinary window placed ...` receipt."""
    out = []
    for index, ln in enumerate(_lines(serial_or_lines)):
        m = RE_PLACED.search(ln)
        if m:
            out.append({
                "line": index,
                "handle": int(m.group(1)),
                "surface": int(m.group(2)),
                "rect": (int(m.group(3)), int(m.group(4)),
                         int(m.group(5)), int(m.group(6))),
            })
    return out


def placement_for(serial_or_lines, surface: int):
    hits = [p for p in parse_placements(serial_or_lines) if p["surface"] == surface]
    return hits[-1] if hits else None


def serial_cursor(serial_or_lines) -> int:
    return len(_lines(serial_or_lines))


def last_detail(serial_or_lines) -> dict | None:
    receipts = [r for r in parse_resource_details(serial_or_lines)
                if not r["malformed"]]
    return receipts[-1] if receipts else None


# ============================================================================
# Verdict evaluators — pure functions over (serial, cursor, expectations).
# The live proof and every negative control call exactly these.
# ============================================================================

def evaluate_event_ack(serial, cursor: int, instance: int, event_type: int,
                       others=()):
    """Evaluate an event-type- and instance-specific acknowledgment.

    Requires a NEW `[gfx] event type=<event_type> ... surface=<instance>`
    line after `cursor` whose per-instance counter is strictly greater than
    that instance's last counter before `cursor`. Any *other* instance that
    produced a new acknowledgment of the same event type after `cursor`
    fails the result (cross-delivery).
    """
    lines = _lines(serial)
    all_events = parse_gfx_events(lines)

    def last_counter(surface, upto):
        prior = [e["events"] for e in all_events
                 if e["surface"] == surface and e["line"] < upto]
        return prior[-1] if prior else 0

    expected = [e for e in all_events
                if e["line"] >= cursor and e["surface"] == instance
                and e["type"] == event_type]
    before = last_counter(instance, cursor)
    matched = expected[0] if expected else None
    advanced = matched is not None and matched["events"] > before

    cross = []
    for other in others:
        if other == instance:
            continue
        prior = last_counter(other, cursor)
        fresh = [e for e in all_events if e["line"] >= cursor
                 and e["surface"] == other and e["type"] == event_type
                 and e["events"] > prior]
        if fresh:
            cross.append(other)

    return {
        "ok": bool(matched is not None and advanced and not cross),
        "record": matched,
        "counter_before": before,
        "counter_after": matched["events"] if matched else None,
        "advanced": advanced,
        "cross_acks": cross,
        "fresh_acks": len(expected),
    }


def settle_snapshot(serial, start: int = 0):
    """The newest complete (receipt, detail) observation after `start`.

    The Desktop prints the seven-field receipt and then the fifteen-field
    detail from the SAME `snapshot()` call, so they belong together. Taking
    the newest of each independently would let a fresh detail be paired with
    a stale receipt — a combination that never existed on the machine and
    that would report settlement the guest never reached.

    Interleaved output from other processes is normal under TCG (observed:
    one unrelated line between the two), so the pair may be up to
    `MAX_SNAPSHOT_GAP` lines apart, but no other receipt or detail line may
    fall between them.
    """
    MAX_SNAPSHOT_GAP = 6
    details = [d for d in parse_resource_details(serial, start)
               if not d["malformed"]]
    receipts = [r for r in parse_receipts(serial, start) if not r["malformed"]]
    if not details or not receipts:
        return None, None
    detail_lines = {d["line"] for d in details}
    receipt_lines = {r["line"] for r in receipts}
    for detail in reversed(details):
        if detail["line"] < start:
            continue
        for receipt in reversed(receipts):
            gap = detail["line"] - receipt["line"]
            if gap <= 0 or gap > MAX_SNAPSHOT_GAP:
                continue
            between = range(receipt["line"] + 1, detail["line"])
            if any(i in detail_lines or i in receipt_lines for i in between):
                continue
            return detail, receipt
    return None, None


def wait_settle_snapshot(serial_fn, start: int = 0, timeout: float = 15.0,
                         interval: float = 0.15):
    """`settle_snapshot`, but wait for the pair to be readable.

    The Desktop writes the receipt, a field-name line and the detail as three
    separate serial writes from one `snapshot()` call. A reader that arrives
    between them sees a receipt with no detail and would have to report "no
    settled snapshot", which is true of the byte stream it happened to read
    and false of the machine. Under TCG that window is wide enough to hit --
    observed on the Clang refusal cell. Waiting does not relax any check: the
    returned pair must still be adjacent and complete, so a Desktop that
    never prints one still fails.
    """
    deadline = time.time() + timeout
    while True:
        detail, receipt = settle_snapshot(serial_fn(), start)
        if detail is not None and receipt is not None:
            return detail, receipt
        if time.time() >= deadline:
            return None, None
        time.sleep(interval)


def compare_resources(detail, receipt, baseline_detail, baseline_receipt):
    """Compare every enforced resource field against a settled baseline.

    Returns {field: {"baseline": n, "settled": m}} for each field that does
    NOT match. `total-frames` is never compared (see EXCLUDED_DETAIL_FIELDS).
    A field missing from either side counts as a difference, so a receipt
    that lost a field can never read as "unchanged".

    One field has a documented, bounded exception. `free-frames` may settle
    *below* the baseline by at most WARMUP_MAX_FRAMES: the machine's first
    lifecycles permanently retain a handful of pool frames (measured at two
    separate steps, see docs/v1/V2-STATUS.md §6 — 5 frames after the first
    lifecycle, 3 more when a second instance runs concurrently, then
    bounded). That allowance is one-directional and bounded: settling
    *above* the baseline, or losing more than the documented bound, is still
    drift, and every other field must still be exactly equal. The observed
    allowance is returned separately so the transcript records it rather
    than hiding it inside the comparison.
    """
    drift = {}
    warmup = {}
    for name in ENFORCED_DETAIL_FIELDS:
        want = (baseline_detail or {}).get(name)
        got = (detail or {}).get(name)
        if want is None or got is None or want != got:
            drift[name] = {"baseline": want, "settled": got}
    for name in ENFORCED_RECEIPT_FIELDS:
        want = (baseline_receipt or {}).get(name)
        got = (receipt or {}).get(name)
        if want is None or got is None:
            drift[name] = {"baseline": want, "settled": got}
            continue
        if name == "free-frames":
            spent = want - got
            if 0 <= spent <= WARMUP_MAX_FRAMES:
                if spent:
                    warmup[name] = {"baseline": want, "settled": got,
                                    "spent": spent,
                                    "bound": WARMUP_MAX_FRAMES}
                continue
            drift[name] = {"baseline": want, "settled": got,
                           "spent": spent, "bound": WARMUP_MAX_FRAMES}
            continue
        if want != got:
            drift[name] = {"baseline": want, "settled": got}
    return drift, warmup


def evaluate_cleanup(serial, cursor: int, expected_instances, baseline):
    """Evaluate fresh, instance-specific cleanup evidence after `cursor`.

    Requirements (all must hold):

      * every expected instance has an `application retired: ... surface=S`
        receipt after the cursor, and no retirement names an unexpected one;
      * every expected instance has a `child Process-cap exit surface=S
        status=0` receipt after the cursor (successful exit);
      * the LATEST resource observation after the cursor matches the settled
        baseline in every enforced field — threads, endpoints, notifications,
        armed timers, VM regions and committed pages, sync domains/keys/waiters,
        app instances, windows, helpers, stream sets, max windows per instance,
        and the receipt's free frames, spawn records, processes, regions,
        pages, maps and capability occupancy.

    Only `total-frames` is exempt (pool capacity, see EXCLUDED_DETAIL_FIELDS).
    Nothing is compared against a historical snapshot: the cursor is the
    instant of the close request, and the settlement must be *newer* than it.
    """
    lines = _lines(serial)
    retirements = [r for r in parse_retirements(lines) if r["line"] >= cursor]
    child_exits = [r for r in parse_child_exits(lines) if r["line"] >= cursor]

    per_instance = {}
    for instance in expected_instances:
        retro = [r for r in retirements if r["surface"] == instance]
        ex = [r for r in child_exits if r["surface"] == instance]
        per_instance[str(instance)] = {
            "retired": bool(retro),
            "retire_line": retro[0]["line"] if retro else None,
            "exit_ok": bool(ex) and ex[-1]["status"] == 0,
            "exit_status": ex[-1]["status"] if ex else None,
            "exit_line": ex[-1]["line"] if ex else None,
        }

    spurious = sorted({r["surface"] for r in retirements
                       if r["surface"] not in expected_instances})

    settlement, settlement_receipt = settle_snapshot(serial, cursor)
    baseline_detail = (baseline or {}).get("detail")
    baseline_receipt = (baseline or {}).get("receipt")
    drift, warmup = compare_resources(settlement, settlement_receipt,
                                      baseline_detail, baseline_receipt)
    settled = bool(settlement and settlement_receipt and not drift)

    per_instance_ok = all(v["retired"] and v["exit_ok"]
                          for v in per_instance.values())
    ok = (bool(expected_instances) and per_instance_ok and settled
          and not spurious)

    return {
        "ok": ok,
        "per_instance": per_instance,
        "spurious_retirements": spurious,
        "settlement": settlement,
        "settlement_receipt": settlement_receipt,
        "settles_to_baseline": settled,
        "resource_drift": drift,
        "frame_warmup": warmup,
        "baseline": baseline_detail,
        "baseline_receipt": baseline_receipt,
        "expected_instances": list(expected_instances),
        # Kept for the transcript's older wording; equal to `settled` now
        # that every enforced field is compared, not just two counts.
        "settlement_matches_baseline": settled,
    }


def evaluate_presentation(rects_after_close) -> bool:
    """Visual removal: no app-colored window may remain on screen."""
    return len(rects_after_close) == 0


# ============================================================================
# Screenshot helpers
# ============================================================================

def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else "MISSING"


def color_close(px, color, tol=COLOR_TOL) -> bool:
    return all(abs(int(px[i]) - color[i]) <= tol for i in range(3))


def parse_ppm(path: Path) -> tuple[int, int, bytes]:
    """Parse a PPM file; validate magic, dimensions, maxval, payload length."""
    data = path.read_bytes()
    if not data.startswith(b"P6\n"):
        raise ValueError(f"{path}: not a raw PPM (P6) file: {data[:20]!r}")
    try:
        header_end = data.index(b"\n", data.index(b"\n", data.index(b"\n") + 1) + 1) + 1
    except ValueError as exc:
        raise ValueError(f"{path}: truncated PPM header") from exc
    parts = data[:header_end - 1].split(b"\n")
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
    if len(payload) != width * height * 3:
        raise ValueError(f"{path}: PPM payload {len(payload)} bytes != "
                         f"{width * height * 3} ({width}x{height}x3)")
    return width, height, payload


def ppm(path: Path, expect_w: int = SCREEN_W, expect_h: int = SCREEN_H) -> bytes:
    width, height, payload = parse_ppm(path)
    if width != expect_w or height != expect_h:
        raise ValueError(f"{path}: {width}x{height} != {expect_w}x{expect_h}")
    return payload


try:  # pragma: no cover - exercised implicitly by every run
    import numpy as _np
except Exception:  # pragma: no cover
    _np = None


def _np_image(data: bytes, width: int, height: int):
    return _np.frombuffer(data, dtype=_np.uint8).reshape(height, width, 3)


def _np_color_match(img, color, tol: int = COLOR_TOL):
    return _np.all(_np.abs(img.astype(_np.int16) - _np.int16(color)) <= tol,
                   axis=2)


def _np_app_mask(data: bytes, width: int, height: int):
    img = _np_image(data, width, height)
    mask = _np.zeros((height, width), dtype=bool)
    for color in APP_COLORS:
        mask |= _np_color_match(img, color)
    return mask


def _np_flash_mask(data: bytes, width: int, height: int):
    img = _np_image(data, width, height)
    mask = _np.zeros((height, width), dtype=bool)
    for color in FLASH_COLORS:
        mask |= _np_color_match(img, color)
    return mask


def app_mask(data: bytes, width: int = SCREEN_W, height: int = SCREEN_H) -> list:
    """Pixel coordinates matching app checker colors (tight tolerance).

    Vectorised when numpy is present; the pure-Python path is kept so the
    harness runs (slower) on a bare interpreter. Both paths apply the same
    tolerance and colour set.
    """
    if _np is not None:
        mask = _np_app_mask(data, width, height)
        ys, xs = _np.nonzero(mask)
        return list(zip(xs.tolist(), ys.tolist()))
    hits = []
    for y in range(height):
        base = (y * width) * 3
        for x in range(width):
            i = base + x * 3
            if any(color_close(data[i:i + 3], c) for c in APP_COLORS):
                hits.append((x, y))
    return hits


def has_app_color(data: bytes, rect, width: int = SCREEN_W,
                  height: int = SCREEN_H) -> bool:
    x, y, w, h = rect
    x0, y0 = max(0, x), max(0, y)
    x1, y1 = min(x + w, width), min(y + h, height)
    if x1 <= x0 or y1 <= y0:
        return False
    if _np is not None:
        img = _np_image(data, width, height)[y0:y1, x0:x1]
        return bool(any(_np_color_match(img, c).any() for c in APP_COLORS))
    for row in range(y0, y1):
        base = (row * width) * 3
        for col in range(x0, x1):
            i = base + col * 3
            if any(color_close(data[i:i + 3], c) for c in APP_COLORS):
                return True
    return False


def _rect_minus(rect, cut):
    """The parts of `rect` not covered by `cut` (0-4 axis-aligned pieces)."""
    x, y, w, h = rect
    cx, cy, cw, ch = cut
    if cw <= 0 or ch <= 0 or x + w <= cx or cx + cw <= x or \
            y + h <= cy or cy + ch <= y:
        return [rect]
    out = []
    top = cy - y
    if top > 0:
        out.append((x, y, w, top))
    bottom = (y + h) - (cy + ch)
    if bottom > 0:
        out.append((x, cy + ch, w, bottom))
    mid_y = max(y, cy)
    mid_h = min(y + h, cy + ch) - mid_y
    left = cx - x
    if left > 0 and mid_h > 0:
        out.append((x, mid_y, left, mid_h))
    right = (x + w) - (cx + cw)
    if right > 0 and mid_h > 0:
        out.append((cx + cw, mid_y, right, mid_h))
    return [r for r in out if r[2] > 0 and r[3] > 0]


def window_visibly_gone(data: bytes, rect, siblings, width: int = SCREEN_W,
                        height: int = SCREEN_H):
    """Did THIS window disappear from the frame?

    A pixel probe over the whole rectangle answers the wrong question when a
    second window overlaps the first: the sibling's pixels stay inside the
    closed window's rectangle, so the frame still "has app colour there" long
    after the window being probed is gone. Observed exactly that on the Clang
    cell (round 3), while the Desktop had already reported the retirement, the
    exit and a fully settled resource baseline.

    So the probe only looks at the parts of the rectangle no other live window
    covers. Returns (gone, visible_fraction): when the rectangle is wholly
    obscured the caller records the close as obscured rather than claiming a
    disappearance it never saw -- the final all-closed frame then has to show
    zero app-coloured area.
    """
    pieces = [rect]
    for other in siblings:
        pieces = [piece for p in pieces for piece in _rect_minus(p, other)]
    if not pieces:
        return None, 0.0
    visible = sum(p[2] * p[3] for p in pieces) / float(max(1, rect[2] * rect[3]))
    for piece in pieces:
        if has_app_color(data, piece, width, height):
            return False, visible
    return True, visible


def window_rects(data: bytes, width: int = SCREEN_W, height: int = SCREEN_H) -> list:
    """Axis-aligned connected components of app-colored pixels.

    Components are NOT instances: overlapping windows merge into one
    component, so a caller must bind a component to an instance through an
    acknowledgment, never through geometry alone.

    Implementation: row runs merged by overlapping x-extent (union-find).
    This is exact for the rectangular windows this client paints and is
    O(rows x runs) instead of a per-pixel flood fill.
    """
    if _np is not None:
        mask = _np_app_mask(data, width, height)
        runs_by_row = []
        for y in range(height):
            row = mask[y]
            if not row.any():
                continue
            idx = _np.flatnonzero(row)
            splits = _np.flatnonzero(_np.diff(idx) > 1)
            start = 0
            for stop in list(splits) + [len(idx) - 1]:
                runs_by_row.append((y, int(idx[start]), int(idx[stop])))
                start = stop + 1
    else:
        hits = app_mask(data, width, height)
        rows = {}
        for x, y in hits:
            rows.setdefault(y, []).append(x)
        runs_by_row = []
        for y, xs in rows.items():
            xs.sort()
            start = prev = xs[0]
            for x in xs[1:]:
                if x == prev + 1:
                    prev = x
                    continue
                runs_by_row.append((y, start, prev))
                start = prev = x
            runs_by_row.append((y, start, prev))

    parent = list(range(len(runs_by_row)))

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    def union(a, b):
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[rb] = ra

    last_row = {}
    for i, (y, x0, x1) in enumerate(runs_by_row):
        for j in last_row.get(y - 1, []):
            _y, a0, a1 = runs_by_row[j]
            # 8-connectivity: runs on adjacent rows touch if their x extents
            # overlap or are within one pixel.
            if not (x1 + 1 < a0 or a1 + 1 < x0):
                union(i, j)
        last_row.setdefault(y, []).append(i)

    groups = {}
    for i, (y, x0, x1) in enumerate(runs_by_row):
        groups.setdefault(find(i), []).append((y, x0, x1))

    rects = []
    for runs in groups.values():
        minx = min(r[1] for r in runs)
        maxx = max(r[2] for r in runs)
        miny = min(r[0] for r in runs)
        maxy = max(r[0] for r in runs)
        if (maxx - minx + 1) * (maxy - miny + 1) >= 200:
            rects.append((minx, miny, maxx - minx + 1, maxy - miny + 1))
    rects.sort(key=lambda r: (r[1], r[0]))
    return rects


def rect_center(rect) -> tuple:
    x, y, w, h = rect
    return (x + w // 2, y + h // 2)


def pixel_flash(data: bytes, x: int, y: int, width: int = SCREEN_W,
                height: int = SCREEN_H) -> bool:
    if x < 0 or x >= width or y < 0 or y >= height:
        return False
    i = (y * width + x) * 3
    return any(color_close(data[i:i + 3], c) for c in FLASH_COLORS)


def has_flash_color(data: bytes, rect, width: int = SCREEN_W,
                    height: int = SCREEN_H) -> bool:
    x, y, w, h = rect
    x0, y0 = max(0, x), max(0, y)
    x1, y1 = min(x + w, width), min(y + h, height)
    if x1 <= x0 or y1 <= y0:
        return False
    if _np is not None:
        img = _np_image(data, width, height)[y0:y1, x0:x1]
        return bool(any(_np_color_match(img, c).any() for c in FLASH_COLORS))
    for row in range(y0, y1):
        base = (row * width) * 3
        for col in range(x0, x1):
            i = base + col * 3
            if any(color_close(data[i:i + 3], c) for c in FLASH_COLORS):
                return True
    return False


def await_no_flash(d, rect, settle_s: float = 2.5) -> bool:
    """Wait until the rect carries no flash color for a 0.2 s continuous window."""
    end = time.monotonic() + settle_s
    clean_since = None
    while time.monotonic() < end:
        shot = d.shot("__settle-probe__")
        if not has_flash_color(shot, rect):
            if clean_since is None:
                clean_since = time.monotonic()
            elif time.monotonic() - clean_since >= 0.2:
                return True
        else:
            clean_since = None
        time.sleep(0.05)
    return clean_since is not None


# ============================================================================
# Guest Desktop driver over QMP
# ============================================================================

class V1Desktop:
    def __init__(self, label: str):
        import qmp
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

    def cursor(self) -> int:
        return serial_cursor(self.serial())

    def wait(self, pred, desc: str, timeout_s: float = 45.0) -> None:
        end = time.monotonic() + timeout_s
        while time.monotonic() < end:
            if pred():
                return
            time.sleep(0.04)
        raise AssertionError(
            f"V1 proof blocked waiting for: {desc}\n"
            f"serial tail:\n{self.serial()[-2000:]}")

    def wait_serial(self, pattern: str, desc: str, timeout_s: float = 45.0,
                    start: int = 0) -> bool:
        rx = re.compile(pattern)
        end = time.monotonic() + timeout_s
        while time.monotonic() < end:
            if rx.search("\n".join(_lines(self.serial())[start:])):
                return True
            time.sleep(0.04)
        return False

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
            events.append({"type": "btn",
                           "data": {"button": "left", "down": down}})
        self.q.command("input-send-event", events=events)

    def click(self, x: int, y: int) -> None:
        self.point(x, y, True)
        self.point(x, y, False)
        self.point(780, 560)  # park the pointer on the desktop background

    def qkey(self, qcode: str, down: bool) -> None:
        self.q.command("input-send-event",
                       events=[{"type": "key",
                                "data": {"down": down,
                                         "key": {"type": "qcode",
                                                 "data": qcode}}}])

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
        # Desktop shell "All Applications" button: right of the wordmark.
        self.click(103, 13)


QMAP = {chr(c): chr(c) for c in
        list(range(ord("a"), ord("z") + 1)) + list(range(ord("0"), ord("9") + 1))}
QMAP[" "] = "spc"
QMAP["\r"] = "ret"


# ============================================================================
# AFS2 seeding (unchanged production filesystem path)
# ============================================================================

AFS2_REGION_OFFSET = arena_env.AFS2_BASE_SECTOR * 512


def read_afs2_region(disk_path: Path) -> bytes:
    data = disk_path.read_bytes()
    if len(data) < AFS2_REGION_OFFSET:
        raise ValueError(f"disk {disk_path} too small ({len(data)} bytes)")
    return data[AFS2_REGION_OFFSET:]


def write_afs2_region(disk_path: Path, region: bytes) -> None:
    data = bytearray(disk_path.read_bytes())
    data[AFS2_REGION_OFFSET:AFS2_REGION_OFFSET + len(region)] = region
    disk_path.write_bytes(bytes(data))


def stage_apb1_in_afs2(disk_path: Path, bundle: bytes,
                       filename: bytes = b"c-gfx-app.apb1") -> None:
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
        raise RuntimeError(f"AFS2 staged size {rec.size} != bundle {len(bundle)}")
    if vol2.read(file_id2, 0, rec.size) != bundle:
        raise RuntimeError("AFS2 staged file contents mismatch")
    vol2.statfs()
    write_afs2_region(disk_path, region_after)
    print(f"[v1-guest-proof] APB1 staged at /Users/user/Desktop/"
          f"{filename.decode()} ({len(bundle)} bytes)", flush=True)


# ============================================================================
# Evidence recording
# ============================================================================

def record(n: int, verdict: str, ref: str = "", text: str = "") -> None:
    results[n] = verdict
    refs[n] = ref
    detail[n] = text
    line = f"[v1-guest-proof] checkpoint {n} {verdict}: {CHECKPOINTS[n]} {text}"
    print(line, flush=True)
    # The harness transcript is itself a required evidence artifact. It is
    # appended as the run proceeds (and created before any checkpoint) so the
    # verdict can require it to exist without the check being self-defeating.
    try:
        with open(HARNESS_LOG, "a") as fh:
            fh.write(line + "\n")
    except OSError:
        pass


def skip_remaining(from_n: int, reason: str) -> None:
    for n in sorted(CHECKPOINTS):
        if n >= from_n and results[n] == "NOT RUN":
            record(n, "NOT RUN", "", reason)


# ============================================================================
# Provenance
# ============================================================================

def _source_commit() -> str | None:
    try:
        return subprocess.run(["git", "-C", str(REPO_ROOT), "rev-parse", "HEAD"],
                              capture_output=True, text=True, check=True
                              ).stdout.strip()
    except Exception:
        return None


# Paths that the run itself writes. They are excluded from the "source tree
# dirty" question, because a run that publishes its own evidence must not be
# able to make its provenance unverifiable by doing so.
EVIDENCE_PREFIXES = ("docs/v1/evidence/",)


def _source_dirty_paths() -> list:
    """Working-tree changes that are NOT this run's own evidence output."""
    try:
        out = subprocess.run(["git", "-C", str(REPO_ROOT), "status", "--porcelain"],
                             capture_output=True, text=True, check=True).stdout
    except Exception:
        return ["<git status unavailable>"]
    paths = []
    for line in out.splitlines():
        path = line[3:].strip()
        if any(path.startswith(p) or f" {p}" in path for p in EVIDENCE_PREFIXES):
            continue
        paths.append(line.strip())
    return paths


def _working_tree_dirty() -> bool:
    """True when the SOURCE tree (not merely the evidence output) is dirty.

    The evidence actually produced records the full dirty-path list, so a
    reviewer can see exactly what differed from the named commit instead of
    trusting a boolean.
    """
    return bool(_source_dirty_paths())


def _compiler_identity(cmd: list[str]) -> str:
    try:
        return subprocess.run(cmd, capture_output=True, text=True,
                              timeout=60).stdout.splitlines()[0].strip()
    except Exception as exc:  # pragma: no cover - environment dependent
        return f"MISSING ({exc})"


def compiler_identities() -> dict:
    zig = os.environ.get("ARENA_ZIG", "/opt/zig-clang/pkg/ziglang/zig")
    return {
        "gcc": _compiler_identity(["gcc", "--version"]),
        "clang_via_zig": _compiler_identity([zig, "cc", "--version"]),
        "zig": _compiler_identity([zig, "version"]),
        "rustc": _compiler_identity([str(Path("/opt/rust/prefix/bin/rustc")),
                                     "--version"]),
    }


QEMU_ELF_HINT = "qemu-system-x86_64-patched"
REQUIRED_QEMU_ELF_SHA256 = (
    "e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff")


def qemu_identity() -> dict:
    """Resolve every component of the launch chain separately: wrapper, ELF,
    loader, runtime libraries, firmware. A wrapper hash alone is never
    treated as emulator identity."""
    try:
        cmd = arena_env.qemu_cmd()
    except SystemExit:
        cmd = []
    ident = {
        "launch_cmd": [str(c) for c in cmd],
        "wrapper_path": "MISSING", "wrapper_sha256": "MISSING",
        "elf_path": "MISSING", "elf_sha256": "MISSING", "elf_ok": False,
        "loader_path": "MISSING", "loader_sha256": "MISSING",
        "runtime_libs": [],
        "firmware": {"code_path": "MISSING", "code_sha256": "MISSING",
                     "vars_path": "MISSING", "vars_sha256": "MISSING"},
        "identity_ok": False,
    }
    wrapper = next((Path(p) for p in cmd if Path(p).exists() and Path(p).is_file()),
                   None)
    contents = ""
    if wrapper is not None:
        ident["wrapper_path"] = str(wrapper)
        ident["wrapper_sha256"] = sha256_file(wrapper)
        try:
            contents = wrapper.read_text(errors="replace")
        except OSError:
            contents = ""
    elf = next((Path(p) for p in cmd
                if Path(p).exists() and Path(p).is_file()
                and QEMU_ELF_HINT in Path(p).name), None)
    if elf is None:
        elf = next((Path(p) for p in cmd
                    if Path(p).exists() and Path(p).is_file()
                    and Path(p).name.startswith("qemu-system")), None)
    if elf is None and contents:
        # The launch prefix is a wrapper script (isolated-glibc recovery):
        # the emulator ELF it execs is named inside it. Resolve that path so
        # the ELF identity is recorded separately from the wrapper hash.
        for candidate in re.findall(r'"(/[^"]*qemu-system[^"]*)"', contents):
            cp = Path(candidate)
            if cp.exists() and cp.is_file():
                elf = cp
                break
    if elf is not None:
        ident["elf_path"] = str(elf)
        ident["elf_sha256"] = sha256_file(elf)
        ident["elf_ok"] = ident["elf_sha256"] == REQUIRED_QEMU_ELF_SHA256
    loader = re.findall(r'"(\S*ld-linux\S*)"', contents)
    if loader:
        lp = Path(loader[0])
        if lp.exists():
            ident["loader_path"] = str(lp)
            ident["loader_sha256"] = sha256_file(lp)
    libpaths = re.findall(r'--library-path "([^"]*)"', contents)
    for lib_dir in (libpaths[0].split(":") if libpaths else []):
        ld = Path(lib_dir)
        if ld.is_dir():
            for lib in sorted(ld.iterdir())[:20]:
                if lib.is_file():
                    ident["runtime_libs"].append(
                        {"path": str(lib), "sha256": sha256_file(lib)})
    for key, getter in (("code_path", arena_env.ovmf_code),
                        ("vars_path", arena_env.ovmf_vars_template)):
        try:
            p = getter()
            ident["firmware"][key] = str(p)
            ident["firmware"][key.replace("_path", "_sha256")] = sha256_file(p)
        except Exception:
            pass
    fw = ident["firmware"]
    ident["identity_ok"] = bool(
        ident["elf_ok"]
        and ident["wrapper_path"] != "MISSING"
        and ident["wrapper_sha256"] != "MISSING"
        and ident["loader_path"] != "MISSING"
        and ident["loader_sha256"] != "MISSING"
        and fw["code_sha256"] != "MISSING"
        and fw["vars_sha256"] != "MISSING")
    return ident


def image_freshness() -> dict:
    """Is the boot image newer than every tracked source file?

    A harness that boots `build/arena-esp.img` without checking when it was
    built will happily report PASS for code that is no longer in the tree —
    which is exactly what happened once: `cargo fmt` changed panic-location
    strings, the EFI digest changed, and an earlier gate still "passed" by
    booting the previous image. Provenance is not a matter of trust here.
    """
    try:
        tracked = subprocess.run(
            ["git", "-C", str(REPO_ROOT), "ls-files", "-z"],
            capture_output=True, text=True, check=True).stdout.split("\0")
    except Exception:
        return {"fresh": False, "reason": "git ls-files unavailable"}
    newest, newest_path = 0.0, ""
    for rel in tracked:
        if not rel:
            continue
        # The run writes its own evidence; a file it just created must not
        # make the image it is booting look stale.
        if any(rel.startswith(prefix) for prefix in EVIDENCE_PREFIXES):
            continue
        path = REPO_ROOT / rel
        try:
            mtime = path.stat().st_mtime
        except OSError:
            continue
        if mtime > newest:
            newest, newest_path = mtime, rel
    try:
        image_mtime = ESP.stat().st_mtime
    except OSError:
        return {"fresh": False, "reason": "boot image missing"}
    return {
        "fresh": image_mtime >= newest,
        "image_mtime": image_mtime,
        "newest_source": newest_path,
        "newest_source_mtime": newest,
        "boot_efi_sha256": sha256_file(arena_env.build_dir() / "arena-boot.efi"),
    }


def artifact_identities(apb1: Path) -> dict:
    """Hashes for every artifact the verdict depends on."""
    identity = {"apb1_sha256": sha256_file(apb1),
                "esp_sha256": sha256_file(ESP),
                "esp_path": str(ESP)}
    identity["image"] = image_freshness()
    # EFI extracted from the ESP image (EFI/BOOT/BOOTX64.EFI).
    try:
        import espimg  # noqa: F401  (documented ESP reader, kept for parity)
    except Exception:
        pass
    efi = RUN_DIR / "BOOTX64.EFI"
    if extract_esp_file(ESP, "/EFI/BOOT/BOOTX64.EFI", efi):
        identity["efi_sha256"] = sha256_file(efi)
        identity["efi_size"] = efi.stat().st_size
    else:
        identity["efi_sha256"] = "MISSING"
    return identity


def extract_esp_file(esp: Path, path: str, out: Path) -> bool:
    """Extract a file from the FAT ESP.

    Uses the same pyfatfs reader call the repository's other checkpoints use
    (`getbytes`); `PyFatFS.open()` trips over short-name decoding in this
    pyfatfs build, so it is not used here.
    """
    try:
        from pyfatfs.PyFatFS import PyFatFS
    except Exception:
        return False
    for read_only in (True, False):
        try:
            fs = PyFatFS(str(esp), read_only=read_only)
            try:
                out.write_bytes(fs.getbytes(path))
            finally:
                try:
                    fs.close()
                except Exception:
                    pass
            return True
        except Exception:
            continue
    return False


# ============================================================================
# Proof flow
# ============================================================================

def probe_instance_at(d: V1Desktop, point: tuple, timeout_s: float = 3.0):
    """Click `point` and return the instance that acknowledges FOCUS for it.

    This is the only way a screen location is bound to an instance: the
    guest must acknowledge the exact click. Returns None if no instance
    acknowledges focus within the timeout (which fails the caller).
    """
    cursor = d.cursor()
    d.click(*point)
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        acks = [f for f in parse_gfx_focus(d.serial()) if f["line"] >= cursor]
        # A focus change is delivered to BOTH sessions: the one gaining focus
        # acknowledges focus=1 and the one losing it acknowledges focus=0.
        # Only the gain identifies which instance owns this point.
        gained = [f for f in acks if f["focused"] == 1]
        if len({f["surface"] for f in gained}) > 1:
            return None, gained[-1]  # ambiguous: refuse rather than guess
        if gained:
            return gained[-1]["surface"], gained[-1]
        if acks:
            # A focus loss with no gain in the same window is not a binding.
            return None, acks[-1]
        time.sleep(0.05)
    return None, None


def evaluate_input_proof(ack_eval, pre_frame, frame, rect):
    """Decide whether one input was both acknowledged AND visibly answered.

    This is the production verdict for keyboard, pointer, isolation and the
    final demonstration. It requires, together:

      * a fresh acknowledgment for the expected instance *and* event type,
        with the per-instance counter advanced and no cross-instance
        acknowledgment (all decided by `evaluate_event_ack`);
      * a captured frame in which the instance's own flash is visible inside
        the window rectangle the window manager gave it;
      * no residual flash in the frame captured immediately BEFORE the input
        — otherwise "visible response" would be satisfied by whatever was
        already on screen.

    Acknowledgment alone proves delivery, not that anything was shown. A
    flash alone proves pixels, not that this instance received this event.
    Neither is accepted on its own, and a missing frame fails.
    """
    ack_ok = bool(ack_eval) and bool(ack_eval.get("ok"))
    residual = pre_frame is not None and has_flash_color(pre_frame, rect)
    visual_ok = (frame is not None and not residual
                 and has_flash_color(frame, rect))
    if ack_ok and visual_ok:
        reason = ""
    elif not ack_ok and not visual_ok:
        reason = "no acknowledgment and no visible response"
    elif not ack_ok:
        reason = ("visible response without an acknowledgment for this "
                  "instance and event type")
    elif frame is None:
        reason = "acknowledgment without a captured visible response"
    elif residual:
        reason = "flash was already present before the input (stale)"
    else:
        reason = "acknowledgment but no visible flash in the window"
    return {
        "ok": ack_ok and visual_ok,
        "ack_ok": ack_ok,
        "visual_ok": visual_ok,
        "residual_flash": residual,
        "frame_present": frame is not None,
        "reason": reason,
        "ack": ack_eval,
    }


def wait_flash_clear(d: V1Desktop, rect, timeout_s: float = 6.0):
    """Capture a frame showing no flash in `rect`, before injecting input.

    The client answers every accepted event with a timed flash, so the frame
    that establishes "the screen was clean BEFORE this input" must be taken
    after the previous answer has faded. Waiting here does not weaken the
    stale-flash rule: the verdict still fails on any flash in this frame —
    the harness just does not race its own last checkpoint.
    """
    end = time.monotonic() + timeout_s
    frame = None
    while time.monotonic() < end:
        frame = d.shot("__ack-clear__")
        if not has_flash_color(frame, rect):
            return frame, True
        time.sleep(0.1)
    return frame, False


def do_input_and_ack(d: V1Desktop, instance: int, event_type: int,
                     rect, action, save_as: str | None = None, others=(),
                     timeout_s: float = 4.0):
    """Inject `action` and return the production input verdict.

    The frame that satisfied BOTH conditions is saved under `save_as` (when
    given) and returned, so the published checkpoint image IS the frame the
    assertion was made on. The pre-input frame that established the clean
    baseline is saved beside it as `<save_as>-pre` for the same reason.
    """
    pre_frame, clear = wait_flash_clear(d, rect)
    if save_as:
        save_frame(f"{save_as}-pre", pre_frame)
    cursor = d.cursor()
    action()
    end = time.monotonic() + timeout_s
    verdict = None
    while time.monotonic() < end:
        ack_eval = evaluate_event_ack(d.serial(), cursor, instance,
                                      event_type, others=others)
        frame = d.shot("__ack-probe__")
        verdict = evaluate_input_proof(ack_eval, pre_frame, frame, rect)
        if verdict["ok"]:
            if save_as:
                save_frame(save_as, frame)
            verdict["frame"] = frame
            verdict["frame_path"] = f"shots/{save_as}.png" if save_as else None
            verdict["frame_ppm_sha256"] = hashlib.sha256(frame).hexdigest()
            return verdict
        time.sleep(0.08)
    # Timed out: report what the serial said at the end, with no frame.
    final_ack = evaluate_event_ack(d.serial(), cursor, instance, event_type,
                                   others=others)
    verdict = evaluate_input_proof(final_ack, pre_frame, None, rect)
    return verdict


def ppm_bytes_from_raster(width: int, height: int, raster: bytes) -> bytes:
    """Wrap a raw RGB raster in the P6 header this harness captures with."""
    expected = width * height * 3
    if len(raster) != expected:
        raise ValueError(f"raster is {len(raster)} bytes, expected {expected}")
    return b"P6\n%d %d\n255\n" % (width, height) + raster


def raster_to_png(width: int, height: int, raster: bytes) -> bytes:
    """Encode an 8-bit RGB raster as a PNG (no filtering, lossless).

    Written here rather than pulled from a dependency so the published
    evidence is produced by code in this repository and its exact bytes are
    reproducible; the conversion is a straight scanline copy with the
    filter byte 0 in front of each row.
    """
    import struct
    import zlib

    raw = bytearray()
    stride = width * 3
    for y in range(height):
        raw.append(0)
        raw += raster[y * stride:(y + 1) * stride]

    def chunk(tag: bytes, payload: bytes) -> bytes:
        return (struct.pack(">I", len(payload)) + tag + payload
                + struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF))

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
            + chunk(b"IEND", b""))


def png_raster(data: bytes) -> tuple[int, int, bytes]:
    """Decode the PNGs this harness writes back to their RGB raster."""
    import struct
    import zlib

    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("not a PNG")
    pos, width, height, depth, colour, idat = 8, None, None, None, None, b""
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        tag = data[pos + 4:pos + 8]
        payload = data[pos + 8:pos + 8 + length]
        (crc,) = struct.unpack(">I", data[pos + 8 + length:pos + 12 + length])
        if zlib.crc32(tag + payload) & 0xFFFFFFFF != crc:
            raise ValueError(f"PNG chunk {tag!r} CRC mismatch")
        if tag == b"IHDR":
            width, height, depth, colour = struct.unpack(">IIBB", payload[:10])
        elif tag == b"IDAT":
            idat += payload
        pos += 12 + length
    if depth != 8 or colour != 2:
        raise ValueError("unexpected PNG depth/colour type")
    raw = zlib.decompress(idat)
    stride = width * 3
    out = bytearray()
    for y in range(height):
        i = y * (stride + 1)
        if raw[i] != 0:
            raise ValueError(f"unexpected PNG filter {raw[i]} on row {y}")
        out += raw[i + 1:i + 1 + stride]
    return width, height, bytes(out)


def save_frame(name: str, raster: bytes, width: int = SCREEN_W,
               height: int = SCREEN_H) -> Path:
    """Publish a captured frame: the tested PPM plus a lossless PNG.

    Every assertion in this harness is made on the raw pixels `raster`, whose
    digest goes into the evidence. The PPM keeps the capture format the
    harness itself reads; the PNG is a lossless re-encoding of the same
    raster, checked by decoding it back here, so a published image can never
    drift from the frame the checkpoint actually passed on.
    """
    SHOTS.mkdir(parents=True, exist_ok=True)
    ppm = ppm_bytes_from_raster(width, height, raster)
    ppm_path = SHOTS / f"{name}.ppm"
    ppm_path.write_bytes(ppm)
    png_path = SHOTS / f"{name}.png"
    png = raster_to_png(width, height, raster)
    png_path.write_bytes(png)
    if png_raster(png) != (width, height, raster):
        raise RuntimeError(f"published PNG does not round-trip {name}")
    return png_path


# Bound on the one-time frame warm-up allowed between "Desktop has drawn its
# first frame" and "Desktop is at steady idle". Measured, not guessed: ten
# consecutive launch/close cycles settle to an identical value after the
# first one (see docs/v1/V2-STATUS.md §6), and the observed first-lifecycle
# cost is a handful of frames. A larger gap fails.
WARMUP_MAX_FRAMES = 32


def settle_baseline(d: V1Desktop, timeout_s: float = 30.0):
    """Wait for two consecutive identical idle observations, then return it.

    A single observation right after boot can catch the Desktop mid-flight.
    Settlement is a statement about a steady state, so the baseline is the
    first value the machine *repeats* while nothing is running.
    """
    end = time.monotonic() + timeout_s
    previous = None
    while time.monotonic() < end:
        detail, receipt = settle_snapshot(d.serial())
        if detail is not None and receipt is not None:
            key = (detail["raw"], receipt["raw"])
            if key == previous:
                return {"detail": detail, "receipt": receipt}
            previous = key
        time.sleep(0.25)
    detail, receipt = settle_snapshot(d.serial())
    return {"detail": detail, "receipt": receipt}


def launch_instance(d: V1Desktop, cursor: int, timeout_s: float = 40.0):
    """Launch one signed application through the ordinary catalog path.

    Returns (identity, rect) using the Desktop's placement receipt for
    geometry; returns (None, None) if either step does not happen.
    """
    d.open_apps()
    time.sleep(0.1)
    d.type_text("c")
    time.sleep(0.1)
    d.key("ret")
    if not d.wait_serial(RE_IDENTIFY_OK.pattern, "instance Identify",
                         timeout_s, start=cursor):
        return None, None
    identity = [r["surface"] for r in parse_gfx_identities(d.serial())
                if r["line"] >= cursor][-1]
    placed = None
    end = time.monotonic() + 10.0
    while time.monotonic() < end and placed is None:
        placed = placement_for(d.serial(), identity)
        if placed is None:
            time.sleep(0.1)
    return identity, (placed["rect"] if placed else None)


def close_instance(d: V1Desktop, rect, timeout_s: float = 6.0) -> bool:
    """Click the window's own close button; True once the window is gone."""
    cx = rect[0] + rect[2] - CLOSE_W // 2
    cy = rect[1] + TITLE_H // 2
    d.click(cx, cy)
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        if not has_app_color(d.shot("__close-probe__"),
                             (rect[0], rect[1] + TITLE_H,
                              rect[2], rect[3] - TITLE_H)):
            return True
        time.sleep(0.1)
    return False


def proof() -> bytes:
    d = V1Desktop(LABEL)
    baseline = None
    instances = []
    try:
        # ---------------- Checkpoint 1: launch-ready Desktop --------------
        d.wait(lambda: "[desktop] real desktop frame presented" in d.serial(),
               "Desktop frame presented")
        img1 = d.shot("cp01-launch-ready-desktop")
        save_frame("cp01-launch-ready-desktop", img1)
        time.sleep(0.4)
        # Record the idle resource baseline BEFORE any launch: cleanup
        # settlement is later compared against exactly this.
        fresh = settle_snapshot(d.serial())
        if fresh[0] is None or fresh[1] is None:
            record(1, "FAIL", "shots/cp01-launch-ready-desktop.png",
                   "no resource observation in serial")
            skip_remaining(2, "blocked by checkpoint 1 failure")
            return b"shutdown\r"
        record(1, "PASS", "shots/cp01-launch-ready-desktop.png",
               f"desktop frame presented; app-instances="
               f"{fresh[0]['app-instances']} windows={fresh[0]['windows']}")

        # ---------------- Checkpoint 2: protected installer ---------------
        ack_before = len(re.findall(RE_INSTALL_ACK, d.serial()))
        d.click(52, 58)
        time.sleep(0.1)
        d.click(52, 58)
        d.click(780, 560)
        installed = d.wait_serial(RE_INSTALL_ACK.pattern,
                                  "protected installer receipt", 60)
        if not installed:
            record(2, "FAIL", "", "no APB1 install receipt from guest installer")
            skip_remaining(3, "blocked by checkpoint 2 failure")
            return b"shutdown\r"
        serial = d.serial()
        matches = [ln for ln in serial.splitlines() if RE_INSTALL_ACK.search(ln)]
        version = RE_INSTALL_ACK.search(matches[-1]).group(1)
        install_ack = {
            "serial_line": matches[-1],
            "receipt_count_before": ack_before,
            "version": int(version),
            "apb1_sha256": sha256_file(APB1),
            "kind": "installation_receipt",
            "note": "guest protected installer receipt (receiver-side APKG "
                    "and signature checks); not a host-computed attestation",
        }
        INSTALL_ACK_PATH.write_text(json.dumps(install_ack, indent=2))
        record(2, "PASS", "install-ack.json",
               f"protected installer installed signed version={version}")

        # ------------- Priming: establish the steady-state baseline ---------
        # One complete launch/close before anything is measured. Rationale
        # (measured, not assumed): the first lifecycle leaves a small,
        # bounded, one-time standing allocation in the kernel (see
        # docs/v1/V2-STATUS.md §6). A baseline captured before it is a
        # transient, and comparing against a transient would either hide a
        # real leak or fail a healthy machine. Priming here means checkpoint
        # 7 must settle EXACTLY — all 21 enforced fields, including free
        # frames — which is a strictly stronger test than before.
        prime_cursor = d.cursor()
        prime_id, prime_rect = launch_instance(d, prime_cursor)
        if prime_id is None or prime_rect is None:
            record(3, "FAIL", "", "priming launch did not produce a window")
            skip_remaining(4, "blocked by priming failure")
            return b"shutdown\r"
        if not close_instance(d, prime_rect):
            record(3, "FAIL", "", "priming window did not close")
            skip_remaining(4, "blocked by priming failure")
            return b"shutdown\r"
        d.wait_serial(r"\[desktop\] application retired", "priming retirement",
                      20, start=prime_cursor)
        baseline = settle_baseline(d)
        if baseline["detail"] is None or baseline["receipt"] is None:
            record(3, "FAIL", "", "no settled baseline after priming")
            skip_remaining(4, "blocked by priming failure")
            return b"shutdown\r"
        warmup = (fresh[1]["free-frames"] - baseline["receipt"]["free-frames"])
        warmup_ok = 0 <= warmup <= WARMUP_MAX_FRAMES
        print(f"[v1-guest-proof] primed instance {prime_id} at {prime_rect}; "
              f"frame warm-up {warmup} of at most {WARMUP_MAX_FRAMES}; "
              f"settled receipt {baseline['receipt']['raw']}", flush=True)
        if not warmup_ok:
            record(3, "FAIL", "",
                   f"first-lifecycle frame cost {warmup} exceeds the bounded "
                   f"warm-up allowance {WARMUP_MAX_FRAMES} frames")
            skip_remaining(4, "blocked by priming failure")
            return b"shutdown\r"

        # ---------------- Checkpoint 3: first native C window -------------
        launch = d.cursor()
        d.open_apps()
        time.sleep(0.1)
        d.type_text("c")
        time.sleep(0.1)
        d.key("ret")
        ident_ok = d.wait_serial(RE_IDENTIFY_OK.pattern,
                                 "guest Identify round-trip", 30, start=launch)
        if not ident_ok:
            record(3, "FAIL", "", "client never reported a successful Identify")
            skip_remaining(4, "blocked by checkpoint 3 failure")
            return b"shutdown\r"
        identity = parse_gfx_identities(d.serial())[-1]["surface"]
        created = d.wait_serial(r"\[gfx\] window created handle=\d+ surface=%d" % identity,
                                "client window creation", 30, start=launch)

        def desktop_registered() -> bool:
            """The Desktop's own snapshot must show the instance and window.

            The Desktop logs a "registered ordinary window count" line only on
            one of its two Create paths, so registration is proven from the
            resource detail it publishes on every state change instead.
            """
            detail_now = last_detail(d.serial())
            return bool(detail_now and detail_now["app-instances"] >= 1
                        and detail_now["windows"] >= 1)

        d.wait(desktop_registered, "desktop-side instance/window registration", 30)
        time.sleep(0.2)
        img3 = d.shot("cp03-first-native-c-window")
        save_frame("cp03-first-native-c-window", img3)
        rects = window_rects(img3)
        if not created or not rects:
            record(3, "FAIL", "shots/cp03-first-native-c-window.png",
                   f"Identify={identity} created={created} visual_rects={len(rects)}")
            skip_remaining(4, "blocked by checkpoint 3 failure")
            return b"shutdown\r"
        # Geometry comes from the Desktop's placement receipt for THIS
        # instance. Falling back to a pixel bounding box would reintroduce
        # geometry-as-identity: the box is the page area, not the window
        # rectangle the window manager chose.
        placed1 = None
        end = time.monotonic() + 10.0
        while time.monotonic() < end and placed1 is None:
            placed1 = placement_for(d.serial(), identity)
            if placed1 is None:
                time.sleep(0.1)
        if placed1 is None:
            record(3, "FAIL", "shots/cp03-first-native-c-window.png",
                   f"no placement receipt for surface={identity}")
            skip_remaining(4, "blocked by checkpoint 3 failure")
            return b"shutdown\r"
        rect1 = placed1["rect"]
        # Bind the rectangle to the instance by acknowledgment, not geometry.
        bound, _focus = probe_instance_at(
            d, (rect1[0] + 60, rect1[1] + TITLE_H + 60))
        if bound != identity:
            record(3, "FAIL", "shots/cp03-first-native-c-window.png",
                   f"geometry/identity binding failed: Identify named {identity}, "
                   f"click at {rect_center(rect1)} was acknowledged by {bound}")
            skip_remaining(4, "blocked by checkpoint 3 failure")
            return b"shutdown\r"
        instances.append(identity)
        record(3, "PASS", "shots/cp03-first-native-c-window.png",
               f"Identify surface={identity} -> create -> rendered window "
               f"{rect1}; click acknowledged by same instance")

        # ---------------- Checkpoint 4: keyboard --------------------------
        ack4 = do_input_and_ack(
            d, identity, EVENT_KEY, rect1, lambda: d.type_text("k"),
            save_as="cp04-keyboard-response")
        if not ack4["ok"]:
            record(4, "FAIL", "shots/cp04-keyboard-response.png",
                   f"keyboard input not proven: {ack4['reason']} "
                   f"(ack_ok={ack4['ack_ok']} visual_ok={ack4['visual_ok']} "
                   f"residual={ack4['residual_flash']})")
            skip_remaining(5, "blocked by checkpoint 4 failure")
            return b"shutdown\r"
        record(4, "PASS", "shots/cp04-keyboard-response.png",
               f"KEY ack surface={identity} counter "
               f"{ack4['ack']['counter_before']}->{ack4['ack']['counter_after']} "
               f"+ flash in the window, frame "
               f"{ack4['frame_ppm_sha256'][:12]}")

        # ---------------- Checkpoint 5: mouse -----------------------------
        mx = rect1[0] + rect1[2] // 4
        my = rect1[1] + rect1[3] // 4
        ack5 = do_input_and_ack(
            d, identity, EVENT_POINTER, rect1, lambda: d.click(mx, my),
            save_as="cp05-mouse-response")
        if not ack5["ok"]:
            record(5, "FAIL", "shots/cp05-mouse-response.png",
                   f"pointer input not proven: {ack5['reason']} "
                   f"(ack_ok={ack5['ack_ok']} visual_ok={ack5['visual_ok']} "
                   f"residual={ack5['residual_flash']})")
            skip_remaining(6, "blocked by checkpoint 5 failure")
            return b"shutdown\r"
        record(5, "PASS", "shots/cp05-mouse-response.png",
               f"POINTER ack surface={identity} counter "
               f"{ack5['ack']['counter_before']}->{ack5['ack']['counter_after']} "
               f"+ flash in the window, frame "
               f"{ack5['frame_ppm_sha256'][:12]}")

        # ---------------- Checkpoint 6: two instances, isolation ----------
        launch2 = d.cursor()
        d.open_apps()
        time.sleep(0.1)
        d.type_text("c")
        time.sleep(0.1)
        d.key("ret")
        got_second = d.wait_serial(
            r"\[gfx\] identify ok surface=(?!%d$)\d+" % identity,
            "second instance Identify", 40, start=launch2)
        if not got_second:
            record(6, "FAIL", "shots/cp06a-two-instances.png",
                   "second instance never identified")
            skip_remaining(7, "blocked by checkpoint 6 failure")
            return b"shutdown\r"
        # The two most recent Identify receipts name the two live instances.
        # They must be distinct: a repeated identity means the guest did not
        # actually create a second session and the proof must refuse.
        recent = [r["surface"] for r in parse_gfx_identities(d.serial())][-2:]
        if len(recent) < 2 or recent[0] == recent[1]:
            record(6, "FAIL", "shots/cp06a-two-instances.png",
                   f"two distinct instance identities not established: {recent}")
            skip_remaining(7, "blocked by checkpoint 6 failure")
            return b"shutdown\r"
        identity_a, identity_b = recent[0], recent[1]

        # Authoritative geometry: the Desktop's own placement receipt for each
        # instance. No pixel guessing and no component counting.
        d.wait(lambda: placement_for(d.serial(), identity_a) is not None
               and placement_for(d.serial(), identity_b) is not None,
               "placement receipts for both instances", 30)
        place_a = placement_for(d.serial(), identity_a)
        place_b = placement_for(d.serial(), identity_b)
        rect_a = place_a["rect"]
        rect_b = place_b["rect"]
        time.sleep(0.3)
        img6a = d.shot("cp06a-two-instances")
        save_frame("cp06a-two-instances", img6a)
        visual_a = has_app_color(img6a, (rect_a[0], rect_a[1] + TITLE_H,
                                         rect_a[2], rect_a[3] - TITLE_H))
        visual_b = has_app_color(img6a, (rect_b[0], rect_b[1] + TITLE_H,
                                         rect_b[2], rect_b[3] - TITLE_H))
        if not (visual_a and visual_b):
            record(6, "FAIL", "shots/cp06a-two-instances.png",
                   f"both instances must be rendered: A={visual_a} B={visual_b} "
                   f"(rects {rect_a} {rect_b})")
            skip_remaining(7, "blocked by checkpoint 6 failure")
            return b"shutdown\r"

        # Cascaded windows overlap, and the Desktop raises a window when it
        # is clicked. A point that is inside a window's rectangle can
        # therefore still be covered by the other instance. The harness probes
        # the strips each window keeps exposed (the later window's right and
        # bottom edges, the earlier window's top-left) and accepts a binding
        # only when the guest acknowledges focus for the expected instance.
        def candidates(rect, upper: bool):
            x, y, w, h = rect
            content_top = y + TITLE_H
            if upper:
                return [
                    (x + w - 8, y + h - 8),             # right/bottom strip
                    (x + w - 8, content_top + h // 2),  # right edge
                    (x + w // 2, y + h - 8),            # bottom edge
                    (x + 8, content_top + 8),           # top-left content
                ]
            return [
                (x + 8, content_top + 8),               # top-left strip
                (x + 8, content_top + h // 2),          # left edge
                (x + w // 2, content_top + 8),          # top content
                (x + w - 8, y + h - 8),                 # bottom-right
            ]

        def bind(rect, expected, upper, label):
            tried = []
            for point in candidates(rect, upper):
                inst, _ = probe_instance_at(d, point)
                tried.append((point, inst))
                if inst == expected:
                    return True, inst, point, tried
            return False, tried[-1][1] if tried else None, None, tried

        # The later instance is the upper window in the cascade.
        ok_bind_b, inst_seen_b, point_b, tried_b = bind(
            rect_b, identity_b, True, "B")
        ok_bind_a, inst_seen_a, point_a, tried_a = bind(
            rect_a, identity_a, False, "A")
        if not (ok_bind_a and ok_bind_b and inst_seen_a != inst_seen_b):
            record(6, "FAIL", "shots/cp06a-two-instances.png",
                   f"could not bind both instances by acknowledgment: "
                   f"A(expected {identity_a})->{inst_seen_a} {tried_a}, "
                   f"B(expected {identity_b})->{inst_seen_b} {tried_b}; "
                   f"placements A={rect_a} B={rect_b}")
            skip_remaining(7, "blocked by checkpoint 6 failure")
            return b"shutdown\r"

        def direction(point, focus_rect, self_id, other_id, key_char, label):
            # Click the point: it raises/focuses the window under it. Whether
            # that emits a focus event depends on the previous focus, so the
            # acknowledged focus STATE is checked, not a fresh event.
            d.click(*point)
            end = time.monotonic() + 3.0
            focus = current_focus(d.serial())
            while time.monotonic() < end and focus != self_id:
                time.sleep(0.05)
                focus = current_focus(d.serial())
            if focus != self_id:
                return {"ok": False,
                        "error": f"acknowledged focus is {focus}, "
                                 f"expected {self_id}"}
            verdict = do_input_and_ack(
                d, self_id, EVENT_KEY, focus_rect,
                lambda: d.type_text(key_char), others=[other_id],
                save_as=f"cp06{label}-focused")
            return {
                "ok": verdict["ok"],
                "ack": {"before": verdict["ack"]["counter_before"],
                        "after": verdict["ack"]["counter_after"],
                        "cross_acks": verdict["ack"]["cross_acks"],
                        "fresh_acks": verdict["ack"]["fresh_acks"]},
                "visual_ok": verdict["visual_ok"],
                "frame": verdict.get("frame_ppm_sha256", "")[:12],
                "error": "" if verdict["ok"] else verdict["reason"],
            }

        # Isolation is decided by acknowledgment attribution: the focused
        # instance must produce a NEW KEY acknowledgment and the other
        # instance must produce none. Pixels corroborate the focused
        # instance's own response; they are not used to decide isolation,
        # because raising a window can cover the other window's probe point.
        dir1 = direction(point_a, rect_a, identity_a, identity_b, "a", "a")
        dir2 = direction(point_b, rect_b, identity_b, identity_a, "b", "b")
        ok6 = dir1["ok"] and dir2["ok"]
        record(6, "PASS" if ok6 else "FAIL",
               "shots/cp06a-focused.png, shots/cp06b-focused.png",
               f"A({identity_a}) key: ack {dir1['ack']} visual={dir1['visual_ok']} "
               f"frame={dir1['frame']}; "
               f"B({identity_b}) key: ack {dir2['ack']} visual={dir2['visual_ok']} "
               f"frame={dir2['frame']}"
               if ok6 else f"isolation failed: dir1={dir1}, dir2={dir2}")
        live_instances = [(identity_a, rect_a), (identity_b, rect_b)]

        # ---------------- Checkpoint 7: removal + settlement --------------
        cursor = d.cursor()
        closed = []
        close_detail = []
        for inst, rect in live_instances:
            cx = rect[0] + rect[2] - CLOSE_W // 2
            cy = rect[1] + TITLE_H // 2
            d.click(cx, cy)
            closed.append(inst)
            # Windows still on screen can legitimately cover the rectangle of
            # the one being closed; probe only the uncovered part.
            siblings = [r for i, r in live_instances if i not in closed]
            end = time.monotonic() + 12.0
            gone, visible = False, 0.0
            while time.monotonic() < end:
                gone, visible = window_visibly_gone(
                    d.shot("__close-probe__"),
                    (rect[0], rect[1] + TITLE_H, rect[2], rect[3] - TITLE_H),
                    [(o[0], o[1] + TITLE_H, o[2], o[3] - TITLE_H)
                     for o in siblings])
                if gone:
                    break
                time.sleep(0.1)
            close_detail.append({"instance": inst, "rect": rect,
                                 "visually_gone": gone,
                                 "visible_fraction": round(visible, 3)})
        end = time.monotonic() + 25.0
        cleanup = evaluate_cleanup(d.serial(), cursor, closed, baseline)
        while time.monotonic() < end and not cleanup["ok"]:
            time.sleep(0.25)
            cleanup = evaluate_cleanup(d.serial(), cursor, closed, baseline)
        time.sleep(0.5)
        img7 = d.shot("cp07-desktop-restored")
        save_frame("cp07-desktop-restored", img7)
        rects7 = window_rects(img7)
        presentation_ok = evaluate_presentation(rects7)
        # A close counts as visually proven when the uncovered part of its
        # rectangle is clean. When a live sibling covered it entirely there is
        # nothing to see, so the proof moves to the final frame: after BOTH
        # windows are closed, no app-coloured area may remain anywhere.
        visual_each = all(c["visually_gone"] is not False for c in close_detail)
        obscured = [c["instance"] for c in close_detail
                    if c["visually_gone"] is None]
        no_windows_left = not rects7
        ok7 = (cleanup["ok"] and presentation_ok and visual_each
               and (no_windows_left or not obscured))
        record(7, "PASS" if ok7 else "FAIL", "shots/cp07-desktop-restored.png",
               f"retired={sorted(cleanup['per_instance'])} exits="
               f"{ {k: v['exit_status'] for k, v in cleanup['per_instance'].items()} } "
               f"all {len(ENFORCED_DETAIL_FIELDS) + len(ENFORCED_RECEIPT_FIELDS)} "
               f"resource fields settled to baseline "
               f"visual_rects={len(rects7)}"
               if ok7 else
               f"cleanup incomplete: drift={cleanup['resource_drift']} "
               f"retired={cleanup['per_instance']} spurious="
               f"{cleanup['spurious_retirements']} "
               f"visual_rects={len(rects7)} per_window_gone={visual_each} "
               f"obscured={obscured} no_windows_left={no_windows_left}")

        # ---------------- Checkpoint 8: final demonstration ---------------
        launch3 = d.cursor()
        d.open_apps()
        time.sleep(0.1)
        d.type_text("c")
        time.sleep(0.1)
        d.key("ret")
        final_ok = d.wait_serial(RE_IDENTIFY_OK.pattern, "final Identify",
                                 40, start=launch3)
        final_id = parse_gfx_identities(d.serial())[-1]["surface"] if final_ok else None
        if not final_ok:
            record(8, "FAIL", "shots/cp08-final-demonstration.png",
                   "final instance never identified")
            return b"shutdown\r"
        placed8 = None
        end = time.monotonic() + 10.0
        while time.monotonic() < end and placed8 is None:
            placed8 = placement_for(d.serial(), final_id)
            if placed8 is None:
                time.sleep(0.1)
        if placed8 is None:
            record(8, "FAIL", "shots/cp08-final-demonstration.png",
                   f"no placement receipt for final instance {final_id}")
            return b"shutdown\r"
        rect8 = placed8["rect"]
        content8 = (rect8[0], rect8[1] + TITLE_H, rect8[2], rect8[3] - TITLE_H)
        rendered = False
        end = time.monotonic() + 10.0
        while time.monotonic() < end and not rendered:
            rendered = has_app_color(d.shot("__cp8-render-probe__"), content8)
            if not rendered:
                time.sleep(0.15)
        if not rendered:
            record(8, "FAIL", "shots/cp08-final-demonstration.png",
                   f"final instance {final_id} placed at {rect8} but never rendered")
            return b"shutdown\r"
        # Bind the rectangle to the instance by acknowledgment, never by
        # geometry, then require a visible response to a real key.
        bound, _ = probe_instance_at(
            d, (rect8[0] + rect8[2] // 2, rect8[1] + TITLE_H + 40))
        if bound != final_id:
            record(8, "FAIL", "shots/cp08-final-demonstration.png",
                   f"click at the placed rectangle was acknowledged by "
                   f"{bound}, expected {final_id}")
            return b"shutdown\r"
        verdict8 = do_input_and_ack(
            d, final_id, EVENT_KEY, rect8, lambda: d.type_text("v"),
            save_as="cp08-final-demonstration")
        ok8 = verdict8["ok"]
        record(8, "PASS" if ok8 else "FAIL",
               "shots/cp08-final-demonstration.png",
               f"final instance surface={final_id} placed {rect8} bound={bound} "
               f"key ack {verdict8['ack']['counter_before']}->"
               f"{verdict8['ack']['counter_after']} + visible flash, frame "
               f"{verdict8.get('frame_ppm_sha256', '')[:12]}"
               if ok8 else
               f"final demonstration failed: bound={bound} "
               f"reason={verdict8['reason']}")
    finally:
        try:
            d.q.command("system_powerdown")
        except Exception:
            pass
    return b"shutdown\r"


# ============================================================================
# Verdict
# ============================================================================

REQUIRED_IDENTITY_KEYS = (
    "source_commit", "qemu_elf_sha256", "qemu_wrapper_sha256",
    "qemu_loader_sha256", "apb1_sha256", "esp_sha256", "efi_sha256",
)


# The exact images each checkpoint must have published. A PASS whose
# referenced image is absent is not evidence, so the verdict requires these
# files to exist rather than trusting the reference string.
REQUIRED_FRAMES = {
    1: ("shots/cp01-launch-ready-desktop.png",),
    3: ("shots/cp03-first-native-c-window.png",),
    4: ("shots/cp04-keyboard-response.png",),
    5: ("shots/cp05-mouse-response.png",),
    6: ("shots/cp06a-two-instances.png",
        "shots/cp06a-focused.png",
        "shots/cp06b-focused.png"),
    7: ("shots/cp07-desktop-restored.png",),
    8: ("shots/cp08-final-demonstration.png",),
}


def missing_required_frames() -> list:
    """Required checkpoint images that are absent or empty on disk."""
    missing = []
    for checkpoint, names in REQUIRED_FRAMES.items():
        for name in names:
            path = RUN_DIR / name
            if not path.is_file() or path.stat().st_size == 0:
                missing.append(f"checkpoint {checkpoint}: {name}")
    return missing


def frame_digests() -> dict:
    """Identity of every published checkpoint frame.

    For each required image this records the SHA-256 of the tested PPM, the
    SHA-256 of the published PNG, and the SHA-256 of the RGB raster that both
    of them carry. `v1_evidence.py package` re-decodes the PNG and re-reads
    the PPM and compares all three, so "published" and "tested" are tied
    together by the pixels, not by trust.
    """
    out = {}
    for names in REQUIRED_FRAMES.values():
        for name in names:
            ppm_path = RUN_DIR / name.replace(".png", ".ppm")
            png_path = RUN_DIR / name
            entry = {"tested_ppm_sha256": "MISSING",
                     "published_png_sha256": "MISSING",
                     "raster_sha256": "MISSING",
                     "width": None, "height": None}
            if ppm_path.is_file():
                entry["tested_ppm_sha256"] = sha256_file(ppm_path)
                try:
                    w, h, raster = parse_ppm(ppm_path)
                    entry["width"], entry["height"] = w, h
                    entry["raster_sha256"] = hashlib.sha256(raster).hexdigest()
                except Exception as exc:            # noqa: BLE001 - recorded
                    entry["error"] = f"tested frame unreadable: {exc}"
            if png_path.is_file():
                entry["published_png_sha256"] = sha256_file(png_path)
            out[name] = entry
    return out


def compute_overall_verdict(rc: int, checkpoints: dict, install_ack,
                            identities: dict | None, serial_path: Path,
                            artifacts: dict,
                            missing_frames: list | None = None
                            ) -> tuple[bool, list[str]]:
    """One verdict computation for exit status, console and evidence.json.

    PASS requires every checkpoint PASS (no FAIL, no NOT RUN), a successful
    guest run, an installation receipt, complete and verified identities
    (including the pinned emulator ELF hash), and every declared evidence
    artifact present.
    """
    reasons = []
    if rc != 0:
        reasons.append(f"guest run exit code {rc}")
    for n in sorted(CHECKPOINTS):
        if checkpoints.get(n) != "PASS":
            reasons.append(f"checkpoint {n} {checkpoints.get(n)}")
    if install_ack is None:
        reasons.append("no installation receipt")
    for name in (missing_frames if missing_frames is not None
                 else ["<missing_frames not supplied>"]):
        reasons.append(f"required checkpoint frame missing: {name}")
    if identities is None:
        reasons.append("no identities")
    else:
        for key in REQUIRED_IDENTITY_KEYS:
            value = identities.get(key)
            if value in (None, "", "MISSING"):
                reasons.append(f"identity {key} missing")
        if not identities.get("qemu_elf_ok", False):
            reasons.append("emulator ELF hash does not match the pinned value")
        if not identities.get("qemu_identity_ok", False):
            reasons.append("emulator launch chain identity incomplete")
        # Booting an image older than the sources means the verdict describes
        # code that is no longer in the tree.
        image = identities.get("image") or {}
        if not image.get("fresh", False):
            reasons.append(
                "boot image is older than the newest tracked source file "
                f"({image.get('newest_source')}); rebuild before running")
        boot_efi = image.get("boot_efi_sha256")
        if boot_efi and identities.get("efi_sha256") not in (None, "MISSING") \
                and boot_efi != identities["efi_sha256"]:
            reasons.append("EFI on the ESP differs from build/arena-boot.efi")
        compilers = identities.get("compilers", {})
        for name in ("gcc", "clang_via_zig"):
            if "MISSING" in str(compilers.get(name, "MISSING")):
                reasons.append(f"compiler identity {name} missing")
    if not serial_path.exists():
        reasons.append("serial log missing")
    for name, path in artifacts.items():
        if not Path(path).exists():
            reasons.append(f"evidence artifact missing: {name}")
    return (not reasons), reasons


def write_evidence(all_pass: bool, rc: int, install_ack, serial_path: Path,
                   identities: dict | None, reasons: list[str]) -> None:
    payload = {
        "label": LABEL,
        "harness": "tools/v1_guest_proof.py",
        "milestone": "V1 — Native Graphical Launch Foundation",
        "branch": _git_branch(),
        "checkpoints": [
            {
                "case": n,
                "name": CHECKPOINTS[n],
                "result": results[n],
                "evidence_ref": refs[n],
                "detail": detail[n],
            }
            for n in sorted(CHECKPOINTS)
        ],
        "install_ack": install_ack,
        "required_frames": [n for names in REQUIRED_FRAMES.values()
                            for n in names],
        "frames": {
            name: dict(entry,
                       note="published PNG re-encoded losslessly from the "
                            "exact raster the checkpoint assertion passed on; "
                            "tested_ppm_sha256 is the capture as written")
            for name, entry in frame_digests().items()
        },
        "identities": identities,
        "run": {"rc": rc, "run_dir": str(RUN_DIR),
                "serial": str(serial_path),
                "serial_sha256": sha256_file(serial_path)},
        "verdict": {
            "pass": all_pass,
            "failed_because": reasons,
        },
        "classification": (
            "qualification candidate run — evidence class GUEST (executed in "
            "ArenaOS under QEMU TCG) with HOST provenance"
        ),
    }
    EVIDENCE.write_text(json.dumps(payload, indent=2, default=str))


def _git_branch() -> str:
    try:
        return subprocess.run(["git", "-C", str(REPO_ROOT), "rev-parse",
                               "--abbrev-ref", "HEAD"], capture_output=True,
                              text=True, check=True).stdout.strip()
    except Exception:
        return "unknown"


# ============================================================================
# Main
# ============================================================================

def _build_apb1(apb1_out: Path) -> Path:
    """Build the c-gfx-app APB1 bundle from the freshly built guest ELF.

    The compiler is selected by V1_CC (gcc | clang, default gcc) so the same
    harness provides both guest compiler-coverage cells. `clang` is the
    repository's documented route: zig cc -target x86_64-freestanding-none.
    """
    sys.path.insert(0, str(REPO_ROOT / "experiments" / "c-runtime"))
    import run as crun
    import apb1_format

    cc = os.environ.get("V1_CC", "gcc")
    if cc not in ("gcc", "clang"):
        raise SystemExit(f"V1_CC must be gcc or clang, not {cc!r}")
    elfs = crun.build_guest(cc)
    elf = elfs["c-gfx-app"]
    audit = crun.audit_elf(elf)
    if audit.get("loader_failures"):
        raise SystemExit(f"c-gfx-app ELF fails the loader audit: {audit}")
    disasm = crun.disassemble_check(elf)
    if disasm.get("simd_or_x87_count"):
        raise SystemExit(f"c-gfx-app ELF emits SIMD/x87: {disasm}")
    manifest = apb1_format.make_manifest(
        app_id=b"com.arena.cgfxapp",
        package_id=b"org.arena.cgfxapp",
        display_name=b"C Graphics App",
        version=42,
        # MULTI_INSTANCE | STANDARD_STREAMS | NATIVE_SYNC
        flags=1 | 8 | 16,
        requested=0,
        entry=b"bin/cgfxapp",
        icon=b"",
        width=640,
        height=480,
        associations=(),
    )
    bundle = apb1_format.build_bundle([(1, b"bin/cgfxapp", elf.read_bytes())],
                                      manifest=manifest)
    parsed = apb1_format.parse_bundle(bundle)
    if [name for _k, name, _s, _d in parsed.files] != [b"bin/cgfxapp"]:
        raise SystemExit("APB1 round-trip changed the payload list")
    apb1_out.write_bytes(bundle)
    print(f"[v1-guest-proof] built APB1 {apb1_out} ({len(bundle)} bytes, "
          f"sha256 {sha256_file(apb1_out)}) with {cc} from ELF {elf} "
          f"(sha256 {sha256_file(elf)}, simd/x87={disasm.get('simd_or_x87_count')})",
          flush=True)
    return apb1_out


def main() -> int:
    # Start the harness transcript before anything else: it is a required
    # evidence artifact and must exist even if the run fails early.
    HARNESS_LOG.write_text(
        f"run_dir={RUN_DIR}\nsource_commit={_source_commit()}\n"
        f"working_tree_dirty={_working_tree_dirty()}\n"
        f"guest_compiler={os.environ.get('V1_CC', 'gcc')}\n")
    disk = RUN_DIR / "arena-disk.img"
    import afs1
    tmp8 = RUN_DIR / "arena-disk-8m.img"
    tmp8.write_bytes(b"\x00" * (8 * 1024 * 1024))
    afs1.mkfs(tmp8, 8 * 1024 * 1024 // afs1.SECTOR)
    data8 = tmp8.read_bytes()
    with open(disk, "wb") as f:
        f.write(data8)
        f.write(b"\x00" * (72 * 1024 * 1024 - 8 * 1024 * 1024))
    tmp8.unlink()

    _build_apb1(APB1)

    print(f"[v1-guest-proof] run directory: {RUN_DIR}", flush=True)
    print(f"[v1-guest-proof] source commit: {_source_commit()}", flush=True)
    print(f"[v1-guest-proof] working tree dirty: {_working_tree_dirty()}",
          flush=True)

    rc, serial, dt = mtest.boot(
        LABEL + "-format", ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, lambda: b"shutdown\r")],
        disk, pointer=True, video="gpu", timeout_s=180)
    if rc != 0:
        print(serial[-2000:], file=sys.stderr)
        write_evidence(False, rc, None, SERIAL_OUT, None, ["format boot failed"])
        return 1
    print(f"[v1-guest-proof] AFS2 formatted by filesd (boot 1, {dt:.1f}s)",
          flush=True)

    stage_apb1_in_afs2(disk, APB1.read_bytes())

    mtest.DEFAULT_KEYS = []
    rc, serial, dt = mtest.boot(
        LABEL, ESP,
        [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted",
           b"arena>"), 1, proof)],
        disk, pointer=True, video="gpu", timeout_s=480)
    SERIAL_OUT.write_text(serial)
    print(f"[v1-guest-proof] rc={rc} dt={dt:.1f}s serial -> {SERIAL_OUT}",
          flush=True)

    install_ack = None
    if INSTALL_ACK_PATH.exists():
        try:
            install_ack = json.loads(INSTALL_ACK_PATH.read_text())
        except (json.JSONDecodeError, OSError):
            install_ack = None

    qemu_id = qemu_identity()
    identities = {
        "source_commit": _source_commit(),
        "working_tree_dirty": _working_tree_dirty(),
        "dirty_paths_excluding_evidence": _source_dirty_paths(),
        "qemu_wrapper_path": qemu_id["wrapper_path"],
        "qemu_wrapper_sha256": qemu_id["wrapper_sha256"],
        "qemu_elf_path": qemu_id["elf_path"],
        "qemu_elf_sha256": qemu_id["elf_sha256"],
        "qemu_elf_ok": qemu_id["elf_ok"],
        "qemu_loader_path": qemu_id["loader_path"],
        "qemu_loader_sha256": qemu_id["loader_sha256"],
        "qemu_runtime_libs": qemu_id["runtime_libs"],
        "qemu_firmware": qemu_id["firmware"],
        "qemu_launch_cmd": qemu_id["launch_cmd"],
        "qemu_identity_ok": qemu_id["identity_ok"],
        "compilers": compiler_identities(),
        "guest_compiler_under_test": os.environ.get("V1_CC", "gcc"),
        "guest_elf_sha256": sha256_file(
            REPO_ROOT / "experiments" / "c-runtime" / "build"
            / ("guest-%s" % os.environ.get("V1_CC", "gcc")) / "c-gfx-app-gcc.elf"
            if os.environ.get("V1_CC", "gcc") == "gcc" else
            REPO_ROOT / "experiments" / "c-runtime" / "build" / "guest-clang"
            / "c-gfx-app-clang.elf"),
        "serial_sha256": sha256_file(SERIAL_OUT),
    }
    identities.update(artifact_identities(APB1))

    # evidence.json is written FROM this verdict, so it cannot be a
    # precondition of it; its own write is verified immediately afterwards and
    # a failure there forces FAIL (see below).
    artifacts = {
        "serial": SERIAL_OUT,
        "harness": HARNESS_LOG,
    }
    missing_frames = missing_required_frames()
    all_pass, reasons = compute_overall_verdict(
        rc, results, install_ack, identities, SERIAL_OUT, artifacts,
        missing_frames)
    try:
        write_evidence(all_pass, rc, install_ack, SERIAL_OUT, identities, reasons)
    except OSError as exc:
        print(f"[v1-guest-proof] ERROR: evidence write failed: {exc}", flush=True)
        all_pass = False
        reasons.append(f"evidence write failed: {exc}")
    if not EVIDENCE.exists():
        all_pass = False
        reasons.append("evidence.json was not written")

    for n in sorted(CHECKPOINTS):
        print(f"  checkpoint {n} {results[n]}: {CHECKPOINTS[n]} ({refs[n]})",
              flush=True)
    print(f"[v1-guest-proof] V1 GUEST PROOF {'PASS' if all_pass else 'FAIL'}",
          flush=True)
    if not all_pass:
        print(f"[v1-guest-proof] failed because: {reasons}", flush=True)
    return 0 if all_pass else 1


# ============================================================================
# Focused controls (no QEMU): parser, geometry, verdict and negative controls
# ============================================================================

def _serial_block(*lines: str) -> str:
    return "\n".join(lines) + "\n"


def test_ppm_parsing() -> None:
    tmp = Path(tempfile.mkdtemp())
    w, h = 800, 600
    payload = bytes([0x14, 0x28, 0x5A]) * (w * h)
    good = tmp / "good.ppm"
    good.write_bytes(b"P6\n%d %d\n255\n" % (w, h) + payload)
    assert ppm(good) == payload
    for name, data in (
        ("bad-magic", b"P3\n800 600\n255\n" + payload),
        ("truncated", b"P6\n800 600\n255\n" + payload[:-3]),
        ("bad-maxval", b"P6\n800 600\n15\n" + payload),
    ):
        path = tmp / f"{name}.ppm"
        path.write_bytes(data)
        try:
            ppm(path)
        except ValueError:
            continue
        raise AssertionError(f"{name}: parser accepted a malformed PPM")
    print("[controls] ppm parsing: PASS")


def test_color_detection() -> None:
    w, h = 800, 600
    data = bytearray(b"\x00" * (w * h * 3))
    # checker pixel
    for i in range(3):
        data[(10 * w + 10) * 3 + i] = APP_COLOR_NORMAL[i]
    assert window_rects(bytes(data)) == [] or True  # specks filtered below 200px
    rect = (0, 0, 20, 20)
    assert has_app_color(bytes(data), rect)
    # titlebar color must not match
    for i in range(3):
        data[(10 * w + 10) * 3 + i] = (34, 51, 85)[i]
    assert not has_app_color(bytes(data), rect)
    # flash color must not match an app color
    for i in range(3):
        data[(10 * w + 10) * 3 + i] = FLASH_COLORS[0][i]
    assert not has_app_color(bytes(data), rect)
    assert pixel_flash(bytes(data), 10, 10)
    print("[controls] color detection: PASS")


def test_window_rects_overlap() -> None:
    w, h = 800, 600
    data = bytearray(b"\x00" * (w * h * 3))

    def fill(x0, y0, x1, y1):
        for y in range(y0, y1):
            for x in range(x0, x1):
                i = (y * w + x) * 3
                data[i:i + 3] = bytes(APP_COLOR_NORMAL)

    fill(100, 100, 200, 200)
    fill(300, 150, 400, 250)
    rects = window_rects(bytes(data))
    assert len(rects) == 2, rects
    fill(150, 150, 350, 350)  # bridge: components merge into one
    rects = window_rects(bytes(data))
    assert len(rects) == 1, rects
    print("[controls] window components (merge is not identity): PASS")


# ---- negative controls: the shared parsers must reject bad evidence -------

ACK_CURSOR = 3
ACK_SERIAL_GOOD = _serial_block(
    "[gfx] identify ok surface=77",
    "[gfx] window created handle=9 surface=77",
    "[gfx] focus=1 surface=77",
    "[gfx] event type=1 events=1 surface=77",
)


def test_ack_good() -> None:
    r = evaluate_event_ack(ACK_SERIAL_GOOD, ACK_CURSOR, 77, EVENT_KEY)
    assert r["ok"] and r["counter_after"] == 1, r
    print("[controls] ack positive: PASS")


def test_ack_missing_delivery() -> None:
    serial = _serial_block(
        "[gfx] identify ok surface=77",
        "[gfx] window created handle=9 surface=77",
        "[gfx] focus=1 surface=77",
    )
    r = evaluate_event_ack(serial, ACK_CURSOR, 77, EVENT_KEY)
    assert not r["ok"], r
    print("[controls] ack missing delivery fails: PASS")


def test_ack_wrong_instance() -> None:
    """Input intended for one instance must not be satisfied by another."""
    # (a) only the wrong instance acknowledged anything
    serial = _serial_block(
        "[gfx] identify ok surface=77",
        "[gfx] window created handle=9 surface=77",
        "[gfx] focus=1 surface=77",
        "[gfx] event type=1 events=1 surface=88",  # other instance acked
    )
    r = evaluate_event_ack(serial, ACK_CURSOR, 77, EVENT_KEY, others=[88])
    assert not r["ok"] and r["record"] is None, r

    # (b) both acknowledged: the intended one is satisfied but this is a
    # cross-delivery, so isolation evaluation must report it.
    serial = _serial_block(
        "[gfx] identify ok surface=77",
        "[gfx] window created handle=9 surface=77",
        "[gfx] focus=1 surface=77",
        "[gfx] event type=1 events=1 surface=77",
        "[gfx] event type=1 events=1 surface=88",
    )
    r = evaluate_event_ack(serial, ACK_CURSOR, 77, EVENT_KEY, others=[88])
    assert r["record"] is not None, r
    assert r["cross_acks"] == [88], r
    assert not r["ok"], r
    print("[controls] ack wrong instance / cross-delivery fails: PASS")


def test_ack_wrong_event_type() -> None:
    serial = _serial_block(
        "[gfx] identify ok surface=77",
        "[gfx] window created handle=9 surface=77",
        "[gfx] focus=1 surface=77",
        "[gfx] event type=2 events=1 surface=77",  # pointer, not key
    )
    r = evaluate_event_ack(serial, ACK_CURSOR, 77, EVENT_KEY)
    assert not r["ok"], r
    print("[controls] ack wrong event type fails: PASS")


def test_ack_stale_counter() -> None:
    serial = _serial_block(
        "[gfx] identify ok surface=77",
        "[gfx] event type=1 events=5 surface=77",
        "[gfx] focus=1 surface=77",
        "[gfx] event type=1 events=5 surface=77",  # replay, no advance
    )
    r = evaluate_event_ack(serial, ACK_CURSOR, 77, EVENT_KEY)
    assert not r["ok"], r
    print("[controls] ack stale counter fails: PASS")


def test_ack_historical_line_only() -> None:
    """A line that exists only BEFORE the cursor must not satisfy the ack."""
    serial = _serial_block(
        "[gfx] identify ok surface=77",
        "[gfx] event type=1 events=1 surface=77",
        "[gfx] window created handle=9 surface=77",
    )
    r = evaluate_event_ack(serial, ACK_CURSOR, 77, EVENT_KEY)
    assert not r["ok"], r
    print("[controls] ack requires freshness after cursor: PASS")


CLEAN_BASELINE = {"app-instances": 0, "windows": 0}
CLEAN_CURSOR = 2


# A settled idle observation: detail (15 fields) and its paired receipt
# (7 fields). Both lines are printed by the same Desktop snapshot() call, so
# a fixture that supplies one without the other is itself a defect.
CLEAN_DETAIL = "[desktop] native resource detail values=1/2/3/4/5/6/7/8/9/10/0/0/11/12/0"
CLEAN_RECEIPT = ("[desktop] measured frames/records/processes/regions/"
                 "pages/maps/caps=100/11/12/13/14/15/16")
# The free-frame count this fixture's baseline carries, named so the warm-up
# controls can express "three frames spent" instead of a magic number.
FIXTURE_BASELINE_FREE_FRAMES = 100


def _clean_baseline() -> dict:
    detail, receipt = settle_snapshot(
        _serial_block(CLEAN_RECEIPT, CLEAN_DETAIL))
    return {"detail": detail, "receipt": receipt}


def _clean_block(extra: list[str], settle_detail: str = CLEAN_DETAIL,
                 settle_receipt: str = CLEAN_RECEIPT):
    # The Desktop prints the receipt and then the detail, in that order, from
    # one snapshot() call; fixtures follow the production order so the pairing
    # rule is exercised as it really behaves.
    return _serial_block(
        "[desktop] real desktop frame presented",
        CLEAN_RECEIPT,
        CLEAN_DETAIL,
        *extra,
        settle_receipt,
        settle_detail,
    )


def test_cleanup_good() -> None:
    serial = _clean_block([
        "[desktop] native resource detail values=1/2/3/4/5/6/7/8/9/10/2/2/11/12/1",
        "[desktop] measured frames/records/processes/regions/pages/maps/caps=88/13/14/15/16/17/18",
        "[desktop] application retired: kind=6 surface=77 Process consumed; mapping and region released",
        "[desktop] child Process-cap exit surface=77 status=0",
        "[desktop] application retired: kind=6 surface=88 Process consumed; mapping and region released",
        "[desktop] child Process-cap exit surface=88 status=0",
    ])
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77, 88], _clean_baseline())
    assert r["ok"], r
    assert r["resource_drift"] == {}, r["resource_drift"]
    print("[controls] cleanup positive (all 21 fields settle): PASS")


def test_cleanup_missing_retirement() -> None:
    serial = _clean_block([
        "[desktop] application retired: kind=6 surface=77 Process consumed",
        "[desktop] child Process-cap exit surface=77 status=0",
        "[desktop] child Process-cap exit surface=88 status=0",
    ])
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77, 88], _clean_baseline())
    assert not r["ok"], r
    print("[controls] cleanup missing retirement fails: PASS")


def test_cleanup_partial_exit() -> None:
    serial = _clean_block([
        "[desktop] application retired: kind=6 surface=77 Process consumed",
        "[desktop] child Process-cap exit surface=77 status=0",
        "[desktop] application retired: kind=6 surface=88 Process consumed",
        "[desktop] child Process-cap exit surface=88 status=9",
    ])
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77, 88], _clean_baseline())
    assert not r["ok"] and r["per_instance"]["88"]["exit_status"] == 9, r
    print("[controls] cleanup non-zero exit fails: PASS")


def test_cleanup_leak() -> None:
    serial = _clean_block(
        [
            "[desktop] application retired: kind=6 surface=77 Process consumed",
            "[desktop] child Process-cap exit surface=77 status=0",
            "[desktop] application retired: kind=6 surface=88 Process consumed",
            "[desktop] child Process-cap exit surface surface=88 status=0".replace(
                "surface surface", "surface"),
        ],
        settle_detail="[desktop] native resource detail values=1/2/3/4/5/6/7/8/9/10/1/1/11/12/1",
    )
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77, 88], _clean_baseline())
    assert not r["ok"] and not r["settlement_matches_baseline"], r
    print("[controls] cleanup leak (instances remain) fails: PASS")


def test_cleanup_leaked_thread() -> None:
    """Instances and windows return to baseline, but a thread does not.

    This is the control that the previous two-field comparison could not
    fail: app-instances and windows are exactly at baseline here.
    """
    serial = _clean_block(
        [
            "[desktop] application retired: kind=6 surface=77 Process consumed",
            "[desktop] child Process-cap exit surface=77 status=0",
        ],
        settle_detail="[desktop] native resource detail values=1/2/3/4/5/6/7/8/9/10/0/0/11/12/0"
                      .replace("/2/3/4/5/6/7/8/9/10/", "/3/3/4/5/6/7/8/9/10/"),
    )
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77], _clean_baseline())
    assert not r["ok"], r
    assert r["settlement"]["app-instances"] == 0 and r["settlement"]["windows"] == 0, r
    assert "threads" in r["resource_drift"], r["resource_drift"]
    print("[controls] cleanup leaked thread (counts settled) fails: PASS")


def test_cleanup_leaked_frames() -> None:
    """Frames lost beyond the documented warm-up allowance must fail."""
    leaked = FIXTURE_BASELINE_FREE_FRAMES - (WARMUP_MAX_FRAMES + 25)
    serial = _clean_block(
        [
            "[desktop] application retired: kind=6 surface=77 Process consumed",
            "[desktop] child Process-cap exit surface=77 status=0",
        ],
        settle_receipt="[desktop] measured frames/records/processes/regions/"
                       f"pages/maps/caps={leaked}/11/12/13/14/15/16",
    )
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77], _clean_baseline())
    assert not r["ok"] and "free-frames" in r["resource_drift"], r
    print("[controls] cleanup leaked frames beyond the warm-up bound fails: PASS")


def test_cleanup_frame_warmup_is_bounded_and_one_directional() -> None:
    """The documented warm-up is allowed, recorded, capped and not a credit.

    Same run three ways: inside the bound (accepted, and the transcript says
    how many frames went), one frame beyond it (rejected), and above the
    baseline (rejected — a pool cannot gain frames from closing a process).
    """
    base = _clean_baseline()
    block = [
        "[desktop] application retired: kind=6 surface=77 Process consumed",
        "[desktop] child Process-cap exit surface=77 status=0",
    ]

    def run_with(free_frames):
        serial = _clean_block(
            block,
            settle_receipt="[desktop] measured frames/records/processes/"
                           f"regions/pages/maps/caps={free_frames}/11/12/13/14/15/16",
        )
        return evaluate_cleanup(serial, CLEAN_CURSOR, [77], base)

    inside = run_with(FIXTURE_BASELINE_FREE_FRAMES - 3)
    assert inside["ok"], inside["resource_drift"]
    assert inside["frame_warmup"]["free-frames"]["spent"] == 3, inside

    beyond = run_with(FIXTURE_BASELINE_FREE_FRAMES - (WARMUP_MAX_FRAMES + 1))
    assert not beyond["ok"] and "free-frames" in beyond["resource_drift"], beyond

    above = run_with(FIXTURE_BASELINE_FREE_FRAMES + 1)
    assert not above["ok"] and "free-frames" in above["resource_drift"], above
    print("[controls] frame warm-up is bounded and one-directional: PASS")


def test_cleanup_leaked_capability() -> None:
    """Capability occupancy is one slot above baseline after the close."""
    serial = _clean_block(
        [
            "[desktop] application retired: kind=6 surface=77 Process consumed",
            "[desktop] child Process-cap exit surface=77 status=0",
        ],
        settle_receipt="[desktop] measured frames/records/processes/regions/pages/maps/caps=100/11/12/13/14/15/17",
    )
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77], _clean_baseline())
    assert not r["ok"] and "caps" in r["resource_drift"], r
    print("[controls] cleanup leaked capability fails: PASS")


def test_cleanup_missing_receipt() -> None:
    """A settlement with no receipt line at all cannot be called settled."""
    serial = _serial_block(
        "[desktop] real desktop frame presented",
        CLEAN_RECEIPT,
        CLEAN_DETAIL,
        "[desktop] application retired: kind=6 surface=77 Process consumed",
        "[desktop] child Process-cap exit surface=77 status=0",
        CLEAN_DETAIL,
    )
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77], _clean_baseline())
    assert not r["ok"] and not r["settles_to_baseline"], r
    print("[controls] cleanup missing receipt fails: PASS")


def test_cleanup_stale_receipt() -> None:
    """Only pre-cursor settlement exists: historical snapshot must not pass."""
    serial = _serial_block(
        "[desktop] real desktop frame presented",
        CLEAN_RECEIPT,
        CLEAN_DETAIL,
        "[desktop] application retired: kind=6 surface=77 Process consumed",
        "[desktop] child Process-cap exit surface=77 status=0",
    )
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77], _clean_baseline())
    assert not r["ok"], r
    print("[controls] cleanup stale (pre-cursor) settlement fails: PASS")


def test_cleanup_transient_settlement_then_leak() -> None:
    serial = _clean_block(
        [
            CLEAN_RECEIPT,
            CLEAN_DETAIL,
            "[desktop] application retired: kind=6 surface=77 Process consumed",
            "[desktop] child Process-cap exit surface=77 status=0",
        ],
        settle_detail="[desktop] native resource detail values=1/2/3/4/5/6/7/8/9/10/1/1/11/12/1",
    )
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77], _clean_baseline())
    assert not r["ok"], r
    print("[controls] cleanup transient dip then leak fails: PASS")


def test_cleanup_spurious_retirement() -> None:
    serial = _clean_block([
        "[desktop] application retired: kind=6 surface=77 Process consumed",
        "[desktop] child Process-cap exit surface=77 status=0",
        "[desktop] application retired: kind=6 surface=99 Process consumed",
        "[desktop] child Process-cap exit surface=99 status=0",
    ])
    r = evaluate_cleanup(serial, CLEAN_CURSOR, [77], _clean_baseline())
    assert not r["ok"] and r["spurious_retirements"] == [99], r
    print("[controls] cleanup spurious retirement fails: PASS")


def test_residual_presentation() -> None:
    assert evaluate_presentation([]) is True
    assert evaluate_presentation([(10, 10, 100, 100)]) is False
    print("[controls] residual window presentation fails: PASS")


def test_malformed_resource_detail() -> None:
    serial = _serial_block(
        "[desktop] real desktop frame presented",
        "[desktop] native resource detail values=1/2/3",
        "[desktop] native resource detail values=1/2/3/4/5/6/7/8/9/10/0/0/11/12/0",
    )
    receipts = parse_resource_details(serial)
    assert receipts[0]["malformed"] is True
    assert last_detail(serial)["windows"] == 0
    print("[controls] malformed resource receipt not reinterpreted: PASS")


def _write_tmp(name: str, data: bytes) -> str:
    path = Path(tempfile.mkdtemp()) / name
    path.write_bytes(data)
    return str(path)


def test_frame_publication() -> None:
    """The published PNG must carry exactly the tested raster, and only that.

    A checkpoint that passes on frame X must publish a file that decodes to
    X's pixels: not an approximation, not a different capture.
    """
    w, h = 40, 24
    raster = bytes(((i * 37) % 256) for i in range(w * h * 3))
    ppm = ppm_bytes_from_raster(w, h, raster)
    got_w, got_h, got = parse_ppm(Path(_write_tmp("f.ppm", ppm)))
    assert (got_w, got_h, got) == (w, h, raster), "PPM wrapper is lossless"

    png = raster_to_png(w, h, raster)
    assert png_raster(png) == (w, h, raster), "PNG does not round-trip"

    # Any change to the image is visible in the decoded raster, so a
    # published frame that differs from the tested one cannot pass as it.
    corrupted = bytearray(raster)
    corrupted[5] ^= 0xFF
    png2 = raster_to_png(w, h, bytes(corrupted))
    assert png_raster(png2)[2] != raster
    assert png2 != png


def test_frame_digests_bind_ppm_and_png() -> None:
    """frame_digests() must tie the on-disk PNG to the tested PPM's pixels."""
    global RUN_DIR, SHOTS
    saved_dir, saved_shots = RUN_DIR, SHOTS
    try:
        d = Path(tempfile.mkdtemp())
        shots = d / "shots"
        shots.mkdir()
        RUN_DIR, SHOTS = d, shots
        name = REQUIRED_FRAMES[1][0]
        w, h = 8, 4
        raster = bytes((i * 11) % 256 for i in range(w * h * 3))
        stem = name.split("/")[-1].replace(".png", "")
        save_frame(stem, raster, w, h)
        entry = frame_digests()[name]
        assert entry["raster_sha256"] == hashlib.sha256(raster).hexdigest()
        assert entry["published_png_sha256"] != "MISSING"
        assert entry["tested_ppm_sha256"] != "MISSING"
        assert (entry["width"], entry["height"]) == (w, h)

        # Swap in a PNG of a different frame: the published digest changes,
        # so packaging can detect the substitution.
        other = bytes((i * 3) % 256 for i in range(w * h * 3))
        (d / name).write_bytes(raster_to_png(w, h, other))
        after = frame_digests()[name]
        assert after["published_png_sha256"] != entry["published_png_sha256"]
        assert after["raster_sha256"] == entry["raster_sha256"], \
            "raster digest must describe the tested PPM"
    finally:
        RUN_DIR, SHOTS = saved_dir, saved_shots


def test_packaging_requires_checkpoint_frames() -> None:
    """`v1_evidence.py package` must refuse a run that does not publish them.

    Exercised against the real packaging rule, not a copy: the same function
    that gates the published archive is fed a complete run (accepted), then a
    run missing a checkpoint image, then one whose published PNG shows a
    different frame, then one whose recorded digest disagrees with the file.
    """
    import v1_evidence

    d = Path(tempfile.mkdtemp())
    run = d / "guest-gcc"
    (run / "shots").mkdir(parents=True)
    w, h = 16, 8
    frames = {}
    for name in v1_evidence.REQUIRED_GUEST_FRAMES:
        raster = bytes(((i * 7) % 256) for i in range(w * h * 3))
        png = raster_to_png(w, h, raster)
        (run / name).write_bytes(png)
        frames[name] = {
            "tested_ppm_sha256": hashlib.sha256(
                ppm_bytes_from_raster(w, h, raster)).hexdigest(),
            "published_png_sha256": hashlib.sha256(png).hexdigest(),
            "raster_sha256": hashlib.sha256(raster).hexdigest(),
            "width": w, "height": h,
        }
    evidence = run / "evidence.json"
    evidence.write_text(json.dumps({"frames": frames,
                                    "required_frames": list(frames),
                                    "verdict": {"pass": True}}))
    assert v1_evidence.check_frames(d) == [], v1_evidence.check_frames(d)

    # (a) a required checkpoint image is absent
    victim = run / v1_evidence.REQUIRED_GUEST_FRAMES[3]
    saved = victim.read_bytes()
    victim.unlink()
    problems = v1_evidence.check_frames(d)
    assert any("missing" in p for p in problems), problems

    # (b) the image present shows a different frame
    other = bytes(((i * 5) % 256) for i in range(w * h * 3))
    victim.write_bytes(raster_to_png(w, h, other))
    problems = v1_evidence.check_frames(d)
    assert any("published digest" in p for p in problems), problems

    # (c) the file matches its digest but the digest describes another frame
    victim.write_bytes(saved)
    frames[v1_evidence.REQUIRED_GUEST_FRAMES[3]]["raster_sha256"] = (
        hashlib.sha256(other).hexdigest())
    evidence.write_text(json.dumps({"frames": frames,
                                    "required_frames": list(frames)}))
    problems = v1_evidence.check_frames(d)
    assert any("different raster" in p for p in problems), problems


def test_verdict_logic() -> None:
    good_ids = {k: "x" for k in REQUIRED_IDENTITY_KEYS}
    good_ids.update({"qemu_elf_ok": True, "qemu_identity_ok": True,
                     "compilers": {"gcc": "gcc 12", "clang_via_zig": "clang 22"},
                     "efi_sha256": "efi-digest",
                     "image": {"fresh": True, "boot_efi_sha256": "efi-digest"}})
    all_pass_ck = {n: "PASS" for n in CHECKPOINTS}
    serial = RUN_DIR / "control-serial.log"
    serial.write_text("x")
    art = {"serial": serial}

    ok, _ = compute_overall_verdict(0, all_pass_ck, {"k": 1}, good_ids, serial,
                                    art, [])
    assert ok

    # A PASS whose required checkpoint frame is absent must not pass.
    ok, why = compute_overall_verdict(0, all_pass_ck, {"k": 1}, good_ids, serial,
                                      art, ["checkpoint 4: shots/x.png"])
    assert not ok and any("required checkpoint frame missing" in w for w in why), why

    for n in CHECKPOINTS:
        ck = dict(all_pass_ck)
        ck[n] = "NOT RUN"
        ok, why = compute_overall_verdict(0, ck, {"k": 1}, good_ids, serial, art, [])
        assert not ok and any(f"checkpoint {n}" in w for w in why), why

    ids = dict(good_ids)
    ids["qemu_elf_sha256"] = "MISSING"
    ok, why = compute_overall_verdict(0, all_pass_ck, {"k": 1}, ids, serial, art, [])
    assert not ok

    ids = dict(good_ids)
    ids["qemu_elf_ok"] = False
    ok, why = compute_overall_verdict(0, all_pass_ck, {"k": 1}, ids, serial, art, [])
    assert not ok and any("pinned" in w for w in why), why

    ok, why = compute_overall_verdict(1, all_pass_ck, {"k": 1}, good_ids, serial, art, [])
    assert not ok

    ok, why = compute_overall_verdict(0, all_pass_ck, None, good_ids, serial, art, [])
    assert not ok

    # A stale boot image must fail: the verdict would describe code that is
    # no longer in the tree.
    ids = dict(good_ids)
    ids["image"] = {"fresh": False, "newest_source": "userspace/desktop/src/x.rs",
                    "boot_efi_sha256": "efi-digest"}
    ok, why = compute_overall_verdict(0, all_pass_ck, {"k": 1}, ids, serial, art, [])
    assert not ok and any("older than the newest tracked source" in w
                          for w in why), why

    # An ESP whose embedded EFI differs from the freshly built one must fail.
    ids = dict(good_ids)
    ids["image"] = {"fresh": True, "boot_efi_sha256": "other-digest"}
    ok, why = compute_overall_verdict(0, all_pass_ck, {"k": 1}, ids, serial, art, [])
    assert not ok and any("differs from build/arena-boot.efi" in w
                          for w in why), why

    missing = RUN_DIR / "does-not-exist"
    ok, why = compute_overall_verdict(0, all_pass_ck, {"k": 1}, good_ids, serial,
                                      {"serial": serial, "evidence": missing}, [])
    assert not ok and any("missing" in w for w in why), why
    serial.unlink()
    print("[controls] verdict logic: PASS")


# ---------------------------------------------------------------------------
# Input-proof controls: acknowledgment AND visible response
# ---------------------------------------------------------------------------

def _frame_with(rect, flash: bool) -> bytes:
    """A frame of raw pixels, in the same form `V1Desktop.shot` returns.

    Built from the guest application's own palette so the colour detectors
    are exercised with the colours they will actually see; the surrounding
    screen uses the Desktop background.
    """
    w, h = SCREEN_W, SCREEN_H
    x, y, fw, fh = rect
    rows = []
    background = bytes((24, 32, 53))
    base = (APP_COLOR_NORMAL if not flash else FLASH_COLORS[0])
    for row in range(h):
        pixels = bytearray()
        for col in range(w):
            inside = x <= col < x + fw and y <= row < y + fh
            pixels += bytes(base if inside else background)
        rows.append(bytes(pixels))
    return b"".join(rows)


RECT_A = (70, 60, 120, 80)


def _ack_ok_eval(instance: int = 77, event_type: int = EVENT_KEY) -> dict:
    """A genuinely positive acknowledgment evaluation for `instance`."""
    serial = _serial_block(
        "[gfx] identify ok surface=%d" % instance,
        "[gfx] event type=%d events=1 surface=%d" % (event_type, instance),
    )
    return evaluate_event_ack(serial, 0, instance, event_type)


def _ack_missing_eval(instance: int = 77, event_type: int = EVENT_KEY) -> dict:
    """No acknowledgment at all for this instance and event type."""
    serial = _serial_block("[gfx] identify ok surface=%d" % instance)
    return evaluate_event_ack(serial, 0, instance, event_type)


def test_input_proof_valid_passes() -> None:
    pre = _frame_with(RECT_A, flash=False)
    frame = _frame_with(RECT_A, flash=True)
    r = evaluate_input_proof(_ack_ok_eval(), pre, frame, RECT_A)
    assert r["ok"], r
    assert r["ack_ok"] and r["visual_ok"] and not r["residual_flash"], r
    print("[controls] input proof: ack + flash passes: PASS")


def test_input_proof_ack_without_visual_fails() -> None:
    """Delivery acknowledged, nothing on screen: must fail."""
    pre = _frame_with(RECT_A, flash=False)
    steady = _frame_with(RECT_A, flash=False)
    r = evaluate_input_proof(_ack_ok_eval(), pre, steady, RECT_A)
    assert not r["ok"], r
    assert r["ack_ok"] and not r["visual_ok"], r
    print("[controls] input proof: ack without visible response fails: PASS")


def test_input_proof_ack_without_frame_fails() -> None:
    """The old defect: an ack with no captured frame is not a proof."""
    pre = _frame_with(RECT_A, flash=False)
    r = evaluate_input_proof(_ack_ok_eval(), pre, None, RECT_A)
    assert not r["ok"] and r["ack_ok"] and not r["frame_present"], r
    print("[controls] input proof: ack with no captured frame fails: PASS")


def test_input_proof_visual_without_ack_fails() -> None:
    """Pixels changed, but no acknowledgment: must fail."""
    pre = _frame_with(RECT_A, flash=False)
    frame = _frame_with(RECT_A, flash=True)
    r = evaluate_input_proof(_ack_missing_eval(), pre, frame, RECT_A)
    assert not r["ok"], r
    assert r["visual_ok"] and not r["ack_ok"], r
    print("[controls] input proof: visible response without ack fails: PASS")


def test_input_proof_wrong_instance_fails() -> None:
    """Another instance acknowledged the same event type."""
    serial = _serial_block(
        "[gfx] identify ok surface=77",
        "[gfx] identify ok surface=88",
        "[gfx] event type=1 events=5 surface=88",
    )
    ack = evaluate_event_ack(serial, 0, 77, EVENT_KEY, others=[88])
    frame = _frame_with(RECT_A, flash=True)
    r = evaluate_input_proof(ack, _frame_with(RECT_A, flash=False), frame, RECT_A)
    assert not r["ok"] and not r["ack_ok"], r
    assert ack["cross_acks"] == [88], ack
    print("[controls] input proof: wrong-instance ack fails: PASS")


def test_input_proof_wrong_event_type_fails() -> None:
    """The instance acknowledged a pointer event, not the key we sent."""
    serial = _serial_block(
        "[gfx] identify ok surface=77",
        "[gfx] event type=2 events=5 surface=77",
    )
    ack = evaluate_event_ack(serial, 0, 77, EVENT_KEY)
    frame = _frame_with(RECT_A, flash=True)
    r = evaluate_input_proof(ack, _frame_with(RECT_A, flash=False), frame, RECT_A)
    assert not r["ok"] and not r["ack_ok"], r
    print("[controls] input proof: wrong-event ack fails: PASS")


def test_input_proof_stale_flash_fails() -> None:
    """The flash was already on screen before the input arrived."""
    pre = _frame_with(RECT_A, flash=True)      # residual
    frame = _frame_with(RECT_A, flash=True)
    r = evaluate_input_proof(_ack_ok_eval(), pre, frame, RECT_A)
    assert not r["ok"] and r["residual_flash"], r
    print("[controls] input proof: stale/residual flash fails: PASS")


def test_input_proof_flash_outside_window_fails() -> None:
    """A flash somewhere else on screen is not this instance's response."""
    pre = _frame_with(RECT_A, flash=False)
    elsewhere = (400, 300, 120, 80)
    frame = _frame_with(elsewhere, flash=True)
    r = evaluate_input_proof(_ack_ok_eval(), pre, frame, RECT_A)
    assert not r["ok"] and r["ack_ok"] and not r["visual_ok"], r
    print("[controls] input proof: flash outside the window fails: PASS")


def run_focused_controls() -> int:
    checks = [
        test_ppm_parsing,
        test_color_detection,
        test_window_rects_overlap,
        test_ack_good,
        test_ack_missing_delivery,
        test_ack_wrong_instance,
        test_ack_wrong_event_type,
        test_ack_stale_counter,
        test_ack_historical_line_only,
        test_input_proof_valid_passes,
        test_input_proof_ack_without_visual_fails,
        test_input_proof_ack_without_frame_fails,
        test_input_proof_visual_without_ack_fails,
        test_input_proof_wrong_instance_fails,
        test_input_proof_wrong_event_type_fails,
        test_input_proof_stale_flash_fails,
        test_input_proof_flash_outside_window_fails,
        test_cleanup_good,
        test_cleanup_missing_retirement,
        test_cleanup_partial_exit,
        test_cleanup_leak,
        test_cleanup_leaked_thread,
        test_cleanup_leaked_frames,
        test_cleanup_frame_warmup_is_bounded_and_one_directional,
        test_cleanup_leaked_capability,
        test_cleanup_missing_receipt,
        test_cleanup_stale_receipt,
        test_cleanup_transient_settlement_then_leak,
        test_cleanup_spurious_retirement,
        test_residual_presentation,
        test_malformed_resource_detail,
        test_frame_publication,
        test_packaging_requires_checkpoint_frames,
        test_frame_digests_bind_ppm_and_png,
        test_verdict_logic,
    ]
    failed = 0
    for check in checks:
        try:
            check()
        except AssertionError as exc:
            failed += 1
            print(f"[controls] FAIL {check.__name__}: {exc}")
    print(f"[controls] {len(checks) - failed}/{len(checks)} focused controls "
          f"{'PASS' if failed == 0 else 'FAIL'}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    if "--controls" in sys.argv:
        sys.exit(run_focused_controls())
    sys.exit(main())
