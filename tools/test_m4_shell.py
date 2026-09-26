#!/usr/bin/env python3
"""M4.6 end-to-end: drive the interactive shell session (ADR-0020).

The in-guest m4 suite (test_m4.py) proves the console line discipline
and validates the shell image; THIS script proves the shipped system:
a full boot, all suites green, then a typed session against the real
UART path — QEMU stdio chardev -> 16550 RX -> IRQ4/vector 33 -> line
discipline -> blocking SYS_CONSOLE_READ -> shell dispatch — with every
command paced on the shell's own prompt (marker-paced, never sleeps):

  help       the builtin list comes back
  echo X     the line discipline's edit/commit and the shell's echo
  ps         SYS_PROC_LIST: the shell sees itself (pid N, threads 1)
  spawn      SYS_SPAWN of registry image 0 (the untouched M4.3
             payload) from the shell: the child's pinned message
             appears MID-SESSION and its exit badge comes back
  bogus      unknown commands answer with a hint, never silence
  shutdown   the Power-gated SYS_SHUTDOWN: kernel logs the requesting
             pid, the canonical clean-halt line lands, QEMU exits 0

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402

# (marker, occurrence, payload) — the feeder writes payload once the
# marker has appeared `occurrence` times in the captured serial. The
# prompt count is the pacing clock: the shell prints "arena> " only
# when it is ready for the next line.
FEED: list[tuple[bytes, int, bytes]] = [
    (b"arena>", 1, b"help\r"),
    (b"arena>", 2, b"echo hello-arena\r"),
    (b"arena>", 3, b"ps\r"),
    (b"arena>", 4, b"spawn\r"),
    (b"arena>", 5, b"bogus\r"),
    (b"arena>", 6, b"shutdown\r"),
]

LABEL = "test-m4-shell"


def main() -> int:
    try:
        esp = mtest.build(LABEL)
    except subprocess.CalledProcessError as e:
        print(f"[{LABEL}] FAIL: kernel build failed:\n{e.stdout}\n{e.stderr}")
        return 1
    try:
        rc, serial, dt = mtest.run_qemu(LABEL, esp, FEED)
    except subprocess.TimeoutExpired:
        print(f"[{LABEL}] FAIL: VM did not terminate within "
              f"{mtest.TIMEOUT_S}s (kernel hang, or the shell never "
              f"reached its prompt / never honored 'shutdown')")
        return 1

    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[{LABEL}] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    check("PANIC" not in serial, "no kernel panic on serial")

    # Every suite still green in this boot (the shell phase must not
    # regress the suites that run before it).
    for ms, n in (("m1", 8), ("m2", 21), ("m3", 13), ("m4", 9)):
        check(f"{ms}: RESULT PASS ({n}/{n})" in serial,
              f"{ms} suite RESULT PASS ({n}/{n}) in this boot")

    # The hand-off: the kernel spawned the shell and said so.
    check("shell spawned: pid" in serial, "kernel announced the spawned shell")
    check("ArenaOS shell" in serial, "the shell's banner reached the console")

    # help: the builtin list, all five verbs named.
    for verb in ("help", "ps", "echo", "spawn", "shutdown"):
        check(re.search(rf"^  {verb}\b", serial, re.MULTILINE) is not None,
              f"help lists the '{verb}' builtin")

    # echo: the typed text came back (line discipline edit/commit +
    # kernel echo + shell response on the same wire).
    check("hello-arena" in serial, "echo returned the typed line")

    # ps: SYS_PROC_LIST saw the shell itself — exactly one live process
    # at that point, with exactly one thread.
    m = re.search(r"^  pid (\d+)  threads (\d+)$", serial, re.MULTILINE)
    check(m is not None, "ps printed a (pid, threads) line")
    if m:
        check(m.group(2) == "1", f"the shell is single-threaded (got {m.group(2)})")

    # spawn: the shell created a process from its Image cap — the
    # untouched M4.3 payload ran mid-session (its pinned message on the
    # wire) and its exit badge came back through the shell's
    # notification.
    check(re.search(r"spawned pid \d+ — waiting for its exit badge", serial)
          is not None, "shell reported the spawned child's pid")
    check("ARENAOS-M43-FIRST-USER-PROCESS" in serial,
          "the spawned payload's message appeared mid-session")
    check("child exited, badge 0x0000000000005aa5" in serial,
          "the child's exit badge arrived at the shell")

    # unknown command: an answer, never silence.
    check("unknown command: 'bogus'" in serial,
          "an unknown command got the hint response")

    # shutdown: Power-gated, logged, canonical clean halt, QEMU exit 0.
    check("shutting down..." in serial, "the shell acknowledged 'shutdown'")
    check(re.search(r"shutdown requested by pid \d+ through its Power cap",
                    serial) is not None,
          "kernel logged the Power-gated shutdown request")
    check("halting via UEFI ResetSystem(shutdown)" in serial,
          "kernel declared its clean halt (ResetSystem path reached)")
    check(rc == 0, f"QEMU exited cleanly via kernel ResetSystem shutdown "
                   f"(rc={rc}, {dt:.1f}s)")
    check(serial.count("arena>") >= 6,
          f"the shell served every typed line ({serial.count('arena>')} prompts)")

    print(f"[{LABEL}] {'=' * 46}")
    print(f"[{LABEL}] M4.6 SHELL SESSION: {'PASS' if ok else 'FAIL'}  "
          f"(serial: build/serial-m4-shell.log)")
    mtest.arena_env.build_dir().joinpath("serial-m4-shell.log").write_text(serial)
    if not ok:
        tail = [ln for ln in serial.splitlines() if ln.strip()][-25:]
        print(f"[{LABEL}] serial tail:")
        for ln in tail:
            print("   |", ln)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
