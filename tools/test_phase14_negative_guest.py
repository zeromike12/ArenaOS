#!/usr/bin/env python3
"""Signed APB1 negative launch cases through Desktop and packaged."""
from __future__ import annotations

import re
import struct
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1
import afs2
import apb1_format
import arena_env
import mtest
from test_m10_apps import Desktop
from test_phase14_pie_guest import BASE, SOURCE, bundle_bytes, tree

LABEL = "phase14-pie-negative"
COUNTERS = re.compile(
    r"measured frames/records/processes/regions/pages/maps/caps="
    r"(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n"
)
DETAIL = re.compile(r"native resource detail values=(\d+(?:/\d+){14})\r?\n")
REFUSAL = re.compile(r"\[desktop\] launch-refusal stage/status=([^/\r\n]+)/(-?\d+)")
PIE_PASS = "[phase14-pie] PASS"
APP_KEYS = {
    "bad-magic": "badmagic",
    "unsupported-relocation": "reloc",
    "relocation-outside-image": "target",
    "relocation-span-overflow": "span",
    "invalid-alignment": "align",
    "writable-executable": "wx",
    "outside-placement-arena": "arena",
}


def fixture_bytes() -> bytes:
    return bytearray(
        (Path(__file__).resolve().parents[1] / "userspace/phase14-pie/fixture.elf").read_bytes()
    )


def program_headers(image: bytearray) -> list[tuple[int, int, int, int, int, int, int]]:
    phoff = struct.unpack_from("<Q", image, 32)[0]
    entsize, count = struct.unpack_from("<HH", image, 54)
    assert entsize == 56 and 0 < count <= 8
    result = []
    for index in range(count):
        at = phoff + index * entsize
        ptype, flags, offset, vaddr, _paddr, filesz, memsz, align = struct.unpack_from(
            "<IIQQQQQQ", image, at
        )
        result.append((at, ptype, flags, offset, vaddr, filesz, memsz, align))
    return result


def rela_file_offset(image: bytearray) -> tuple[int, int]:
    dynamic = next(ph for ph in program_headers(image) if ph[1] == 2)
    dynamic_offset, dynamic_size = dynamic[3], dynamic[5]
    rela_vaddr = None
    for at in range(dynamic_offset, dynamic_offset + dynamic_size, 16):
        tag, value = struct.unpack_from("<qQ", image, at)
        if tag == 7:
            rela_vaddr = value
        if tag == 0:
            break
    assert rela_vaddr is not None
    for _at, ptype, _flags, offset, vaddr, filesz, _memsz, _align in program_headers(image):
        if ptype == 1 and vaddr <= rela_vaddr < vaddr + filesz:
            return offset + rela_vaddr - vaddr, dynamic_offset
    raise AssertionError("DT_RELA is not file-backed by PT_LOAD")


def mutate(name: str) -> bytearray:
    image = fixture_bytes()
    headers = program_headers(image)
    if name == "bad-magic":
        image[0] = 0
    elif name == "unsupported-relocation":
        rela, _dynamic = rela_file_offset(image)
        struct.pack_into("<Q", image, rela + 8, 9)  # symbol 0, type 9
    elif name == "relocation-outside-image":
        rela, _dynamic = rela_file_offset(image)
        struct.pack_into("<Q", image, rela, 0x20000)
    elif name == "relocation-span-overflow":
        rela, _dynamic = rela_file_offset(image)
        struct.pack_into("<Q", image, rela, 0xFFFFFFFFFFFFFFF8)
    elif name == "invalid-alignment":
        load_at = next(ph[0] for ph in headers if ph[1] == 1)
        struct.pack_into("<Q", image, load_at + 48, 3)
    elif name == "writable-executable":
        load_at = next(ph[0] for ph in headers if ph[1] == 1)
        struct.pack_into("<I", image, load_at + 4, 7)
    elif name == "outside-placement-arena":
        # Keep a structurally valid ET_DYN below the canonical user-half,
        # but move its link-time image above the reserved PIE arena. The
        # signed install and Image validation succeed; kernel placement
        # must refuse it with STATUS_NO_SPACE.
        shift = 0x700000000000  # 112 TiB; below the 128 TiB user-half limit.
        rela, dynamic_offset = rela_file_offset(image)
        struct.pack_into("<Q", image, 24, struct.unpack_from("<Q", image, 24)[0] + shift)
        for at, ptype, _flags, _offset, vaddr, _filesz, _memsz, _align in headers:
            if ptype in (1, 2):
                struct.pack_into("<Q", image, at + 16, vaddr + shift)
        dynamic = next(ph for ph in headers if ph[1] == 2)
        for at in range(dynamic_offset, dynamic_offset + dynamic[5], 16):
            tag, value = struct.unpack_from("<qQ", image, at)
            if tag in (4, 5, 6, 7):  # DT_HASH/STRTAB/SYMTAB/RELA pointers
                struct.pack_into("<Q", image, at + 8, value + shift)
            if tag == 0:
                break
        # `rela` is the file location calculated before moving virtual addresses.
        for at in range(rela, rela + 30 * 24, 24):
            target = struct.unpack_from("<Q", image, at)[0]
            addend = struct.unpack_from("<q", image, at + 16)[0]
            struct.pack_into("<Q", image, at, target + shift)
            struct.pack_into("<q", image, at + 16, addend + shift)
    else:
        raise AssertionError(f"unknown mutation {name}")
    return image


