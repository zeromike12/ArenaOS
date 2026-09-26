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

  M4.3 — first user process (the milestone capstone):
  * first_process — the real rust-lld image is loaded into its own
                     address space (proc::create + elf::load + a stack
                     leaf at 0x203000 — 8 frames) and RUNS in ring 3
                     via spawn_with_cr3 + enter_user at e_entry. The
                     payload itself verifies META's magic and the
                     zero-filled bss from the user side, stamps bss
                     slot 0, writes its linker-pinned message through
                     debug_write, and exits with META.exit_ok (42;
                     43/44/45/99 are its diagnostic codes). The kernel
                     derives every expectation from the image FILE:
                     the captured console bytes must equal the pinned
                     message as stored in the file, the bss stamp must
                     read back under the process CR3 as META_MAGIC's
                     first 8 bytes (LE), call accounting exact, thread
                     reaped, destroy reclaims everything.

  M4.4 — IPC v1 (ADR-0018: endpoints, sync call/reply, badged merged
  notifications, cap transfer in messages; registry slots 7-11):
  * ipc_echo       — the echo-server demo across TWO real processes:
                     the server parks in ipc_recv (Blocked state), the
                     client's ipc_call delivers two pattern words plus
                     its Memory cap and blocks; the server echoes both
                     words via ipc_reply (the client verifies the echo
                     IN RING 3), then badges the client via notify; the
                     client's wait takes the badge on the immediate
                     path. Both exit 42 only if every ring-3 check
                     held (43-47 client / 53-56 server diagnostics
                     name any failure). Kernel-side: dispatch and IPC
                     counters exact (7 syscalls, 2 blocking events),
                     the cap landed in the server's space with rights
                     intact while the sender kept its original (copy,
                     never move), threads reaped, objects destroyed
                     without refusal, frames exact.

  M4.5 — spawn protocol (ADR-0019: SYS_SPAWN registry slot 12, image
  capabilities, explicit attenuating inheritance, exit notifications):
  M4.6 — console input service (ADR-0020: COM1 RX line discipline on
  IRQ4/vector 33, SYS_CONSOLE_READ registry slot 13):
  * console_line   — the line discipline driven through the SAME feed()
                     the RX ISR calls: backspace edits, empty lines
                     never queued, truncation counted, queue overflow
                     drops the OLDEST line; a ring-3 reader parks on
                     the empty queue and a fed line wakes it (length +
                     bytes verified IN RING 3); the shell image
                     validated as spawn-registry image 1 — the boot
                     sequence spawns it as the initial service the
                     moment the suite passes (the interactive session
                     itself is tools/test_m4_shell.py's job).

  * spawn_restart  — the supervisor restart demo across THREE real
                     address spaces: a ring-3 supervisor writes its
                     inheritance spec to its data page, spawns the
                     untouched M4.3 image through SYS_SPAWN (its
                     notification cap + a badge lent to the child's
                     exit), waits for the badge, then RESTARTS: spawns
                     again and waits again. The child's console message
                     appearing TWICE is the restart, visible on the
                     wire. The supervisor exits 42 only if both lives
                     came back badged (73-76 name the failed step).
                     Kernel-side: the spawn registry witnesses both
                     children exited with the image's own success code,
                     the inherited Memory cap landed attenuated
                     (exactly READ|COPY) at each child's slot 0, the
                     parent's Process handles (READ|DESTROY) landed in
                     spawn order, the parent kept its original (copy,
                     not move), counters exact (9 dispatches, 2 exit
                     notifications, 2 parked waits), threads reaped,
                     frames exact.

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
    "first_process",
    "ipc_echo",
    "spawn_restart",
    "console_line",
]

if __name__ == "__main__":
    sys.exit(mtest.run_milestone("m4", EXPECTED_TESTS))
