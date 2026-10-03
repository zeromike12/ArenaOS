#!/usr/bin/env python3
"""M5.4 exit criterion #3: the CRASH-CONSISTENCY GATE — the harness
kills QEMU mid-write, reboots, and verifies recovery WITHOUT an fsck
ritual. No repair tool runs anywhere: fsd simply mounts, and the
volume must already be right.

The crash model is SIGKILL on the QEMU process: writes it submitted
are in the host page cache and survive the process death; unsubmitted
ones are lost. The guest therefore experiences a strict PREFIX of its
issued device operations — exactly what AFS1's commit protocol is
built for (data and metadata sectors first, the ping-pong commit
record LAST, so any prefix leaves either the old generation or the
new one, never a blend).

A seed boot commits base.txt cleanly. Then four rounds, each killing
at a different observed point of a shell `write` (the assertion set is
identical — atomicity, not timing, is the claim):

  R0: at the first storaged WRITE line  — inside the CREATE commit
  R1: at the third                      — deeper into the same commit
  R2: just after fsd's CREATE commit log — inside the WRITE phase
  R3: at the 14th WRITE line            — inside the WRITE commit's
                                          metadata runs
  R4: the instant the shell confirms the write — a crash AFTER the
      full commit, where the file MUST come back complete

After EVERY kill, a verification boot (no reformat) must show:
  - the m5 suite passing 6/6 with fstest's PERSISTED branch: the
    pre-crash arena.txt verifies byte-for-byte after the crash —
    the suite itself is a consistency witness;
  - `cat crashN.txt` answering either the EXACT payload or an honest
    FS_ERR_NOT_FOUND — never a torn prefix;
  - `ls` reporting the file absent, or present at size 0 (CREATE
    committed, WRITE not) or at the full payload size — never
    between;
  - `cat base.txt` still exact (older commits are untouchable);
  - a clean rc=0 shutdown.
And the host runs afs1.audit() — the fsck-lite over the newest
committed generation — after every round: empty problem list.

Exit code: 0 = PASS, 1 = FAIL.
"""

import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "test-m5-crash"
BASE = b"BASELINE-SURVIVES-ALL-CRASHES"

# (kill marker counted from the moment the write command was fed,
#  required occurrences after that point, extra delay in seconds).
# A `str` marker is a template formatted with the round's file name;
# bytes are used verbatim.
ROUNDS: list[tuple[bytes | str, int, float]] = [
    (b"storaged: WRITE", 1, 0.0),
    (b"storaged: WRITE", 3, 0.0),
    (b"fsd: commit seq", 1, 0.03),
    (b"storaged: WRITE", 14, 0.0),
    # the shell's own confirmation — unique text (storaged's mount-read
    # logs also contain "wrote", and fsd mounts CONCURRENTLY with the
    # shell's first prompt, so a loose marker kills too early)
    ("bytes to '{name}'", 1, 0.0),
]

ok = True


def check(cond: bool, msg: str) -> None:
    global ok
    print(f"[{LABEL}] {'PASS' if cond else 'FAIL'}: {msg}")
    if not cond:
        ok = False


def payload(i: int) -> bytes:
    return f"CRASH-ROUND-{i}-".encode() + b"X" * 60


def dump(serial: str) -> None:
    print(f"[{LABEL}] ---- serial tail ----")
    print("\n".join(serial.splitlines()[-25:]))


