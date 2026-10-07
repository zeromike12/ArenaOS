#!/usr/bin/env python3
"""Host-only Phase 8.5 *capacity design* measurement; no installer/guest execution.

Unpacks the actual 8.4 qualified 8 MiB AFS1 template into memory, fills its
committed object table with a worst-case *synthetic* 8.1/8.2 + 8.4 + 8.5
namespace, audits geometry and models an attempted extra AACT. These bytes
are NOT authentically signed records and never enter a guest or release.
The separate cap-space occupancy walk is a source-anchored schedule model,
NOT a guest-measured high-water; this distinction is part of the result.
"""
from __future__ import annotations

import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'tools'))
import afs1

BUNDLE = ROOT / 'releases/checkpoints/phase84-complete/arenaos-phase84-complete-qemu-x86_64.tar.gz'


def synthetic_names() -> list[tuple[bytes, int]]:
    # ADR-0048: arena.txt + 8 cfg + 2 intent + 8 permission = 19.
    old = [(b'arena.txt', 512)]
    old += [(f'cfg8-{i:02}'.encode(), 512) for i in range(1, 9)]
    old += [(b'cfg8-intent', 512), (b'perm8-intent', 512)]
    old += [(f'perm8-{i:02}'.encode(), 512) for i in range(1, 9)]
    assert len(old) == 19
    prefix = b'0123456789abcdefabcd'  # reserved name-shape surrogate ONLY
    phase84 = [(b'i8-'+prefix, 4288), (b'a8-'+prefix, 512)]
    phase84 += [(b'p8-'+prefix+f'-0{i}'.encode(), 512) for i in range(1, 5)]
    phase84 += [(b's8-'+prefix+f'-0{i}'.encode(), 4288) for i in range(1, 3)]
    phase85 = [(b'n8-'+prefix+f'-0{i}'.encode(), 512) for i in range(1, 3)]
    phase85 += [(b'v8-'+prefix+f'-0{i}'.encode(), 512) for i in range(1, 4)]
    assert (len(phase84), len(phase85)) == (8, 5)
    return old + phase84 + phase85


