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
]

if __name__ == "__main__":
    sys.exit(mtest.run_milestone("m5", EXPECTED_TESTS))
