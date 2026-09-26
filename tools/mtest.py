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

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402

TIMEOUT_S = 120  # TCG is slow; a healthy boot takes <10s
MEM_MIB = 512


def build(label: str) -> Path:
    print(f"[{label}] building kernel image + ESP ...")
    subprocess.run(
        ["bash", str(arena_env.REPO_ROOT / "tools/build.sh"), "--image"],
        check=True,
        capture_output=True,
        text=True,
    )
    esp = arena_env.REPO_ROOT / "build/arena-esp.img"
    assert esp.exists(), "build.sh did not produce the ESP image"
    return esp


# The default feed: when the shell prompts for the first time, shut the
# machine down (ADR-0020 — boots end at the shell, never by themselves).
DEFAULT_FEED: list[tuple[bytes, int, bytes]] = [(b"arena>", 1, b"shutdown\r")]


def run_qemu(label: str, esp: Path,
             feed: list[tuple[bytes, int, bytes]] | None = None,
             ) -> tuple[int, str, float]:
    bdir = arena_env.build_dir()
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
            "-drive", f"if=pflash,format=raw,readonly=on,file={arena_env.ovmf_code()}",
            "-drive", f"if=pflash,format=raw,file={vars_img}",
            "-drive", f"format=raw,file={esp}",
        ]
        # Milestone-5 fixture (ADR-0021): fresh scratch disk attached as
        # virtio-blk-pci — the kernel's bus-0 scan must find it.
        + arena_env.scratch_disk_args()
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
    with open(serial_log, "wb") as logf:
        proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=logf,
                                stderr=subprocess.DEVNULL)
        stop = threading.Event()

        def feeder() -> None:
            sent = 0
            while not stop.is_set() and sent < len(script):
                try:
                    data = serial_log.read_bytes()
                except OSError:
                    data = b""
                marker, nth, payload = script[sent]
                if data.count(marker) >= nth:
                    try:
                        assert proc.stdin is not None
                        proc.stdin.write(payload)
                        proc.stdin.flush()
                    except (BrokenPipeError, OSError):
                        return  # VM gone; nothing left to feed
                    sent += 1
                time.sleep(0.05)

        th = threading.Thread(target=feeder, daemon=True)
        th.start()
        try:
            rc = proc.wait(timeout=TIMEOUT_S)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
            stop.set()
            raise
        stop.set()
    dt = time.monotonic() - t0
    serial = serial_log.read_text(errors="replace") if serial_log.exists() else ""
    return rc, serial, dt


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
                  feed: list[tuple[bytes, int, bytes]] | None = None) -> int:
    """Full pipeline for one milestone's markers. Returns process exit code."""
    label = f"test-{milestone}"
    try:
        esp = build(label)
    except subprocess.CalledProcessError as e:
        print(f"[{label}] FAIL: kernel build failed:\n{e.stdout}\n{e.stderr}")
        return 1
    try:
        rc, serial, dt = run_qemu(label, esp, feed)
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
