#!/usr/bin/env python3
"""Signed APB1 installation and repeated native PIE launch proof."""
from __future__ import annotations

import re
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

LABEL = "phase14-pie"
BASE = arena_env.AFS2_BASE_SECTOR * 512
APP_ID = b"org.arenaos.phase14pie"
SOURCE = b"phase14-pie.apb1"
PIE_PASS = re.compile(
    r"\[phase14-pie\] PASS base=0x([0-9a-f]+) entry=0x([0-9a-f]+) "
    r"relocated=0x([0-9a-f]+) data=0x([0-9a-f]+) bss=0x([0-9a-f]+) exit=42"
)
KERNEL_PLACE = re.compile(
    r"\[arena INFO  aslr\] PIE placement image=\d+ pid=\d+ "
    r"bias=0x([0-9a-f]+) base=0x([0-9a-f]+) entry=0x([0-9a-f]+) "
    r"candidates=(\d+); RX/R/RW finalized, W\^X and lower/upper/stack guards checked"
)
COUNTERS = re.compile(
    r"measured frames/records/processes/regions/pages/maps/caps="
    r"(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n"
)
RESOURCE_DETAIL = re.compile(
    r"native resource detail values=" + r"(\d+(?:/\d+){14})\r?\n"
)


def tree(disk: Path) -> dict[str, bytes]:
    for _ in range(100):
        try:
            volume = afs2.Volume(disk.read_bytes()[BASE:])
            afs2.check(volume)
            return afs2.walk(volume)
        except Exception:
            time.sleep(0.1)
    raise AssertionError("AFS2 volume did not become host-readable")


def bundle_bytes() -> bytes:
    path = Path(__file__).resolve().parents[1] / "userspace/phase14-pie/phase14-pie.apb1"
    bundle = path.read_bytes()
    parsed = apb1_format.parse_bundle(bundle)
    fixture = path.with_name("fixture.elf").read_bytes()
    assert parsed.files[0][1] == b"bin/pie" and parsed.payload == fixture
    return bundle


def seed_bundle(disk: Path, bundle: bytes) -> None:
    volume = afs2.Volume(disk.read_bytes()[BASE:])
    desktop = volume.resolve("/Users/user/Desktop")
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


def launch_from_all_apps(
    desktop: Desktop,
    ordinal: int,
    expected_passes: int,
    expected_receipt: tuple[int, ...],
    expected_detail: tuple[int, ...],
) -> tuple[int, ...]:
    # The magnifier enters the verified catalog's search mode; the separate
    # All Applications control opens the unfiltered list without focusing it.
    desktop.click(94, 12)
    desktop.q.type_text("Phase14 PIE", gap_s=0.035)
    desktop.shot(f"all-apps-pie-{ordinal}")
    counters_before = len(COUNTERS.findall(desktop.serial()))
    send_key(desktop, "ret")
    desktop.wait(
        lambda: len(PIE_PASS.findall(desktop.serial())) == expected_passes,
        f"installed PIE launch {ordinal} did not pass startup/relocation checks",
        timeout_s=60,
    )
    desktop.wait(
        lambda: desktop.serial().count("[desktop] child Process-cap exit status=42")
        >= expected_passes,
        f"installed PIE launch {ordinal} did not exit with status 42",
        timeout_s=60,
    )
    desktop.wait(
        lambda: len(COUNTERS.findall(desktop.serial())) >= counters_before + 2
        and tuple(map(int, COUNTERS.findall(desktop.serial())[-1])) == expected_receipt
        and tuple(map(int, RESOURCE_DETAIL.findall(desktop.serial())[-1][0].split("/")))
        == expected_detail,
        f"launch {ordinal} did not restore its exact steady-state resource receipt",
        timeout_s=60,
    )
    return expected_receipt


