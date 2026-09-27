#!/usr/bin/env python3
"""Milestone 5 automated boot test (docs/TESTING.md, ADR-0005).

Pipeline and verdict logic live in tools/mtest.py (including the
cross-milestone regression guard: the same boot must still carry
PASSing m1/m2/m3/m4 RESULT lines, and the harness attaches the fresh
virtio-blk scratch disk every run — arena_env.scratch_disk_args).

Current coverage:
  M5.1 — driver substrate (ADR-0021):
  * pci_scan      — the kernel's boot-time bus-0 walk (drivers/pci.rs:
                    config PIO 0xCF8/0xCFC, BAR sizing, capability
                    walk) found the harness's virtio-blk fixture:
                    vendor 0x1AF4, device 0x1001 transitional or
                    0x1042 modern, virtio type 2 (block), all four
                    virtio 1.0 structures (common/notify/isr/device)
                    resolved onto sized memory BARs, MSI-X table
                    recorded, and MEM|BUS MASTER readable back from
                    the command register — the kernel-policy write
                    ring 3 never performs.
  * untyped_alloc — a ring-3 payload allocates two OWNED frames via
                    SYS_ALLOC_FRAME (16), self-maps the first via
                    SYS_MAP_MEMORY (17) at the kernel-chosen VA,
                    stamps and reads back a magic word through the
                    window, and observes the CONSUMED slot refuse a
                    re-map with -2; the kernel then destroys the
                    second cap (exactly one frame returns) and
                    proc::destroy reclaims the mapped frame — total
                    teardown frame-exact.
  * mmio_user     — the kernel mints an Mmio cap over the HPET
                    main-counter page (the only way ring 3 ever sees
                    device registers); a payload self-maps it
                    READ-ONLY and observes the counter strictly
                    increase across a bounded delay (kernel verifies
                    the same from its own alias first); the cap
                    survives mapping, and teardown frees exactly the
                    RAM — the MMIO leaf is skipped by the walk, never
                    handed to the frame allocator.
  * irq_relay     — the interrupt→notification bridge (IDT vectors
                    48..63): registration seams refuse out-of-range
                    vectors, the empty badge, double-register, and
                    double-release; then the LIVE path — a parked
                    kernel waiter, a LAPIC self-IPI at vector 48, the
                    relay stub's dual EOI, relay::handle →
                    ipc::notify, and the waiter wakes with the exact
                    badge (1 delivery, 1 notify, thread reaped); a
                    spurious hit on unregistered vector 49 is counted
                    and survived.

  M5.2 — the userspace block service (ADR-0022):
  * block_service — the kernel spawns storaged (registry image 2, the
                    userspace virtio-blk driver: Mmio-cap window grant,
                    endpoint serve side, interrupt notification) and
                    blktest (image 3, the client: endpoint call side).
                    The client allocates a buffer frame, lends a COPY
                    of its cap through IPC, and drives a
                    write→clear→read-back→verify cycle against the
                    scratch disk THROUGH the service boundary — the
                    device DMAs the caller's own page (zero copy).
                    Completions arrive as device MSI-X interrupts on a
                    SYS_IRQ_RELAY-armed vector, relayed into the
                    driver's SYS_WAIT (never polled). Kernel-side
                    proofs: both exit badges exact, both exit codes 42
                    (the client verified the pattern byte-for-byte),
                    exactly TWO relay-vector deliveries (one per
                    request), proc::destroy sweeps the dead driver's
                    relay, and teardown is frame-exact.

  M5.3 — the filesystem service (ADR-0023):
  * fs_service    — the kernel spawns fsd (registry image 4, the AFS1
                    server: block-endpoint call side + FS-endpoint serve
                    side) and fstest (image 5, the client: FS call side
                    + poison side). fstest CREATEs "arena.txt" (name in
                    the IPC v1.1 inline message), WRITEs 512 pattern
                    bytes — its LENT frame is FORWARDED by fsd to
                    storaged so the device DMAs the client's own page
                    (zero-copy end to end) — CLOSEs, re-OPENs by name,
                    READs back into the cleared frame, verifies
                    byte-for-byte, walks LS (exactly one file, right
                    name and size), then shuts both services down by
                    their own hands — exit 42, the FRESH-volume
                    contract (a volume that survived a reboot takes
                    the persisted branch instead: verify with ZERO
                    writes, exit 43 — exercised by test_m5_persist.py
                    and test_m5_crash.py). Kernel-side proofs: three
                    exit badges exact, and the client's exit code
                    SELECTING its relay-delivery contract (42 → 34,
                    43 → 14 — the derived disk-operation counts),
                    proc::destroy sweeps the dead driver's relay, and
                    teardown is frame-exact. AFTER the boot, this
                    script parses the scratch image itself (tools/afs1.py):
                    the newest commit must carry "arena.txt" with size
                    512, its extents must resolve to real allocated
                    sectors, and those sectors must hold the exact
                    pattern fstest wrote — the on-disk layout proof.

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402

EXPECTED_TESTS = [
    "pci_scan",
    "untyped_alloc",
    "mmio_user",
    "irq_relay",
    "block_service",
    "fs_service",
]


def verify_disk_state() -> list[str]:
    """Parse the post-boot scratch image: the on-disk AFS1 proof.

    The m5 suite's fstest committed exactly one file; the host-side
    mirror of the layout (tools/afs1.py) must find it in the NEWEST
    commit — name, size, extents, bitmap, and the literal pattern
    bytes in the data sectors. Returns a list of failures (empty =
    verified).
    """
    import afs1
    import arena_env

    problems: list[str] = []
    path = arena_env.build_dir() / "scratch.img"
    try:
        disk = afs1.Disk(path.read_bytes())
    except OSError as e:
        return [f"cannot read {path}: {e}"]
    try:
        geo = disk.superblock()
        seq, ot, bm = disk.commit()
        if geo["total_sectors"] != afs1.SCRATCH_TOTAL_SECTORS_EXPECTED:
            problems.append(f"superblock total_sectors {geo['total_sectors']}")
        # mkfs's first commit is seq 1; fs_service commits CREATE and
        # WRITE → the newest generation must be seq 3.
        if seq != 3:
            problems.append(f"newest commit seq {seq}, expected 3 (mkfs + create + write)")
        obj = disk.find(b"arena.txt", ot)
        if obj is None:
            problems.append("arena.txt is not in the committed object table")
            return problems
        if obj["size"] != 512:
            problems.append(f"arena.txt size {obj['size']}, expected 512")
        extents = disk.extents(obj["extent_head"])
        covered = sum(length for _, length in extents) * afs1.SECTOR
        if covered < 512:
            problems.append(f"extents {extents} cover {covered} bytes, expected >= 512")
        bitmap = disk.bitmap(bm)
        for start, length in extents:
            for s in range(start, start + length):
                if not (bitmap[s // 8] >> (s % 8)) & 1:
                    problems.append(f"data sector {s} is not marked used in the committed bitmap")
        # The data itself: fstest's pattern, byte-for-byte, on the platter.
        data = bytearray()
        for start, length in extents:
            for s in range(start, start + length):
                data += disk.sector(s)
        for i in range(512):
            want = afs1.pattern_byte(i)
            if data[i] != want:
                problems.append(
                    f"sector data byte {i} is {data[i]:#04x}, expected {want:#04x} (the committed pattern mismatched)"
                )
                break
        print(
            f"[test-m5] disk proof: commit seq {seq}, objtab@{ot}, bitmap@{bm}, "
            f"arena.txt size {obj['size']}, extents {extents}, pattern verified in the committed sectors"
        )
    except afs1.Afs1Error as e:
        problems.append(f"AFS1 parse failed: {e}")
    return problems


if __name__ == "__main__":
    rc = mtest.run_milestone("m5", EXPECTED_TESTS)
    if rc == 0:
        for problem in verify_disk_state():
            print(f"[test-m5] FAIL: disk state: {problem}")
            rc = 1
        if rc == 0:
            print("[test-m5] PASS: the post-boot scratch image holds the committed file (on-disk layout verified)")
    sys.exit(rc)
