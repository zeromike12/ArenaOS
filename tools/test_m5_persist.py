#!/usr/bin/env python3
"""M5.4 exit criterion #2: TWO-BOOT PERSISTENCE — the same scratch.img,
written in boot N, read in boot N+1, with the host parsing the real
committed sectors after each boot (tools/afs1.py, the layout's source
of truth).

Boot 1 (fresh AFS1 volume):
  - the m5 suite's fs_service runs its FRESH contract (fstest exit 42,
    the kernel asserts the 34-device-op derivation),
  - the production shell then `write persist.txt <marker>` and reads
    it back — through the REAL fsd + storaged, on the SAME disk the
    suite just committed to,
  - clean shutdown (rc must be 0 — a persistence claim built on a
    crashed boot proves nothing).
  Host proof #1: the image audits clean, and persist.txt's committed
  sectors hold the exact marker bytes.

Boot 2 (NO reformat — the volume from boot 1):
  - the suite's branch probe finds arena.txt already committed: the
    PERSISTED contract runs (fstest exit 43 — verify with ZERO writes,
    the kernel asserts the 14-op derivation),
  - fsd's mount reclaims the superseded ping-pong generation (the
    cross-reboot half of AFS1's two-generation discipline),
  - the shell `cat persist.txt` streams back the EXACT boot-1 bytes,
    `rm persist.txt` unlinks the rebooted file transactionally, and
    the final `ls` counts only arena.txt,
  - clean shutdown.
  Host proof #2: the image audits clean, persist.txt is gone from the
  newest commit, and arena.txt's 512 pattern bytes are STILL byte-exact
  (an unlink must never disturb its neighbours).

Exit code: 0 = PASS, 1 = FAIL (serial tails printed for diagnosis).
"""

import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "test-m5-persist"
MARKER = b"ARENA-PERSISTED-ACROSS-REBOOTS"

ok = True


def check(cond: bool, msg: str) -> None:
    global ok
    print(f"[{LABEL}] {'PASS' if cond else 'FAIL'}: {msg}")
    if not cond:
        ok = False


def file_bytes(d: afs1.Disk, obj: dict) -> bytes:
    """Concatenate the file's committed sectors (v1 is append-only:
    chain order IS file order) and trim to the recorded size."""
    out = bytearray()
    for (st, ln) in d.extents(obj["extent_head"]):
        for s in range(st, st + ln):
            out += d.sector(s)
    return bytes(out[: obj["size"]])


def dump(serial: str) -> None:
    print(f"[{LABEL}] ---- serial tail ----")
    print("\n".join(serial.splitlines()[-25:]))


