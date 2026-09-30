#!/usr/bin/env python3
"""ADR-0046: eight guest commits, ninth refusal, old bytes and resources flat."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
from test_m81_update import get, resource_use  # noqa: E402

LABEL = "m81-table"


def main() -> int:
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    ok = True
    rc, serial, dt = mtest.boot(LABEL + "-intent", esp,
                                [((b"arena>", b"permission app reaped through held Process cap"), 1, b"stackstress\r"),
                                 (b"arena>", 2, b"write cfg-intent-one x\r"),
                                 (b"arena>", 3, b"shutdown\r")], disk)
    print(f"[{LABEL}] staged: rc={rc}, {dt:.1f}s")
    reference = resource_use(serial)
    ok &= rc == 0 and "configup: SKIP" in serial and not afs1.audit(disk)
    ok &= reference is not None and "m8: stackstress PASS" in serial
    values = []
    for i in range(1, 9):
        current = b"one" if i % 2 else b"two"
        next_ = b"two" if i % 2 else b"one"
        marker = b"guest-v1" if i % 2 else b"guest-v2"
        script = [((b"arena>", b"permission app reaped through held Process cap"), 1, b"stackstress\r"),
                  (b"arena>", 2, b"rm cfg-intent-" + current + b"\r"),
                  (b"arena>", 3, b"write cfg-intent-" + next_ + b" x\r"),
                  (b"arena>", 4, b"shutdown\r")]
        rc, serial, dt = mtest.boot(LABEL + f"-commit{i}", esp, script, disk)
        (arena_env.build_dir() / f"serial-{LABEL}-commit{i}.log").write_text(serial)
        values.append(marker)
        exact = all(get(disk, n) == val for n, val in enumerate(values, 1))
        accounting = resource_use(serial) == reference and "m8: stackstress PASS" in serial
        passed = (rc == 0 and "configup: SET COMMITTED" in serial
                  and "configup: TRUSTED UPDATE PASS" in serial
                  and "m5: RESULT PASS (6/6)" in serial
                  and accounting and exact and not afs1.audit(disk))
        print(f"[{LABEL}] generation {i}: {'PASS' if passed else 'FAIL'} "
              f"(rc={rc}, {dt:.1f}s, old bytes={exact}, boot-relative counters={resource_use(serial)})")
        ok &= passed
        if not ok:
            return 1
    rc, serial, dt = mtest.boot(LABEL + "-full", esp,
                                [((b"arena>", b"permission app reaped through held Process cap"), 1, b"stackstress\r"),
                                 (b"arena>", 2, b"shutdown\r")], disk)
    (arena_env.build_dir() / f"serial-{LABEL}-full.log").write_text(serial)
    # get() intentionally names only canonical generations 1..8. Do not
    # construct a ninth name: inspect the actual committed namespace.
    d = afs1.Disk(disk.read_bytes())
    _, ot, _ = d.commit()
    passed = (rc == 0 and "configup: SET NO_SPACE (eight immutable generations)" in serial
              and "configup: table preflight stayed bounded, not degraded" in serial
              and "m8: stackstress PASS" in serial and resource_use(serial) == reference
              and all(get(disk, n) == val for n, val in enumerate(values, 1))
              and not any(o['name'].startswith(b'cfg8-0') and o['name'] not in
                          [f'cfg8-{n:02d}'.encode() for n in range(1, 9)]
                          for o in d.objects(ot) if o['type'] == afs1.OBJ_FILE)
              and not afs1.audit(disk))
    print(f"[{LABEL}] ninth attempt: {'PASS' if passed else 'FAIL'} "
          f"(rc={rc}, {dt:.1f}s, eight complete immutable generations)")
    if passed:
        print(f"[{LABEL}] PASS: table preflight stayed bounded, not degraded")
    ok &= passed
    print(f"[{LABEL}] EIGHT-SLOT EXHAUSTION: {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