def make_signed_bundle(name: str, image: bytes) -> tuple[bytes, bytes, str]:
    app_id = f"org.arenaos.p14.{APP_KEYS[name]}".encode()
    display = f"P14 {name.replace('-', ' ')}".encode()
    assert len(app_id) <= 32 and len(display) <= 32
    manifest = apb1_format.make_manifest(
        app_id=app_id,
        package_id=b"org.arena.editor",
        display_name=display,
        version=14,
        flags=1 | 4,  # MULTI_INSTANCE | HEADLESS
        requested=0,
        entry=b"bin/pie",
        icon=b"",
        width=0,
        height=0,
        associations=(),
    )
    return (
        apb1_format.build_bundle([(1, b"bin/pie", image)], manifest=manifest),
        app_id,
        display.decode(),
    )


def seed_source(disk: Path, bundle: bytes) -> None:
    volume = afs2.Volume(disk.read_bytes()[BASE:])
    desktop = volume.resolve("/Users/user/Desktop")
    try:
        volume.unlink(desktop, SOURCE, 1)
    except afs2.FsError as error:
        if str(error) != "ENOENT":
            raise
    source = volume.create(desktop, SOURCE, 1)
    volume.write(source, 0, bundle, 1)
    with disk.open("r+b") as stream:
        stream.seek(BASE)
        stream.write(volume.image())


def send_key(desktop: Desktop, qcode: str) -> None:
    desktop.q.command(
        "input-send-event",
        events=[desktop.q._ev(qcode, True), desktop.q._ev(qcode, False)],
    )


