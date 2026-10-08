#!/usr/bin/env python3
"""Phase-12 guest qualification of the 32-window Desktop envelope.

The real Desktop boots with 32 distinct session clocks, starts 32 ordinary
ring-3 applications across all six built-in kinds, refuses launch 33 without
mutating its resource snapshot or any described cap slot, closes half, reuses
the freed session/process slots, then retires every child. This is a guest
observation, not a host projection.
"""
from __future__ import annotations

import re
import time
from pathlib import Path

import arena_env
import mtest
from test_m10_apps import Desktop, receipts

LABEL = "m12-scale"
SESSION_RESERVATION = re.compile(
    r"\[desktop\] session reservation shared/snapshot pages=(\d+)/(\d+)"
)
CAP_HIGH_WATER = re.compile(r"\[desktop\] measured broker cap high-water=(\d+)")
CAP_INVENTORY = re.compile(
    r"\[desktop\] full-session refusal cap inventory (before|after) "
    r"occupied=(\d+) digest=(\d+)"
)
RESOURCE_INVENTORY = re.compile(
    r"\[desktop\] full-session refusal resource inventory (before|after)="
    r"(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)"
)
RETIRED_KIND = re.compile(r"\[desktop\] application retired: kind=(\d+) ")


def wait_for(d: Desktop, predicate, description: str, timeout_s: float = 30) -> None:
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(0.04)
    raise AssertionError(description)


def resource_shape(receipt: tuple[int, ...]) -> tuple[int, ...]:
    """Identity-bearing totals; maps and capabilities stay guest-measured."""
    return receipt[1:5]


def wait_for_stable_receipt(d: Desktop, stable_s: float = 1.0, timeout_s: float = 30) -> tuple[int, ...]:
    """Wait until asynchronous app connection stops changing the measured receipt."""
    end = time.monotonic() + timeout_s
    last = receipts(d.serial())[-1]
    unchanged_since = time.monotonic()
    while time.monotonic() < end:
        current = receipts(d.serial())[-1]
        if current != last:
            last = current
            unchanged_since = time.monotonic()
        elif time.monotonic() - unchanged_since >= stable_s:
            return current
        time.sleep(0.04)
    raise AssertionError(("guest resource receipt did not settle", last, receipts(d.serial())[-1]))


def key(d: Desktop, name: str) -> None:
    d.q.command(
        "input-send-event",
        events=[d.q._ev(name, True), d.q._ev(name, False)],
    )


