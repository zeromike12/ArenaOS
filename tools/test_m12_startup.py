#!/usr/bin/env python3
"""Phase 12.4/12.5: real ring-3 startup, bounded-heap and refusal proof.

The independently linked guest validates ABI v2, allocates through the
reusable runtime, fills its 32-page bound, and refuses the 33rd page. A second
valid startup case occupies reserved slot 63 with a deliberately attenuated
Notification and proves allocator OOM preserves that exact cap. Four startup
RED controls refuse before the application closure.
"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402

LABEL = "m12-startup"
EXPECTED = [
    "runtime_guest",
    "listed_cap_rights_red",
    "startup_cap_rights_red",
    "startup_page_count_red",
    "malformed_record_red",
    "unlisted_capability_red",
    "heap_slot_collision_red",
]


def main() -> int:
    rc = mtest.run_milestone("m12", EXPECTED)
    if rc:
        return rc
    serial = (Path(__file__).resolve().parents[1] / "build/serial-m12.log").read_text()
    needed = (
        "m12:startup:tls: PASS",
        "m12:startup:handles: PASS",
        "m12:startup:capability-handles: PASS",
        "m12:startup:process-group: PASS",
        "m12:startup:heap: PASS",
        "m12:startup:heap-slot-red: PASS",
        "m12:startup:application: PASS",
        "m12: RESULT PASS (7/7)",
    )
    missing = [marker for marker in needed if marker not in serial]
    if missing:
        print(f"[{LABEL}] FAIL: missing guest/manager evidence: {missing}")
        return 1
    if "m12:startup:badcap: APPLICATION-RAN" in serial or serial.count(
        "arena-runtime: startup refused"
    ) < 5:
        print(f"[{LABEL}] FAIL: startup refusal controls did not remain fail-closed")
        return 1
    if "m12:startup:heap: FAIL" in serial:
        print(f"[{LABEL}] FAIL: bounded heap did not prove capacity, reuse and OOM")
        return 1
    print(
        f"[{LABEL}] PASS: startup guest validated argv/env/cap/entry/RSP, "
        "per-thread FS-base TLS across a timed scheduler handoff, and the "
        "generation-safe runtime handle table with real capability copy, "
        "attenuation and close plus Process-cap child spawn/wait/reap and "
        "bare-PID refusal; "
        "bounded heap "
        "filled/reused 32 pages and refused page 33; reserved-slot collision "
        "preserved the live Notification; five startup RED controls, including "
        "an unlisted live cap, and exact "
        "process-teardown resource return passed"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
