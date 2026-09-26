#!/usr/bin/env python3
"""Milestone 4 automated boot test (docs/TESTING.md, ADR-0005).

Pipeline and verdict logic live in tools/mtest.py (including the
cross-milestone regression guard: the same boot must still carry
PASSing m1/m2/m3 RESULT lines). M4's contract grows with each roadmap
step.

Current coverage:
  M4.1 — executable format + image loader (ADR-0016: ELF64 container,
  ArenaOS strict-subset semantics; the test image is a genuine
  cargo/rust-lld artifact from userspace/payload, embedded at compile
  time by tools/build.sh):
  * elf_parse  — the embedded artifact validates field by field
                 (ET_EXEC, EM_X86_64, entry 0x200000, exactly two
                 PT_LOADs: text RX at 0x200000, data RW at 0x201000
                 with filesz < memsz proving the NOLOAD bss gap) and
                 the parsed header is cross-checked against the
                 payload's own META manifest — two independent
                 descriptions of the same image that must agree.
  * elf_reject — 25 mutation classes of the real artifact, each of
                 which MUST be refused: identification block (magic,
                 class, endianness, versions, type, machine, ehsize,
                 phentsize, phnum bounds, phoff overrun), truncation,
                 W+X segment, unknown flag bits, filesz > memsz, zero
                 memsz, unaligned vaddr, kernel-half vaddr, segment
                 overlap, file span past EOF, entry outside an
                 executable segment (two ways), PT_INTERP, and no
                 PT_LOAD at all.
  * elf_load   — a fresh process receives the image: exactly 3 pages
                 mapped, exactly 7 frames spent (root + 3 page-table
                 frames + 3 leaves), PTE flags asserted W^X-exact
                 (text present+user, NOT write, NOT NX; data/bss
                 present+user+write+NX), double-load refused at zero
                 frame cost, load into a dead process refused, and the
                 contents read back under the TARGET's CR3 through
                 STAC-bracketed accesses (SMAP armed): entry stub
                 EB FE, META magic + entry fact, all 4096 bss bytes
                 zero; proc::destroy reclaims every frame exactly.

  M4.2 — syscall ABI v1 (ADR-0017: RAX = call number, six arguments in
  RDI/RSI/RDX/R10/R8/R9, typed i64 status out, RBX/RBP/R12-R15
  preserved; registry: 1 debug_write, 2 thread_exit, 4/5 the M3 proof
  calls, 6 abi_echo6):
  * syscall_abi    — a hand-assembled ring-3 payload proves the v1
                     contract from the user side: abi_echo6's
                     fingerprint (non-negative by construction) arrives
                     and is stored to the data page, where the kernel
                     side compares it against echo6_fingerprint itself;
                     six callee-saved canaries survive the call;
                     debug_write returns the exact byte count (and the
                     kernel-side capture buffer holds the message);
                     an out-of-region buffer answers -3, len=0 and
                     oversized len answer -2, an unknown number answers
                     -1 — every typed status OBSERVED IN RING 3 (the
                     payload exits 43 on any violation and only 42
                     passes); dispatcher accounting exact; frames exact.
  * thread_exit_abi — a payload exits with a full-width 64-bit code
                     (0xBEEFC0DE0042): recorded faithfully, the thread
                     reaped through the scheduler (live count back to
                     baseline), dispatch counted once, frames exact.

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402

EXPECTED_TESTS = [
    "elf_parse",
    "elf_reject",
    "elf_load",
    "syscall_abi",
    "thread_exit_abi",
]

if __name__ == "__main__":
    sys.exit(mtest.run_milestone("m4", EXPECTED_TESTS))
