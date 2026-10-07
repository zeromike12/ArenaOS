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
SERIAL_MULTIWINDOW = "[phase13-multiwindow] one process owns three separately backed ordinary windows"
SERIAL_WINDOW_RETIRED = "[phase13-window] DestroyWindow retired one surface; process and siblings remain live"
SERIAL_WINDOW_FINAL = "[phase13-window] final surface retired; process exits cleanly"
SERIAL_VM = "[phase13-vm] guarded reserve, lazy commit, RW/RO/RX protection, W^X refusal, exact release/accounting passed"
SERIAL_HEAP = "[phase13-heap] lazy 16 MiB VM heap, 256 KiB Vec, 64-page commit batches, reuse, 64 KiB alignment, fallible OOM passed"
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
    document = volume.create(desktop, b"z-associated.txt", 1)
    volume.write(document, 0, b"Phase 13 associated document\n", 1)
    with disk.open("r+b") as f:
        f.seek(BASE)
        f.write(volume.image())


def send_key(d, qcode):
    d.q.command("input-send-event", events=[d.q._ev(qcode, True), d.q._ev(qcode, False)])


def right_click(d, x, y):
    d.point(x, y)
    d.q.command("input-send-event", events=[{"type": "btn", "data": {"button": "right", "down": True}}])
    d.q.command("input-send-event", events=[{"type": "btn", "data": {"button": "right", "down": False}}])


def checkerboard_pixels(ppm, x, y):
    data = crop(ppm, x, y, 300, 160)
    colors = {data[i:i + 3] for i in range(0, len(data), 3)}
    return len(colors) >= 2


def pixel(ppm, x, y):
    offset = (y * 800 + x) * 3
    return ppm[offset:offset + 3]


def marker_set(ppm, points):
    expected = [
        bytes((0xE0, 0x40, 0x20)),
        bytes((0xF0, 0xD0, 0x20)),
        bytes((0x20, 0xD0, 0x80)),
    ]
    return [pixel(ppm, x, y) for x, y in points] == expected