def workflow(label: str, disk: Path) -> bytes:
    d = Desktop(label)
    try:
        base_rows = receipts(d.serial())
        assert base_rows, "settled boot resource baseline is absent"
        base = base_rows[0]
        reserve = SESSION_RESERVATION.search(d.serial())
        assert reserve, "Desktop session reservation receipt absent"
        shared, snapshot = map(int, reserve.groups())
        assert (shared, snapshot) == (471, 469), (shared, snapshot)
        # Five complete cycles exercise all six ordinary app kinds; the final
        # two launches are Files and Terminal so slot 31 is a resizable window.
        kinds = [i % 6 for i in range(30)] + [1, 0]
        assert len(kinds) == 32 and kinds.count(0) + kinds.count(1) == 12
        spawned = d.serial().count("[desktop] real application spawned;")
        badge_audits = d.serial().count("[application] ABI-v2 badge dispatch audit PASS")
        for i, kind in enumerate(kinds, 1):
            d.click(255 + kind * 58, 570)
            wait_for(
                d,
                lambda i=i: d.serial().count("[desktop] real application spawned;")
                >= spawned + i,
                f"ordinary Desktop session {i}/32 did not acquire its held Process cap",
                timeout_s=20,
            )
            wait_for(
                d,
                lambda i=i: d.serial().count(
                    "[application] ABI-v2 badge dispatch audit PASS"
                ) >= badge_audits + i,
                f"ordinary Desktop session {i}/32 did not finish its live ABI badge audit",
                timeout_s=30,
            )

        expected_full = (
            base[0],
            base[1] + 32,
            base[2] + 32,
            base[3] + 64,
            base[4] + 32 * (shared + snapshot),
            base[5],  # Shared-map count is recorded from the guest, not projected.
            base[6],  # Exact cap occupancy is separately inventoried in the guest.
        )
        # Wait for all 32 clients to map and filesd to finish its twelve
        # Terminal/Files I/O mappings before taking the full snapshot.
        wait_for(
            d,
            lambda: bool(receipts(d.serial()))
            and resource_shape(receipts(d.serial())[-1]) == resource_shape(expected_full),
            "32-session resource snapshot did not reach the exact expected working set",
            timeout_s=60,
        )
        full = wait_for_stable_receipt(d)
        assert full[0] < base[0] and full[0] > 8192, (base, full)
        assert base[4] >= snapshot and full[4] == base[4] + 32 * (shared + snapshot), (base, full)
        assert full[3] == base[3] + 64 and full[3] < 80, (base, full)
        assert base[5] < full[5] < 128, (base, full)
        assert base[6] + 64 <= full[6] <= base[6] + 65 and full[6] + 1 < 128, (base, full)
        highwaters = [int(n) for n in CAP_HIGH_WATER.findall(d.serial())]
        assert highwaters and max(highwaters) == base[6] + 65, (base, highwaters)
        assert d.serial().count("[desktop] real application spawned;") == spawned + 32
        assert (
            d.serial().count("[application] ABI-v2 badge dispatch audit PASS")
            == badge_audits + 32
        ), "a built-in did not pass caller-word isolation and wrong-cap dispatch controls"

        # Attempt 33 while every ordinary session is live. The Desktop itself
        # compares all 128 caller-owned cap descriptors slot-by-slot and emits
        # a forced post-refusal SYS_OBSERVE receipt (including free frames).
        receipt_count = len(receipts(d.serial()))
        d.click(255 + 5 * 58, 570)
        wait_for(
            d,
            lambda: "[desktop] launch refused at bounded capacity" in d.serial()
            and "[desktop] full-session refusal cap inventory exact-equal=yes" in d.serial()
            and "[desktop] full-session refusal resource inventory exact-equal=yes" in d.serial()
            and len(receipts(d.serial())) > receipt_count,
            "33rd launch lacked refusal, exact cap-inventory, or fresh resource-snapshot evidence",
        )
        after_refusal = receipts(d.serial())[-1]
        inventory = CAP_INVENTORY.findall(d.serial())
        assert len(inventory) == 2, inventory
        before_cap, after_cap = inventory
        assert before_cap[0] == "before" and after_cap[0] == "after", inventory
        assert before_cap[1:] == after_cap[1:], inventory
        resource_inventory = RESOURCE_INVENTORY.findall(d.serial())
        assert len(resource_inventory) == 2, resource_inventory
        before_resources, after_resources = resource_inventory
        assert before_resources[0] == "before" and after_resources[0] == "after", resource_inventory
        before_receipt = tuple(map(int, before_resources[1:]))
        after_receipt = tuple(map(int, after_resources[1:]))
        assert before_receipt == after_receipt == after_refusal, (
            before_receipt,
            after_receipt,
            after_refusal,
        )
        assert before_receipt[1:5] == full[1:5], (full, before_receipt)
        assert base[5] < before_receipt[5] < 128, (base, before_receipt)
        assert base[6] + 64 <= before_receipt[6] <= base[6] + 65, (
            base,
            before_receipt,
        )
        assert int(before_cap[1]) == before_receipt[6], (before_cap, before_receipt)
        assert "[desktop] full-session refusal resource inventory exact-equal=yes" in d.serial()
        assert d.serial().count("[desktop] real application spawned;") == spawned + 32
        assert "[desktop] full-session cap audit distinct Process caps=32" in d.serial()
        assert "distinct Notifications=33" in d.serial()
        region_audit = re.search(
            r"full-session cap audit distinct Process caps=32 baseline SharedRegion caps=(\d+) ",
            d.serial(),
        )
        # The manager drops its duplicate region ref after delegation; each
        # child retains the exact SharedRegion cap and map/pin counts prove
        # the 64 session-owned regions remain live.
        assert region_audit and int(region_audit[1]) == 1, d.serial()

        # Close the upper half, then refill the exact freed ProcessGroup and
        # window-table entries. F8 closes the focused topmost window each time.
        retired = d.serial().count("[desktop] application retired:")
        retired_kind_count = len(RETIRED_KIND.findall(d.serial()))
        for i in range(16):
            key(d, "f8")
            wait_for(
                d,
                lambda i=i: d.serial().count("[desktop] application retired:")
                >= retired + i + 1,
                f"half-capacity teardown stopped before child {i + 1}/16",
            )
        retired_kinds = [
            int(kind)
            for kind in RETIRED_KIND.findall(d.serial())[retired_kind_count:]
        ]
        assert len(retired_kinds) == 16 and all(0 <= kind < 6 for kind in retired_kinds), (
            retired_kinds,
            d.serial(),
        )
        half_expected = (
            base[0],
            base[1] + 16,
            base[2] + 16,
            base[3] + 32,
            base[4] + 16 * (shared + snapshot),
            base[5],  # The exact active mapping count is separately measured.
            base[6],  # Exact cap occupancy is separately inventoried in the guest.
        )
        wait_for(
            d,
            lambda: bool(receipts(d.serial()))
            and resource_shape(receipts(d.serial())[-1]) == resource_shape(half_expected),
            "closing half of the 32-session set did not reclaim exact records, regions, pages, maps and caps",
            timeout_s=60,
        )
        half = receipts(d.serial())[-1]
        assert half[0] < base[0] and half[0] > 8192, (base, half)
        assert base[5] < half[5] < full[5] < 128, (base, half, full)
        assert base[6] < half[6] < before_receipt[6], (base, half, before_receipt)

        refill = [i % 6 for i in range(15)] + [0]
        assert refill.count(0) + refill.count(1) == 7
        refill_audits = d.serial().count(
            "[application] ABI-v2 badge dispatch audit PASS"
        )
        for i, kind in enumerate(refill, 1):
            d.click(255 + kind * 58, 570)
            wait_for(
                d,
                lambda i=i: d.serial().count("[desktop] real application spawned;")
                >= spawned + 32 + i,
                f"reused Desktop session slot {i}/16 did not spawn",
                timeout_s=20,
            )
            wait_for(
                d,
                lambda i=i: d.serial().count(
                    "[application] ABI-v2 badge dispatch audit PASS"
                ) >= refill_audits + i,
                f"reused Desktop session slot {i}/16 did not finish its live ABI badge audit",
                timeout_s=30,
            )
        expected_reused = (
            base[0],
            base[1] + 32,
            base[2] + 32,
            base[3] + 64,
            base[4] + 32 * (shared + snapshot),
            base[5],  # The exact active mapping count is separately measured.
            base[6],  # Exact cap occupancy is separately inventoried in the guest.
        )
        wait_for(
            d,
            lambda: bool(receipts(d.serial()))
            and resource_shape(receipts(d.serial())[-1]) == resource_shape(expected_reused),
            "reused 32-session working set did not return to its exact resource envelope",
            timeout_s=60,
        )
        reused = receipts(d.serial())[-1]
        assert reused[0] < base[0] and reused[0] > 8192, (base, reused)
        assert base[5] < reused[5] < 128, (base, reused)
        assert base[6] + 64 <= reused[6] <= base[6] + 65, (base, reused)

        # Present all reused clients, then wait for their late connection maps
        # to settle before beginning exact resource teardown accounting.
        d.settled("reused-full", (0, 26, 800, 500))
        reused = wait_for_stable_receipt(d)
        assert resource_shape(reused) == resource_shape(expected_reused), (expected_reused, reused)
        # Retire all 32 sessions. Every identity-bearing count returns to the
        # settled baseline; empty intermediate page tables remain explicitly
        # budgeted by the measured free-frame delta.
        retired = d.serial().count("[desktop] application retired:")
        for i in range(32):
            key(d, "f8")
            wait_for(
                d,
                lambda i=i: d.serial().count("[desktop] application retired:")
                >= retired + i + 1,
                f"final scale teardown stopped before child {i + 1}/32",
            )
        wait_for(
            d,
            lambda: bool(receipts(d.serial()))
            and receipts(d.serial())[-1][1:] == base[1:],
            "final 32-session teardown did not return exact records/processes/regions/pages/maps/caps",
            timeout_s=60,
        )
        final = receipts(d.serial())[-1]
        retained_frames = base[0] - final[0]
        assert retained_frames >= 64, (base, final, retained_frames)
        assert final[0] > 8192, (base, final)
        assert d.serial().count("[desktop] application retired:") >= retired + 32
        print(
            f"[{label}] 32 ordinary sessions; baseline={base}; full={full}; "
            f"mutation-free 33rd refusal; half={half}; reused-full={reused}; "
            f"final={final}; retained page-table frames={retained_frames}; "
            "unique clock/Process-cap/region audit and exact cleanup PASS",
            flush=True,
        )
        return b"shutdown\r"
    finally:
        d.dispose()