def main() -> int:
    try:
        esp = mtest.build(LABEL)
    except subprocess.CalledProcessError as e:
        print(f"[{LABEL}] FAIL: kernel build failed:\n{e.stdout}\n{e.stderr}")
        return 1

    scratch = arena_env.make_scratch_disk()  # fresh AFS1 for boot 1

    # ---------------- boot 1: write on a fresh volume ----------------
    rc1, s1, dt1 = mtest.boot(
        LABEL + "-b1", esp,
        [(b"arena>", 1, b"write persist.txt " + MARKER + b"\r"),
         (b"arena>", 2, b"cat persist.txt\r"),
         (b"arena>", 3, b"shutdown\r")],
        scratch)
    print(f"[{LABEL}] boot 1 done (rc={rc1}, {dt1:.1f}s)")
    check(rc1 == 0, "boot 1 shut down cleanly (rc=0)")
    check("m5: RESULT PASS (6/6)" in s1, "boot 1: the m5 suite passed (6/6)")
    check("fstest: PASS (fresh)" in s1,
          "boot 1: fs_service ran the FRESH contract (exit 42)")
    check(f"wrote {len(MARKER)} bytes to 'persist.txt'" in s1,
          "boot 1: the shell created persist.txt through fsd")
    check(b"\n" + MARKER + b"\n" in s1.encode(errors="replace"),
          "boot 1: cat streamed the marker back in the same boot")
    check("halting via UEFI ResetSystem(shutdown)" in s1,
          "boot 1: the kernel declared its clean halt")
    if not ok:
        dump(s1)
        return 1

    # host proof #1: the committed sectors hold the marker
    probs = afs1.audit(scratch)
    check(not probs, f"boot 1: host audit clean ({probs[:2] if probs else 'no problems'})")
    d = afs1.Disk(Path(scratch).read_bytes())
    seq, ot, _ = d.commit()
    obj = d.find(b"persist.txt", ot)
    check(obj is not None, "boot 1: persist.txt exists in the newest commit "
                           f"(seq {seq})")
    if obj:
        check(obj["size"] == len(MARKER),
              f"boot 1: committed size {obj['size']} == {len(MARKER)}")
        check(file_bytes(d, obj) == MARKER,
              "boot 1: the marker bytes are physically in the committed sectors")
    if not ok:
        return 1

    # ---------------- boot 2: the SAME disk, no reformat ----------------
    rc2, s2, dt2 = mtest.boot(
        LABEL + "-b2", esp,
        [(b"arena>", 1, b"cat persist.txt\r"),
         (b"arena>", 2, b"ls\r"),
         (b"arena>", 3, b"rm persist.txt\r"),
         (b"arena>", 4, b"ls\r"),
         (b"arena>", 5, b"shutdown\r")],
        scratch)
    print(f"[{LABEL}] boot 2 done (rc={rc2}, {dt2:.1f}s)")
    check(rc2 == 0, "boot 2 shut down cleanly (rc=0)")
    check("m5: RESULT PASS (6/6)" in s2, "boot 2: the m5 suite passed (6/6)")
    check("succeeded BEFORE any create" in s2,
          "boot 2: fstest's branch probe found the file from boot 1")
    check("fstest: PASS (persisted)" in s2,
          "boot 2: fs_service ran the PERSISTED contract (exit 43, zero writes)")
    check("RE-FOUND a file committed by an EARLIER BOOT" in s2,
          "boot 2: the kernel confirmed the persisted branch's op contract")
    check(re.search(r"fsd: reclaimed \d+ superseded metadata sector", s2) is not None,
          "boot 2: fsd's mount reclaimed the superseded ping-pong generation")
    check(b"\n" + MARKER + b"\n" in s2.encode(errors="replace"),
          "boot 2: cat streamed back the EXACT bytes written in boot 1")
    check(f"persist.txt  {len(MARKER)} bytes" in s2,
          "boot 2: the first ls listed persist.txt with its committed size")
    check("removed 'persist.txt'" in s2,
          "boot 2: rm unlinked the rebooted file transactionally")
    tail = s2.split("removed 'persist.txt'", 1)[1]
    check("  1 file(s)" in tail,
          "boot 2: the final ls counts only the suite's arena.txt")
    check("halting via UEFI ResetSystem(shutdown)" in s2,
          "boot 2: the kernel declared its clean halt")
    if not ok:
        dump(s2)
        return 1

    # host proof #2: unlink is surgical — persist.txt gone, arena.txt intact
    probs = afs1.audit(scratch)
    check(not probs, f"boot 2: host audit clean ({probs[:2] if probs else 'no problems'})")
    d = afs1.Disk(Path(scratch).read_bytes())
    seq2, ot2, _ = d.commit()
    check(d.find(b"persist.txt", ot2) is None,
          f"boot 2: persist.txt is gone from the newest commit (seq {seq2})")
    arena = d.find(b"arena.txt", ot2)
    check(arena is not None and arena["size"] == afs1.SECTOR,
          "boot 2: arena.txt survived at 512 bytes")
    if arena:
        want = bytes(afs1.pattern_byte(i) for i in range(afs1.SECTOR))
        check(file_bytes(d, arena) == want,
              "boot 2: arena.txt's pattern is STILL byte-exact after the unlink")

    print(f"[{LABEL}] ==============================================")
    if ok:
        print(f"[{LABEL}] TWO-BOOT PERSISTENCE: PASS  "
              f"(serials: build/serial-{LABEL}-b1.log, build/serial-{LABEL}-b2.log)")
        return 0
    print(f"[{LABEL}] TWO-BOOT PERSISTENCE: FAIL")
    return 1


if __name__ == "__main__":
    sys.exit(main())
