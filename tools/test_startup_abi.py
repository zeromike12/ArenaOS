#!/usr/bin/env python3
"""Independent ADR-0083 startup wire oracle and hostile mutation controls."""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import startup_abi as abi

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "userspace/arena-platform/tests/data/startup-v2.bin"
RUNTIME_FIXTURE = ROOT / "userspace/arena-runtime/tests/data/startup-runtime-v2.bin"
RUNTIME_BAD_RIGHTS = ROOT / "userspace/arena-runtime/tests/data/startup-runtime-bad-rights-v2.bin"
RUNTIME_HEAP_SLOT = ROOT / "userspace/arena-runtime/tests/data/startup-runtime-heap-slot-v2.bin"


class TestStartupAbi(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.page = FIXTURE.read_bytes()

    def test_rust_golden_vector_is_canonical_and_roundtrips_independently(self):
        self.assertEqual(self.page, abi.encode_sample())
        view = abi.parse(self.page)
        self.assertEqual(view["application_id"][:16], b"com.arena.editor")
        self.assertEqual(view["arguments"], (b"com.arena.editor", b"notes.txt"))
        self.assertEqual(view["environment"], (b"LANG=en",))
        self.assertEqual(view["capabilities"], (
            (1, 1, abi.CAP_BADGED_ENDPOINT, abi.RIGHT_WRITE),
            (2, 5, 1, abi.RIGHT_READ),
        ))
        self.assertEqual(view["references"], (0, abi.NONE, abi.NONE, abi.NONE))
        self.assertEqual(view["total_bytes"], 216)

    def test_independent_guest_vector_and_live_capability_red_control(self):
        valid = RUNTIME_FIXTURE.read_bytes()
        bad_rights = RUNTIME_BAD_RIGHTS.read_bytes()
        heap_slot = RUNTIME_HEAP_SLOT.read_bytes()
        self.assertEqual(valid, abi.encode_runtime_sample())
        self.assertEqual(
            bad_rights,
            abi.encode_runtime_sample(abi.RIGHT_READ | abi.RIGHT_DESTROY),
        )
        view = abi.parse(valid)
        self.assertEqual(view["application_id"][:17], b"com.arena.startup")
        self.assertEqual(view["arguments"], (b"startup-probe", b"alpha"))
        self.assertEqual(view["environment"], (b"MODE=proof",))
        self.assertEqual(
            view["capabilities"],
            ((1, 5, abi.CAP_NOTIFICATION, abi.RIGHT_READ | abi.RIGHT_WRITE),),
        )
        self.assertEqual(view["references"], (abi.NONE, abi.NONE, abi.NONE, abi.NONE))
        # This is a structurally valid page. The runtime, not the wire parser,
        # must refuse because the actual child slot has READ only.
        self.assertEqual(
            abi.parse(bad_rights)["capabilities"],
            ((1, 5, abi.CAP_NOTIFICATION, abi.RIGHT_READ | abi.RIGHT_DESTROY),),
        )
        self.assertEqual(
            heap_slot,
            abi.encode_runtime_sample(
                abi.RIGHT_READ | abi.RIGHT_WRITE | abi.RIGHT_COPY | abi.RIGHT_DESTROY,
                instance_slot=5,
                include_boot_image=True,
            ),
        )
        self.assertEqual(abi.parse(heap_slot)["instance_slot"], 5)
        self.assertEqual(
            abi.parse(heap_slot)["capabilities"],
            (
                (
                    1,
                    5,
                    abi.CAP_NOTIFICATION,
                    abi.RIGHT_READ | abi.RIGHT_WRITE | abi.RIGHT_COPY | abi.RIGHT_DESTROY,
                ),
                (2, 5, abi.CAP_BOOT_IMAGE, abi.RIGHT_READ),
            ),
        )

    def test_mutated_offsets_padding_and_tail_refuse(self):
        for offset, value in ((50, 1), (len(self.page) - 1, 1)):
            mutated = bytearray(self.page)
            mutated[offset] = value
            with self.subTest(offset=offset), self.assertRaises(abi.Refusal):
                abi.parse(bytes(mutated))
        mutated = bytearray(self.page)
        mutated[abi.OFF_ARGS:abi.OFF_ARGS + 4] = (1).to_bytes(4, "little")
        with self.assertRaisesRegex(abi.Refusal, "offsets"):
            abi.parse(bytes(mutated))
        mutated = bytearray(self.page)
        strings = int.from_bytes(mutated[abi.OFF_STRINGS:abi.OFF_STRINGS + 4], "little")
        mutated[strings] = 0
        with self.assertRaisesRegex(abi.Refusal, "string encoding"):
            abi.parse(bytes(mutated))

    def test_capability_role_identity_and_bound_mutations_refuse(self):
        mutations = []
        wrong_slot = bytearray(self.page)
        wrong_slot[abi.HEADER_BYTES:abi.HEADER_BYTES + 2] = (2).to_bytes(2, "little")
        mutations.append(wrong_slot)
        wrong_ref = bytearray(self.page)
        wrong_ref[abi.OFF_CWD:abi.OFF_CWD + 2] = (1).to_bytes(2, "little")
        mutations.append(wrong_ref)
        bad_id = bytearray(self.page)
        bad_id[abi.OFF_APP_ID] = ord("!")
        mutations.append(bad_id)
        too_many = bytearray(self.page)
        too_many[abi.OFF_ARGC:abi.OFF_ARGC + 2] = (33).to_bytes(2, "little")
        mutations.append(too_many)
        for mutated in mutations:
            with self.assertRaises(abi.Refusal):
                abi.parse(bytes(mutated))

    def test_32_instance_slots_match_lifecycle_and_manager_without_aliasing(self):
        highest = abi.encode_runtime_sample(instance_slot=abi.INSTANCE_SLOTS - 1)
        self.assertEqual(abi.parse(highest)["instance_slot"], 31)
        with self.assertRaisesRegex(abi.Refusal, "bounds"):
            abi.encode_runtime_sample(instance_slot=abi.INSTANCE_SLOTS)
        lifecycle = (ROOT / "userspace/arena-platform/src/lifecycle.rs").read_text()
        startup = (ROOT / "userspace/arena-platform/src/startup.rs").read_text()
        desktop = (ROOT / "userspace/desktop/src/bin/desktop.rs").read_text()
        self.assertIn("MAX_APP_INSTANCES: usize = crate::startup::INSTANCE_SLOTS;", lifecycle)
        self.assertIn("pub const INSTANCE_SLOTS: usize = 32;", startup)
        self.assertIn("instance_slot: i as u16,", desktop)
        self.assertNotIn("i % arena_desktop::apps::STARTUP_INSTANCE_SLOTS", desktop)

    def test_cap_inventory_syscall_number_and_slot_bound_are_mirrored(self):
        userspace_abi = (ROOT / "userspace/abi.rs").read_text()
        kernel_syscall = (ROOT / "kernel/kernel/src/arch/x86_64/syscall.rs").read_text()
        kernel_caps = (ROOT / "kernel/kernel/src/cap.rs").read_text()
        kernel_spawn = (ROOT / "kernel/kernel/src/spawn.rs").read_text()
        self.assertIn("pub const SYS_CAP_OCCUPIED: u64 = 54;", userspace_abi)
        self.assertIn("pub const SYS_CAP_OCCUPIED: u64 = 54;", kernel_syscall)
        self.assertIn("pub const CAP_SLOTS: usize = 128;", userspace_abi)
        self.assertIn("pub const CAP_SLOTS: usize = 128;", kernel_caps)
        self.assertIn("pub const CAP_KIND_PROCESS: u8 = 4;", userspace_abi)
        platform_startup = (ROOT / "userspace/arena-platform/src/startup.rs").read_text()
        self.assertIn("pub const CAP_KIND_PROCESS: u8 = 4;", platform_startup)
        self.assertIn("pub const MAX_SPAWN_INHERIT: usize = 7;", userspace_abi)
        self.assertIn("pub const MAX_INHERIT: usize = 7;", kernel_spawn)

    def test_slot_zero_transport_requires_exact_read_only_one_page(self):
        self.assertTrue(abi.validate_startup_cap((abi.CAP_SHARED_REGION, 1, 9), 1))
        self.assertFalse(abi.validate_startup_cap((abi.CAP_SHARED_REGION, 1, 11), 1))
        self.assertFalse(abi.validate_startup_cap((abi.CAP_SHARED_REGION, 1, 9), 2))
        self.assertFalse(abi.validate_startup_cap((abi.CAP_BADGED_ENDPOINT, 1, 9), 1))


if __name__ == "__main__":
    unittest.main(verbosity=2)
