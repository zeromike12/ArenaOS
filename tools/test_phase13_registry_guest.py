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
SERIAL_THREADS = "[phase13-threads] four concurrent ring-3 threads shared heap and read-only VM; distinct FS.base TLS, quota, exact stack caps, join/detach, and cleanup passed"
SERIAL_SYNC = "[phase13-sync] contended Mutex, multi-waiter Condvar wake-one/all, sequence-before-wait, Once contention, timeout, invalid-cap refusal, and key accounting passed"
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
SERIAL_HEADLESS = "[phase13-headless] Startup ABI v2 verified one attenuated Notification and one SyncDomain; no window caps present"
SERIAL_HEADLESS_EXIT = "[desktop] child Process-cap exit status=42"
SERIAL_HELPER_EXIT = (
    "[desktop] helper id=org.arenaos.phase13streamer Process-cap exit status=46; owner-group reap=ok"
)
SERIAL_HELPER_THREAD = "[phase13-sync] helper process death reclaimed its key and parked waiter"
SERIAL_CRASHER_EXIT = (
    "[desktop] helper id=org.arenaos.phase13crasher Process-cap exit status=262; owner-group reap=ok"
)
COUNTERS = re.compile(
    r"measured frames/records/processes/regions/pages/maps/caps="
    r"(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n"
)
DETAILS = re.compile(
    r"native resource detail values="
    + r"(\d+(?:/\d+){14})\r?\n"
)


def pixel(image, x, y):
    offset = (y * 800 + x) * 3
    return tuple(image[offset:offset + 3])


