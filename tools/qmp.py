#!/usr/bin/env python3
"""Minimal QMP client — the harness's virtual keyboard (M6.3, ADR-0026).

Until now every automated boot drove ArenaOS through ONE channel: bytes
written to the serial chardev. The input milestone needs a second one —
keystrokes on a real device — and QEMU's monitor protocol provides it:
`input-send-event` injects host-side key events into the guest's
virtio-input device exactly as a user pressing keys in the QEMU window
would.

This is TEST INFRASTRUCTURE ONLY. Nothing inside ArenaOS knows QMP
exists; the guest sees device interrupts and evdev events, nothing else.

Protocol (QMP spec): connect to the unix socket, read the greeting,
send `qmp_capabilities`, then send commands as one JSON object per
line and read one reply object per line.
"""

import json
import socket
import sys
import time
from pathlib import Path

# evdev keycodes for the characters the harness types. QEMU's
# `qcode` names are its own portable spelling of the same keys, and
# they are what `input-send-event` accepts — the guest still sees the
# Linux evdev codes inputd's keymap is written against (a = 30,
# r = 19, e = 18, n = 49, space = 57, enter = 28 ...).
QCODE = {
    "a": "a", "b": "b", "c": "c", "d": "d", "e": "e", "f": "f",
    "g": "g", "h": "h", "i": "i", "j": "j", "k": "k", "l": "l",
    "m": "m", "n": "n", "o": "o", "p": "p", "q": "q", "r": "r",
    "s": "s", "t": "t", "u": "u", "v": "v", "w": "w", "x": "x",
    "y": "y", "z": "z",
    "0": "0", "1": "1", "2": "2", "3": "3", "4": "4",
    "5": "5", "6": "6", "7": "7", "8": "8", "9": "9",
    " ": "spc", "\r": "ret", "\n": "ret", "-": "minus", "=": "equal",
    ".": "dot", ",": "comma", "/": "slash", ";": "semicolon",
    "\x08": "backspace", "\t": "tab",
}

# Characters that need a shift key held. Typing these exercises the
# driver's MODIFIER tracking (press shift, press key, release key,
# release shift) — the keymap's shifted table is only reachable this
# way, so the typing test uses capitals deliberately.
SHIFTED = {c: c.lower() for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ"}
SHIFTED.update({
    "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6",
    "&": "7", "*": "8", "(": "9", ")": "0", "_": "-", "+": "=",
    ":": ";", "<": ",", ">": ".", "?": "/",
})


class QmpError(RuntimeError):
    """A QMP command was refused, or the channel never came up."""


class Qmp:
    """One QMP session over a unix socket."""

    def __init__(self, path: str, connect_timeout_s: float = 10.0):
        deadline = time.monotonic() + connect_timeout_s
        last: Exception | None = None
        while time.monotonic() < deadline:
            try:
                self.sock = socket.socket(socket.AF_UNIX)
                self.sock.connect(path)
                break
            except OSError as e:  # QEMU may not have bound it yet
                last = e
                time.sleep(0.05)
        else:
            raise QmpError(f"could not connect to QMP socket {path}: {last}")
        self.sock.settimeout(10.0)
        self.f = self.sock.makefile("rwb")
        greeting = self._readline()
        if "QMP" not in greeting:
            raise QmpError(f"no QMP greeting on {path}: {greeting}")
        self.command("qmp_capabilities")

    def _readline(self) -> dict:
        line = self.f.readline()
        if not line:
            raise QmpError("QMP channel closed")
        return json.loads(line)

    def command(self, execute: str, **arguments) -> dict:
        """Send one command; return its `return` payload.

        Asynchronous events (QEMU pushes SHUTDOWN, RESET, ...) are
        skipped: a reply object always carries `return` or `error`.
        """
        msg = {"execute": execute}
        if arguments:
            msg["arguments"] = arguments
        self.f.write(json.dumps(msg).encode() + b"\n")
        self.f.flush()
        while True:
            reply = self._readline()
            if "error" in reply:
                raise QmpError(f"{execute} refused: {reply['error']}")
            if "return" in reply:
                return reply["return"]

    @staticmethod
    def _ev(qcode: str, down: bool) -> dict:
        return {"type": "key",
                "data": {"down": down, "key": {"type": "qcode",
                                               "data": qcode}}}

    def key(self, ch: str) -> None:
        """Type one character: press then release, one SYN batch.

        QEMU's virtio-input holds events until it sees the sync and
        then flushes the whole batch with a single interrupt — so one
        call here is one keystroke as the guest experiences it. A
        shifted character rides inside a shift press/release pair, so
        the guest must track the modifier across events to decode it.
        """
        base = SHIFTED.get(ch)
        shift = base is not None
        q = QCODE.get(base if shift else ch)
        if q is None:
            raise QmpError(f"no qcode mapping for {ch!r}")
        events = []
        if shift:
            events.append(self._ev("shift", True))
        events.append(self._ev(q, True))
        events.append(self._ev(q, False))
        if shift:
            events.append(self._ev("shift", False))
        self.command("input-send-event", events=events)

    def type_text(self, text: str, gap_s: float = 0.02) -> None:
        """Type a string, one keystroke at a time."""
        for ch in text:
            self.key(ch)
            if gap_s:
                time.sleep(gap_s)

    def close(self) -> None:
        try:
            self.f.close()
            self.sock.close()
        except OSError:
            pass


def _main(argv: list[str]) -> int:
    """CLI: wait for a serial marker, then type text on the keyboard.

    Usage: qmp.py <qmp-socket> <serial-log> <marker> <text> [timeout_s]

    The bash stability loop drives boots without the Python harness, so
    it needs the same marker-paced typing as one command. Waiting on a
    marker (never a sleep) keeps the loop's discipline intact.
    """
    if not 4 <= len(argv) <= 5:
        print(__doc__)
        return 2
    sock, log_path, marker, text = argv[:4]
    timeout_s = float(argv[4]) if len(argv) == 5 else 60.0
    log = Path(log_path)
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            if marker.encode() in log.read_bytes():
                break
        except OSError:
            pass
        time.sleep(0.05)
    else:
        return 1  # the marker never appeared; the boot has other problems
    session = Qmp(sock)
    try:
        session.type_text(text)
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    sys.exit(_main(sys.argv[1:]))