def main() -> int:
    try:
        esp = mtest.build(LABEL)
    except subprocess.CalledProcessError as e:
        print(f"[{LABEL}] FAIL: kernel build failed:\n{e.stdout}\n{e.stderr}")
        return 1

    scratch = arena_env.make_scratch_disk()  # fresh AFS1 for the seed

    # ---------------- seed boot: a clean committed baseline ----------
    rc, s, dt = mtest.boot(
        LABEL + "-seed", esp,
        [(b"arena>", 1, b"write base.txt " + BASE + b"\r"),
         (b"arena>", 2, b"shutdown\r")],
        scratch)
    print(f"[{LABEL}] seed boot done (rc={rc}, {dt:.1f}s)")
    check(rc == 0, "the seed boot shut down cleanly (rc=0)")
    check(f"wrote {len(BASE)} bytes to 'base.txt'" in s,
          "the seed boot committed base.txt")
    check(not afs1.audit(scratch), "the seed volume audits clean")
    if not ok:
        dump(s)
        return 1

    # ---------------- crash rounds -----------------------------------
    for i, kill in enumerate(ROUNDS):
        data = payload(i)
        name = f"crash{i}.txt"
        trigger = kill[0]
        if isinstance(trigger, str):
            trigger = trigger.format(name=name).encode()
        kill = (trigger, kill[1], kill[2])
        rc, s, dt = mtest.boot(
            LABEL + f"-kill{i}", esp,
            [(b"arena>", 1, b"write " + name.encode() + b" " + data + b"\r")],
            scratch, kill=kill)
        print(f"[{LABEL}] round {i}: killed QEMU "
              f"(trigger {kill[0].decode()!r} x{kill[1]} +{kill[2]}s, {dt:.1f}s)")
        check(rc is None, f"round {i}: the boot was SIGKILLed, not shut down")

        # verification boot — fsd just MOUNTS; nothing repairs anything
        rc, s, dt = mtest.boot(
            LABEL + f"-verify{i}", esp,
            [(b"arena>", 1, b"cat " + name.encode() + b"\r"),
             (b"arena>", 2, b"cat base.txt\r"),
             (b"arena>", 3, b"ls\r"),
             (b"arena>", 4, b"shutdown\r")],
            scratch)
        print(f"[{LABEL}] round {i}: verification boot done (rc={rc}, {dt:.1f}s)")
        check(rc == 0, f"round {i}: the post-crash boot shut down cleanly")
        check("m5: RESULT PASS (6/6)" in s,
              f"round {i}: the suite passed on the crashed volume (6/6)")
        check("fstest: PASS (persisted)" in s,
              f"round {i}: the pre-crash arena.txt verified byte-for-byte "
              f"after the crash (zero writes, zero repairs)")
        check("fsd: mounted AFS1" in s,
              f"round {i}: the production fsd mounted without an fsck ritual")

        # The cat answer must match one of the THREE legal committed
        # states — full payload, committed-empty (CREATE's commit
        # landed, WRITE's did not: cat prints an honest 0 bytes), or
        # not-found (nothing committed). Anything in between — a torn
        # prefix — is the failure this gate exists for.
        region = s.split(f"cat {name}", 1)[1].split("arena>", 1)[0]
        full = ("\n" + data.decode() + "\n") in region
        absent = "cat: open: fs status -1" in region
        empty = "(0 bytes)" in region
        state = ("full" if full else "empty" if empty else
                 "absent" if absent else "ILLEGAL")
        check(state != "ILLEGAL",
              f"round {i}: cat answered a legal committed state ({state})")
        check(not (state == "ILLEGAL" and data[:12].decode() in region),
              f"round {i}: no torn prefix reached the console")

        # ls must agree with cat's state: absent, 0 bytes, or full size
        m = re.search(rf"{re.escape(name)}  (\d+) bytes", s)
        size = int(m.group(1)) if m else None
        legal = {None: ("absent",), 0: ("empty",), len(data): ("full",)}
        check(state in legal.get(size, ()),
              f"round {i}: ls reported size {size}, agreeing with cat's "
              f"{state} state — only never-committed, committed-empty, "
              f"or committed-full are legal")

        check(("\n" + BASE.decode() + "\n") in s,
              f"round {i}: base.txt is still exact (older commits untouchable)")
        check("halting via UEFI ResetSystem(shutdown)" in s,
              f"round {i}: the kernel declared its clean halt")
        probs = afs1.audit(scratch)
        check(not probs,
              f"round {i}: host audit of the crashed volume clean "
              f"({probs[:2] if probs else 'no problems'})")
        if not ok:
            dump(s)
            return 1

    print(f"[{LABEL}] ==============================================")
    print(f"[{LABEL}] CRASH CONSISTENCY: PASS  ({len(ROUNDS)} kill/reboot "
          f"rounds, no repair tool ran; serials: build/serial-{LABEL}-*.log)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
