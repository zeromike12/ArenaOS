#!/usr/bin/env python3
"""ADR-0053 targeted guest staging proof against actual AFS1 + fsd IPC.

Only PUBLIC frozen OpenSSL-signed artifacts enter the guest platter; no
private seed, host signing service, image activation or production key.
This is a targeted integration gate, NOT 100/100 final qualification.
"""
import hashlib
import shutil
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1
import arena_env
import mtest
import package_record as record
from test_package_record import RFC_SEED, SUB_PUB, openssl_sign

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / 'audit/phase84-crypto/corpus'
ROOT_FILE = (CORPUS / 'package-79a0ebe5653addf98c8060fc.bin').read_bytes()
SUB_FILE = (CORPUS / 'package-6a1b68578b2f1cdaabc41975.bin').read_bytes()
POLICY = (CORPUS / 'policy-06487f2f7e48586e876b8552.bin').read_bytes()
ID = record.make_id('app.test')
PREFIX = hashlib.sha256(ID).hexdigest()[:20]
INPUT = ('i8-' + PREFIX).encode()
INTENT = ('a8-' + PREFIX).encode()
STAGE1 = ('s8-' + PREFIX + '-01').encode()
STAGE2 = ('s8-' + PREFIX + '-02').encode()
POLICY1 = ('p8-' + PREFIX + '-01').encode()


def check(ok, why):
    print(f'[m84-stage] {"PASS" if ok else "FAIL"}: {why}', flush=True)
    return ok


def contents(path):
    d = afs1.Disk(path.read_bytes())
    _, table, _ = d.commit()
    result = {}
    for obj in d.objects(table):
        if obj['type'] != afs1.OBJ_FILE: continue
        result[obj['name']] = b''.join(d.sector(s) for base, count in d.extents(obj['extent_head'])
                                       for s in range(base, base+count))[:obj['size']]
    return result


