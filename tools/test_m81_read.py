#!/usr/bin/env python3
"""ADR-0046 incremental READ boundary, NOT the transactional 8.1 gate.

Boot a resident ring-3 configd and separate endpoint-only reader on a
real AFS1 disk. A second boot sees a visible malformed reserved record
written by the trusted raw-FS shell and refuses to return an old value.
No authenticated SET or atomic update is claimed yet.
"""
import shutil
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import config_record  # noqa: E402
import mtest  # noqa: E402

LABEL = "m81-read"
PASS = "configread: ORDINARY READ BOUNDARY PASS (no fsd or marker grant)"
REFUSED = "configread: SET absent/wrong-kind refused x20 by receiver"


def check(ok: bool, msg: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {msg}")
    return ok


def main() -> int:
    esp = mtest.build(LABEL)
    scratch = arena_env.make_scratch_disk()
    seeded = arena_env.build_dir() / "m81-value-scratch.img"
    ok = True
    for tag, feed, state in [
        ("fresh", [(b"arena>", 1, b"shutdown\r")], "UNSET"),
        ("seed", [(b"arena>", 1, b"write cfg8-01 bad\r"),
                  (b"arena>", 2, b"shutdown\r")], "UNSET"),
        ("corrupt", [(b"arena>", 1, b"ls\r"),
                     (b"arena>", 2, b"shutdown\r")], "CORRUPT"),
    ]:
        rc, serial, dt = mtest.boot(f"{LABEL}-{tag}", esp, feed, scratch)
        (arena_env.build_dir() / f"serial-{LABEL}-{tag}.log").write_text(serial)
        print(f"[{LABEL}] {tag}: rc={rc}, {dt:.1f}s")
        ok &= check(rc == 0 and "PANIC" not in serial and "halting machine:" not in serial
                    and "m7: RESULT PASS (2/2)" in serial and PASS in serial
                    and "configread: boot-root reader reaped" in serial,
                    f"{tag}: historical boot, real isolated reader, frame/record-reaped clean halt")
        ok &= check(REFUSED in serial and serial.count("configd: SET refused without receiving-service authority") == 20,
                    f"{tag}: no marker and copied wrong-kind endpoint refused 20x at receiver")
        marker = f"configread: READ {state}"
        ok &= check(marker in serial and
                    ("configread: READ VALUE" not in serial),
                    f"{tag}: FS-backed READ yields {state}, never an invented value")
        if tag == "seed":
            ok &= check("wrote 3 bytes to 'cfg8-01'" in serial,
                        "trusted raw-FS shell created an intentionally malformed visible record")
        if tag == "corrupt":
            ok &= check("configd: READ fail-closed (corrupt or unavailable)" in serial
                        and "cfg8-01  3 bytes" in serial,
                        "malformed reserved object remains visible; shell and fsd stay alive")
        ok &= check(not afs1.audit(scratch), f"{tag}: real AFS1 platter audits clean")
        if tag == "fresh":
            # M5's fresh branch REQUIRES only arena.txt. Preserve that
            # already-committed baseline for offline host provisioning.
            shutil.copy2(scratch, seeded)
    # A different, already guest-formatted scratch disk copied after the
    # first boot: M5 requires its arena.txt to be the only fresh file.
    # Offline host provisioning adds a valid immutable generation to
    # the live AFS1 metadata before another QEMU boot. This proves READ,
    # NOT a guest update transaction or crash-safe host injection.
    raw = bytearray(seeded.read_bytes())
    volume = afs1.Disk(bytes(raw))
    _, objtab, bitmap_head = volume.commit()
    objects = volume.objects(objtab)
    free_index = next(i for i, obj in enumerate(objects) if obj["type"] == afs1.OBJ_FREE)
    data_sector, extent_sector = afs1.SCRATCH_TOTAL_SECTORS_EXPECTED - 8, afs1.SCRATCH_TOTAL_SECTORS_EXPECTED - 7
    bitmap = volume.bitmap(bitmap_head)
    for sector in (data_sector, extent_sector):
        assert bitmap[sector // 8] & (1 << (sector % 8)) == 0, "fixture sectors must be unused"
        bitmap[sector // 8] |= 1 << (sector % 8)
    data = config_record.pack(1, b"seed-v1")
    raw[data_sector * 512:(data_sector + 1) * 512] = data
    raw[extent_sector * 512:(extent_sector + 1) * 512] = afs1.pack_extent_block(0, [(data_sector, 1)])
    obj_offset = objtab * 512 + free_index * afs1.OBJ_RECORD
    raw[obj_offset:obj_offset + afs1.OBJ_RECORD] = (
        afs1.pack_object(afs1.OBJ_FILE, b"cfg8-01", 512, extent_sector))
    raw[bitmap_head * 512:(bitmap_head + afs1.BITMAP_SECTORS) * 512] = bitmap
    seeded.write_bytes(raw)
    ok &= check(not afs1.audit(seeded), "host-formatted committed generation audits clean before boot")
    rc, value, dt = mtest.boot(f"{LABEL}-value", esp, [(b"arena>", 1, b"shutdown\r")], seeded)
    (arena_env.build_dir() / f"serial-{LABEL}-value.log").write_text(value)
    print(f"[{LABEL}] preseeded value: rc={rc}, {dt:.1f}s")
    ok &= check(rc == 0 and PASS in value and REFUSED in value
                and "configread: READ VALUE seed-v1 seq1 (exact host fixture bytes)" in value
                and "m7: RESULT PASS (2/2)" in value and not afs1.audit(seeded),
                "real fsd/storaged DMA delivers exact precommitted bytes, no write inferred")

    # Two visible immutable generations: corrupt the NEWEST payload
    # without repairing its checksum. Returning the intact seq1 value
    # would be a silent downgrade, not fail-closed recovery.
    newest = arena_env.build_dir() / "m81-bad-newest.img"
    shutil.copy2(seeded, newest)
    raw = bytearray(newest.read_bytes())
    volume = afs1.Disk(bytes(raw))
    _, objtab, bitmap_head = volume.commit()
    free_index = next(i for i, obj in enumerate(volume.objects(objtab)) if obj["type"] == afs1.OBJ_FREE)
    data_sector, extent_sector = afs1.SCRATCH_TOTAL_SECTORS_EXPECTED - 6, afs1.SCRATCH_TOTAL_SECTORS_EXPECTED - 5
    bitmap = volume.bitmap(bitmap_head)
    for sector in (data_sector, extent_sector):
        assert bitmap[sector // 8] & (1 << (sector % 8)) == 0
        bitmap[sector // 8] |= 1 << (sector % 8)
    corrupted = bytearray(config_record.pack(2, b"newest"))
    corrupted[33] ^= 1  # data corrupt, checksum still describes old data
    raw[data_sector * 512:(data_sector + 1) * 512] = corrupted
    raw[extent_sector * 512:(extent_sector + 1) * 512] = afs1.pack_extent_block(0, [(data_sector, 1)])
    offset = objtab * 512 + free_index * afs1.OBJ_RECORD
    raw[offset:offset + afs1.OBJ_RECORD] = afs1.pack_object(afs1.OBJ_FILE, b"cfg8-02", 512, extent_sector)
    raw[bitmap_head * 512:(bitmap_head + afs1.BITMAP_SECTORS) * 512] = bitmap
    newest.write_bytes(raw)
    ok &= check(not afs1.audit(newest), "newest-corrupt disk still has consistent AFS1 metadata")
    rc, rejected, dt = mtest.boot(f"{LABEL}-bad-newest", esp, [(b"arena>", 1, b"shutdown\r")], newest)
    (arena_env.build_dir() / f"serial-{LABEL}-bad-newest.log").write_text(rejected)
    print(f"[{LABEL}] newer corrupted value: rc={rc}, {dt:.1f}s")
    ok &= check(rc == 0 and PASS in rejected and REFUSED in rejected
                and "configread: READ CORRUPT (fail-closed)" in rejected
                and "configread: READ VALUE" not in rejected and not afs1.audit(newest),
                "visible corrupt newest record rejects older valid seq1: no silent downgrade")

    rc, missing, dt = mtest.run_qemu(f"{LABEL}-absent", esp, net=False, rng=False,
                                      kbd=False, vcon=False, tcp_peer=False)
    (arena_env.build_dir() / f"serial-{LABEL}-absent.log").write_text(missing)
    print(f"[{LABEL}] optional-device-absent: rc={rc}, {dt:.1f}s")
    ok &= check(rc == 0 and PASS in missing and REFUSED in missing
                and "configread: READ UNSET" in missing and "servicemgr: OFFLINE" in missing,
                "missing optional devices do not forge authority or prevent disk-only read")
    print(f"[{LABEL}] CONFIG-READ-BOUNDARY: {'PASS' if ok else 'FAIL'} (8.1 still incomplete)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
