#!/usr/bin/env python3
"""Independent Python vectors and adversarial perm8-* namespace scanner."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import permission_record as p
from afs1 import fnv1a64

def main():
    allow = b'\x01\x01\x01\x00'
    deny = b'\x01\x01\x00\x00'
    assert p.pack(1, allow)[-8:] == bytes.fromhex('cfb0b51fb98fbf0b')
    assert p.pack(2, deny)[-8:] == bytes.fromhex('adb4f41588849ac4')
    assert p.unpack(p.pack(1, allow), 1).payload == allow
    for inp in (b'', b'\x01\x01\x02\x00', b'\x02\x01\x01\x00',
                b'\x01\x02\x01\x00', b'\x01\x01\x01\x01', allow + b'\x00'):
        try: p.pack(1, inp)
        except ValueError: pass
        else: raise AssertionError(f'accepted unbounded/nonpolicy payload {inp!r}')
    old, new = p.pack(1, allow), p.pack(2, deny)
    for offset in (0, 8, 12, 16, 24, 26, 32, 33, 34, 35, 36, 503, 504):
        bad = bytearray(new)
        bad[offset] ^= 0x80
        if offset != 504:
            bad[504:] = fnv1a64(bad[:504]).to_bytes(8, 'little')
        try: p.recover([('perm8-01', old), ('perm8-02', bytes(bad))])
        except p.PolicyCorrupt: pass
        else: raise AssertionError(f'visible corruption at {offset} fell back to old ALLOW')
    for bad in ('perm8-00', 'perm8-09', 'perm8-1', 'perm8-0x', 'perm8-010'):
        try: p.recover([('perm8-01', old), (bad, b'')])
        except p.PolicyCorrupt: pass
        else: raise AssertionError(f'accepted reserved name {bad}')
    for layout in ([('perm8-01', old), ('perm8-01', old)],
                   [('perm8-01', old), ('perm8-03', p.pack(3, allow))],
                   [('perm8-01', b''), ('perm8-02', new)]):
        try: p.recover(layout)
        except p.PolicyCorrupt: pass
        else: raise AssertionError(f'accepted duplicate/gap/empty older {layout}')
    state = p.recover([('cfg8-01', b'not policy'), ('perm8-01', old), ('perm8-02', b'')])
    assert state.current.payload == allow and state.next_name == 'perm8-02'
    assert p.recover([(p.name(i), p.pack(i, allow if i % 2 else deny))
                      for i in range(1, 9)]).next_name is None
    print('POLICY RECORD: PASS (independent vectors, malformed decisions/names, full scan, eight-generation bound)')
    return 0

if __name__ == '__main__': sys.exit(main())
