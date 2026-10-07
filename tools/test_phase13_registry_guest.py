#!/usr/bin/env python3
"""Real installed APB1 discovery, launch, UI and teardown proof."""
from pathlib import Path
import re
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1
import afs2
import apb1_format
import arena_env
import mtest
from test_m10_apps import Desktop, crop

LABEL = "phase13-registry"
BASE = arena_env.AFS2_BASE_SECTOR * 512
SOURCE = b"phase13.apb1"
APP_ID = b"org.arenaos.phase13app"
SERIAL_APPS = "[phase13-installed-app] ABI-v2 startup verified; real window published"
COUNTERS = re.compile(
    r"measured frames/records/processes/regions/pages/maps/caps="
    r"(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n"
)


def tree(disk):
    for _ in range(100):
        try:
            volume = afs2.Volume(disk.read_bytes()[BASE:])
            afs2.check(volume)
            return afs2.walk(volume)
        except Exception:
            time.sleep(0.1)
    raise AssertionError("AFS2 volume did not become host-readable")


def app_bundle():
    root = Path(__file__).resolve().parents[1]
    crate = root / "userspace/phase13-app"
    subprocess.run(
        ["cargo", "build", "--release"],
        cwd=crate,
        env=arena_env.rust_env(),
        check=True,
    )
    executable = crate / "target/x86_64-unknown-none/release/arena-phase13-installed-app"
    assert executable.is_file() and executable.stat().st_size < 256 * 1024
    manifest = apb1_format.make_manifest(
        app_id=APP_ID,
        package_id=b"org.arena.editor",
        display_name=b"Phase13 Probe",
        version=13,
        flags=1,
        requested=0,
        entry=b"bin/probe",
        icon=b"",
        width=320,
        height=180,
        associations=(b"text/plain",),
    )
    return apb1_format.build_bundle([(1, b"bin/probe", executable.read_bytes())], manifest=manifest)


def seed_bundle(disk, bundle):
    volume = afs2.Volume(disk.read_bytes()[BASE:])
    desktop = volume.resolve("/Users/user/Desktop")
    source = volume.create(desktop, SOURCE, 1)
    volume.write(source, 0, bundle, 1)
    with disk.open("r+b") as f:
        f.seek(BASE)
        f.write(volume.image())


def send_key(d, qcode):
    d.q.command("input-send-event", events=[d.q._ev(qcode, True), d.q._ev(qcode, False)])


def checkerboard_pixels(ppm, x, y):
    data = crop(ppm, x, y, 300, 160)
    colors = {data[i:i + 3] for i in range(0, len(data), 3)}
    return len(colors) >= 2