def main() -> None:
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk(afs2=True)
    feed = [
        (
            (
                b"[desktop] audited 32 distinct client clocks; filesystem endpoint slot13; filesd lineage slot20",
                b"[desktop] real desktop frame presented",
                b"AFS2 file service online",
                b"arena>",
            ),
            1,
            lambda: workflow(LABEL, disk),
        )
    ]
    rc, serial, seconds = mtest.boot(
        LABEL, esp, feed, disk, pointer=True, timeout_s=900
    )
    assert rc == 0, (rc, seconds, serial[-6000:])
    required = (
        "servicemgr: full fixture notification budget 64/64; sixty-fifth refused, 13 probe slots reclaimed",
        "servicemgr: extended cap occupancy [32,127)=0",
        "servicemgr: reserved APB1 slot127 descriptor=1 kind=12 rights=6",
        "[desktop] audited 32 distinct client clocks; filesystem endpoint slot13; filesd lineage slot20",
        "[desktop] full-session cap audit distinct Process caps=32",
        "[desktop] full-session refusal cap inventory exact-equal=yes",
        "[desktop] full-session refusal resource inventory exact-equal=yes",
        "[desktop] launch refused at bounded capacity (status -4)",
    )
    missing = [marker for marker in required if marker not in serial]
    assert not missing, missing
    assert "[desktop] full-session refusal cap inventory exact-equal=no" not in serial
    assert "[desktop] full-session refusal resource inventory exact-equal=no" not in serial
    assert "[desktop] failed stage" not in serial and "PANIC" not in serial
    native_receipts = receipts(serial)
    assert native_receipts, "no settled resource receipts in the guest serial"
    region_peak = max(receipt[3] for receipt in native_receipts)
    page_peak = max(receipt[4] for receipt in native_receipts)
    print(
        f"[{LABEL}] PASS: 32 live ordinary Desktop sessions with 32 unique clocks, "
        f"64 session-owned regions ({region_peak} total records) and {page_peak} total measured pages; "
        "all six built-in kinds, mapping/cap/frame headroom, exact mutation-free 33rd refusal, "
        "16-close/16-reuse, full teardown; "
        f"guest boot {seconds:.1f}s",
        flush=True,
    )


if __name__ == "__main__":
    main()
