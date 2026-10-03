#!/usr/bin/env python3
"""Shared milestone boot-test pipeline (docs/TESTING.md, ADR-0005).

Every tools/test_mN.py is a thin wrapper over run_milestone(): build the
kernel + ESP image, boot it in QEMU/EDK2 headless with fresh NVRAM, capture
serial, assert the milestone's marker grammar, and assert the VM terminated
by itself through the kernel's declared clean-halt path.

Since M4.6 (ADR-0020) a healthy boot no longer halts by itself: after the
suites the kernel spawns the shell and the machine lives until someone
types `shutdown`. So the serial runs through a stdio chardev (input and
output on one pipe pair, no monitor interleaving) and every boot gets a
marker-paced feeder thread: the default script sends `shutdown` when the
shell's first `arena>` prompt appears — which means EVERY milestone boot
now also proves the whole console chain (UART RX IRQ -> line discipline ->
blocking SYS_CONSOLE_READ -> shell -> Power-gated SYS_SHUTDOWN ->
ResetSystem). test_m4_shell.py overrides the script to drive a full
interactive session. A feed item is (marker bytes, occurrence count,
payload bytes): the payload is written once the marker has appeared that
many times in the captured serial.

Verdict logic (three distinct failure modes):
  * timeout            -> kernel hung
  * nonzero QEMU exit  -> kernel crashed / reset loop
  * markers            -> kernel ran but a self-test failed or panicked

Since M6.3 (ADR-0026) a boot has a SECOND input channel: the virtual
keyboard. `keys` is a typing script in the same (marker, count,
payload) shape, injected through QMP (tools/qmp.py) one keystroke at a
time — the path a user's keyboard in a QEMU window takes. The default
script types the m6 input fixture, because that suite's test waits on
real key events rather than answering itself.

Fixtures (arena_env): every boot attaches the fresh AFS1-formatted
scratch disk as virtio-blk-pci (M5, ADR-0021/0023), the slirp NIC as
virtio-net-pci (M6.1, ADR-0024), the entropy source (M6.2, ADR-0025),
and the virtio keyboard (M6.3, ADR-0026). `run_qemu(net=False,
rng=False, kbd=False)` reproduces a pre-v0.6.0 invocation for the
honest-SKIP compatibility tests.

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import os
import re
import shutil
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import qmp  # noqa: E402
import vcon as vconsole  # noqa: E402 — `vcon` is a run_qemu parameter
import tcp_fixture  # noqa: E402
import udp_dns_fixture  # noqa: E402

TIMEOUT_S = 120  # TCG is slow; a healthy boot takes <10s
MEM_MIB = 512


def semantic_exit_rc(rc: int | None, serial: str, label: str) -> int | None:
    """QEMU ResetSystem is also used by halt_machine: rc=0 is NOT PASS.

    Successful boots in this repository all complete M3/M4's historical
    markers and the shell's explicit shutdown. Individual suites still
    require their own operation-specific PASS/SKIP markers and disk/wire
    proofs. Never turn a SIGKILL crash fixture's rc=None into success.
    """
    if rc != 0:
        return rc
    fatal = ('[arena ERROR halt]', '[arena ERROR ipc]', 'halting machine:',
             'PANIC', 'RESULT FAIL')
    required = ('m3: RESULT PASS (13/13)', 'm4: RESULT PASS (9/9)',
                'halting via UEFI ResetSystem(shutdown)')
    hit = next((x for x in fatal if x in serial), None)
    missing = next((x for x in required if x not in serial), None)
    if hit or missing:
        print(f'[{label}] GUEST FAILURE despite QEMU rc=0: '
              f'{"fatal " + hit if hit else "missing " + missing}')
        return 97  # harness verdict, not a fabricated QEMU process status
    return rc


def build(label: str, desktop: bool = False) -> Path:
    print(f"[{label}] building kernel image + ESP ...")
    env=os.environ.copy()
    if desktop:env.pop('ARENA_GRAPHICS_FIXTURE',None)
    else:env['ARENA_GRAPHICS_FIXTURE']='phase9'
    subprocess.run(
        ["bash", str(arena_env.REPO_ROOT / "tools/build.sh"), "--image"],
        check=True,
        capture_output=True,
        text=True,
        env=env,
    )
    esp = arena_env.REPO_ROOT / "build/arena-esp.img"
    assert esp.exists(), "build.sh did not produce the ESP image"
    return esp


# The default feed: when the shell prompts for the first time, shut the
# machine down (ADR-0020 — boots end at the shell, never by themselves).
DEFAULT_FEED: list[tuple[bytes, int, bytes]] = [(b"arena>", 1, b"shutdown\r")]

# A typing script item is (marker bytes, occurrence count, text): once
# the marker has appeared that many times on the serial, the text is
# typed on the VIRTUAL KEYBOARD through QMP (M6.3, ADR-0026) — one
# keystroke at a time, exactly as a user would. Marker-paced, never
# sleep-based: the usual harness discipline, applied to a second
# channel.
KeyScript = list[tuple[bytes | tuple[bytes, ...], int, str]]

# The default typing script, symmetric with DEFAULT_FEED: the m6 suite's
# input_service test blocks until REAL key events arrive (unlike the net
# and rng fixtures, which answer by themselves), so every boot that
# attaches the keyboard must also type on it. `arena` is the fixture
# sequence inputtest verifies byte-for-byte — which means every
# milestone boot now proves the whole keyboard chain too (device
# interrupt -> evdev event -> keymap -> IPC), just as every boot has
# proved the serial console chain since M4.6.
DEFAULT_KEYS: KeyScript = [(b"inputd: virtio-input ready", 1, "arena")]


def _typist(script: KeyScript, serial_log: Path, sock: Path,
            stop: threading.Event, label: str) -> None:
    """Watch the serial log and type each script item at its marker."""
    if not script:
        return
    session: qmp.Qmp | None = None
    try:
        for marker, nth, text in script:
            while not stop.is_set():
                try:
                    data = serial_log.read_bytes()
                except OSError:
                    data = b""
                markers = marker if isinstance(marker, tuple) else (marker,)
                if all(data.count(m) >= nth for m in markers):
                    break
                time.sleep(0.02)
            if stop.is_set():
                return
            if session is None:
                session = qmp.Qmp(str(sock))
            session.type_text(text)
    except Exception as e:  # noqa: BLE001 — a typist fault must not hang the run
        print(f"[{label}] NOTE: the QMP typist failed: {e}")
    finally:
        if session is not None:
            session.close()


# A console script item is (marker, nth, text): send `text` on the
# virtio-console socket once `text`'s marker has appeared `nth` times in
# what the GUEST has sent back over that same socket. Marker-paced like
# every other actor in this harness — the console's own stream is the
# clock, so nothing depends on host timing.
ConsoleScript = vconsole.ConsoleScript

# The default console script answers the m6 suite's contest client. Its
# fixture line arrives on the socket; the reply goes back down the same
# socket and must come out of the guest's receive queue byte-for-byte.
DEFAULT_CONSOLE: ConsoleScript = [
    (b"contest: hello from ArenaOS", 1, "host-says-hello\n"),
]


def timeout_diagnostics(label: str, serial_log: Path, vcon_capture: Path,
                        qmp_sock: Path) -> None:
    """Preserve an actual hung boot and locate its CPU before SIGKILL.

    In particular, an OVMF-only boot must not be misreported as a guest
    deadlock. No verdict is changed and there is no automatic retry.
    """
    snapshot = arena_env.build_dir() / f"serial-{label}-timeout.log"
    if serial_log.exists():
        shutil.copyfile(serial_log, snapshot)
    serial = snapshot.read_bytes() if snapshot.exists() else b""
    vcon_bytes = vcon_capture.stat().st_size if vcon_capture.exists() else -1
    info = [f"serial={len(serial)} bytes; virtconsole={vcon_bytes} bytes",
            f"serial tail={serial[-1200:]!r}"]
    try:
        session = qmp.Qmp(str(qmp_sock), connect_timeout_s=2)
        info.append(f"QMP status={session.command('query-status')}")
        for _ in range(2):
            registers = session.command('human-monitor-command', **{'command-line': 'info registers'})
            info.append(f"CPU={registers}")
            rip = re.search(r'RIP=([0-9a-fA-F]+)', registers)
            if rip:
                # Capture the actual firmware/guest instructions too. A
                # fixed CPL0 RIP in OVMF cannot be attributed to ArenaOS
                # without seeing what code the firmware is executing.
                info.append(f"RIP-instructions={session.command('human-monitor-command', **{'command-line': 'x/16i 0x'+rip.group(1)})}")
            time.sleep(0.05)
        session.close()
    except Exception as exc:  # diagnostics must not override the timeout
        info.append(f"QMP diagnosis unavailable: {exc!r}")
    report = arena_env.build_dir() / f"timeout-{label}.txt"
    report.write_text("\n".join(info) + "\n")
    print(f"[{label}] timed out; preserved serial and QMP CPU evidence in {report}")


def run_qemu(label: str, esp: Path,
             feed: list[tuple[bytes, int, bytes]] | None = None,
             net: bool = True,
             rng: bool = True,
             kbd: bool = True,
             keys: KeyScript | None = None,
             vcon: bool = True,
             console: ConsoleScript | None = None,
             tcp_peer: bool = True,
             disk: bool = True,
             ) -> tuple[int, str, float]:
    bdir = arena_env.build_dir()
    qmp_sock = bdir / f"qmp-{label}.sock"
    vcon_sock = bdir / f"vcon-{label}.sock"
    vcon_capture = bdir / f"vcon-{label}.txt"
    vcon_capture.write_bytes(b"")
    if keys is None:
        keys = DEFAULT_KEYS if kbd else []
    if console is None:
        console = DEFAULT_CONSOLE if vcon else []
    vars_img = bdir / "ovmf-vars.img"
    shutil.copyfile(arena_env.ovmf_vars_template(), vars_img)  # fresh NVRAM every run
    serial_log = bdir / "serial.log"
    if serial_log.exists():
        serial_log.unlink()

    cmd = (
        arena_env.qemu_cmd()
        + arena_env.qemu_data_args()
        + [
            "-M", "q35",
            "-m", f"{MEM_MIB}M",
            "-cpu", "qemu64,+nx,+smep,+smap",
            # Prefer the ESP boot drive to the raw scratch virtio device on
            # each freshly seeded OVMF NVRAM boot.
            "-boot", "order=c",
            "-drive", f"if=pflash,format=raw,readonly=on,file={arena_env.ovmf_code()}",
            "-drive", f"if=pflash,format=raw,file={vars_img}",
            "-drive", f"format=raw,file={esp}",
        ]
        # Milestone-5 fixture (ADR-0021): fresh scratch disk attached as
        # virtio-blk-pci — the kernel's bus-0 scan must find it.
        + (arena_env.scratch_disk_args() if disk else [])
        # Milestone-6 fixture (ADR-0024): the slirp NIC for netd's link
        # proof — nettest's ARP request goes to 10.0.2.2 and comes back.
        # net=False reproduces a pre-v0.6.0 invocation (the honest-SKIP
        # compatibility window test).
        + (arena_env.net_args() if net else [])
        # Milestone-6.2 fixture (ADR-0025): the entropy source for
        # rngd's variance proof. rng=False reproduces an invocation
        # without it (the honest-SKIP compatibility window test).
        + (arena_env.rng_args() if rng else [])
        # Milestone-6.3 fixture (ADR-0026): the virtual keyboard inputd
        # drives, plus the QMP control socket the typist injects
        # keystrokes through. kbd=False reproduces an invocation
        # without a keyboard (the honest-SKIP compatibility window).
        + (arena_env.input_args() if kbd else [])
        # Milestone-6.4 fixture (ADR-0027): the virtio-console port and
        # its host-side unix socket — the second console channel.
        # vcon=False reproduces an invocation without it.
        + (arena_env.console_args(vcon_sock) if vcon else [])
        + arena_env.qmp_args(qmp_sock)
        + [
            "-display", "none",
            # Serial on a stdio chardev: output captured to the log file,
            # input written by the feeder. NOT mon:stdio — the monitor's
            # "(qemu)" chatter and Ctrl-a escapes would pollute the
            # marker grammar (ADR-0020).
            "-chardev", "stdio,id=con0,signal=off",
            "-serial", "chardev:con0",
            "-no-reboot",
        ]
    )
    print(f"[{label}] booting QEMU/EDK2:", " ".join(cmd[:3]), "...")
    script = DEFAULT_FEED if feed is None else feed
    t0 = time.monotonic()
    # QEMU's stderr is KEPT, not discarded. When the emulator refuses to
    # start — a bad device argument, a socket already bound, a missing
    # file — it says so on stderr and exits in a tenth of a second with
    # an empty serial log, and a harness that throws that away turns a
    # one-line explanation into a bisect. (Learned while chasing an
    # rc=1 that QEMU had explained perfectly the whole time.)
    err_log = arena_env.build_dir() / f"qemu-stderr-{label}.log"
    dns_stop = threading.Event()
    dns_sock = udp_dns_fixture.bind() if net else None
    dns_log = bdir / f"udp-dns-{label}.log"
    dns_log.write_text("")
    dns_thread = None
    if dns_sock is not None:
        dns_thread = threading.Thread(target=udp_dns_fixture.actor,
            args=(dns_sock, dns_stop, dns_log,
                  os.environ.get("ARENA_DNS_WRONG_TXID") == "1"), daemon=True)
        dns_thread.start()
    peer_stop = threading.Event()
    peer_listener = tcp_fixture.bind() if net and tcp_peer else None
    peer_log = bdir / f"tcp-{label}.log"
    peer_log.write_text("")
    peer_thread = None
    if peer_listener is not None:
        peer_thread = threading.Thread(target=tcp_fixture.actor,
            args=(peer_listener, peer_stop, peer_log), daemon=True)
        peer_thread.start()
    with open(serial_log, "wb") as logf, open(err_log, "wb") as errf:
        proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=logf,
                                stderr=errf)
        stop = threading.Event()

        def feeder() -> None:
            sent = 0
            while not stop.is_set() and sent < len(script):
                try:
                    data = serial_log.read_bytes()
                except OSError:
                    data = b""
                marker, nth, payload = script[sent]
                # Some lifecycle tests require BOTH a service-ready
                # event and the shell prompt. Sending at only READY can
                # queue the command while the pre-shell config updater
                # is still draining, splitting its echoed line. A tuple
                # is a conjunction, not a timed retry or weaker proof.
                markers = marker if isinstance(marker, tuple) else (marker,)
                if all(data.count(m) >= nth for m in markers):
                    try:
                        assert proc.stdin is not None
                        proc.stdin.write(payload() if callable(payload) else payload)
                        proc.stdin.flush()
                    except (BrokenPipeError, OSError):
                        return  # VM gone; nothing left to feed
                    sent += 1
                time.sleep(0.05)

        th = threading.Thread(target=feeder, daemon=True)
        th.start()
        typist = threading.Thread(
            target=_typist, args=(keys, serial_log, qmp_sock, stop,
                                  label),
            daemon=True)
        actor = threading.Thread(
            target=vconsole.actor,
            args=(console, vcon_sock, vcon_capture, stop, label),
            daemon=True)
        actor.start()
        typist.start()
        try:
            rc = proc.wait(timeout=TIMEOUT_S)
        except subprocess.TimeoutExpired:
            timeout_diagnostics(label, serial_log, vcon_capture, qmp_sock)
            proc.kill()
            proc.wait()
            stop.set()
            peer_stop.set()
            dns_stop.set()
            if peer_thread is not None: peer_thread.join(timeout=2)
            if dns_thread is not None: dns_thread.join(timeout=2)
            raise
        stop.set()
    peer_stop.set()
    if peer_thread is not None: peer_thread.join(timeout=2)
    dns_stop.set()
    if dns_thread is not None: dns_thread.join(timeout=2)
    dt = time.monotonic() - t0
    serial = serial_log.read_text(errors="replace") if serial_log.exists() else ""
    verdict = semantic_exit_rc(rc, serial, label)
    if verdict == 0 and net and "m7: RESULT PASS (2/2)" in serial and dns_log.read_text().count("DNS_FIXTURE_QUERY ") != 3:
        print(f"[{label}] FAIL: controlled host did not receive exactly three DNS queries: {dns_log}")
        verdict = 97
    return verdict, serial, dt


def boot(label: str, esp: Path,
         feed: list[tuple[bytes, int, bytes]],
         scratch: Path,
         kill: tuple[bytes, int, float] | None = None,
         timeout_s: int = TIMEOUT_S,
         video: str = "default",
         pointer: bool = False,
         ) -> tuple[int | None, str, float]:
    """One QEMU boot against an EXPLICIT scratch-disk path (M5.4).

    Unlike `run_qemu`, this never reformats: multi-boot scripts own
    the disk's lifecycle (two-boot persistence, crash rounds).

    `kill` arms the crash gate: `(marker, nth, delay_s)`. Once the
    feeder has sent its LAST payload, the killer waits until `marker`
    appears `nth` more times in the serial AFTER that point, sleeps
    `delay_s`, then SIGKILLs QEMU — the harness's process-crash
    model. Completed writes are in the host page cache and survive
    the process death; unsubmitted ones are lost. The guest therefore
    sees a strict PREFIX of its issued device operations — exactly
    the failure mode AFS1's ping-pong commit (superblock record
    written last) is designed to survive. Returns rc=None for a
    killed boot.
    """
    bdir = arena_env.build_dir()
    vars_img = bdir / f"ovmf-vars-{label}.img"
    shutil.copyfile(arena_env.ovmf_vars_template(), vars_img)
    qmp_sock = bdir / f"qmp-{label}.sock"
    vcon_sock = bdir / f"vcon-{label}.sock"
    serial_log = bdir / f"serial-{label}.log"
    if serial_log.exists():
        serial_log.unlink()

    cmd = (
        arena_env.qemu_cmd()
        + arena_env.qemu_data_args()
        + [
            "-M", "q35",
            "-m", f"{MEM_MIB}M",
            "-cpu", "qemu64,+nx,+smep,+smap",
            # Prefer the ESP boot drive to the raw scratch virtio device on
            # each freshly seeded OVMF NVRAM boot.
            "-boot", "order=c",
            "-drive", f"if=pflash,format=raw,readonly=on,file={arena_env.ovmf_code()}",
            "-drive", f"if=pflash,format=raw,file={vars_img}",
            "-drive", f"format=raw,file={esp}",
            "-drive", f"file={scratch},format=raw,if=none,id=scr0",
            "-device", "virtio-blk-pci,drive=scr0",
            # M6 fixtures (ADR-0024/0025): the slirp NIC and the entropy
            # source — multi-boot scripts (persistence, crash) carry them
            # too so every boot is uniform.
            *arena_env.net_args(),
            *arena_env.rng_args(),
            *arena_env.input_args(),
            *(["-device", "virtio-tablet-pci"] if pointer else []),
            *arena_env.console_args(vcon_sock),
            *arena_env.qmp_args(qmp_sock),
            *(["-vga", "none"] if video in ("none", "gpu", "gpu-big") else []),
            *(["-device", "virtio-gpu-pci,xres=800,yres=600"] if video in ("gpu", "gpu+std") else []),
            *(["-device", "virtio-gpu-pci"] if video == "gpu-big" else []),
            "-display", "none",
            "-chardev", "stdio,id=con0,signal=off",
            "-serial", "chardev:con0",
            "-no-reboot",
        ]
    )
    t0 = time.monotonic()
    killed = False
    feeder_errors: list[Exception] = []
    # QEMU's stderr is KEPT, not discarded. When the emulator refuses to
    # start — a bad device argument, a socket already bound, a missing
    # file — it says so on stderr and exits in a tenth of a second with
    # an empty serial log, and a harness that throws that away turns a
    # one-line explanation into a bisect. (Learned while chasing an
    # rc=1 that QEMU had explained perfectly the whole time.)
    err_log = arena_env.build_dir() / f"qemu-stderr-{label}.log"
    dns_stop = threading.Event()
    dns_sock = udp_dns_fixture.bind()
    dns_log = bdir / f"udp-dns-{label}.log"
    dns_log.write_text("")
    dns_thread = threading.Thread(target=udp_dns_fixture.actor,
        args=(dns_sock, dns_stop, dns_log,
                  os.environ.get("ARENA_DNS_WRONG_TXID") == "1"), daemon=True)
    dns_thread.start()
    peer_stop = threading.Event()
    peer_listener = tcp_fixture.bind()
    peer_thread = threading.Thread(target=tcp_fixture.actor,
        args=(peer_listener, peer_stop, bdir / f"tcp-{label}.log"), daemon=True)
    peer_thread.start()
    with open(serial_log, "wb") as logf, open(err_log, "wb") as errf:
        proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=logf,
                                stderr=errf)
        stop = threading.Event()

        def feeder() -> None:
            nonlocal killed
            sent = 0
            # A boot-time service may transact before the shell can
            # accept any feeder command. With no feed, arm from byte 0;
            # command-driven crash tests still arm only after their last
            # input, preserving their existing isolation from boot logs.
            armed_at: int | None = 0 if kill and not feed else None
            marker, nth, delay = kill if kill else (b"", 0, 0.0)
            while not stop.is_set():
                try:
                    data = serial_log.read_bytes()
                except OSError:
                    data = b""
                while sent < len(feed):
                    m, n, payload = feed[sent]
                    markers = m if isinstance(m, tuple) else (m,)
                    if not all(data.count(part) >= n for part in markers):
                        break
                    try:
                        assert proc.stdin is not None
                        proc.stdin.write(payload() if callable(payload) else payload)
                        proc.stdin.flush()
                    except (BrokenPipeError, OSError):
                        return
                    except Exception as error:
                        feeder_errors.append(error)
                        proc.terminate()
                        return
                    sent += 1
                    if sent == len(feed):
                        # arm the killer at the CURRENT serial length:
                        # only guest output produced from here on can
                        # trigger the crash.
                        armed_at = len(data)
                if kill and armed_at is not None:
                    window = data[armed_at:]
                    if window.count(marker) >= nth:
                        time.sleep(delay)
                        proc.kill()  # SIGKILL: the crash itself
                        killed = True
                        return
                if sent >= len(feed) and not kill:
                    return  # nothing left to do; the guest shuts down
                time.sleep(0.02)

        th = threading.Thread(target=feeder, daemon=True)
        th.start()
        # The keyboard fixture: these boots run the full suite too, so
        # they type the input_service sequence like every other boot.
        typist = threading.Thread(
            target=_typist, args=(DEFAULT_KEYS, serial_log, qmp_sock, stop,
                                  label),
            daemon=True)
        typist.start()
        # The console actor. NOT optional: the fixture chardev is
        # `server=on,wait=on`, so QEMU does not finish starting until
        # somebody is at the far end of the port. A boot() without this
        # thread hangs before the firmware runs — which is exactly how
        # the persistence and crash tests failed when the device was
        # added here and the actor was not.
        actor = threading.Thread(
            target=vconsole.actor,
            args=(DEFAULT_CONSOLE, vcon_sock, bdir / f"vcon-{label}.txt",
                  stop, label),
            daemon=True)
        actor.start()
        try:
            rc: int | None = proc.wait(timeout=timeout_s)
        except subprocess.TimeoutExpired:
            timeout_diagnostics(label, serial_log, bdir / f"vcon-{label}.txt", qmp_sock)
            proc.kill()
            proc.wait()
            rc = None
            killed = True
            print(f"[{label}] NOTE: boot timed out before a clean exit — "
                  f"SIGKILLed (treated as a crash)")
        stop.set()
        th.join(timeout=2)
    peer_stop.set()
    peer_thread.join(timeout=2)
    dns_stop.set()
    dns_thread.join(timeout=2)
    if feeder_errors:
        raise RuntimeError(f"{label}: guest workflow assertion failed") from feeder_errors[0]
    if killed:
        rc = None
    dt = time.monotonic() - t0
    serial = serial_log.read_text(errors="replace") if serial_log.exists() else ""
    verdict = semantic_exit_rc(rc, serial, label)
    if verdict == 0 and "m7: RESULT PASS (2/2)" in serial and dns_log.read_text().count("DNS_FIXTURE_QUERY ") != 3:
        print(f"[{label}] FAIL: controlled host did not receive exactly three DNS queries: {dns_log}")
        verdict = 97
    return verdict, serial, dt


def evaluate(label: str, milestone: str, expected_tests: list[str],
             rc: int, serial: str, dt: float) -> bool:
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[{label}] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    if "PANIC" in serial:
        # Surface the panic line for diagnosis before anything else.
        for line in serial.splitlines():
            if "PANIC" in line:
                print(f"[{label}] kernel panic marker: {line.strip()}")
    check("PANIC" not in serial, "no kernel panic on serial")

    for name in expected_tests:
        m = re.search(rf"^{milestone}:test:{re.escape(name)}: (PASS|FAIL)(.*)$",
                      serial, re.MULTILINE)
        if m is None:
            check(False, f"self-test '{name}' reported (marker missing)")
        else:
            reason = m.group(2).strip()
            check(m.group(1) == "PASS", f"self-test '{name}' {reason}")

    m = re.search(rf"^{milestone}: RESULT (PASS|FAIL) \((\d+)/(\d+)\)$",
                  serial, re.MULTILINE)
    check(m is not None, "milestone RESULT line present")
    if m:
        check(m.group(1) == "PASS", f"RESULT is PASS ({m.group(2)}/{m.group(3)})")
        check(int(m.group(3)) == len(expected_tests),
              f"RESULT covers all {len(expected_tests)} expected tests "
              f"(got {m.group(3)})")

    # Cross-milestone regression guard (ADR-0005: old tests are never
    # deleted): every boot replays all earlier suites before this
    # milestone's, so any prior-milestone RESULT line on the serial that
    # is not PASS must fail THIS run — an old-suite failure must never
    # ride along invisibly inside a green new-milestone boot.
    for pm in re.finditer(r"^(m\d+): RESULT (PASS|FAIL) \((\d+)/(\d+)\)$",
                          serial, re.MULTILINE):
        if pm.group(1) != milestone:
            check(pm.group(2) == "PASS",
                  f"prior-milestone regression: {pm.group(1)} RESULT "
                  f"{pm.group(2)} ({pm.group(3)}/{pm.group(4)}) in this boot")

    # rc==0 alone does NOT prove a clean halt: a triple fault also ends with
    # QEMU exiting 0 under -no-reboot. The kernel's declared-halt log line is
    # the discriminator (it is printed only on the clean-shutdown path).
    check(
        "halting via UEFI ResetSystem(shutdown)" in serial,
        "kernel declared its clean halt (ResetSystem path reached)",
    )
    check(rc == 0,
          f"QEMU exited cleanly via kernel ResetSystem shutdown (rc={rc}, {dt:.1f}s)")
    return ok


def run_milestone(milestone: str, expected_tests: list[str],
                  feed: list[tuple[bytes, int, bytes]] | None = None,
                  keys: KeyScript | None = None) -> int:
    """Full pipeline for one milestone's markers. Returns process exit code."""
    label = f"test-{milestone}"
    try:
        esp = build(label)
    except subprocess.CalledProcessError as e:
        print(f"[{label}] FAIL: kernel build failed:\n{e.stdout}\n{e.stderr}")
        return 1
    try:
        rc, serial, dt = run_qemu(label, esp, feed, keys=keys)
    except subprocess.TimeoutExpired:
        print(f"[{label}] FAIL: VM did not terminate within {TIMEOUT_S}s (kernel hang?)")
        return 1
    arena_env.build_dir().joinpath(f"serial-{milestone}.log").write_text(serial)
    ok = evaluate(label, milestone, expected_tests, rc, serial, dt)
    print(f"[{label}] {'=' * 46}")
    print(f"[{label}] MILESTONE {milestone.upper()}: {'PASS' if ok else 'FAIL'}  "
          f"(serial: build/serial-{milestone}.log)")
    if not ok:
        tail = [ln for ln in serial.splitlines() if ln.strip()][-25:]
        print(f"[{label}] serial tail:")
        for ln in tail:
            print("   |", ln)
    return 0 if ok else 1
