#!/usr/bin/env python3
"""A real host TCP peer for M7.6, reachable from QEMU at 10.0.2.2:54321.

Unlike an in-tree fake TCP responder, the host OS implements the other
side of the handshake, sequence and checksum rules. Listener binding is
synchronous; a boot never races the fixture coming up. The per-boot log
records the exact request and whether the peer saw an orderly FIN.
"""
import socket
import sys
import threading
from pathlib import Path

PORT = 54321
REQUEST = b"arena-tcp"
RESPONSE = bytes(b"arenaos!"[i % 8] ^ i for i in range(200))


def bind() -> socket.socket:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("127.0.0.1", PORT))
    sock.listen(1)
    sock.settimeout(0.2)
    return sock


def actor(listener: socket.socket, stop: threading.Event, log: Path) -> None:
    try:
        while not stop.is_set():
            try:
                conn, _ = listener.accept()
                break
            except socket.timeout:
                continue
        else:
            return
        with conn:
            conn.settimeout(20)
            request = bytearray()
            while len(request) < len(REQUEST):
                part = conn.recv(len(REQUEST) - len(request))
                if not part:
                    break
                request.extend(part)
            if bytes(request) != REQUEST:
                log.write_text(f"FAIL request={bytes(request)!r}\n")
                return
            conn.sendall(RESPONSE)
            conn.shutdown(socket.SHUT_WR)  # peer FIN; guest must ACK it
            tail = conn.recv(1)  # guest FIN (EOF), not a guessed sleep
            log.write_text("TCP_FIXTURE_PASS request=arena-tcp bytes=200 eof="
                           + str(tail == b"") + "\n")
    except (OSError, TimeoutError) as exc:
        log.write_text(f"FAIL actor={exc!r}\n")
    finally:
        listener.close()


def main() -> int:
    log = Path(sys.argv[1])
    log.write_text("")
    listener = bind()
    print("READY", flush=True)
    stop = threading.Event()
    try:
        actor(listener, stop, log)
    finally:
        listener.close()
    return 0 if "TCP_FIXTURE_PASS" in log.read_text() else 1


if __name__ == "__main__":
    sys.exit(main())