def interaction(disk: Path, bundle: bytes) -> bytes:
    desktop = Desktop(LABEL)
    try:
        baseline_rows = COUNTERS.findall(desktop.serial())
        assert baseline_rows, "native resource baseline is missing"
        boot_baseline = tuple(map(int, baseline_rows[-1]))
        detail_rows = RESOURCE_DETAIL.findall(desktop.serial())
        assert detail_rows, "native resource detail baseline is missing"
        boot_detail = tuple(map(int, detail_rows[-1][0].split("/")))
        before = desktop.serial().count("[desktop] APB1 installed; signed version=14")
        desktop.click(52, 58)
        time.sleep(0.1)
        desktop.click(52, 58)
        desktop.wait(
            lambda: desktop.serial().count("[desktop] APB1 installed; signed version=14")
            == before + 1,
            "Desktop did not install the signed Phase-14 APB1 application",
            timeout_s=60,
        )
        installed = tree(disk)
        root = f"/System/Applications/{APP_ID.decode()}/14/"
        assert installed[root + "bin/pie"] == Path(
            __file__
        ).resolve().parents[1].joinpath("userspace/phase14-pie/fixture.elf").read_bytes()
        parsed = apb1_format.parse_bundle(bundle)
        assert installed[root + "APB1.record"] == parsed.metadata + parsed.signature
        assert installed[f"/Users/user/Desktop/{SOURCE.decode()}"] == bundle
        assert not any(path.startswith("/System/.apb1-staging/") for path in installed)

        # The first Desktop SharedRegion startup-page map can allocate three
        # empty parent page-table frames in its long-lived address space.
        # SYS_SHARED_UNMAP deliberately retains those tables for reuse; the
        # mapping, region, Image and process still retire exactly. Subsequent
        # launches must return to the now-warmed steady state.
        warmed_baseline = list(boot_baseline)
        assert warmed_baseline[0] >= 3
        warmed_baseline[0] -= 3
        warmed_baseline = tuple(warmed_baseline)
        steady = launch_from_all_apps(desktop, 1, 1, warmed_baseline, boot_detail)
        assert steady == warmed_baseline
        launch_from_all_apps(desktop, 2, 2, steady, boot_detail)

        app_runs = PIE_PASS.findall(desktop.serial())
        placements = KERNEL_PLACE.findall(desktop.serial())
        assert len(app_runs) == len(placements) == 2, (app_runs, placements)
        for app, kernel in zip(app_runs, placements):
            base, entry = int(app[0], 16), int(app[1], 16)
            bias, kernel_base, kernel_entry, candidate_count = kernel
            bias = int(bias, 16)
            assert base == int(kernel_base, 16) == bias
            assert entry == int(kernel_entry, 16)
            assert 0x400000000000 <= bias < 0x600000000000
            assert bias % (2 * 1024 * 1024) == 0
            assert int(candidate_count) >= (1 << 23)
        assert app_runs[0][0] != app_runs[1][0], app_runs
        return b"shutdown\r"
    finally:
        desktop.dispose()


def main() -> None:
    bundle = bundle_bytes()
    parsed = apb1_format.parse_bundle(bundle)
    assert parsed.files[0][1] == b"bin/pie"
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk(afs2=True)
    rc, serial, _ = mtest.boot(
        LABEL + "-afs2-seed",
        esp,
        [(b"filesd: AFS2 mounted", 1, b"shutdown\r")],
        disk,
        pointer=True,
        timeout_s=180,
    )
    assert rc == 0 and "filesd: AFS2 mounted" in serial, serial[-3000:]
    seed_bundle(disk, bundle)
    assert tree(disk)[f"/Users/user/Desktop/{SOURCE.decode()}"] == bundle
    rc, serial, elapsed = mtest.boot(
        LABEL,
        esp,
        [
            (
                (b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"),
                1,
                lambda: interaction(disk, bundle),
            )
        ],
        disk,
        pointer=True,
        timeout_s=180,
    )
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    assert rc == 0, serial[-5000:]
    assert "[phase14-pie] PASS" in serial
    assert "rngd: submitted a device-filled 256-bit seed to the kernel CSPRNG" in serial
    assert serial.count("[phase14-pie] PASS") == 2
    assert not afs1.audit(disk)
    print(
        f"[{LABEL}] protected signed APB1 installed and discovered; two native PIE launches "
        f"passed with distinct kernel-selected bases and exact process/Image teardown "
        f"in {elapsed:.1f}s PASS",
        flush=True,
    )


if __name__ == "__main__":
    main()