def interaction(disk):
    d = Desktop(LABEL)
    try:
        baseline_rows = COUNTERS.findall(d.serial())
        assert baseline_rows, "native resource baseline is missing"
        baseline = tuple(map(int, baseline_rows[-1]))

        before_install = d.serial().count("[desktop] APB1 installed; signed version=")
        d.click(52, 58)
        time.sleep(0.1)
        d.click(52, 58)
        d.wait(
            lambda: d.serial().count("[desktop] APB1 installed; signed version=") == before_install + 1,
            "Desktop did not complete its protected APB1 install",
        )
        installed = tree(disk)
        assert f"/System/Applications/{APP_ID.decode()}/13/bin/probe" in installed
        assert installed[f"/Users/user/Desktop/{SOURCE.decode()}"]

        # The real All Applications bar control opens the receiver-verified
        # catalog. Seven Down events select the seventh row (six built-ins,
        # then the installed package); Enter requests a fresh Image cap.
        background = d.shot("desktop-before-all-apps")
        d.click(120, 12)
        opened = d.shot(
            "all-apps-open",
            lambda p: crop(p, 184, 120, 432, 356) != crop(background, 184, 120, 432, 356),
        )
        assert crop(opened, 205, 214, 190, 16) != bytes(190 * 16 * 3)
        navigation = opened
        for selected_row in range(1, 7):
            row_y = 208 + selected_row * 24
            before_row = crop(navigation, 186, row_y, 428, 23)
            send_key(d, "down")
            navigation = d.shot(
                f"all-apps-keyboard-row-{selected_row}",
                lambda p, row_y=row_y, before_row=before_row:
                    crop(p, 186, row_y, 428, 23) != before_row,
            )
        send_key(d, "ret")
        d.wait(lambda: d.serial().count(SERIAL_APPS) == 1, "All Applications keyboard launch did not run the installed ELF")
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[2] == baseline[2] + 1,
               "installed launch did not add one live application process")
        first = d.shot("installed-window-one", lambda p: checkerboard_pixels(p, 82, 130))
        assert checkerboard_pixels(first, 82, 130), "the installed ELF did not publish its owned checkerboard surface"

        # Search filters the same verified registry; a pointer selection
        # launches the multi-instance app a second time.
        d.click(120, 12)
        empty_query = d.shot("all-apps-empty-query")
        d.q.type_text("phase13", gap_s=0.025)
        searched = d.shot("all-apps-search", lambda p: crop(p, 194, 150, 400, 28) != crop(empty_query, 194, 150, 400, 28))
        assert crop(searched, 194, 150, 400, 28) != crop(empty_query, 194, 150, 400, 28)
        # The filtered catalog has one row; this click is a pointer launch.
        d.click(250, 220)
        d.wait(lambda: d.serial().count(SERIAL_APPS) == 2, "search result pointer launch did not create the second app process")
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[2] == baseline[2] + 2,
               "second installed app launch did not add its process")
        both = d.shot("installed-window-two", lambda p: checkerboard_pixels(p, 108, 154))
        assert checkerboard_pixels(both, 82, 130) and checkerboard_pixels(both, 108, 154), \
            "two separate application instances did not own visible windows"

        d.click(378, 70)
        d.wait(lambda: d.serial().count("[desktop] application retired:") >= 1,
               "closing the first installed window did not reap its ProcessGroup")
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[2] == baseline[2] + 1,
               "first close did not reduce process resources by one")
        d.click(404, 94)
        d.wait(lambda: d.serial().count("[desktop] application retired:") >= 2,
               "closing the second installed window did not reap its ProcessGroup")
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[1:] == baseline[1:],
               "installed app close-all did not return identity resources to baseline")
        d.shot("installed-windows-closed")
        return b"shutdown\r"
    finally:
        d.dispose()


def main():
    bundle = app_bundle()
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk()
    rc, serial, _ = mtest.boot(
        LABEL + "-seed", esp, [(b"arena>", 1, b"shutdown\r")], disk, pointer=True
    )
    assert rc == 0 and "no AFS2 region" in serial
    with disk.open("r+b") as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    assert not afs1.audit(disk)
    rc, serial, _ = mtest.boot(
        LABEL + "-afs2", esp,
        [(b"filesd: AFS2 mounted", 1, b"shutdown\r")], disk, pointer=True
    )
    assert rc == 0 and "filesd: AFS2 mounted" in serial
    seed_bundle(disk, bundle)
    assert tree(disk)[f"/Users/user/Desktop/{SOURCE.decode()}"] == bundle
    feed = [
        ((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"), 1,
         lambda: interaction(disk)),
    ]
    rc, serial, elapsed = mtest.boot(LABEL, esp, feed, disk, pointer=True, timeout_s=180)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    assert rc == 0, serial[-5000:]
    assert serial.count(SERIAL_APPS) == 2
    assert "[desktop] installed application registry unavailable" not in serial
    assert "[desktop] application retired:" in serial
    assert "[app] abnormal exit stage" not in serial
    assert not afs1.audit(disk)
    print(
        f"[{LABEL}] verified APB1 install -> registry -> keyboard/search/pointer launch of the signed native ELF; "
        f"two owned windows/processes, exact ProcessGroup teardown to baseline; clean shutdown in {elapsed:.1f}s PASS",
        flush=True,
    )


if __name__ == "__main__":
    main()