def install_and_refuse(
    label: str,
    disk: Path,
    bundle: bytes,
    app_id: bytes,
    display_name: str,
    expected_stage: str,
    expected_status: int,
    outside_arena: bool,
    required_marker: str | None = None,
) -> bytes:
    desktop = Desktop(label)
    try:
        before_installs = desktop.serial().count("[desktop] APB1 installed; signed version=14")
        desktop.click(52, 58)
        time.sleep(0.1)
        desktop.click(52, 58)
        desktop.wait(
            lambda: desktop.serial().count("[desktop] APB1 installed; signed version=14")
            == before_installs + 1,
            f"signed malformed APB1 {display_name} was not installed",
            timeout_s=60,
        )
        installed = tree(disk)
        root = f"/System/Applications/{app_id.decode()}/14/"
        parsed = apb1_format.parse_bundle(bundle)
        assert installed[root + "bin/pie"] == parsed.payload
        assert installed[root + "APB1.record"] == parsed.metadata + parsed.signature
        assert installed[f"/Users/user/Desktop/{SOURCE.decode()}"] == bundle
        assert not any(path.startswith("/System/.apb1-staging/") for path in installed)

        # Establish the last pre-launch resource receipt after signed install.
        desktop.click(94, 12)
        desktop.q.type_text(display_name, gap_s=0.025)
        desktop.shot("negative-app-filter")
        before = desktop.serial()
        counters_before = len(COUNTERS.findall(before))
        send_key(desktop, "ret")
        expected_unsigned = expected_status & ((1 << 64) - 1)
        refusal_line = (
            f"[desktop] launch-refusal stage/status={expected_stage}/{expected_unsigned}"
        )
        desktop.wait(
            lambda: refusal_line in desktop.serial(),
            f"{display_name} did not refuse through {expected_stage}/{expected_status}",
            timeout_s=60,
        )
        desktop.wait(
            lambda: len(COUNTERS.findall(desktop.serial())) >= counters_before + 2,
            f"{display_name} did not emit settled rollback receipts",
            timeout_s=30,
        )
        after = desktop.serial()
        last_counts = tuple(map(int, COUNTERS.findall(after)[-1]))
        prior_counts = tuple(map(int, COUNTERS.findall(before)[-1]))
        # The broker prints resource counters at boot and on refusal, so the
        # intervening signed installation/UI work has no explicit snapshot.
        # Require every ownership counter to return exactly; bound the global
        # free-frame delta to the three empty Desktop page-table frames used
        # by the no-space Startup ABI map plus two AFS2/UI working frames.
        assert last_counts[1:] == prior_counts[1:], (display_name, prior_counts, last_counts)
        frame_delta = prior_counts[0] - last_counts[0]
        frame_limit = 5 if outside_arena else 2
        assert 0 <= frame_delta <= frame_limit, (display_name, prior_counts, last_counts)
        before_detail = tuple(map(int, DETAIL.findall(before)[-1].split("/")))
        after_detail = tuple(map(int, DETAIL.findall(after)[-1].split("/")))
        assert after_detail == before_detail, (display_name, before_detail, after_detail)
        assert PIE_PASS not in after
        assert len(REFUSAL.findall(after)) == 1
        if required_marker is not None:
            assert required_marker in after, (display_name, required_marker)
        if outside_arena:
            assert "[arena INFO  aslr] PIE placement" not in after
        return b"shutdown\r"
    finally:
        desktop.dispose()


def main() -> None:
    cases = [
        ("bad-magic", "installed-image-resolve", -2, False),
        ("unsupported-relocation", "installed-image-resolve", -2, False),
        ("relocation-outside-image", "installed-image-resolve", -2, False),
        ("relocation-span-overflow", "installed-image-resolve", -2, False),
        ("invalid-alignment", "installed-image-resolve", -2, False),
        ("writable-executable", "installed-image-resolve", -2, False),
        ("outside-placement-arena", "headless-kernel-spawn", -10, True),
    ]
    selected = set(sys.argv[1:])
    if selected:
        unknown = selected - {case[0] for case in cases}
        if unknown:
            raise SystemExit(f"unknown negative case(s): {sorted(unknown)}")
        cases = [case for case in cases if case[0] in selected]
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk(afs2=True)
    rc, serial, _elapsed = mtest.boot(
        LABEL + "-afs2-seed",
        esp,
        [(b"filesd: AFS2 mounted", 1, b"shutdown\r")],
        disk,
        pointer=True,
        timeout_s=180,
    )
    assert rc == 0 and "filesd: AFS2 mounted" in serial, serial[-3000:]

    for name, stage, status, outside_arena in cases:
        bundle, app_id, display = make_signed_bundle(name, bytes(mutate(name)))
        assert apb1_format.parse_bundle(bundle).payload == bytes(mutate(name))
        seed_source(disk, bundle)
        label = f"phase14-neg-{name}"
        rc, serial, elapsed = mtest.boot(
            label,
            esp,
            [
                (
                    (b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"),
                    1,
                    lambda label=label, bundle=bundle, app_id=app_id, display=display,
                    stage=stage, status=status, outside_arena=outside_arena:
                    install_and_refuse(
                        label,
                        disk,
                        bundle,
                        app_id,
                        display,
                        stage,
                        status,
                        outside_arena,
                    ),
                )
            ],
            disk,
            pointer=True,
            timeout_s=180,
        )
        (arena_env.build_dir() / f"serial-{label}.log").write_text(serial)
        assert rc == 0, serial[-5000:]
        assert "halting via UEFI ResetSystem(shutdown)" in serial
        print(
            f"[{label}] signed APB1 install retained exact bytes; production launch refused "
            f"at {stage}/{status}; ownership receipt restored and free-frame cache bounded "
            f"({elapsed:.1f}s) PASS",
            flush=True,
        )
    assert not afs1.audit(disk)
    print(f"[{LABEL}] seven signed malformed/unsupported PIE launch cases PASS", flush=True)


if __name__ == "__main__":
    main()
