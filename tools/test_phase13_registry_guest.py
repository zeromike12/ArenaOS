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
HEADLESS_SOURCE = b"zz-headless.apb1"
APP_ID = b"org.arenaos.phase13app"
HEADLESS_APP_ID = b"org.arenaos.zzheadless"
STREAMER_ID = b"org.arenaos.phase13streamer"
SLEEPER_ID = b"org.arenaos.phase13sleeper"
CRASHER_ID = b"org.arenaos.phase13crasher"
ORPHAN_ID = b"org.arenaos.phase13orphan"
SERIAL_APPS = "[phase13-installed-app] ABI-v2 startup verified; real window published"
SERIAL_MULTIWINDOW = "[phase13-multiwindow] one process owns three separately backed ordinary windows"
SERIAL_WINDOW_RETIRED = "[phase13-window] DestroyWindow retired one surface; process and siblings remain live"
SERIAL_WINDOW_FINAL = "[phase13-window] final surface retired; process exits cleanly"
SERIAL_VM = "[phase13-vm] guarded reserve, lazy commit, RW/RO/RX protection, W^X refusal, exact release/accounting passed"
SERIAL_HEAP = "[phase13-heap] lazy 16 MiB VM heap, 256 KiB Vec, 64-page commit batches, reuse, 64 KiB alignment, fallible OOM passed"
SERIAL_STREAM_FULL = "[phase13-stream] output full; extra write returned WouldBlock"
SERIAL_STREAM_STDOUT = "[phase13-stream] stdout partial transfer reached the broker"
SERIAL_STREAM_STDERR = "[phase13-stream] stderr channel reached the broker"
SERIAL_STREAM_STDIN = "[phase13-stream] stdin received exact native keyboard bytes"
SERIAL_STREAM_STDOUT_EOF = "[desktop] native stream channel stdout reached EOF"
SERIAL_STREAM_STDERR_EOF = "[desktop] native stream channel stderr reached EOF"
SERIAL_HELPER_STREAM = "[phase13-helper-stream] owner woke child; exact stdin/stdout bytes transferred; EOF after reap"
SERIAL_HELPER_CRASH_EOF = "[phase13-helper-stream] crashed child's output reached EOF after exact Process-cap reap"
SERIAL_HELPER_STREAM_POLICY = "[phase13-helper-stream] signed stream policy and explicit launch request must match"
SERIAL_HELPER_STREAM_CHILD = "[phase13-helper-stream] child transferred stdin and stdout bytes over its exact stream set"
SERIAL_HEADLESS = "[phase13-headless] Startup ABI v2 verified one attenuated Notification; no window caps present"
SERIAL_HEADLESS_EXIT = "[desktop] child Process-cap exit status=42"
SERIAL_HELPER_EXIT = (
    "[desktop] helper id=org.arenaos.phase13streamer Process-cap exit status=46; owner-group reap=ok"
)
SERIAL_CRASHER_EXIT = (
    "[desktop] helper id=org.arenaos.phase13crasher Process-cap exit status=262; owner-group reap=ok"
)
COUNTERS = re.compile(
    r"measured frames/records/processes/regions/pages/maps/caps="
    r"(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n"
)


