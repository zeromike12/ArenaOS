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
  ps         SYS_PROC_LIST: the shell sees itself + the resident
             services (storaged, fsd) parked in recv
  ls / write / cat  the M5.3 filesystem builtins: the shell lists the
             suite-committed arena.txt, creates note.txt, and streams
             it back — all through fsd's endpoint (slot 3), with file
             data DMA'd by storaged straight into/out of the shell's
             own lent frame
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
    # M5.3: the filesystem builtins — the suite's fs_service committed
    # arena.txt earlier IN THIS BOOT, so `ls` must already show it
    # (production fsd mounted the same disk the suite wrote: the
    # in-boot persistence-across-re-open proof), then the shell creates
    # its own file and reads it back through fsd + storaged.
    (b"arena>", 4, b"ls\r"),
    (b"arena>", 5, b"write note.txt hello-fs\r"),
    (b"arena>", 6, b"cat note.txt\r"),
    (b"arena>", 7, b"ls\r"),
    # M5.4: rm — transactional UNLINK through fsd. note.txt dies by
    # name; an absent file is refused honestly; the final ls counts
    # only the suite's arena.txt again.
    (b"arena>", 8, b"rm note.txt\r"),
    (b"arena>", 9, b"rm nosuch.txt\r"),
    (b"arena>", 10, b"ls\r"),
    (b"arena>", 11, b"spawn\r"),
    (b"arena>", 12, b"bogus\r"),
    (b"arena>", 13, b"shutdown\r"),
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

    # help: the builtin list, all nine verbs named (the text is one
    # debug_write chunk, so line starts survive the shared console).
    for verb in ("help", "ps", "echo", "ls", "cat", "write", "rm",
                 "spawn", "shutdown"):
        check(re.search(rf"^  {verb}\b", serial, re.MULTILINE) is not None,
              f"help lists the '{verb}' builtin")

    # echo: the typed text came back (line discipline edit/commit +
    # kernel echo + shell response on the same wire).
    check("hello-arena" in serial, "echo returned the typed line")

    # ps: SYS_PROC_LIST — the machine has several resident processes:
    # storaged (the block service), fsd (the filesystem service),
    # since M6.1 netd (the network service, when the NIC fixture is
    # attached — the checks below are >=-style so its presence is
    # optional), all spawned at boot before the shell and parked in
    # recv, plus the shell itself. All are single-threaded; the
    # shell's pid is the one the kernel announced.
    ps_lines = re.findall(r"^  pid (\d+)  threads (\d+)$", serial, re.MULTILINE)
    check(len(ps_lines) >= 2,
          f"ps listed both resident processes (got {len(ps_lines)})")
    m = re.search(r"shell spawned: pid (\d+)", serial)
    check(m is not None, "the kernel announced the shell's pid")
    if m:
        shell_rows = [t for (p, t) in ps_lines if p == m.group(1)]
        check(len(shell_rows) >= 1, "ps listed the shell itself")
        check(all(t == "1" for t in shell_rows),
              f"the shell is single-threaded (got {shell_rows})")
    ms = re.search(r"storaged spawned: pid (\d+)", serial)
    check(ms is not None, "the kernel announced the spawned block service")
    mf = re.search(r"fsd spawned: pid (\d+)", serial)
    check(mf is not None, "the kernel announced the spawned filesystem service")
    check("fsd: mounted AFS1" in serial,
          "the production fsd mounted the AFS1 volume the suite committed")
    if ms:
        check(any(p == ms.group(1) and t == "1" for (p, t) in ps_lines),
              "ps listed storaged, single-threaded and parked in recv")

    # M5.3 filesystem builtins: ls sees the SUITE's committed file
    # (production fsd mounted the same on-disk state — persistence
    # across re-open, in one boot), write creates a new file, cat
    # streams it back through fsd + storaged, and the second ls shows
    # both files.
    check(re.search(r"arena\.txt  512 bytes", serial) is not None,
          "ls listed the suite's committed arena.txt (512 bytes)")
    check("wrote 8 bytes to 'note.txt'" in serial,
          "write created note.txt with 8 bytes through fsd")
    # the harness normalizes CRLF to LF; the leading newline separates
    # cat's output from the kernel's echo of the typed command line.
    check(re.search(r"\nhello-fs\n", serial) is not None,
          "cat streamed note.txt's contents back byte-for-byte")
    check(re.search(r"note\.txt  8 bytes", serial) is not None,
          "the second ls listed note.txt (8 bytes)")
    check("  2 file(s)" in serial, "the second ls counted exactly 2 files")

    # M5.4: the rm exchanges — UNLINK removed note.txt transactionally
    # (its sectors join the two-generation dead list inside fsd), the
    # absent-file refusal is honest, and the final ls sees only the
    # suite's arena.txt.
    check("removed 'note.txt'" in serial,
          "rm deleted note.txt through fsd's UNLINK transaction")
    check("rm: no such file: 'nosuch.txt'" in serial,
          "rm of an absent file was honestly refused")
    tail = serial.split("removed 'note.txt'", 1)[1]
    check("  1 file(s)" in tail,
          "the final ls counted exactly 1 file after the rm")
    check("note.txt  8 bytes" not in tail,
          "the final ls no longer listed note.txt")

    # fsd and storaged are resident services: ps must list both,
    # single-threaded and parked in recv.
    if mf:
        check(any(p == mf.group(1) and t == "1" for (p, t) in ps_lines),
              "ps listed fsd, single-threaded and parked in recv")

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
    check(serial.count("arena>") >= 10,
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
