#!/usr/bin/env python3
"""ADR-0046 guest transactional SET under AFS1's ordered-sector crash model.

The shell is a TRUSTED raw-FS test-intent writer, never the unprivileged
reader or update authority. A separate image receives a kernel-issued
marker and transfers it to configd. Every crash is SIGKILL of QEMU, then
host audits the same disk BEFORE recovery boot; no repair tool runs.
"""
import re
import shutil
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import config_record  # noqa: E402
import mtest  # noqa: E402

LABEL = "m81-update"
ONE = b"guest-v1"
TWO = b"guest-v2"
PASS = "configup: TRUSTED UPDATE PASS (receiver marker, disk rescan, independent exact READ)"


def check(ok: bool, msg: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {msg}")
    return ok


def get(disk: Path, seq: int):
    volume = afs1.Disk(disk.read_bytes())
    _, ot, _ = volume.commit()
    obj = volume.find(config_record.name(seq).encode(), ot)
    if obj is None or obj['size'] == 0:
        return None if obj is None else b''
    assert obj['size'] == 512
    data = b''.join(volume.sector(s) for base, length in volume.extents(obj['extent_head'])
                    for s in range(base, base + length))[:512]
    return config_record.unpack(data, seq).payload


def resource_use(serial: str):
    """Exact boot-relative counts, not absolute UEFI-dependent free RAM.

    OVMF can hand the kernel 15 more/fewer usable frames on a different
    boot (different initial map/key). Subtract the actual post-EBS free
    pool before comparing, with zero tolerated *consumption* drift.
    """
    start = re.findall(r"frames: post-EBS reconcile leaked 0 frame\(s\); (\d+) free", serial)
    snap = re.findall(r"m8: stackstress baseline frames=(\d+) records=(\d+) processes=(\d+)",
                      serial)
    if len(start) != 1 or len(snap) != 1:
        return None
    free, records, processes = map(int, snap[0])
    return (int(start[0]) - free, records, processes)


def boot(tag: str, esp: Path, disk: Path, feed=None, kill=None):
    rc, serial, dt = mtest.boot(f"{LABEL}-{tag}", esp, feed or [], disk, kill=kill)
    (arena_env.build_dir() / f"serial-{LABEL}-{tag}.log").write_text(serial)
    print(f"[{LABEL}] {tag}: rc={rc}, {dt:.1f}s")
    return rc, serial


def main() -> int:
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    ok = True
    rc, initial = boot("stage-one", esp, disk, [(b"arena>", 1, b"stackstress\r"),
                                               (b"arena>", 2, b"write cfg-intent-one x\r"),
                                               (b"arena>", 3, b"shutdown\r")])
    ok &= check(rc == 0 and "configup: SKIP" in initial
                and "m8: stackstress PASS" in initial
                and "wrote 1 bytes to 'cfg-intent-one'" in initial and not afs1.audit(disk),
                "trusted intent staged without automatic write or updater authority in shell")
    rc, first = boot("commit-one", esp, disk, [(b"arena>", 1, b"stackstress\r"),
                                               (b"arena>", 2, b"shutdown\r")])
    ok &= check(rc == 0 and PASS in first and "configup: SET COMMITTED" in first
                and "m8: stackstress PASS" in first and get(disk, 1) == ONE
                and not afs1.audit(disk),
                "separate marker-holding updater committed exact seq1 to real disk")
    rc, repeat = boot("repeat-one", esp, disk, [(b"arena>", 1, b"stackstress\r"),
                                               (b"arena>", 2, b"shutdown\r")])
    ok &= check(rc == 0 and PASS in repeat and "configup: SET UNCHANGED" in repeat
                and "m8: stackstress PASS" in repeat and get(disk, 1) == ONE
                and get(disk, 2) is None and not afs1.audit(disk),
                "idempotent repeated intent spends no second generation")
    counters = [resource_use(serial) for serial in (initial, first, repeat)]
    ok &= check(all(count is not None for count in counters)
                and counters[0] == counters[1] == counters[2],
                f"Power-gated boot-relative frame/record/process consumption exact across skip/commit/no-op: {counters}")
    rc, stage = boot("stage-two", esp, disk,
                     [(b"arena>", 1, b"rm cfg-intent-one\r"),
                      (b"arena>", 2, b"write cfg-intent-two x\r"),
                      (b"arena>", 3, b"shutdown\r")])
    ok &= check(rc == 0 and "wrote 1 bytes to 'cfg-intent-two'" in stage
                and get(disk, 1) == ONE and get(disk, 2) is None and not afs1.audit(disk),
                "trusted next intent staged without mutating old committed value")
    if not ok:
        return 1
    baseline = arena_env.build_dir() / "m81-update-two-base.img"
    shutil.copyfile(disk, baseline)
    seq, _, _ = afs1.Disk(baseline.read_bytes()).commit()
    # Each round starts from the SAME immutable old value and staged
    # trusted intent; the host checks committed disk BEFORE any retry.
    points = [
        ("precreate", b"configd: SET CREATE submitted"),
        ("empty", b"configd: SET CREATE committed empty generation"),
        ("prewrite", b"configd: SET WRITE submitted"),
        ("write-commit", f"fsd: commit seq {seq+2} (write)".encode()),
        ("write-reply", b"configd: SET WRITE committed (fsd replied)"),
        ("close", b"configd: SET CLOSE completed"),
        ("reply", b"configd: SET exact new value verified before reply"),
    ]
    for tag, marker in points:
        crash = arena_env.build_dir() / f"m81-update-{tag}.img"
        shutil.copyfile(baseline, crash)
        rc, killed = boot(f"kill-{tag}", esp, crash, kill=(marker, 1, 0.0))
        ok &= check(rc is None and marker.decode() in killed,
                    f"{tag}: SIGKILL after observed real guest boundary, never a clean exit")
        problems = afs1.audit(crash)
        old = get(crash, 1)
        new = get(crash, 2)
        legal = ((None, b'', TWO) if tag == "precreate" else
                 (b'', TWO) if tag in ("empty", "prewrite") else (TWO,))
        ok &= check(not problems and old == ONE and new in legal,
                    f"{tag}: committed platter new={new!r} within this boundary's exact legal states; audit clean")
        if not ok:
            break
        rc, recovered = boot(f"recover-{tag}", esp, crash,
                             [(b"arena>", 1, b"shutdown\r")])
        expected_reply = ("configup: SET UNCHANGED" if new == TWO
                          else "configup: SET COMMITTED")
        ok &= check(rc == 0 and PASS in recovered and expected_reply in recovered,
                    f"{tag}: same-disk reboot reconciled ambiguous kill (new if durable, otherwise retry)")
        ok &= check(get(crash, 1) == ONE and get(crash, 2) == TWO and not afs1.audit(crash),
                    f"{tag}: after recovery both immutable generations survive byte-exact")
        if not ok:
            break
    if ok:
        # Host-prepared allocator exhaustion, not a guest write: mark
        # every free sector allocated in the newest bitmap. AFS1's
        # audit explicitly permits unreachable allocation leaks. On
        # mount it reclaims eight old metadata sectors; CREATE can
        # commit empty, but WRITE cannot claim data+metadata. The
        # service must type NO_SPACE (disk), never COMMITTED, and the
        # previous value remains physically byte-exact.
        full = arena_env.build_dir() / "m81-update-disk-full.img"
        shutil.copyfile(baseline, full)
        raw = bytearray(full.read_bytes())
        d = afs1.Disk(bytes(raw))
        _, _, bitmap = d.commit()
        start = bitmap * afs1.SECTOR
        raw[start:start + afs1.BITMAP_SECTORS * afs1.SECTOR] = (
            b"\xff" * (afs1.BITMAP_SECTORS * afs1.SECTOR))
        full.write_bytes(raw)
        ok &= check(not afs1.audit(full), "offline allocator-full fixture has a valid committed AFS1 namespace")
        rc, refused = boot("disk-full", esp, full, [(b"arena>", 1, b"shutdown\r")])
        ok &= check(rc == 0 and "configup: SET NO_SPACE (disk allocation refused)" in refused
                    and "configup: DEGRADED latch + exact old READ after FS_NO_SPACE" in refused
                    and "configup: SET COMMITTED" not in refused and get(full, 1) == ONE
                    and get(full, 2) in (None, b'') and not afs1.audit(full),
                    "FS allocator refused WRITE; second marked SET DEGRADED, old READ/record intact")
    if ok:
        # Visible record data corrupted OFFLINE while the AFS1 commit
        # remains intact: the privileged updater must refuse to create
        # a successor, just as the unprivileged reader refuses fallback.
        damaged = arena_env.build_dir() / "m81-update-visible-corrupt.img"
        shutil.copyfile(baseline, damaged)
        raw = bytearray(damaged.read_bytes())
        vol = afs1.Disk(bytes(raw))
        _, ot, _ = vol.commit()
        obj = vol.find(b'cfg8-01', ot)
        assert obj is not None
        block = vol.extents(obj['extent_head'])[0][0]
        raw[block * 512 + 33] ^= 1  # leave the record checksum stale
        damaged.write_bytes(raw)
        ok &= check(not afs1.audit(damaged), "visible-corrupt fixture still audits at AFS1 level")
        rc, refused = boot("visible-corrupt", esp, damaged,
                           [(b"arena>", 1, b"shutdown\r")])
        ok &= check(rc == 0 and "configread: READ CORRUPT (fail-closed)" in refused
                    and "configup: SET CORRUPT (visible record refused)" in refused
                    and get(damaged, 2) is None and not afs1.audit(damaged),
                    "reader and marker-holder both refuse a visible malformed predecessor")
    if ok:
        # Two conflicting test-intent files must never be treated as a
        # default preference; the first boot can legitimately commit
        # seq2 before the trusted shell stages the second name.
        ambiguous = arena_env.build_dir() / "m81-update-two-intents.img"
        shutil.copyfile(baseline, ambiguous)
        rc, staged = boot("stage-conflict", esp, ambiguous,
                          [(b"arena>", 1, b"write cfg-intent-one x\r"),
                           (b"arena>", 2, b"shutdown\r")])
        ok &= check(rc == 0 and "wrote 1 bytes to 'cfg-intent-one'" in staged
                    and get(ambiguous, 2) == TWO and not afs1.audit(ambiguous),
                    "second intent staged after the already-authorized update")
        rc, refused = boot("intent-conflict", esp, ambiguous,
                           [(b"arena>", 1, b"shutdown\r")])
        ok &= check(rc == 0 and "configup: REFUSED malformed test intent (no SET)" in refused
                    and get(ambiguous, 2) == TWO and get(ambiguous, 3) is None
                    and not afs1.audit(ambiguous),
                    "conflicting opt-in input typed refusal; no guessed update")
    print(f"[{LABEL}] AUTHORIZED TRANSACTION/CRASH: {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
