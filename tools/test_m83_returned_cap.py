#!/usr/bin/env python3
"""ADR-0052: real ring-3 cleanup of an unexpected IPC-landed reply cap.

The isolated faultd server answers forty linked-library calls with an inert
READ-only Notification cap. Its two independent normal-mode faulttest
clients prove typed refusal, no describable landing, exact cap occupancy,
no fixed-table exhaustion and a later ordinary no-cap PING. The original
M6.5 death/restart proof still has to complete on the same boot.
"""
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = 'm83-returned-cap'
PASS = ('m83: returncap PASS (40 real reply caps rejected and discarded; '
        'slot 2 empty, occupancy 2/32 exact; ordinary no-cap PING unchanged)')


def main():
    esp = mtest.build(LABEL)
    rc, serial, secs = mtest.run_qemu(LABEL, esp)
    (arena_env.build_dir() / f'serial-{LABEL}.log').write_text(serial)
    ok = (rc == 0 and serial.count(PASS) == 2
          and 'm6: RESULT PASS (6/6)' in serial
          and 'faulttest: PASS — the HANG call returned STATUS_SERVICE_GONE' in serial
          and 'faulttest: PASS — the RESTARTED service answered on the SAME endpoint' in serial
          and 'm83: returncap FAIL' not in serial
          and '[arena ERROR halt]' not in serial
          and 'halting via UEFI ResetSystem(shutdown)' in serial)
    print(f'[{LABEL}] {"PASS" if ok else "FAIL"}: rc={rc}, {secs:.1f}s, '
          f'controlled server/client proofs={serial.count(PASS)}, '
          'historical death/restart and clean shutdown')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