def resource_counts(d):
    rows = COUNTERS.findall(d.serial())
    assert rows, "native resource receipt is missing"
    return tuple(map(int, rows[-1]))


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

        # The second Desktop icon is a text document. Open With is an explicit
        # File-cap offer; cancelling must create no process and grant no cap.
        desktop_before_menu = d.shot("desktop-before-open-with-menu")
        right_click(d, 52, 132)
        menu = d.shot(
            "desktop-document-menu",
            lambda p: crop(p, 100, 160, 120, 24)
            != crop(desktop_before_menu, 100, 160, 120, 24),
        )
        d.click(72, 172)
        chooser = d.shot(
            "open-with-cancel-surface",
            lambda p: crop(p, 224, 120, 392, 356)
            != crop(desktop_before_menu, 224, 120, 392, 356),
        )
        assert crop(chooser, 205, 130, 150, 18) != bytes(150 * 18 * 3)
        send_key(d, "esc")
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[1:] == baseline[1:],
               "cancelled Open With left process or capability authority behind")
        assert "[phase13-app] exact read-only document capability verified" not in d.serial()

        # Save a handler default without launching it, then explicitly choose
        # the installed package and prove it received only a read-only File cap.
        desktop_before_menu = d.shot("desktop-before-open-with-default")
        right_click(d, 52, 132)
        d.shot(
            "desktop-document-menu-default",
            lambda p: crop(p, 100, 160, 120, 24)
            != crop(desktop_before_menu, 100, 160, 120, 24),
        )
        d.click(72, 172)
        d.q.type_text("phase13", gap_s=0.025)
        before_default = tuple(map(int, COUNTERS.findall(d.serial())[-1]))
        send_key(d, "d")
        d.wait(lambda: "[desktop] AFS2 application handler default saved" in d.serial(),
               "handler default was not durably saved in AFS2")
        assert tuple(map(int, COUNTERS.findall(d.serial())[-1]))[1:] == before_default[1:], \
            "changing a handler default changed process or capability authority"
        association_file = tree(disk)["/Users/user/.arena-app-associations"]
        assert association_file[:4] == b"ASOC" and APP_ID in association_file, \
            "handler default was not persisted in the protected user AFS2 namespace"
        send_key(d, "ret")
        d.wait(lambda: "[phase13-app] exact read-only document capability verified" in d.serial(),
               "selected installed handler did not read the exact document read-only")
        d.wait(lambda: d.serial().count(SERIAL_VM) == 1,
               "installed document handler did not pass its ring-3 VM mechanism proof")
        d.wait(lambda: d.serial().count(SERIAL_HEAP) == 1,
               "installed document handler did not pass its scalable heap proof")
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[2] == baseline[2] + 1,
               "Open With did not spawn the selected installed application")
        assert tree(disk)[f"/Users/user/Desktop/z-associated.txt"] == b"Phase 13 associated document\n", \
            "read-only Open With modified the source document"
        d.click(378, 70)
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[1:] == baseline[1:],
               "read-only document application did not teardown to baseline")

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
        d.wait(lambda: d.serial().count(SERIAL_VM) == 2,
               "All Applications launch did not pass its ring-3 VM mechanism proof")
        d.wait(lambda: d.serial().count(SERIAL_HEAP) == 2,
               "All Applications launch did not pass its scalable heap proof")
        d.wait(lambda: d.serial().count(SERIAL_MULTIWINDOW) == 1,
               "installed app did not create three windows in its one process")
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[2] == baseline[2] + 1,
               "installed launch did not add one live application process")
        first_points = ((78, 100), (104, 124), (130, 148))
        first = d.shot(
            "installed-window-one",
            lambda p: checkerboard_pixels(p, 82, 130) and marker_set(p, first_points),
        )
        assert checkerboard_pixels(first, 82, 130), "the installed ELF did not publish its owned checkerboard surface"
        assert marker_set(first, first_points), \
            "one installed Process did not publish three distinct compositor-owned window surfaces"

        # Search filters the same verified registry; a pointer selection
        # launches the multi-instance app a second time.
        d.click(120, 12)
        empty_query = d.shot("all-apps-empty-query")
        d.q.type_text("phase13", gap_s=0.025)
        searched = d.shot("all-apps-search", lambda p: crop(p, 194, 150, 400, 28) != crop(empty_query, 194, 150, 400, 28))
        assert crop(searched, 194, 150, 400, 28) != crop(empty_query, 194, 150, 400, 28)
        # The filtered catalog has one row; this click is a pointer launch.
        d.click(250, 220)
        d.wait(lambda: d.serial().count(SERIAL_APPS) == 2,
               "search result pointer launch did not create the second app process", timeout_s=30)
        d.wait(lambda: d.serial().count(SERIAL_VM) == 3,
               "pointer launch did not pass its ring-3 VM mechanism proof")
        d.wait(lambda: d.serial().count(SERIAL_HEAP) == 3,
               "pointer launch did not pass its scalable heap proof")
        d.wait(lambda: d.serial().count(SERIAL_MULTIWINDOW) == 2,
               "second application instance did not create three windows in its one process", timeout_s=30)
        d.wait(lambda: tuple(map(int, COUNTERS.findall(d.serial())[-1]))[2] == baseline[2] + 2,
               "second installed app launch did not add its process")
        second_points = ((156, 172), (182, 196), (208, 220))
        both = d.shot(
            "installed-window-two",
            lambda p: checkerboard_pixels(p, 108, 154) and marker_set(p, second_points),
        )
        assert checkerboard_pixels(both, 82, 130) and checkerboard_pixels(both, 108, 154), \
            "two separate application instances did not own visible windows"
        assert marker_set(both, second_points), \
            "the second process did not publish its three independently backed surfaces"

        retired_apps = d.serial().count("[desktop] application retired:")

        def close_extra(x, y, retired_count):
            before = resource_counts(d)
            d.click(x, y)
            d.wait(lambda: d.serial().count(SERIAL_WINDOW_RETIRED) >= retired_count,
                   "DestroyWindow did not leave the process and sibling windows live")
            expected = list(before)
            expected[3] -= 2  # exact surface and private snapshot regions
            expected[4] -= 940  # their bounded shared pages
            expected[5] -= 3  # broker/client surface maps and broker snapshot map
            d.wait(lambda: resource_counts(d)[2:] == tuple(expected[2:]),
                   "closing one window did not release exactly its owned resources")
            return resource_counts(d)

        # Close only the first instance's top window. Its process and two
        # siblings must remain live, while the second instance is untouched.
        first_after_one = close_extra(430, 118, 1)
        assert first_after_one[2] == baseline[2] + 2
        first_remaining = ((78, 100), (104, 124))
        after_one = d.shot(
            "first-instance-one-window-closed",
            lambda p: [pixel(p, *point) for point in first_remaining]
            == [bytes((0xE0, 0x40, 0x20)), bytes((0xF0, 0xD0, 0x20))]
            and pixel(p, 130, 148) != bytes((0x20, 0xD0, 0x80))
            and marker_set(p, second_points),
        )
        assert marker_set(after_one, second_points)

        close_extra(402, 98, 2)
        d.click(376, 74)
        d.wait(lambda: d.serial().count("[desktop] application retired:") == retired_apps + 1,
               "closing the first instance's final window did not reap its process")
        d.wait(lambda: resource_counts(d)[2] == baseline[2] + 1,
               "first AppInstance teardown did not leave only the second process")

        close_extra(506, 194, 3)
        close_extra(480, 170, 4)
        d.click(454, 146)
        d.wait(lambda: d.serial().count("[desktop] application retired:") == retired_apps + 2,
               "closing the second instance's final window did not reap its process")
        d.wait(lambda: resource_counts(d)[1:] == baseline[1:],
               "independent window teardown did not return identity resources to baseline")
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
    assert serial.count(SERIAL_MULTIWINDOW) == 2
    assert serial.count(SERIAL_VM) == 3
    assert serial.count(SERIAL_HEAP) == 3
    assert "[phase13-app] exact read-only document capability verified" in serial
    assert "[desktop] installed application registry unavailable" not in serial
    assert "[desktop] application retired:" in serial
    assert "[app] abnormal exit stage" not in serial
    assert not afs1.audit(disk)
    print(
        f"[{LABEL}] verified APB1 install -> registry -> keyboard/search/pointer launch of the signed native ELF; "
        f"two instances with three windows each, individual DestroyWindow while siblings stay live, "
        f"exact resource teardown to baseline; clean shutdown in {elapsed:.1f}s PASS",
        flush=True,
    )


if __name__ == "__main__":
    main()
