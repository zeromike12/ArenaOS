#!/usr/bin/env python3
"""M6.3 live-typing boot test (docs/TESTING.md, ADR-0026).

The m6 suite's `input_service` test proves the DEVICE path: evdev
events harvested from a virtqueue by interrupt and decoded through a
service boundary. This script proves the milestone's actual claim —
**a person typing on the keyboard drives the shell** — end to end,
through the PRODUCTION path, with the serial input channel completely
unused.

The boot is fed NOTHING on the serial port (`feed=[]`): every byte the
shell sees comes from `virtio-keyboard-pci`, typed through QMP
(tools/qmp.py) one keystroke at a time, exactly as a user's keyboard
in a QEMU window produces them. inputd decodes the keycodes and pushes
the bytes through its ConsoleInput capability into the kernel's
console line discipline — the same queue the UART RX ISR feeds — so
the shell's blocking read, its echo, and its line editing all work
without the shell knowing a keyboard exists.

What is asserted (each one a thing that can only be true if the whole
chain works):
  1. the typed characters are ECHOED back — the line discipline saw
     them as input, not as output somebody printed;
  2. the shell EXECUTED the typed command and produced its output;
  3. SHIFTED characters decode correctly — the driver tracked the
     modifier across separate events;
  4. BACKSPACE erases — a typed character was removed from the line
     before the shell ever saw it, so the corrected command ran and
     the mistyped one never did;
  5. the machine HALTS on a `shutdown` typed on the keyboard: the
     Power-gated syscall reached from a keystroke;
  6. the serial console stayed live the whole time (the boot's own log
     kept flowing), so the keyboard ADDED a source rather than
     replacing one.

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed).
"""

import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402
import arena_env  # noqa: E402

LABEL = "test-m6-typing"

# Typed on the VIRTUAL KEYBOARD, each line paced by the shell prompt it
# answers. Nothing is ever written to the serial port.
#
#  1. `echo Hello-From-The-Keyboard` — capitals force the shift path.
#  2. `psX<backspace>` — the X is erased by the line discipline before
#     the shell reads the line, so `ps` runs and `psX` never does.
#  3. `shutdown` — the machine stops because someone typed it.
# Every boot replays the full suite first, and the m6 input_service
# test waits on real keystrokes — so this boot supplies that fixture
# too, from the same keyboard, before the shell ever prompts.
# Require the REAL shell prompt AND the completion of the independent
# permission app's startup/reap log. With an extra Phase-8.4 resident,
# typing on the prompt alone can interleave that app's log between the
# echoed 'e' and 'cho', splitting the byte-exact serial-echo proof even
# though the keyboard and command both work. Wait on both observable
# boundaries; never weaken the byte-exact echo check or use sleeps.
KEY_SCRIPT = [
    (b"inputd: virtio-input ready", 1, "arena"),
    ((b"arena>", b"servicemgr: permission app reaped through held Process cap"),
     1, "echo Hello-From-The-Keyboard\r"),
    (b"arena>", 2, "psX\x08\r"),
    (b"arena>", 3, "shutdown\r"),
]

ECHOED = "echo Hello-From-The-Keyboard"
OUTPUT = "Hello-From-The-Keyboard"


def main() -> int:
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[{LABEL}] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    try:
        esp = mtest.build(LABEL)
    except subprocess.CalledProcessError as e:
        print(f"[{LABEL}] FAIL: kernel build failed:\n{e.stdout}\n{e.stderr}")
        return 1
    try:
        # feed=[] is the whole point: the serial input channel is dead
        # for this boot. If the keyboard path does not work, nothing
        # shuts the machine down and the run times out.
        rc, serial, dt = mtest.run_qemu(LABEL, esp, feed=[], keys=KEY_SCRIPT)
    except subprocess.TimeoutExpired:
        print(f"[{LABEL}] FAIL: the machine never shut down — the typed "
              "keystrokes did not reach the shell")
        return 1
    arena_env.build_dir().joinpath("serial-m6-typing.log").write_text(serial)

    check("PANIC" not in serial, "no kernel panic during the typing session")
    check("inputd spawned: pid" in serial,
          "the production inputd was spawned with the ConsoleInput capability")
    check("console mode — keystrokes feed the shell" in serial,
          "inputd's capability probe put it in console mode")

    # 1. The echo: the line discipline treated the typed bytes as INPUT
    #    and echoed them, character by character, exactly as it does
    #    for the serial port.
    check(ECHOED in serial,
          f"the typed characters were echoed back by the line discipline "
          f"({ECHOED!r})")
    # 3. Capitals prove the shift modifier was tracked across events.
    check("Hello-From-The-Keyboard" in serial,
          "SHIFTED characters decoded correctly (the driver tracked the "
          "modifier across separate press/release events)")
    # 2. The shell ran the command: its output is the echoed line's
    #    argument on a line of its own, so the string appears twice.
    check(serial.count(OUTPUT) >= 2,
          f"the shell EXECUTED the typed command and printed its output "
          f"({serial.count(OUTPUT)} occurrences of {OUTPUT!r}: the echo "
          "plus the result)")

    # 4. Backspace: the mistyped `psX` was corrected to `ps` before the
    #    shell ever saw the line.
    check("unknown command" not in serial,
          "the shell never saw a bad command — BACKSPACE erased the "
          "mistyped character inside the line discipline")
    check("  pid " in serial,
          "the corrected `ps` command ran and listed the live processes")

    # 5. The halt came from a typed `shutdown`.
    check("shutdown requested by pid" in serial,
          "the Power-gated SYS_SHUTDOWN was reached from a KEYSTROKE")
    check("halting via UEFI ResetSystem(shutdown)" in serial,
          "the kernel declared its clean halt")

    # 6. The serial console was never displaced: the boot log kept
    #    flowing on it throughout, and every prior suite still passed
    #    in this same boot.
    check("console input armed: com1 rx" in serial,
          "the serial console stayed armed alongside the keyboard "
          "(the keyboard ADDS a source, it does not replace one)")
    for prior in ("m1", "m2", "m3", "m4", "m5", "m6"):
        check(f"{prior}: RESULT PASS" in serial,
              f"prior-suite regression: {prior} RESULT PASS in the typing boot")

    check(rc == 0, f"QEMU exited cleanly (rc={rc}, {dt:.1f}s)")

    print(f"[{LABEL}] {'=' * 46}")
    print(f"[{LABEL}] M6.3 LIVE TYPING: {'PASS' if ok else 'FAIL'}  "
          "(serial: build/serial-m6-typing.log)")
    if not ok:
        tail = [ln for ln in serial.splitlines() if ln.strip()][-25:]
        print(f"[{LABEL}] serial tail:")
        for ln in tail:
            print("   |", ln)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
