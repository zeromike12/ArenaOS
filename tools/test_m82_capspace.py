#!/usr/bin/env python3
"""ADR-0048 fixed cap-table foundation plus integrated volatile broker.

The broker is one real live manager child at this snapshot; its
short-lived app and readiness worker must both have been reaped.
"""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402
from test_m81_update import resource_use  # noqa: E402

LABEL = "m82-capspace"


def main() -> int:
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    rc, serial, elapsed = mtest.boot(LABEL, esp,
        [((b"arena>", b"permission app reaped through held Process cap"),
          1, b"stackstress\r"),
         (b"arena>", 2, b"shutdown\r")], disk)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    counters = resource_use(serial)
    passed = (rc == 0 and "m3:test:capability_spaces: PASS" in serial
              and "capability_spaces: 32 slots/space" in serial
              and "m8: stackstress PASS" in serial and counters is not None
              and counters[1:] == (11, 11)
              and "servicemgr: permission PING result + exit before deadline; worker reaped" in serial
              and "servicemgr: permission app reaped through held Process cap" in serial
              and "servicemgr: permissiond READY" in serial
              and "configup: SKIP" in serial and "PANIC" not in serial
              and "halting via UEFI ResetSystem(shutdown)" in serial)
    print(f"[{LABEL}] rc={rc} {elapsed:.1f}s; post-EBS-relative "
          f"frames/records/processes={counters}")
    print(f"[{LABEL}] CAPSPACE32: {'PASS' if passed else 'FAIL'} "
          "(guest last-slot copy/move/full-refusal, three live restart cycles)")
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