def pixel(image, x, y):
    offset = (y * 800 + x) * 3
    return tuple(image[offset:offset + 3])


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
    helper_crate = root / "userspace/phase13-headless"
    subprocess.run(
        ["cargo", "build", "--release"],
        cwd=helper_crate,
        env=arena_env.rust_env(),
        check=True,
    )
    helper = helper_crate / "target/x86_64-unknown-none/release/arena-phase13-headless"
    assert helper.is_file() and helper.stat().st_size < 256 * 1024
    helper_specs = [
        (STREAMER_ID, b"bin/streamer", 7),
        (SLEEPER_ID, b"bin/sleeper", 3),
        (CRASHER_ID, b"bin/crasher", 7),
        (ORPHAN_ID, b"bin/orphan", 1),
    ]
    helper_list = bytearray(8 + 88 * len(helper_specs))
    helper_list[:4] = b"AHL1"
    helper_list[4] = 1
    helper_list[5] = len(helper_specs)
    for index, (helper_id, helper_path, flags) in enumerate(helper_specs):
        start = 8 + index * 88
        helper_list[start:start + len(helper_id)] = helper_id
        helper_list[start + 32:start + 32 + len(helper_path)] = helper_path
        helper_list[start + 80:start + 84] = flags.to_bytes(4, "little")
    manifest = apb1_format.make_manifest(
        app_id=APP_ID,
        package_id=b"org.arena.editor",
        display_name=b"Phase13 Probe",
        version=13,
        flags=9,
        requested=0,
        entry=b"bin/probe",
        icon=b"",
        width=320,
        height=180,
        associations=(b"text/plain",),
    )
    files = [(1, b"bin/probe", executable.read_bytes())]
    files.extend((1, path, helper.read_bytes()) for _, path, _ in helper_specs)
    files.append((2, b"META-INF/arena.helpers", bytes(helper_list)))
    files.sort(key=lambda record: record[1])
    return apb1_format.build_bundle(files, manifest=manifest)


def headless_bundle():
    root = Path(__file__).resolve().parents[1]
    crate = root / "userspace/phase13-headless"
    subprocess.run(
        ["cargo", "build", "--release"],
        cwd=crate,
        env=arena_env.rust_env(),
        check=True,
    )
    executable = crate / "target/x86_64-unknown-none/release/arena-phase13-headless"
    assert executable.is_file() and executable.stat().st_size < 256 * 1024
    manifest = apb1_format.make_manifest(
        app_id=HEADLESS_APP_ID,
        package_id=b"org.arena.editor",
        display_name=b"Runtime Probe",
        version=13,
        flags=4,
        requested=0,
        entry=b"bin/headless",
        icon=b"",
        width=0,
        height=0,
        associations=(),
    )
    return apb1_format.build_bundle(
        [(1, b"bin/headless", executable.read_bytes())], manifest=manifest
    )


def seed_bundle(disk, bundle, headless):
    volume = afs2.Volume(disk.read_bytes()[BASE:])
    desktop = volume.resolve("/Users/user/Desktop")
    source = volume.create(desktop, SOURCE, 1)
    volume.write(source, 0, bundle, 1)
    headless_source = volume.create(desktop, HEADLESS_SOURCE, 1)
    volume.write(headless_source, 0, headless, 1)
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


