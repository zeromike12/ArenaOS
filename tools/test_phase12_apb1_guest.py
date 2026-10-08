#!/usr/bin/env python3
"""Real Phase-12 APB1 Desktop authority handoff and AFS2 install proof."""
from pathlib import Path
import sys
import time
import re

sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1
import afs2
import apb1_format
import arena_env
import mtest
from test_m10_apps import Desktop

LABEL = "phase12-apb1-guest"
BASE = arena_env.AFS2_BASE_SECTOR * 512
DESKTOP_SOURCE = b"editor.apb1"


def tree(disk):
    for _ in range(100):
        try:
            volume = afs2.Volume(disk.read_bytes()[BASE:])
            afs2.check(volume)
            return afs2.walk(volume)
        except Exception:
            time.sleep(0.1)
    raise AssertionError("AFS2 volume did not become host-readable")


def seed_desktop_bundle(disk, bundle):
    volume = afs2.Volume(disk.read_bytes()[BASE:])
    desktop = volume.resolve("/Users/user/Desktop")
    source = volume.create(desktop, DESKTOP_SOURCE, 1)
    volume.write(source, 0, bundle, 1)
    with disk.open("r+b") as f:
        f.seek(BASE)
        f.write(volume.image())


def install_from_files(disk):
    d = Desktop(LABEL)
    try:
        d.wait(lambda: "AFS2 file service online" in d.serial(), "filesd did not mount AFS2")
        d.shot("apb1-desktop-icon")
        before = d.serial().count("[desktop] APB1 installed; signed version=")
        d.click(52, 58)  # double-click the APB1 icon in the Desktop background
        time.sleep(0.1)
        d.click(52, 58)
        d.wait(
            lambda: d.serial().count("[desktop] APB1 installed; signed version=") == before + 1,
            "Files did not complete the Desktop -> packaged -> protected filesd APB1 install",
        )
        d.wait(lambda: "/System/Applications/com.arena.editor/42/bin/editor" in tree(disk),
               "installed APB1 payload did not appear in the AFS2 application namespace")
        return b"pkg restart\r"
    finally:
        d.dispose()


def main():
    fixtures = apb1_format.default_fixtures()
    bundle = fixtures["editor.apb1"]
    parsed = apb1_format.parse_bundle(bundle)
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk()
    rc, serial, _ = mtest.boot(
        LABEL + "-seed", esp, [(b"arena>", 1, b"shutdown\r")], disk, pointer=True
    )
    assert rc == 0 and "no AFS2 region" in serial
    with disk.open("r+b") as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    assert not afs1.audit(disk)
    afs1_before = disk.read_bytes()[:arena_env.SCRATCH_MIB * 1024 * 1024]

    rc, serial, _ = mtest.boot(
        LABEL + "-afs2", esp,
        [(b"filesd: AFS2 mounted", 1, b"shutdown\r")], disk, pointer=True
    )
    assert rc == 0 and "filesd: AFS2 mounted" in serial
    assert disk.read_bytes()[:arena_env.SCRATCH_MIB * 1024 * 1024] == afs1_before
    seed_desktop_bundle(disk, bundle)
    assert tree(disk)["/Users/user/Desktop/" + DESKTOP_SOURCE.decode()] == bundle

    feed = [
        ((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"), 1,
         lambda: install_from_files(disk)),
        (b"servicemgr: packaged replacement ready on original endpoint", 1, b"pkg resources\r"),
        (b"pkg: observed frames=", 1, b"shutdown\r"),
    ]
    rc, serial, elapsed = mtest.boot(
        LABEL, esp, feed, disk, pointer=True, timeout_s=180
    )
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    assert rc == 0, serial[-5000:]
    assert "m7: RESULT PASS (2/2)" in serial
    assert "[desktop] APB1 installed; signed version=42" in serial
    assert serial.count("servicemgr: APB1 wrong-kind handoff refused kind=3 rights=5") >= 2
    assert serial.count("servicemgr: APB1 install-only BadgedEndpoint handed to READY packaged in late slot5") >= 2
    assert "[desktop] APB1 authority boundary: ordinary Filesd capability denied protected install" in serial
    assert "packaged: APB1 scope proof: install-only endpoint denied generic LIST" in serial
    assert "packaged: APB1 scope proof: same-kind capability with wrong rights denied (kind=12 rights=14)" in serial
    assert "servicemgr: packaged replacement ready on original endpoint" in serial
    assert "servicemgr: reserved APB1 slot127 descriptor=1 kind=12 rights=6" in serial
    assert "servicemgr: extended cap occupancy [32,127)=0" in serial
    readiness_marker = "servicemgr: packaged READY (full boot scan; exact PING + exit + deadline)"
    authority_marker = "servicemgr: observed cap occupancy apb1-authority-held="
    handoff_marker = "servicemgr: APB1 install-only BadgedEndpoint handed to READY packaged in late slot5"
    readiness_at = [m.start() for m in re.finditer(re.escape(readiness_marker), serial)]
    authority_at = [m.start() for m in re.finditer(re.escape(authority_marker), serial)]
    handoff_at = [m.start() for m in re.finditer(re.escape(handoff_marker), serial)]
    assert len(readiness_at) >= 2 and len(authority_at) >= 2 and len(handoff_at) >= 2, (
        readiness_at,
        authority_at,
        handoff_at,
    )
    for ready, held, forwarded in zip(readiness_at[:2], authority_at[:2], handoff_at[:2]):
        assert ready < held < forwarded, (ready, held, forwarded)
    low32 = {
        label: int(count)
        for label, count in re.findall(
            r"servicemgr: observed cap occupancy (apb1-authority-held|packaged-ready)=(\d+)",
            serial,
        )
    }
    assert low32["apb1-authority-held"] == low32["packaged-ready"], low32

    final_tree = tree(disk)
    prefix = "/System/Applications/com.arena.editor/42/"
    expected = {prefix + path.decode(): bundle[parsed.payload_offset + offset:
               parsed.payload_offset + offset + size]
                for (kind, path, size, _digest), offset in zip(
                    parsed.files,
                    _payload_offsets(parsed.files),
                )}
    for path, data in expected.items():
        assert final_tree.get(path) == data, (path, final_tree.get(path), data)
    assert final_tree[prefix + "APB1.record"] == parsed.metadata + parsed.signature
    assert final_tree["/Users/user/Desktop/" + DESKTOP_SOURCE.decode()] == bundle
    assert "/System/.apb1-staging" in final_tree
    assert not any(path.startswith("/System/.apb1-staging/") for path in final_tree)
    assert disk.read_bytes()[:arena_env.SCRATCH_MIB * 1024 * 1024] == afs1_before
    assert not afs1.audit(disk)
    print(
        f"[{LABEL}] APB1 installed from the real Desktop icon affordance; host verified signed record and "
        f"{len(expected)} readback payload files at /System/Applications; source and AFS1 "
        f"preserved; packaged restart re-established slot127 authority and repeated denial "
        f"controls; clean shutdown in {elapsed:.1f}s PASS",
        flush=True,
    )


def _payload_offsets(files):
    offset = 0
    result = []
    for _kind, _path, size, _digest in files:
        result.append(offset)
        offset += size
    return result


if __name__ == "__main__":
    main()
