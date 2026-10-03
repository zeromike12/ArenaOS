#!/usr/bin/env python3
"""Controlled host UDP DNS peer for the M7 virtual-network wire fixture.

This is a separately bound host socket reached over QEMU SLIRP at
10.0.2.2:1053; it is not a guest stub or proof of public DNS access.
The response is 61 real UDP payload bytes so both IPC continuation and
no_std native API continuation cross their 56-byte inline boundary.
"""
import socket
import sys
import threading
from pathlib import Path

PORT = 1053
QUESTION = b"\x07example\x03com\x00\x00\x01\x00\x01"


def bind() -> socket.socket:
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", PORT))
    sock.settimeout(0.2)
    return sock


def answer(packet: bytes, wrong_txid: bool = False) -> bytes | None:
    if (len(packet) != 29 or packet[2:12] != b"\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00"
            or packet[12:] != QUESTION):
        return None
    txid = (int.from_bytes(packet[:2], 'big') + int(wrong_txid)) & 0xffff
    header = txid.to_bytes(2, 'big') + b"\x81\x80\x00\x01\x00\x02\x00\x00\x00\x00"
    def record(address: bytes) -> bytes:
        return b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04" + address
    result = header + QUESTION + record(b"\x5d\xb8\xd7\x0e") + record(b"\x5d\xb8\xd7\x0f")
    assert len(result) == 61
    return result


def actor(sock: socket.socket, stop: threading.Event, log: Path, wrong_txid: bool = False) -> None:
    try:
        with log.open('w') as receipt:
            while not stop.is_set():
                try:
                    data, peer = sock.recvfrom(512)
                except socket.timeout:
                    continue
                except OSError:
                    return
                response = answer(data, wrong_txid)
                if response is None:
                    receipt.write(f"DNS_FIXTURE_REJECT query={data.hex()}\n")
                    receipt.flush()
                    continue
                sock.sendto(response, peer)
                receipt.write(f"DNS_FIXTURE_QUERY peer={peer[0]}:{peer[1]} query=29 txid={data[:2].hex()} answer=61 wrong_txid={wrong_txid}\n")
                receipt.flush()
    finally:
        sock.close()


def main() -> int:
    log = Path(sys.argv[1])
    sock = bind()
    print('READY', flush=True)
    actor(sock, threading.Event(), log, '--wrong-txid' in sys.argv[2:])
    return 0

if __name__ == '__main__':
    sys.exit(main())
