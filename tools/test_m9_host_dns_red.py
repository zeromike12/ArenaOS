#!/usr/bin/env python3
"""Real guest controlled-peer RED/GREEN: wrong DNS TXID, unchanged EFI."""
import hashlib
import os
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env
import mtest


def main() -> int:
    esp = mtest.build('m9-dns-peer-red')
    efi = arena_env.REPO_ROOT / 'build/arena-boot.efi'
    digest = hashlib.sha256(efi.read_bytes()).hexdigest()
    os.environ['ARENA_DNS_WRONG_TXID'] = '1'
    try:
        red, serial, _ = mtest.boot('m9-dns-peer-red', esp, [], arena_env.make_scratch_disk(), timeout_s=55)
    finally:
        os.environ.pop('ARENA_DNS_WRONG_TXID', None)
    red_log = (arena_env.build_dir() / 'udp-dns-m9-dns-peer-red.log').read_text()
    assert red != 0 and 'm7: RESULT FAIL' in serial and '[arena ERROR halt]' in serial
    assert 'wrong_txid=True' in red_log and 'DNS_FIXTURE_QUERY ' in red_log
    assert hashlib.sha256(efi.read_bytes()).hexdigest() == digest
    green, serial, _ = mtest.boot('m9-dns-peer-green', esp, mtest.DEFAULT_FEED, arena_env.make_scratch_disk(), timeout_s=55)
    assert green == 0 and 'm7: RESULT PASS (2/2)' in serial
    assert (arena_env.build_dir() / 'udp-dns-m9-dns-peer-green.log').read_text().count('DNS_FIXTURE_QUERY ') == 3
    assert hashlib.sha256(efi.read_bytes()).hexdigest() == digest
    print('[m9-host-dns] real guest wrong-TXID RED; same-EFI restored host peer and three guest wire queries GREEN: PASS')
    return 0

if __name__ == '__main__':
    sys.exit(main())