def green_pixels(image, x, y, width, height):
    return sum(
        1
        for row in range(y, y + height)
        for column in range(x, x + width)
        if (lambda rgb: rgb[1] > rgb[0] * 1.4 and rgb[1] > rgb[2] * 1.2)(
            pixel(image, column, row)
        )
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
        flags=25,
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
        flags=20,
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


def send_chord(d, qcode):
    d.q.command(
        "input-send-event",
        events=[d.q._ev("ctrl", True), d.q._ev(qcode, True), d.q._ev(qcode, False), d.q._ev("ctrl", False)],
    )


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


def resource_details(d):
    rows = DETAILS.findall(d.serial())
    assert rows, "detailed native resource receipt is missing"
    return [tuple(map(int, row.split("/"))) for row in rows]


def wait_detail(d, predicate, description, timeout_s=45):
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        details = resource_details(d)
        if predicate(details[-1]):
            return details[-1]
        time.sleep(0.05)
    raise AssertionError((description, resource_details(d)[-1]))


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
        baseline_detail = resource_details(d)[-1]

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
        send_chord(d, "d")
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
        d.wait(lambda: d.serial().count(SERIAL_THREADS) == 1,
               "installed document handler did not pass its ring-3 user-thread proof")
        d.wait(lambda: d.serial().count(SERIAL_SYNC) == 1,
               "installed document handler did not pass its native synchronization proof")
        wait_stream_proof(d, 1)
        d.wait(lambda: d.serial().count("[desktop] real application spawned;") >= 1,
               "Open With did not spawn the selected installed application")
        assert tree(disk)[f"/Users/user/Desktop/z-associated.txt"] == b"Phase 13 associated document\n", \
            "read-only Open With modified the source document"
        document_detail = wait_detail(
            d,
            lambda detail: detail[10:14] == (1, 1, 1, 1)
            and detail[14] == 1
            and detail[9] == 2,
            "read-only document AppInstance did not retain its helper, stream and two parked workers",
        )

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
        unpinned_row = crop(navigation, 554, 352, 64, 20)
        send_chord(d, "p")
        navigation = d.shot(
            "all-apps-pinned",
            lambda p: crop(p, 554, 352, 64, 20) != unpinned_row,
        )
        favorites_file = tree(disk)["/Users/user/.arena-app-favorites"]
        assert favorites_file[:4] == b"AFAV" and APP_ID in favorites_file, \
            "selected app pin was not persisted in AFS2"
        assert "[desktop] AFS2 application favorite saved" in d.serial(), \
            "Desktop did not acknowledge the durable pin update"
        assert green_pixels(navigation, 520, 357, 65, 20) >= 10, \
            "All Applications did not mark the already-running installed app"
        send_key(d, "ret")
        d.wait(lambda: d.serial().count(SERIAL_APPS) == 1,
               "All Applications keyboard launch did not run the installed ELF", timeout_s=60)
        d.wait(lambda: d.serial().count(SERIAL_VM) == 2,
               "All Applications launch did not pass its ring-3 VM mechanism proof")
        d.wait(lambda: d.serial().count(SERIAL_HEAP) == 2,
               "All Applications launch did not pass its scalable heap proof")
        d.wait(lambda: d.serial().count(SERIAL_THREADS) == 2,
               "All Applications launch did not pass its ring-3 user-thread proof")
        d.wait(lambda: d.serial().count(SERIAL_SYNC) == 2,
               "All Applications launch did not pass its native synchronization proof")
        wait_stream_proof(d, 2)
        d.wait(lambda: d.serial().count(SERIAL_MULTIWINDOW) == 1,
               "installed app did not create three windows in its one process")
        wait_detail(
            d,
            lambda detail: detail[10:14] == (2, 4, 2, 2)
            and detail[14] == 3
            and detail[9] == 2,
            "first ordinary launch did not add one three-window AppInstance",
        )
        first_points = ((80, 100), (130, 150), (156, 170))
        first = d.shot(
            "installed-window-one",
            lambda p: checkerboard_pixels(p, 82, 130) and marker_set(p, first_points),
        )
        assert checkerboard_pixels(first, 82, 130), "the installed ELF did not publish its owned checkerboard surface"
        assert marker_set(first, first_points), \
            "one installed Process did not publish three distinct compositor-owned window surfaces"

        # A click on the verified installed app's dock tile activates its
        # current AppInstance instead of creating another process.
        active_before_dock = resource_counts(d)
        activations_before_dock = d.serial().count("[desktop] dock activated a live application window")
        d.click(574, 568)
        d.wait(
            lambda: d.serial().count("[desktop] dock activated a live application window")
            == activations_before_dock + 1,
            "running dock tile did not activate an owned live window",
        )
        d.wait(
            lambda: resource_counts(d)[1:] == active_before_dock[1:],
            "running dock item spawned a duplicate process or changed authority",
        )

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
                   "search result pointer launch did not create the second app process", timeout_s=60)
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
        d.wait(lambda: d.serial().count(SERIAL_THREADS) == 3,
               "pointer launch did not pass its ring-3 user-thread proof")
        d.wait(lambda: d.serial().count(SERIAL_SYNC) == 3,
               "pointer launch did not pass its native synchronization proof")
        wait_stream_proof(d, 3)
        d.wait(lambda: d.serial().count(SERIAL_MULTIWINDOW) == 2,
               "second application instance did not create three windows in its one process", timeout_s=30)
        wait_detail(
            d,
            lambda detail: detail[10:14] == (3, 7, 3, 3)
            and detail[14] == 3
            and detail[9] == 2,
            "second ordinary launch did not add one three-window AppInstance",
        )
        both = d.shot(
            "installed-window-two",
            lambda p: checkerboard_pixels(p, 108, 154),
        )
        assert checkerboard_pixels(both, 82, 130) and checkerboard_pixels(both, 108, 154), \
            "two separate application instances did not own visible windows"

        # The focused installed app waits on its private stream notification.
        # Keyboard events become bytes only through the Desktop's exact stdin
        # writer for that AppInstance.
        d.q.type_text("native-stream", gap_s=0.025)
        send_key(d, "ret")
        d.wait(lambda: d.serial().count(SERIAL_STREAM_STDIN) == 1,
               "native stdin did not receive the exact keyboard byte sequence")

        # Window publication and the Desktop's measurement line are separate
        # event-loop observations. The read-only document AppInstance stays
        # open here with its exact File cap and two parked ring-3 workers.
        both_windows_expected = list(baseline)
        both_windows_expected[1] += 6    # three primaries and three live helpers
        both_windows_expected[2] += 6    # three primaries and three live helpers
        both_windows_expected[3] += 17   # seven windows plus three stream regions
        both_windows_expected[4] += 6583 # seven bounded windows plus three stream pages
        both_windows_expected[5] += 28   # windows, streams and exact document map
        d.wait(lambda: resource_counts(d)[1:6] == tuple(both_windows_expected[1:6]),
               "seven ordinary windows did not reach their complete resource inventory")
        settled_inventory = resource_counts(d)
        assert settled_inventory[6] > baseline[6]
        time.sleep(0.2)
        assert resource_counts(d) == settled_inventory, \
            "seven-window resource inventory changed after it was considered settled"

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

        # Close one ordinary window from each instance. Both processes and
        # their sibling windows remain live for the mixed workload below.
        # The second instance's third window is the global topmost cascade
        # slot; closing it exposes the first instance's third close button.
        first_after_one = close_extra(557, 74, 1)
        assert first_after_one[2] == baseline[2] + 6
        after_one = d.shot(
            "first-instance-one-window-closed",
            lambda p: checkerboard_pixels(p, 82, 130)
            and checkerboard_pixels(p, 108, 154),
        )
        assert checkerboard_pixels(after_one, 82, 130)
        assert checkerboard_pixels(after_one, 108, 154)

        second_after_one = close_extra(453, 145, 2)
        assert second_after_one[2] == baseline[2] + 6
        three_instance_inventory = resource_counts(d)
        three_instance_detail = wait_detail(
            d,
            lambda detail: detail[10:14] == (3, 5, 3, 3)
            and detail[14] == 2
            and detail[9] == 2,
            "document and two ordinary AppInstances did not remain independently live",
        )

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
        headless_before_detail = resource_details(d)[-1]
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
        headless_live_detail = wait_detail(
            d,
            lambda detail: detail[10] == headless_before_detail[10] + 1
            and detail[11] == headless_before_detail[11],
            "headless launch changed the AppInstance count without preserving window count",
        )
        assert headless_live_detail[11] == headless_before_detail[11]
        assert "ordinary windows=0; no surface or Desktop endpoint inherited" in d.serial(), \
            "headless launch allocated a surface/window or inherited its Desktop endpoint"
        no_window = d.shot("headless-running-no-window")
        assert checkerboard_pixels(no_window, 82, 130) \
            and checkerboard_pixels(no_window, 108, 154), \
            "headless launch hid or replaced the existing ordinary windows"
        d.wait(lambda: "[phase13-headless] timer completed; process exiting for manager reap" in d.serial(),
               "headless process did not finish its real timer wait")
        d.wait(lambda: d.serial().count(SERIAL_HEADLESS_EXIT) == exit_status_before_headless + 1,
               "Desktop did not observe the exact exit=42 through the Process cap")
        d.wait(lambda: d.serial().count("[desktop] application retired:") == retired_before_headless + 1,
               "Desktop did not reap the exited headless process")
        d.wait(lambda: resource_counts(d)[1:] == three_instance_inventory[1:],
               "headless timer/process resources did not return to the three-instance inventory")
        wait_detail(
            d,
            lambda detail: detail[10:14] == (3, 5, 3, 3)
            and detail[14] == 2
            and detail[9] == 2,
            "headless teardown disturbed live windows, helpers, streams or parked workers",
        )

        # Add two more verified installed instances. Together with the open
        # read-only document and the two partially closed multi-window apps,
        # these give five installed AppInstances, five live helpers, five
        # standard-stream sets and eleven ordinary windows.
        for launch_index in range(2):
            before_apps = d.serial().count(SERIAL_APPS)
            send_key(d, "esc")
            desktop_before_search = d.shot(f"pressure-desktop-{launch_index}")
            d.click(120, 12)
            opened_search = d.shot(
                f"pressure-all-apps-open-{launch_index}",
                lambda p, desktop_before_search=desktop_before_search:
                    crop(p, 184, 120, 432, 356)
                    != crop(desktop_before_search, 184, 120, 432, 356),
            )
            empty_search_field = crop(opened_search, 190, 154, 420, 22)
            d.q.type_text("phase13", gap_s=0.025)
            d.shot(
                f"pressure-installed-search-{launch_index}",
                lambda p, empty_search_field=empty_search_field:
                    crop(p, 190, 154, 420, 22) != empty_search_field,
            )
            d.click(250, 220)
            d.wait(
                lambda before_apps=before_apps:
                    d.serial().count(SERIAL_APPS) == before_apps + 1,
                f"pressure installed launch {launch_index + 1}/2 did not start the signed ELF",
                timeout_s=60,
            )
            expected_apps = 4 + launch_index
            d.wait(lambda expected_apps=expected_apps: d.serial().count(SERIAL_MULTIWINDOW) == expected_apps - 1,
                   "pressure installed application did not create its three ordinary windows",
                   timeout_s=45)
            d.wait(lambda expected_apps=expected_apps: d.serial().count(SERIAL_VM) == expected_apps,
                   "pressure installed application did not pass its VM proof", timeout_s=45)
            d.wait(lambda expected_apps=expected_apps: d.serial().count(SERIAL_HEAP) == expected_apps,
                   "pressure installed application did not pass its heap proof", timeout_s=45)
            d.wait(lambda expected_apps=expected_apps: d.serial().count(SERIAL_THREADS) == expected_apps,
                   "pressure installed application did not pass its thread proof", timeout_s=45)
            d.wait(lambda expected_apps=expected_apps: d.serial().count(SERIAL_SYNC) == expected_apps,
                   "pressure installed application did not pass its synchronization proof", timeout_s=45)
            wait_stream_proof(d, expected_apps)

        wait_detail(
            d,
            lambda detail: detail[10:14] == (5, 11, 5, 5)
            and detail[14] == 3
            and detail[9] == 2,
            "five installed AppInstances did not retain eleven windows and two live user threads",
        )

        # Fill to a representative mixed workload: 21 built-in instances plus
        # five signed APB1 instances, 32 ordinary windows, and the document's
        # two parked ring-3 workers. Built-in favorites remain the first six
        # dock entries; the installed favorite occupies the seventh.
        builtin_kinds = [index % 6 for index in range(21)]
        spawned_before_pressure = d.serial().count("[desktop] real application spawned;")
        audits_before_pressure = d.serial().count(
            "[application] ABI-v2 badge dispatch audit PASS"
        )
        for index, kind in enumerate(builtin_kinds, 1):
            d.click(226 + kind * 58, 570)
            d.wait(
                lambda index=index: d.serial().count("[desktop] real application spawned;")
                >= spawned_before_pressure + index,
                f"mixed workload built-in session {index}/21 did not spawn",
                timeout_s=20,
            )
            d.wait(
                lambda index=index: d.serial().count(
                    "[application] ABI-v2 badge dispatch audit PASS"
                ) >= audits_before_pressure + index,
                f"mixed workload built-in session {index}/21 did not pass its capability audit",
                timeout_s=30,
            )

        full_detail = wait_detail(
            d,
            lambda detail: detail[10:14] == (26, 32, 5, 5)
            and detail[14] == 3
            and detail[9] == 2,
            "mixed workload did not reach 26 AppInstances, 32 windows, five helpers and five streams",
            timeout_s=90,
        )
        full_receipt = resource_counts(d)
        assert full_detail[1] >= baseline_detail[1] + 33, (baseline_detail, full_detail)
        # Applications use badge-minted clients of the existing Desktop
        # endpoint, so the endpoint object count stays fixed while client
        # process and capability counts rise.
        assert full_detail[2] >= baseline_detail[2]
        assert full_detail[3] > baseline_detail[3]
        assert full_detail[4] >= baseline_detail[4] + 5, (baseline_detail, full_detail)
        assert full_detail[5] > baseline_detail[5]
        assert full_detail[6] > baseline_detail[6]
        assert full_detail[7] >= baseline_detail[7] + 5, (baseline_detail, full_detail)
        assert full_detail[8] >= baseline_detail[8] + 2
        assert full_detail[9] == 2
        assert full_receipt[1] > baseline[1] and full_receipt[2] > baseline[2]
        assert full_receipt[3] > baseline[3] and full_receipt[4] > baseline[4]
        assert full_receipt[5] > baseline[5] and full_receipt[6] < 127

        # Close exactly half the windows from the top of the stack, then
        # reuse those AppInstance and window-table slots with sixteen more
        # real built-in processes.
        previous_windows = 32
        for index in range(16):
            send_key(d, "f8")
            half_detail = wait_detail(
                d,
                lambda detail, target=previous_windows - 1: detail[11] == target,
                f"mixed workload half-close stopped before window {index + 1}/16",
            )
            previous_windows -= 1
        assert half_detail[10:14] == (10, 16, 5, 5) and half_detail[14] == 3, half_detail
        refill_kinds = [index % 6 for index in range(16)]
        spawned_before_refill = d.serial().count("[desktop] real application spawned;")
        audits_before_refill = d.serial().count(
            "[application] ABI-v2 badge dispatch audit PASS"
        )
        for index, kind in enumerate(refill_kinds, 1):
            d.click(226 + kind * 58, 570)
            d.wait(
                lambda index=index: d.serial().count("[desktop] real application spawned;")
                >= spawned_before_refill + index,
                f"mixed workload reuse launch {index}/16 did not spawn",
                timeout_s=20,
            )
            d.wait(
                lambda index=index: d.serial().count(
                    "[application] ABI-v2 badge dispatch audit PASS"
                ) >= audits_before_refill + index,
                f"mixed workload reuse launch {index}/16 did not pass its capability audit",
                timeout_s=30,
            )
        reused_detail = wait_detail(
            d,
            lambda detail: detail[10:14] == (26, 32, 5, 5)
            and detail[14] == 3
            and detail[9] == 2,
            "mixed workload reuse did not restore its full live inventory",
            timeout_s=90,
        )
        assert reused_detail[1:4] == full_detail[1:4]
        assert reused_detail[5:14] == full_detail[5:14]
        assert reused_detail[14] == full_detail[14]

        # Close all 32 windows. Each F8 must retire one real compositor
        # window, and the final window for each AppInstance must reap its
        # primary process and its still-live helper through the ProcessGroup.
        for index in range(32):
            previous_windows = 32 - index
            send_key(d, "f8")
            wait_detail(
                d,
                lambda detail, target=previous_windows - 1: detail[11] == target,
                f"mixed workload teardown stopped before window {index + 1}/32",
                timeout_s=45,
            )
        d.wait(lambda: resource_counts(d)[1:] == baseline[1:],
               "mixed workload identity resources did not return to boot baseline")
        final_detail = wait_detail(
            d,
            lambda detail: detail[10:] == baseline_detail[10:]
            and detail[1:4] == baseline_detail[1:4]
            and detail[5:10] == baseline_detail[5:10],
            "mixed workload thread, IPC, timer-independent VM, sync and app counts did not return to baseline",
            timeout_s=90,
        )
        assert final_detail[4] <= full_detail[4]
        assert d.serial().count("[phase13-threads] closing the document window woke and joined both live workers") == 1
        assert d.serial().count("[desktop] AppInstance ProcessGroup teardown members=2; final members=0") == 5
        assert d.serial().count(SERIAL_HELPER_EXIT) == 5
        assert d.serial().count(SERIAL_CRASHER_EXIT) == 10
        assert d.serial().count("[phase13-helper] live helper left for AppInstance owner cleanup") == 5
        assert resource_counts(d)[1:] == baseline[1:]
        return b"shutdown\r"
    finally:
        d.dispose()


