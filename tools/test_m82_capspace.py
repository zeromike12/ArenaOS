#!/usr/bin/env python3
"""ADR-0048 fixed cap-table foundation plus integrated volatile broker.

The broker AND ADR-0053 verify-only package receiver are two real live
manager children. Their short-lived apps/readiness workers must be reaped.
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
              # ADR-0075 raised the fixed table from 32 to 64 slots.
              and "capability_spaces: 64 slots/space" in serial
              and "m8: stackstress PASS" in serial and counters is not None
              # Phase 8.3 was 11/11; packaged adds one, displayd adds
              # one (13/13 before live graphics). ADR-0060 adds exactly
              # three disjoint BootImage residents: compositor and TWO
              # owned windows, measured 16/16 in a real QEMU guest.
              # No extra dynamic child or readiness worker remains live.
              and counters[1:] == (16, 16)
              and "servicemgr: packaged READY (full boot scan; exact PING + exit + deadline)" in serial
              and "packaged: boot with exact FS/W endpoint/R STAGE/R registrar/W lifecycle/R; namespace scan verified" in serial
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
