#!/usr/bin/env python3
"""M6.4 live proof: the virtio-console port IS a console (ADR-0027).

The m6 suite's `console_service` test proves the DEVICE — bytes out the
transmit queue and back in on the receive queue, verified through an
IPC boundary. This script proves the thing the milestone actually
claims: that the port is a second way to USE the machine.

So it takes both of the machine's other input channels away —
`feed=[]` leaves the serial input dead, `keys=[]` leaves the keyboard
untouched — and drives the shell entirely over the chardev socket, the
way a person with `nc -U` would:

    echo Hello-From-The-Port     the line is echoed back ON THE PORT
                                 (the kernel's line discipline echoes
                                 to the console, and the console is
                                 mirrored to the port), then the shell
                                 executes it and its output arrives the
                                 same way
    psX<backspace>               the SAME one line editor the serial
                                 port and the keyboard use: the typo is
                                 erased before the shell ever sees the
                                 line, so `ps` runs and `psX` does not
    shutdown                     the machine halts because someone
                                 typed it on the port

And it asserts the output direction independently: the shell's banner
and prompt must ARRIVE on the port, having been written by the kernel
to its own serial console and mirrored out through consoled's
`ConsoleOutput` capability.

Serial is still the kernel's own channel throughout — the log is on it,
this script reads it, and the two streams must agree about what
happened.

Exit status 0 = every check passed.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "test-m6-console"

# Typed ON THE PORT, paced by markers seen coming OUT of the port.
# `\r` is the commit: the line discipline treats CR and LF alike, and a
# terminal sends CR.
CONSOLE_SCRIPT: mtest.ConsoleScript = [
    # The suite's own console fixture still has to be answered first —
    # this boot runs the whole milestone suite before the shell, and
    # `console_service` is waiting for exactly this reply.
    (b"contest: hello from ArenaOS", 1, "host-says-hello\n"),
    # The shell is up and prompting ON THE PORT.
    (b"arena> ", 1, "echo Hello-From-The-Port\r"),
    # The echo of the typed line, then the shell's own output.
    (b"Hello-From-The-Port", 2, "psX\x08\r"),
    # `ps` has listed the live processes.
    (b"pid ", 1, "shutdown\r"),
]


def main() -> int:
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[{LABEL}] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    esp = mtest.build(LABEL)
    # feed=[] : the serial input channel is DEAD for this boot.
    # keys=[] : the keyboard is attached but nobody touches it.
    rc, serial, dt = mtest.run_qemu(LABEL, esp, feed=[], keys=[],
                                    console=CONSOLE_SCRIPT)
    bdir = arena_env.build_dir()
    (bdir / "serial-m6-console.log").write_text(serial)
    port = (bdir / f"vcon-{LABEL}.txt").read_bytes().decode(errors="replace")

    check("PANIC" not in serial, "no kernel panic during the port session")
    check("consoled spawned: pid" in serial
          and "4=ConsoleOutput/R" in serial,
          "the production consoled was spawned with BOTH console "
          "capabilities")
    check("consoled: console mode" in serial,
          "consoled's capability probes put it in console mode")

    # --- the OUTPUT direction: the machine's console reaches the port ---
    check("ArenaOS shell v0.8" in port or "ArenaOS shell" in port,
          "the shell's banner ARRIVED on the port (kernel console output "
          "→ mirror → SYS_CONSOLE_PULL → transmit queue)")
    check("arena> " in port,
          "the prompt arrived on the port — the port is a console, not a "
          "log tap")

    # --- the INPUT direction: typing on the port drives the shell -------
    check("echo Hello-From-The-Port" in port,
          "the typed line was ECHOED back on the port by the kernel's "
          "line discipline")
    check(port.count("Hello-From-The-Port") >= 2,
          "the shell EXECUTED the command typed on the port and its "
          "output came back the same way (echo + result)")
    check("unknown command" not in serial,
          "the shell never saw a bad command — BACKSPACE erased the "
          "mistyped character inside the SAME line discipline the serial "
          "port and the keyboard use")
    check(re.search(r"pid \d+", port) is not None,
          "the corrected `ps` ran and listed the live processes on the "
          "port")
    check(re.search(r"shutdown requested by pid \d+ through its Power cap",
                    serial) is not None,
          "the Power-gated SYS_SHUTDOWN was reached from a keystroke "
          "typed ON THE PORT")
    check("halting via UEFI ResetSystem(shutdown)" in serial,
          "the kernel declared its clean halt")

    # --- the two channels agree -----------------------------------------
    check("arena>" in serial,
          "serial stayed live alongside the port (a channel was ADDED, "
          "not moved)")
    # This boot deliberately leaves the keyboard untouched, so the input
    # test must SKIP honestly while the console test PASSes — the two
    # host-driven fixtures are independent, and neither may fake the
    # other's evidence.
    check("m6:test:console_service: PASS" in serial,
          "the suite's console_service test passed in this boot")
    check("m6:test:input_service: SKIP" in serial,
          "the untouched keyboard reported an honest SKIP in the same "
          "boot (one host actor absent does not corrupt the other's "
          "verdict)")
    check("m6:test:net_service: PASS" in serial
          and "m6:test:rng_service: PASS" in serial,
          "the self-driving fixtures (link + entropy) passed alongside")
    check(rc == 0, f"QEMU exited cleanly (rc={rc}, {dt:.1f}s)")

    print(f"[{LABEL}] M6.4 CONSOLE CHANNEL: {'PASS' if ok else 'FAIL'}  "
          f"(serial: build/serial-m6-console.log, port: "
          f"build/vcon-{LABEL}.txt)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