def verify_favorites_reload(disk):
    d = Desktop(LABEL + "-favorites-reload")
    try:
        assert "[desktop] AFS2 application dock favorites loaded" in d.serial(), \
            "Desktop did not reload favorites from the existing AFS2 image"
        background = d.shot("favorites-reload-before-launcher")
        d.click(120, 12)
        d.q.type_text("phase13", gap_s=0.04)
        pinned = d.shot(
            "favorites-reload-pinned-row",
            lambda p: crop(p, 554, 208, 64, 20)
            != crop(background, 554, 208, 64, 20),
        )
        assert crop(pinned, 554, 208, 64, 20) != bytes(64 * 20 * 3), \
            "persisted favorite is absent from the reloaded All Applications row"
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
    rc, serial, elapsed = mtest.boot(LABEL, esp, feed, disk, pointer=True, timeout_s=600)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    assert rc == 0, serial[-5000:]
    assert serial.count(SERIAL_APPS) == 4
    assert serial.count(SERIAL_MULTIWINDOW) == 4
    assert serial.count(SERIAL_VM) == 5
    assert serial.count(SERIAL_HEAP) == 5
    assert serial.count(SERIAL_THREADS) == 5
    assert serial.count(SERIAL_SYNC) == 5
    assert serial.count(SERIAL_HEADLESS) == 1
    assert serial.count("[phase13-headless] timer completed; process exiting for manager reap") == 1
    assert serial.count("[phase13-helper] unknown signed helper ID refused without spawn") == 5
    assert serial.count(SERIAL_HELPER_STREAM_POLICY) == 5
    assert serial.count(SERIAL_HELPER_STREAM_CHILD) == 5
    # Five installed instances each exercise four short-lived helpers and
    # retain one timer-backed helper through their ProcessGroup teardown.
    assert serial.count("[phase13-helper] no inherited notification factory; WRITE-only owner signal cannot wait") == 25
    assert serial.count("[phase13-helper] exact Startup ABI inventory: private timer Notification") == 25
    assert serial.count("[phase13-helper] readiness badge sent through separate WRITE-only owner signal") == 5
    assert serial.count(SERIAL_HELPER_STREAM) == 5
    assert serial.count(SERIAL_HELPER_CRASH_EOF) == 10
    assert serial.count("[phase13-helper] owner-authorized terminate/reap passed") == 5
    assert serial.count(SERIAL_HELPER_THREAD) == 5
    assert serial.count("[phase13-helper] crashed helper status=262 observed and reaped") == 5
    assert serial.count("[phase13-helper] live helper left for AppInstance owner cleanup") == 5
    assert serial.count(SERIAL_HELPER_EXIT) == 5
    assert serial.count(SERIAL_CRASHER_EXIT) == 10
    assert serial.count("[phase13-helper] retired private timer object reclaimed after reap") == 5
    assert serial.count("[desktop] AppInstance ProcessGroup teardown members=2; final members=0") == 5
    # Headless exit and each installed app's final-window exit have
    # application-specific lifecycle markers above; the generic status 42
    # receipt also includes the 21 built-in pressure-fixture processes.
    assert serial.count("[phase13-threads] closing the document window woke and joined both live workers") == 1
    assert "[phase13-app] exact read-only document capability verified" in serial
    assert "[desktop] installed application registry unavailable" not in serial
    assert "[desktop] application retired:" in serial
    assert "[app] abnormal exit stage" not in serial
    assert not afs1.audit(disk)
    reload_feed = [
        ((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"),
         1, lambda: verify_favorites_reload(disk)),
    ]
    rc, reload_serial, reload_elapsed = mtest.boot(
        LABEL + "-favorites-reload",
        esp,
        reload_feed,
        disk,
        pointer=True,
        timeout_s=90,
    )
    assert rc == 0, reload_serial[-3000:]
    print(
        f"[{LABEL}] verified APB1 install -> registry -> keyboard/search/pointer launch of the signed native ELF; "
        f"five installed instances, 26 mixed AppInstances and 32 windows, five live helpers, "
        f"two parked user threads, half-close/reuse/full teardown, one headless app, persisted "
        f"AFS2 favorite reloaded by the next boot, exact identity-resource teardown to baseline; "
        f"clean shutdown in {elapsed:.1f}s + "
        f"reload {reload_elapsed:.1f}s PASS",
        flush=True,
    )


if __name__ == "__main__":
    main()
