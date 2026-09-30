#!/usr/bin/env python3
"""ADR-0048 fixed cap-table foundation; no 8.2 permission broker claimed."""
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
        [(b"arena>", 1, b"stackstress\r"),
         (b"arena>", 2, b"shutdown\r")], disk)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    counters = resource_use(serial)
    passed = (rc == 0 and "m3:test:capability_spaces: PASS" in serial
              and "capability_spaces: 32 slots/space" in serial
              and "m8: stackstress PASS" in serial and counters is not None
              and counters[1:] == (10, 10)
              and "configup: SKIP" in serial and "PANIC" not in serial
              and "halting via UEFI ResetSystem(shutdown)" in serial)
    print(f"[{LABEL}] rc={rc} {elapsed:.1f}s; post-EBS-relative "
          f"frames/records/processes={counters}")
    print(f"[{LABEL}] CAPSPACE32: {'PASS' if passed else 'FAIL'} "
          "(guest last-slot copy/move/full-refusal, three live restart cycles)")
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
