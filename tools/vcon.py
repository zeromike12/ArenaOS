#!/usr/bin/env python3
"""The host end of the guest's virtio-console port (M6.4, ADR-0027).

`consoled` turns a `virtconsole` port into a second console. The host
side of that port is a unix-socket chardev, and this module is whoever
is sitting at it: it records everything the guest sends (that capture
IS the evidence for guest→host assertions) and sends scripted replies
when markers appear in that stream.

Marker-paced, never sleep-based — the same discipline as the serial
feeder and the QMP typist. The console's own output is the clock.

Test-only: nothing inside ArenaOS knows this exists. The guest sees a
virtqueue; this sees a socket.

As a CLI (for the bash stability loop, which runs without the Python
harness):

    vcon.py <socket> <marker> <reply> [timeout_s]
"""
import socket
import sys
import threading
import time
from pathlib import Path

# (marker, nth occurrence, text to send) — the shape every actor in
# this harness uses.
ConsoleScript = list[tuple[bytes, int, str]]


def connect(sock: Path | str, timeout_s: float = 30.0,
            stop: threading.Event | None = None) -> socket.socket | None:
    """Connect to the chardev socket, retrying while QEMU starts."""
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if stop is not None and stop.is_set():
            return None
        conn = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            conn.connect(str(sock))
            return conn
        except OSError:
            conn.close()
            time.sleep(0.05)
    return None


def actor(script: ConsoleScript, sock: Path, capture: Path | None,
          stop: threading.Event, label: str = "vcon",
          connect_timeout_s: float = 30.0) -> None:
    """Be the human at the other end of the port.

    A failure here prints a NOTE and returns: a harness actor must
    never hang the run it is observing.
    """
    conn = connect(sock, connect_timeout_s, stop)
    seen = bytearray()
    # APPENDED to, never rewritten: rewriting the whole accumulated
    # buffer once per chunk is O(n^2) disk traffic, and this port now
    # carries an entire boot log. The harness must never be the reason
    # the guest looks slow — QEMU's TSC follows HOST time while its PIT
    # follows virtual time, so host I/O stalls show up inside the
    # machine as a failed clock-calibration test.
    cap_file = capture.open("wb") if capture is not None else None
    if conn is None:
        if cap_file is not None:
            cap_file.close()
        return
    try:
        # A HALF SECOND, not 50 ms: `recv` returns the instant data
        # arrives, so this timeout only bounds how often an IDLE actor
        # wakes to re-check `stop`. Polling ten times faster bought
        # nothing and cost real host CPU beside an emulator whose TSC
        # is host-time-based while its PIT is virtual-time-based — the
        # m2 suite's calibration test can see the difference.
        conn.settimeout(0.5)
        pending = list(script)
        while not stop.is_set():
            try:
                chunk = conn.recv(4096)
                if not chunk:
                    break
                seen += chunk
                if cap_file is not None:
                    cap_file.write(chunk)
                    cap_file.flush()
            except TimeoutError:
                pass
            except OSError:
                break
            while pending:
                marker, nth, text = pending[0]
                if seen.count(marker) < nth:
                    break
                conn.sendall(text.encode())
                pending.pop(0)
    except Exception as e:  # noqa: BLE001 — an actor fault must not hang the run
        print(f"[{label}] NOTE: the console actor failed: {e}")
    finally:
        if cap_file is not None:
            cap_file.close()
        conn.close()


def _main(argv: list[str]) -> int:
    if not 3 <= len(argv) <= 4:
        print(__doc__)
        return 2
    sock, marker, reply = argv[:3]
    timeout_s = float(argv[3]) if len(argv) == 4 else 60.0
    stop = threading.Event()
    script: ConsoleScript = [(marker.encode(), 1, reply)]
    actor(script, Path(sock), None, stop, "vcon", timeout_s)
    return 0


if __name__ == "__main__":
    sys.exit(_main(sys.argv[1:]))