def host_seed(path, records):
    """Power-off host fixture insertion into an already committed platter.

    The host writes only public test bytes, available through raw FS to the
    trusted shell anyway. It does not fabricate guest CREATE/WRITE replies.
    Preserve AFS1's live commit, object table, bitmap, and all existing files.
    Allocate at the disk tail after checking the bitmap and audit the result.
    """
    raw = bytearray(path.read_bytes())
    disk = afs1.Disk(raw)
    _, table, bitmap_head = disk.commit()
    objs = disk.objects(table)
    bitmap = disk.bitmap(bitmap_head)
    total = afs1.SCRATCH_TOTAL_SECTORS_EXPECTED
    reserved = set()
    for name, data in records.items():
        assert 1 <= len(name) < afs1.NAME_MAX and data
        existing = next((i for i, obj in enumerate(objs)
                         if obj['type'] == afs1.OBJ_FILE and obj['name'] == name), None)
        if existing is not None:
            index = existing
            # Only for an offline test candidate input, never accepted p8/s8.
            assert name in (INPUT, INTENT)
            for base, length in disk.extents(objs[index]['extent_head']):
                for s in range(base, base+length): reserved.add(s)
            reserved.add(objs[index]['extent_head'])
        else:
            index = next(i for i, obj in enumerate(objs) if obj['type'] == afs1.OBJ_FREE)
        need = (len(data) + 511) // 512
        free = [s for s in range(total-128, total)
                if not bitmap[s//8] & (1 << (s%8)) and s not in reserved]
        assert len(free) >= need+1
        sectors, extent = free[:need], free[need]
        assert sectors == list(range(sectors[0], sectors[0]+need))
        for s in sectors + [extent]:
            reserved.add(s)
            bitmap[s//8] |= 1 << (s%8)
        for j, s in enumerate(sectors):
            raw[s*512:(s+1)*512] = data[j*512:(j+1)*512].ljust(512,b'\0')
        raw[extent*512:(extent+1)*512] = afs1.pack_extent_block(0, [(sectors[0],need)])
        raw[table*512+index*64:table*512+(index+1)*64] = (
            afs1.pack_object(afs1.OBJ_FILE,name,len(data),extent))
        objs[index] = {'type':afs1.OBJ_FILE,'name':name,'size':len(data),'extent_head':extent}
    raw[bitmap_head*512:(bitmap_head+afs1.BITMAP_SECTORS)*512] = bitmap
    path.write_bytes(raw)
    assert not afs1.audit(path), afs1.audit(path)


def boot(esp, disk, tag, commands):
    feed = []
    for i, cmd in enumerate(commands, 1):
        feed.append(((b'arena>', b'packaged READY') if i == 1 else b'arena>', i, cmd+b'\r'))
    rc, serial, elapsed = mtest.boot('m84-'+tag, esp, feed, disk)
    (arena_env.build_dir()/f'serial-m84-{tag}.log').write_text(serial)
    print(f'[m84-stage] {tag}: rc={rc} {elapsed:.1f}s',flush=True)
    return rc, serial


def main():
    esp = mtest.build('m84-stage')
    disk = arena_env.make_scratch_disk()
    rc, s = boot(esp, disk, 'baseline', [b'shutdown'])
    if not check(rc == 0 and 'packaged READY' in s and not afs1.audit(disk),
                 'baseline clean boot/ready and AFS1 platter'): return 1
    baseline = arena_env.build_dir()/'m84-empty-baseline.img'
    shutil.copyfile(disk, baseline)
    host_seed(disk, {INPUT:ROOT_FILE, INTENT:POLICY})
    if not check(contents(disk)[INPUT] == ROOT_FILE and contents(disk)[INTENT] == POLICY,
                 'host-seeded independently signed public inputs byte exact'): return 1
    negatives = [b'pkg stage-noauth app.test',b'pkg stage-wrong app.test',
                 b'pkg stage-attenuated app.test'] + [b'pkg stage-wrongkind app.test']*40
    rc, s = boot(esp, disk, 'stage-root', [b'pkg query app.test']+negatives+
                [b'pkg stage app.test',b'pkg policy app.test',b'pkg query app.test',
                 b'pkg stage app.test',b'shutdown'])
    rec = contents(disk)
    expected = hashlib.sha256(ROOT_FILE).hexdigest()
    ok = check(rc == 0 and 'pkg: UNSET' in s
               and s.count('pkg: refused status -2') == len(negatives)
               and 'post-dispatch cap occupancy leaked' not in s
               and s.count('ELIGIBLE (staged, NOT installed/active) version 7 digest '+expected) == 3
               and 'POLICY committed generation 1' in s and 'packaged READY' in s
               and rec.get(STAGE1) == ROOT_FILE and rec.get(POLICY1) == POLICY
               and STAGE2 not in rec and not afs1.audit(disk),
               'guest STAGE exact bytes, POLICY root chain, QUERY and idempotent no-op')
    if not ok:
        print('\n'.join(l for l in s.splitlines() if 'pkg:' in l or 'packaged:' in l))
        return 1
    accepted_base=arena_env.build_dir()/'m84-stage-one-accepted.img'
    shutil.copyfile(disk,accepted_base)
    for tag,version in (('equal-version-conflict',7),('version-downgrade',6)):
        platter=arena_env.build_dir()/f'm84-{tag}.img'
        shutil.copyfile(accepted_base,platter)
        unsigned=record.signed_package(ID,version,b'independent different bytes',record.ROOT)
        conflict=unsigned+openssl_sign(RFC_SEED,record.PKG_DOMAIN+unsigned)
        host_seed(platter,{INPUT:conflict})
        r,serial=boot(esp,platter,tag,[b'pkg stage app.test',b'pkg query app.test',b'shutdown'])
        after=contents(platter)
        ok &= check(r==0 and 'pkg: refused status -2' in serial
                    and 'ELIGIBLE (staged, NOT installed/active) version 7 digest '+expected in serial
                    and after.get(STAGE1)==ROOT_FILE and STAGE2 not in after,
                    f'{tag}: receiver refuses and retains exact current staged bytes')
        if not ok:return 1
    # Power shell asks the manager over its PRIVATE notification. Manager
    # alone owns Process/DESTROY; the receiver is replaced on its original
    # endpoint and must scan/reverify this same durable platter again.
    feed=[((b'permission app reaped through held Process cap',b'packaged READY',b'arena>'),
            1,b'perm snapshot\r'),
          (b'arena>',2,b'pkg restart\r'),
          ((b'packaged replacement ready on original endpoint',b'arena>'),
            1,b'perm snapshot\r'),
          (b'arena>',4,b'pkg query app.test\r'),
          (b'arena>',5,b'shutdown\r')]
    rc,restarted,_=mtest.boot('m84-manager-restart',esp,feed,disk)
    (arena_env.build_dir()/'serial-m84-manager-restart.log').write_text(restarted)
    import re
    counts=re.findall(r'permission: resource snapshot free/records/processes (\d+)/(\d+)/(\d+)',restarted)
    ok &= check(rc==0 and len(counts)==2 and counts[0]==counts[1]
                and 'packaged reaped through held Process cap' in restarted
                and 'packaged replacement ready on original endpoint' in restarted
                and 'ELIGIBLE (staged, NOT installed/active) version 7 digest '+expected in restarted,
                'manager-only Process-cap restart: same endpoint, durable rescan, exact resources')
    if not ok: return 1
    rc, s = boot(esp, disk, 'same-disk-reboot', [b'pkg query app.test', b'shutdown'])
    ok &= check(rc == 0 and 'packaged READY' in s and
                'ELIGIBLE (staged, NOT installed/active) version 7 digest '+expected in s,
                'same-disk reboot re-verifies staged bytes and root policy')
    if not ok: return 1
    # Power-off host fixture replaces input candidate ONLY, never immutable
    # accepted stage/policy. The guest must refuse its altered signature.
    bad = bytearray(ROOT_FILE); bad[-1] ^= 1
    host_seed(disk, {INPUT:bytes(bad)})
    rc, s = boot(esp, disk, 'bad-signature', [b'pkg stage app.test',
                                             b'pkg query app.test',b'shutdown'])
    ok &= check(rc == 0 and 'pkg: refused status -1' in s
                and 'ELIGIBLE (staged, NOT installed/active) version 7 digest '+expected in s
                and contents(disk)[STAGE1] == ROOT_FILE and STAGE2 not in contents(disk),
                'guest rejects altered candidate signature without mutating accepted history')
    if not ok: return 1
    host_seed(disk, {INPUT:SUB_FILE})
    rc,s = boot(esp,disk,'stage-subordinate',[b'pkg stage app.test',b'pkg query app.test',
                                                b'pkg stage app.test',b'shutdown'])
    rec=contents(disk)
    expect2=hashlib.sha256(SUB_FILE).hexdigest()
    ok &= check(rc == 0 and s.count('ELIGIBLE (staged, NOT installed/active) version 8 digest '+expect2)==3
                and rec.get(STAGE2)==SUB_FILE and rec.get(STAGE1)==ROOT_FILE
                and not afs1.audit(disk),
                'policy-authorized subordinate stage2, predecessor retained, idempotent repeat')
    if not ok: return 1
    unsigned = record.signed_policy(ID, 2, SUB_PUB, 7, 1,
                                     (hashlib.sha256(SUB_FILE).digest(),))
    revoked = unsigned + openssl_sign(RFC_SEED, record.POL_DOMAIN + unsigned)
    # Host-only public fixture replacement; the receiver still demands a
    # fresh transferred marker and a root signature on every update.
    host_seed(disk, {INTENT:revoked})
    rc,s=boot(esp,disk,'revoke-stage2',[b'pkg policy app.test', b'pkg query app.test',
                                       b'pkg stage app.test',b'shutdown'])
    rec=contents(disk)
    ok &= check(rc == 0 and 'POLICY committed generation 2' in s
                and 'pkg: INELIGIBLE' in s and 'pkg: refused status -2' in s
                and rec.get(POLICY1)==POLICY and rec.get(STAGE2)==SUB_FILE
                and len([k for k in rec if k.startswith(b's8-')])==2
                and not afs1.audit(disk),
                'root-signed revocation makes prior stage ineligible and refuses duplicate')
    if not ok: return 1
    rc,s=boot(esp,disk,'reboot-revoked',[b'pkg query app.test',b'pkg query other.app',b'shutdown'])
    ok &= check(rc==0 and 'packaged READY' in s and 'pkg: INELIGIBLE' in s
                and 'pkg: refused status -6' in s,
                'same-platter REVOKE persists; different namespace collision refuses')
    if not ok: return 1
    # Third stage is a root-signed *eligible* higher version, but the
    # fixed historical budget is two. No s8-03 may be created.
    next_unsigned=record.signed_package(ID,9,b'public third candidate',record.ROOT)
    third=next_unsigned+openssl_sign(RFC_SEED,record.PKG_DOMAIN+next_unsigned)
    host_seed(disk,{INPUT:third})
    rc,s=boot(esp,disk,'third-stage-capacity',[b'pkg stage app.test',b'shutdown'])
    ok &= check(rc==0 and 'pkg: refused status -3' in s and STAGE2 in contents(disk)
                and not any(k.startswith(b's8-') and k not in (STAGE1,STAGE2) for k in contents(disk)),
                'third signed stage typed NO_SPACE before CREATE, history unmodified')
    if not ok: return 1
    digest=hashlib.sha256(SUB_FILE).digest()
    for gen in (3,4):
        unsigned=record.signed_policy(ID,gen,SUB_PUB,7,1,(digest,))
        signed=unsigned+openssl_sign(RFC_SEED,record.POL_DOMAIN+unsigned)
        host_seed(disk,{INTENT:signed})
        rc,s=boot(esp,disk,f'policy-{gen}',[b'pkg policy app.test',b'shutdown'])
        ok &= check(rc==0 and f'POLICY committed generation {gen}' in s
                    and contents(disk).get(('p8-'+PREFIX+f'-0{gen}').encode())==signed,
                    f'root-signed policy generation {gen} durably chained')
        if not ok: return 1
    rc,s=boot(esp,disk,'fifth-policy-capacity',[b'pkg policy app.test',b'shutdown'])
    ok &= check(rc==0 and 'pkg: refused status -3' in s
                and not any(k.startswith(b'p8-') and k not in
                            {('p8-'+PREFIX+f'-0{n}').encode() for n in (1,2,3,4)}
                            for k in contents(disk)),
                'fifth policy typed NO_SPACE before CREATE, all four immutable generations retained')
    if not ok: return 1
    # Refusal at the real guest receiver, not merely the Python/Rust oracle.
    # Candidate data can be corrupt without poisoning accepted history:
    # no p8/s8 files exist yet on these independently copied platters.
    variants = {
        'wrong-key': (SUB_FILE, -2),
        'truncated': (ROOT_FILE[:-1], -1),
        'altered-manifest': (ROOT_FILE[:44]+bytes([ROOT_FILE[44]^1])+ROOT_FILE[45:], -1),
        'altered-payload': (ROOT_FILE[:128]+bytes([ROOT_FILE[128]^1])+ROOT_FILE[129:], -1),
        'noncanonical-id': (ROOT_FILE[:12]+b'A'+ROOT_FILE[13:], -1),
    }
    for label, (artifact, status) in variants.items():
        platter=arena_env.build_dir()/f'm84-refusal-{label}.img'
        shutil.copyfile(baseline,platter)
        host_seed(platter,{INPUT:artifact})
        rc,s=boot(esp,platter,label,[b'pkg stage app.test',b'pkg query app.test',b'shutdown'])
        records=contents(platter)
        ok &= check(rc==0 and 'packaged READY' in s
                    and f'pkg: refused status {status}' in s and 'pkg: UNSET' in s
                    and STAGE1 not in records and POLICY1 not in records
                    and not afs1.audit(platter),
                    f'{label}: guest typed refusal, no accepted stage or policy mutation')
        if not ok: return 1
    alias_platter=arena_env.build_dir()/'m84-alias-policy.img'
    shutil.copyfile(baseline,alias_platter)
    alias=(CORPUS/'policy-a8f728dff763708fd84e7eb1.bin').read_bytes()
    host_seed(alias_platter,{INTENT:alias})
    rc,s=boot(esp,alias_platter,'alias-policy',[b'pkg policy app.test',
                                                b'pkg query app.test',b'shutdown'])
    ok &= check(rc==0 and 'pkg: refused status -1' in s and 'pkg: UNSET' in s
                and POLICY1 not in contents(alias_platter) and not afs1.audit(alias_platter),
                'root-signed ZIP-215 subordinate-key alias refused at guest receiver')
    if not ok:return 1
    # Maximum eight cumulative digests are a valid root-signed policy.
    # The frozen 448-byte header has NO ninth digest slot. The issuer-side
    # `Policy::add_revocation` host test proves a distinct ninth returns
    # typed NO_SPACE *before signing*; no guest request can encode nine
    # digests without changing the accepted persistent wire format.
    eight=tuple(sorted(hashlib.sha256(b'm84-public-'+bytes([i])).digest() for i in range(8)))
    header=record.signed_policy(ID,1,SUB_PUB,7,1,eight)
    full_policy=header+openssl_sign(RFC_SEED,record.POL_DOMAIN+header)
    bounded=arena_env.build_dir()/'m84-eight-revoked.img'
    shutil.copyfile(baseline,bounded)
    host_seed(bounded,{INTENT:full_policy,INPUT:ROOT_FILE})
    rc,s=boot(esp,bounded,'eight-revoked',[b'pkg policy app.test',
                                          b'pkg stage app.test',b'shutdown'])
    ok &= check(rc==0 and 'POLICY committed generation 1' in s
                and 'ELIGIBLE (staged, NOT installed/active) version 7' in s
                and contents(bounded).get(POLICY1)==full_policy and not afs1.audit(bounded),
                'guest verifies root-signed eight-digest cumulative capacity, no unrepresented ninth')
    if not ok:return 1
    corrupt=arena_env.build_dir()/'m84-corrupt-visible.img'
    shutil.copyfile(baseline,corrupt)
    invalid=bytearray(ROOT_FILE); invalid[-1]^=1
    host_seed(corrupt,{STAGE1:bytes(invalid)})
    rc,s,_=mtest.boot('m84-corrupt-visible',esp,[(b'arena>',1,b'pkg query app.test\r'),
                                               (b'arena>',2,b'shutdown\r')],corrupt)
    (arena_env.build_dir()/'serial-m84-corrupt-visible.log').write_text(s)
    ok &= check(rc==0 and 'boot namespace scan refused: corrupt/offline, no READY' in s
                and 'packaged READY' not in s and 'packaged OFFLINE' in s
                and 'pkg: transport refused' in s and contents(corrupt)[STAGE1]==bytes(invalid)
                and not afs1.audit(corrupt),
                'malformed visible highest stage blocks READY, no fallback or false eligibility')
    # Entire object table occupied by a power-off host fixture, but the
    # actual receiver (not the host) must preflight and return typed
    # NO_SPACE without ever creating a partial s8-01 file.
    table=arena_env.build_dir()/'m84-object-table-full.img'
    shutil.copyfile(baseline,table)
    fillers={f'f84-{n:02}'.encode():b'x' for n in range(29)}
    host_seed(table,{INPUT:ROOT_FILE,INTENT:POLICY,**fillers})
    assert len(contents(table))==32
    rc,s=boot(esp,table,'object-table-full',[b'pkg stage app.test',b'pkg query app.test',b'shutdown'])
    ok &= check(rc==0 and 'pkg: refused status -3' in s and 'pkg: UNSET' in s
                and STAGE1 not in contents(table) and not afs1.audit(table),
                '32/32 AFS1 objects: typed pre-CREATE NO_SPACE, no partial stage')
    if not ok: return 1
    # Metadata-consistent allocator-full platter with no public free-
    # sector API. CREATE may be ambiguous; fail closed as DEGRADED until
    # restart rather than claiming safe preflight or a staged object.
    full=arena_env.build_dir()/'m84-allocator-full.img'
    shutil.copyfile(baseline,full)
    host_seed(full,{INPUT:ROOT_FILE,INTENT:POLICY})
    raw=bytearray(full.read_bytes())
    diskmeta=afs1.Disk(raw)
    _,_,bitmap_head=diskmeta.commit()
    raw[bitmap_head*512:(bitmap_head+afs1.BITMAP_SECTORS)*512]=b'\xff'*(afs1.BITMAP_SECTORS*512)
    full.write_bytes(raw)
    assert not afs1.audit(full)
    rc,s=boot(esp,full,'allocator-full',[b'pkg stage app.test',b'pkg query app.test',b'shutdown'])
    ok &= check(rc==0 and s.count('pkg: refused status -4')==2
                and contents(full).get(STAGE1)==b'' and not afs1.audit(full),
                'allocator-full WRITE leaves visible empty highest stage; DEGRADED never acknowledges')
    if not ok: return 1
    rc,s,_=mtest.boot('m84-allocator-reboot',esp,[(b'arena>',1,b'pkg query app.test\r'),
                                                (b'arena>',2,b'shutdown\r')],full)
    (arena_env.build_dir()/'serial-m84-allocator-reboot.log').write_text(s)
    ok &= check(rc==0 and 'packaged READY' not in s and 'boot namespace scan refused' in s
                and 'pkg: transport refused' in s and contents(full).get(STAGE1)==b'',
                'same-platter reboot detects partial highest and refuses old/false stage')
    print(f'[m84-stage] TARGETED GUEST RESULT: {"PASS" if ok else "FAIL"}; not milestone qualification')
    return 0 if ok else 1

if __name__ == '__main__':sys.exit(main())