def wait_stream_proof(d, expected):
    d.wait(
        lambda: all(
            d.serial().count(marker) == expected
            for marker in (
                SERIAL_STREAM_FULL,
                SERIAL_STREAM_STDOUT,
                SERIAL_STREAM_STDERR,
                SERIAL_STREAM_STDOUT_EOF,
                SERIAL_STREAM_STDERR_EOF,
            )
        ),
        f"native stream proof count did not reach {expected}",
    )


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
        wait_stream_proof(d, 1)
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
        wait_stream_proof(d, 2)
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
        before_search = d.shot("all-apps-before-search")
        d.click(94, 12)
        empty_query = d.shot(
            "all-apps-empty-query",
            lambda p: min(pixel(p, 600, 180)) > 240
            and pixel(p, 600, 180) != pixel(before_search, 600, 180),
        )
        d.q.type_text("phase13", gap_s=0.04)
        searched = d.shot(
            "all-apps-search",
            lambda p: sum(
                1
                for i in range(0, len(crop(p, 186, 232, 60, 20)), 3)
                if max(crop(p, 186, 232, 60, 20)[i:i + 3]) < 180
            ) == 0,
        )
        assert pixel(searched, 600, 180) == pixel(empty_query, 600, 180), \
            "All Applications search surface disappeared while filtering"
        # The filtered catalog has one row; this click is a pointer launch.
        d.click(250, 220)
        try:
            d.wait(lambda: d.serial().count(SERIAL_APPS) == 2,
                   "search result pointer launch did not create the second app process", timeout_s=30)
        except AssertionError:
            registers = d.q.command(
                "human-monitor-command", **{"command-line": "info registers"}
            )
            threads = d.q.command(
                "human-monitor-command", **{"command-line": "info cpus"}
            )
            (arena_env.build_dir() / "phase13-registry-qmp-failure.txt").write_text(
                f"registers={registers}\nthreads={threads}\nserial-tail={d.serial()[-8000:]}\n"
            )
            raise
        d.wait(lambda: d.serial().count(SERIAL_VM) == 3,
               "pointer launch did not pass its ring-3 VM mechanism proof")
        d.wait(lambda: d.serial().count(SERIAL_HEAP) == 3,
               "pointer launch did not pass its scalable heap proof")
        wait_stream_proof(d, 3)
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

        # The focused installed app waits on its private stream notification.
        # Keyboard events become bytes only through the Desktop's exact stdin
        # writer for that AppInstance.
        d.q.type_text("native-stream", gap_s=0.025)
        send_key(d, "ret")
        d.wait(lambda: d.serial().count(SERIAL_STREAM_STDIN) == 1,
               "native stdin did not receive the exact keyboard byte sequence")

        # Window publication and the Desktop's measurement line are separate
        # event-loop observations. Wait for the complete six-window resource
        # inventory before taking the baseline for an individual close.
        both_windows_expected = list(baseline)
        both_windows_expected[1] += 2    # Process records
        both_windows_expected[2] += 2    # live processes
        both_windows_expected[3] += 14   # six windows plus two stream regions
        both_windows_expected[4] += 5642 # six bounded windows plus two stream pages
        both_windows_expected[5] += 22   # 18 window maps plus four stream mappings
        both_windows_expected[6] += 6    # Process, file-lineage, and stream cap per instance
        d.wait(lambda: resource_counts(d)[1:] == tuple(both_windows_expected[1:]),
               "six ordinary windows did not reach their complete resource inventory")
        settled_inventory = resource_counts(d)
        time.sleep(0.2)
        assert resource_counts(d) == settled_inventory, \
            "six-window resource inventory changed after it was considered settled"

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
            try:
                d.wait(lambda: resource_counts(d)[2:] == tuple(expected[2:]),
                       "closing one window did not release exactly its owned resources")
            except AssertionError as error:
                raise AssertionError(
                    f"{error}; before={tuple(before)} expected={tuple(expected)} "
                    f"actual={resource_counts(d)}"
                ) from error
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

        before_headless_install = d.serial().count("[desktop] APB1 installed; signed version=")
        d.click(52, 206)
        time.sleep(0.1)
        d.click(52, 206)
        d.wait(
            lambda: d.serial().count("[desktop] APB1 installed; signed version=")
            == before_headless_install + 1,
            "Desktop did not complete the protected headless APB1 install",
        )
        installed = tree(disk)
        assert f"/System/Applications/{HEADLESS_APP_ID.decode()}/13/bin/headless" in installed

        # A headless registry record resolves to an exact verified Image and
        # a tracked Process, but must not add a compositor window or inherit
        # the Desktop session endpoint/surface/document capabilities.
        headless_before = resource_counts(d)
        retired_before_headless = d.serial().count("[desktop] application retired:")
        exit_status_before_headless = d.serial().count(SERIAL_HEADLESS_EXIT)
        background = d.shot("before-headless-launch")
        d.click(120, 12)
        d.q.type_text("runtime", gap_s=0.025)
        d.shot("all-apps-headless-search")
        send_key(d, "ret")
        d.wait(lambda: SERIAL_HEADLESS in d.serial(),
               "signed headless app did not validate its exact startup capability inventory")
        d.wait(lambda: resource_counts(d)[2] == headless_before[2] + 1,
               "headless launch did not retain one live exact Process capability")
        assert "ordinary windows=0; no surface or Desktop endpoint inherited" in d.serial(), \
            "headless launch allocated a surface/window or inherited its Desktop endpoint"
        no_window = d.shot("headless-running-no-window")
        assert crop(no_window, 100, 100, 600, 400) == crop(background, 100, 100, 600, 400), \
            "headless application created visible window content"
        d.wait(lambda: "[phase13-headless] timer completed; process exiting for manager reap" in d.serial(),
               "headless process did not finish its real timer wait")
        d.wait(lambda: d.serial().count(SERIAL_HEADLESS_EXIT) == exit_status_before_headless + 1,
               "Desktop did not observe the exact exit=42 through the Process cap")
        d.wait(lambda: d.serial().count("[desktop] application retired:") == retired_before_headless + 1,
               "Desktop did not reap the exited headless process")
        d.wait(lambda: resource_counts(d)[1:] == baseline[1:],
               "headless timer/process resources did not return to baseline")
        return b"shutdown\r"
    finally:
        d.dispose()