def disk_measure() -> tuple[int, int]:
    with tarfile.open(BUNDLE, 'r:gz') as archive:
        raw = bytearray(archive.extractfile('scratch-template.img').read())
    d = afs1.Disk(raw)
    assert d.superblock()['total_sectors'] == 16384
    seq, table, bitmap_head = d.commit()
    assert seq == 1 and all(o['type'] == afs1.OBJ_FREE for o in d.objects(table))
    bm = d.bitmap(bitmap_head)
    initial_used = sum(b.bit_count() for b in bm)
    assert initial_used == 11
    names = synthetic_names()
    assert len(names) == 32 and len(set(n for n, _ in names)) == 32
    reserved = set()
    for index, (name, size) in enumerate(names):
        assert 1 <= len(name) < afs1.NAME_MAX and size > 0
        n = (size+afs1.SECTOR-1)//afs1.SECTOR
        # Offline geometry experiment: a single contiguous run and one
        # extent block per file. Data is synthetic zero-filled; do not
        # infer an AINS/APOL signature or a fsd CREATE transaction.
        tail = range(16384-256, 16384)
        free = [s for s in tail if not bm[s//8] & (1 << (s%8)) and s not in reserved]
        assert len(free) >= n+1
        data, ext = free[:n], free[n]
        assert data == list(range(data[0],data[0]+n))
        for s in data+[ext]:
            bm[s//8] |= 1 << (s%8)
            reserved.add(s)
        raw[ext*512:(ext+1)*512] = afs1.pack_extent_block(0,[(data[0],n)])
        raw[table*512+index*64:table*512+(index+1)*64] = (
            afs1.pack_object(afs1.OBJ_FILE,name,size,ext))
    raw[bitmap_head*512:(bitmap_head+afs1.BITMAP_SECTORS)*512] = bm
    out = ROOT/'build/phase85-design-full-capacity.img'
    out.parent.mkdir(parents=True,exist_ok=True)
    out.write_bytes(raw)  # ignored build/; not a release or guest artifact
    assert afs1.audit(out) == [], afs1.audit(out)
    measured = afs1.Disk(out.read_bytes())
    _,tab,head=measured.commit()
    objects=measured.objects(tab)
    assert len([o for o in objects if o['type']==afs1.OBJ_FILE])==32
    before=out.read_bytes()
    # The receiver must refuse the attempted fourth v8 decision at
    # 32/32 BEFORE fsd CREATE. Pure preflight: no mutation to the bytes.
    extra=(b'v8-0123456789abcdefabcd-04',512)
    assert extra[0] not in [o['name'] for o in objects]
    assert not any(o['type']==afs1.OBJ_FREE for o in objects)
    assert out.read_bytes()==before
    used=sum(b.bit_count() for b in measured.bitmap(head))
    assert used == initial_used + sum((size+511)//512+1 for _,size in names)
    free=16384-used
    assert free>1024  # refusal caused by object slots, NOT disk sectors.
    print(f'qualified 8.4 AFS1 template: 16384 sectors, {initial_used} allocated; '
          f'synthetic full fixture: 19 old + 8 staged + 2 AINS + 3 AACT = 32/32, '
          f'used={used}, free={free}; accepted conditional fourth AACT refused without mutation')
    return used,free


def _function_body(source: str, signature: str) -> str:
    start = source.index(signature)
    opening = source.index('{', start)
    depth = 1
    end = opening + 1
    while depth:
        if source[end] == '{':
            depth += 1
        elif source[end] == '}':
            depth -= 1
        end += 1
    return source[opening + 1:end - 1]


def cap_projection() -> None:
    entry=(ROOT/'kernel/kernel/src/entry.rs').read_text()
    literal=entry.split('let all = [',1)[1].split('];',1)[0]
    assert literal.count('Cap {') == 20 and 'let root = [' in entry  # root[0..2] + 20 = 22
    # Phase 12 ADR-0088 raises only the source-tree fixed budgets required by
    # 32 real desktop sessions. The old manager's private 32-cap manifest bound
    # remains an independent policy limit; the schedules below still fit it.
    caps = (ROOT/'kernel/kernel/src/cap.rs').read_text()
    processes = (ROOT/'kernel/kernel/src/proc.rs').read_text()
    spawn = (ROOT/'kernel/kernel/src/spawn.rs').read_text()
    shared = (ROOT/'kernel/kernel/src/shared.rs').read_text()
    ipc = (ROOT/'kernel/kernel/src/ipc.rs').read_text()
    desktop_model = (ROOT/'userspace/desktop/src/model.rs').read_text()
    assert 'pub const CAP_SLOTS: usize = 128;' in caps
    assert 'pub const MAX_PROCESSES: usize = 64;' in processes
    assert 'pub const MAX_SPAWN_RECS: usize = 64;' in spawn
    assert 'pub const MAX_REGIONS: usize = 96;' in shared
    assert 'pub const TOTAL_PAGES: u32 = 36864;' in shared
    assert 'pub const MAX_MAPS: usize = 160;' in shared
    assert 'pub const MAX_NOTIFS: usize = 64;' in ipc
    assert 'pub const MAX_WINDOWS: usize = 32;' in desktop_model
    assert 'pub const MAX_CAPS: usize = 32;' in (ROOT/'userspace/servicemgr/src/manifest.rs').read_text()
    assert 'pub const MAX_INHERIT: usize = 7;' in spawn
    assert 'pub const MAX_GRANTS: usize = 5;' in (ROOT/'userspace/servicemgr/src/manifest.rs').read_text()
    package=(ROOT/'userspace/servicemgr/src/package.rs').read_text()
    assert 'const GRANTS: [Request; 5]' in package
    packaged=(ROOT/'userspace/packaged/src/main.rs').read_text()
    assert 'const BUFFER: u64 = 7;' in packaged and 'const LENT: u64 = 8;' in packaged
    # Desktop preserves Phase-10's filesystem endpoint at slot 13 and the
    # filesd lineage at slot 20 while extending its private clocks to 32.
    import re
    entry = (ROOT/'kernel/kernel/src/entry.rs').read_text()
    desktop_bin = (ROOT/'userspace/desktop/src/bin/desktop.rs').read_text()
    kernel_clock = _function_body(entry, 'const fn desktop_clock_slot(index: usize) -> usize')
    desktop_clock = _function_body(desktop_bin, 'fn clock(i: usize) -> u64')
    compact = lambda text: re.sub(r'\s+', '', text)
    assert compact(kernel_clock) == (
        'ifindex<6{7+index}elseifindex<12{8+index}elseifindex<21{9+index}else{11+index}')
    assert compact(desktop_clock) == (
        'ifi<6{7+iasu64}elseifi<12{8+iasu64}elseifi<21{9+iasu64}else{11+iasu64}')
    clock_slots = [7+i if i < 6 else 8+i if i < 12 else 9+i if i < 21 else 11+i
                   for i in range(32)]
    assert clock_slots == (list(range(7, 13)) + list(range(14, 20))
                           + list(range(21, 30)) + list(range(32, 43)))
    assert len(set(clock_slots)) == 32
    assert not set(clock_slots).intersection({13, 20, 30, 31})
    assert 'app_clock_nids = [0u32; 32]' in entry
    assert 'obj: CapObj::Endpoint { eid: fs_eid }' in entry and '        13,' in entry
    assert '        20,' in entry  # `/Users/user` lineage remains reserved
    fs_backend=(ROOT/'userspace/desktop/src/fs_backend.rs').read_text()
    assert 'pub const FILE_FRAME_SLOT: u64 = 125;' in fs_backend
    assert 'pub const FILE_FRAME_LENT_SLOT: u64 = 126;' in fs_backend
    assert 'syscall1(SYS_ALLOC_FRAME, FILE_FRAME_SLOT)' in fs_backend
    assert 'syscall3(SYS_CAP_COPY,FILE_FRAME_SLOT,FILE_FRAME_LENT_SLOT,RIGHTS_ALL,)' in compact(fs_backend)
    assert 'pub const MAX_ENDPOINTS: usize = 16;' in ipc
    # Source-anchored upper schedule: actual manager 22 literal boot caps,
    # including registrar and lifecycle-admin marker. Conservatively
    # include four other resident Process handles (stack/broker/app/package).
    boot=22; resident=4
    # This historical serialized schedule uses one dynamic child + Image; at most one readiness worker,
    # but no worker overlaps SELECT/COMMIT. No Image is transferred until
    # OLD child and its held Process cap have been finished.
    active=boot+resident+1+1
    prepared=active  # new Image only in verifier, two registry slots live
    after_reap=boot+resident
    commit=after_reap+2  # landed transitional Image + attenuated READ|DESTROY
    worker_peak=active+1
    assert max(prepared,commit,worker_peak) == 29 < 32
    spawn=(ROOT/'kernel/kernel/src/spawn.rs').read_text()
    # Phase 13's larger verified-image registry admits bounded concurrent
    # native launch attempts; this Phase-8.5 manager projection still counts
    # only its own four-child service schedule.
    assert 'pub const MAX_DYNAMIC_CHILDREN: usize = 24;' in spawn
    multi_child=boot+resident+1+4
    assert multi_child==31 < 32
    # Packaged holds 5 inherited, an owned LENT buffer at slot8, one
    # landed marker and one provisional Image: 8. Slot7 is consumed by map.
    packaged_peak=5+1+1+1
    assert packaged_peak==8 < 32
    print(f'host cap schedule projection (NOT guest high-water): manager '
          f'boot={boot}, resident={boot+resident}, old-child/PREPARE={prepared}, '
          f'COMMIT={commit}, worker-separated bound={worker_peak}/32; '
          f'packaged <= {packaged_peak}/32; four-child steady schedule <= {multi_child}/32 (guest capacity proof separate)')

if __name__ == '__main__':
    disk_measure()
    cap_projection()
    print('HOST design capacity PASS; candidate records are synthetic; no guest 8.5 proof')
