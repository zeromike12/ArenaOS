#!/usr/bin/env python3
"""One owned Bash coprocess prebinding both independently routed host peers."""
import socket
import sys
import threading
from pathlib import Path
import tcp_fixture
import udp_dns_fixture


def main() -> int:
    tcp_log = Path(sys.argv[1])
    udp_log = Path(sys.argv[2])
    listener = tcp_fixture.bind()
    try:
        sock = udp_dns_fixture.bind()
    except Exception:
        listener.close()
        raise
    stop = threading.Event()
    tcp = threading.Thread(target=tcp_fixture.actor,
                           args=(listener, stop, tcp_log), daemon=True)
    tcp.start()
    print('READY', flush=True)
    try:
        udp_dns_fixture.actor(sock, stop, udp_log)
    finally:
        stop.set()
        listener.close()
        tcp.join(timeout=1)
    return 0

if __name__ == '__main__':
    sys.exit(main())