def main():
    bundle = app_bundle()
    headless = headless_bundle()
    for image in arena_env.build_dir().glob(f"{LABEL}-*.ppm"):
        image.unlink()
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
    seed_bundle(disk, bundle, headless)
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
    assert serial.count(SERIAL_HEADLESS) == 1
    assert serial.count("[phase13-headless] timer completed; process exiting for manager reap") == 1
    assert serial.count("[phase13-helper] unknown signed helper ID refused without spawn") == 3
    assert serial.count(SERIAL_HELPER_STREAM_POLICY) == 3
    assert serial.count(SERIAL_HELPER_STREAM_CHILD) == 3
    # Fifteen helper launches exceed the 13 dynamic notification slots left
    # after the fixed 32-session boot inventory; each wait/reap must retire its
    # private timer object before the next launch.
    assert serial.count("[phase13-helper] no inherited notification factory; WRITE-only owner signal cannot wait") == 15
    assert serial.count("[phase13-helper] exact Startup ABI inventory: private timer Notification") == 15
    assert serial.count("[phase13-helper] readiness badge sent through separate WRITE-only owner signal") == 3
    assert serial.count(SERIAL_HELPER_STREAM) == 3
    assert serial.count(SERIAL_HELPER_CRASH_EOF) == 6
    assert serial.count("[phase13-helper] owner-authorized terminate/reap passed") == 3
    assert serial.count("[phase13-helper] crashed helper status=262 observed and reaped") == 3
    assert serial.count("[phase13-helper] live helper left for AppInstance owner cleanup") == 3
    assert serial.count(SERIAL_HELPER_EXIT) == 3
    assert serial.count(SERIAL_CRASHER_EXIT) == 6
    assert serial.count("[phase13-helper] retired private timer object reclaimed after reap") == 3
    assert serial.count("[desktop] AppInstance ProcessGroup teardown members=2; final members=0") == 3
    # The read-only Open With launch, both ordinary instances, and the
    # headless app each exit with the fixture's exact status 42.
    assert serial.count(SERIAL_HEADLESS_EXIT) == 4
    assert "[phase13-app] exact read-only document capability verified" in serial
    assert "[desktop] installed application registry unavailable" not in serial
    assert "[desktop] application retired:" in serial
    assert "[app] abnormal exit stage" not in serial
    assert not afs1.audit(disk)
    print(
        f"[{LABEL}] verified APB1 install -> registry -> keyboard/search/pointer launch of the signed native ELF; "
        f"two instances with three windows each, individual DestroyWindow while siblings stay live, "
        f"one cap-attenuated headless app with no windows, exact resource teardown to baseline; "
        f"clean shutdown in {elapsed:.1f}s PASS",
        flush=True,
    )


if __name__ == "__main__":
    main()
