#!/usr/bin/env python3
"""Fail-closed instruction audit for the ArenaOS EFI kernel image.

The release PE omits its COFF symbol table. A retained, fixed-layout table in
.rdata names the exact audited architecture-leaf ranges. This script parses
real `llvm-objdump` instruction records, verifies complete coverage of every
executable section, checks every x87/MMX/SSE/AVX
instruction against those ranges, and enforces the reviewed instruction
counts inside each range.
"""
from __future__ import annotations

import argparse
import collections
import re
import struct
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

MAGIC = b"ARFP15M1"
TABLE_VERSION = 1
RANGE_NAMES = (
    "context_switch",
    "probe",
    "build_clean",
    "install_clean",
    "read_xcr0",
    "write_xcr0",
)

EXPECTED = (
    collections.Counter(
        {
            "fxsave64": 1,
            "fxrstor64": 1,
            "push": 8,
            "pushfq": 1,
            "pop": 8,
            "popfq": 1,
            "mov": 2,
            "ret": 1,
        }
    ),
    collections.Counter(
        {
            "sub": 1,
            "fxsave64": 2,
            "fninit": 2,
            "fldz": 8,
            "fxrstor64": 1,
            "add": 1,
            "ret": 1,
        }
    ),
    collections.Counter(
        {
            "sub": 1,
            "fxsave64": 2,
            "fninit": 2,
            "fldz": 8,
            "ldmxcsr": 1,
            "pxor": 16,
            "fxrstor64": 1,
            "add": 1,
            "ret": 1,
        }
    ),
    collections.Counter({"fxrstor64": 1, "ret": 1}),
    collections.Counter({"xor": 1, "xgetbv": 1, "shl": 1, "or": 1, "ret": 1}),
    collections.Counter({"xor": 1, "mov": 2, "shr": 1, "xsetbv": 1, "ret": 1}),
)

ALLOWED = tuple(frozenset(rule) for rule in EXPECTED)
INSN_RE = re.compile(r"^\s*([0-9a-fA-F]+):\s*([A-Za-z][A-Za-z0-9.]*)\s*(.*?)\s*$")
RAW_INSN_RE = re.compile(
    r"^\s*([0-9a-fA-F]+):\s*((?:[0-9a-fA-F]{2}\s+)+)"
    r"([A-Za-z][A-Za-z0-9.]*)\s*(.*?)\s*$"
)
VECTOR_REG_RE = re.compile(r"\b(?:xmm|ymm|zmm|mm)[0-9]+\b", re.IGNORECASE)


@dataclass(frozen=True)
class Instruction:
    address: int
    mnemonic: str
    operands: str
    size: int = 0


@dataclass(frozen=True)
class AddressRange:
    name: str
    start: int
    end: int


def parse_instructions(disassembly: str) -> list[Instruction]:
    instructions: list[Instruction] = []
    for line in disassembly.splitlines():
        # llvm-objdump adds a `# address <label>` comment to RIP-relative
        # operands. Comments and symbol labels are not machine instructions.
        line = line.split("#", 1)[0]
        raw_match = RAW_INSN_RE.match(line)
        if raw_match:
            instructions.append(
                Instruction(
                    int(raw_match.group(1), 16),
                    raw_match.group(3).lower(),
                    raw_match.group(4).strip().lower(),
                    len(raw_match.group(2).split()),
                )
            )
            continue
        match = INSN_RE.match(line)
        if match:
            instructions.append(
                Instruction(
                    int(match.group(1), 16),
                    match.group(2).lower(),
                    match.group(3).strip().lower(),
                )
            )
    return instructions


def is_fp_simd_instruction(instruction: Instruction) -> bool:
    mnemonic = instruction.mnemonic
    if VECTOR_REG_RE.search(instruction.operands):
        return True
    if mnemonic.startswith("f") or mnemonic.startswith("v"):
        return True
    return mnemonic in {
        "emms",
        "ldmxcsr",
        "stmxcsr",
        "lfence",
        "mfence",
        "sfence",
        "xgetbv",
        "xsetbv",
    }


def locate_range(address: int, ranges: tuple[AddressRange, ...]) -> int | None:
    matches = [i for i, span in enumerate(ranges) if span.start <= address < span.end]
    if len(matches) > 1:
        raise ValueError(f"overlapping trusted ranges at 0x{address:x}")
    return matches[0] if matches else None


def check_instruction(instruction: Instruction, ranges: tuple[AddressRange, ...]) -> None:
    if not is_fp_simd_instruction(instruction):
        return
    index = locate_range(instruction.address, ranges)
    if index is None:
        raise ValueError(
            f"unapproved FP/SIMD instruction at 0x{instruction.address:x}: "
            f"{instruction.mnemonic} {instruction.operands}".rstrip()
        )
    if instruction.mnemonic not in ALLOWED[index]:
        raise ValueError(
            f"unapproved instruction in {ranges[index].name} leaf at "
            f"0x{instruction.address:x}: {instruction.mnemonic} {instruction.operands}".rstrip()
        )


def parse_pe_ranges(
    image: bytes,
) -> tuple[tuple[AddressRange, ...], tuple[AddressRange, ...]]:
    if len(image) < 0x40 or image[:2] != b"MZ":
        raise ValueError("input is not a PE image")
    pe_offset = struct.unpack_from("<I", image, 0x3C)[0]
    if pe_offset + 24 > len(image) or image[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise ValueError("invalid PE signature")
    machine, section_count = struct.unpack_from("<HH", image, pe_offset + 4)
    if machine != 0x8664:
        raise ValueError(f"expected PE x86-64 machine, got 0x{machine:04x}")
    optional_size = struct.unpack_from("<H", image, pe_offset + 20)[0]
    optional = pe_offset + 24
    if optional + optional_size > len(image) or struct.unpack_from("<H", image, optional)[0] != 0x20B:
        raise ValueError("expected a complete PE32+ optional header")
    image_base = struct.unpack_from("<Q", image, optional + 24)[0]
    section_headers = optional + optional_size
    sections: list[tuple[str, int, int, int, int]] = []
    executable_sections: list[AddressRange] = []
    for index in range(section_count):
        offset = section_headers + index * 40
        if offset + 40 > len(image):
            raise ValueError("truncated PE section table")
        raw_name = image[offset : offset + 8].split(b"\0", 1)[0]
        name = raw_name.decode("ascii", errors="replace")
        virtual_size, virtual_address, raw_size, raw_offset = struct.unpack_from("<IIII", image, offset + 8)
        characteristics = struct.unpack_from("<I", image, offset + 36)[0]
        if raw_offset + raw_size > len(image):
            raise ValueError(f"section {name} extends beyond the PE file")
        sections.append((name, virtual_address, virtual_size, raw_offset, raw_size))
        if characteristics & 0x20000000:  # IMAGE_SCN_MEM_EXECUTE
            if virtual_size == 0 or raw_size < virtual_size:
                raise ValueError(
                    f"executable section {name} has no complete file-backed virtual range"
                )
            start = image_base + virtual_address
            executable_sections.append(AddressRange(name, start, start + virtual_size))

    if not executable_sections:
        raise ValueError("PE image has no executable sections")
    executable_sections.sort(key=lambda span: span.start)
    if any(left.end > right.start for left, right in zip(executable_sections, executable_sections[1:])):
        raise ValueError("executable PE sections overlap")

    table_offset = None
    for name, _va, _vsize, raw_offset, raw_size in sections:
        if name not in (".rdata", ".data"):
            continue
        found = image.find(MAGIC, raw_offset, raw_offset + raw_size)
        if found >= 0:
            if table_offset is not None:
                raise ValueError("multiple FP audit tables found")
            table_offset = found
    if table_offset is None:
        raise ValueError("missing retained ARFP15M1 trusted-range table")
    header_end = table_offset + 16
    if header_end + 16 * len(RANGE_NAMES) > len(image):
        raise ValueError("truncated FP audit table")
    version, count = struct.unpack_from("<II", image, table_offset + 8)
    if version != TABLE_VERSION or count != len(RANGE_NAMES):
        raise ValueError(f"unsupported FP audit table version/count {version}/{count}")

    ranges: list[AddressRange] = []
    for index, name in enumerate(RANGE_NAMES):
        start, end = struct.unpack_from("<QQ", image, header_end + index * 16)
        # Some COFF linkers preserve the absolute VA (preferred); others
        # encode an RVA in data relocations. Normalize the latter.
        if start < image_base:
            start += image_base
        if end < image_base:
            end += image_base
        if start <= 0 or end <= start:
            raise ValueError(f"invalid {name} leaf range 0x{start:x}..0x{end:x}")
        if not any(
            section.name == ".text" and start >= section.start and end <= section.end
            for section in executable_sections
        ):
            raise ValueError(f"{name} leaf does not reside in executable .text")
        ranges.append(AddressRange(name, start, end))
    ordered = sorted(ranges, key=lambda span: span.start)
    if any(left.end > right.start for left, right in zip(ordered, ordered[1:])):
        raise ValueError("trusted FP/SIMD leaf ranges overlap")
    return tuple(ranges), tuple(executable_sections)


def audit(
    disassembly: str,
    ranges: tuple[AddressRange, ...],
    executable_sections: tuple[AddressRange, ...],
) -> int:
    if len(ranges) != len(RANGE_NAMES):
        raise ValueError("trusted-range table does not contain all required leaves")
    if not executable_sections:
        raise ValueError("PE image has no executable sections to audit")
    instructions = parse_instructions(disassembly)
    if not instructions:
        raise ValueError("disassembler produced no instruction records")
    for section in executable_sections:
        section_instructions = sorted(
            (
                instruction
                for instruction in instructions
                if section.start <= instruction.address < section.end
            ),
            key=lambda instruction: instruction.address,
        )
        if not section_instructions:
            raise ValueError(f"disassembly contains no instructions for executable {section.name}")
        cursor = section.start
        for instruction in section_instructions:
            if instruction.size <= 0:
                raise ValueError("disassembler omitted raw instruction bytes")
            if instruction.address != cursor:
                raise ValueError(f"disassembly gap in executable {section.name} at 0x{cursor:x}")
            cursor = instruction.address + instruction.size
            if cursor > section.end:
                raise ValueError(
                    f"instruction extends beyond executable {section.name} at 0x{instruction.address:x}"
                )
        if cursor != section.end:
            raise ValueError(f"disassembly ends before executable {section.name} at 0x{cursor:x}")

    counts = [collections.Counter() for _ in ranges]
    seen = [0 for _ in ranges]
    for instruction in instructions:
        if not any(
            section.start <= instruction.address < section.end
            for section in executable_sections
        ):
            raise ValueError(
                f"disassembler returned an instruction outside executable sections at 0x{instruction.address:x}"
            )
        index = locate_range(instruction.address, ranges)
        if index is not None:
            seen[index] += 1
            counts[index][instruction.mnemonic] += 1
        check_instruction(instruction, ranges)
    for index, name in enumerate(RANGE_NAMES):
        if not seen[index]:
            raise ValueError(f"disassembly contains no instructions for {name} leaf")
        if counts[index] != EXPECTED[index]:
            raise ValueError(
                f"{name} leaf instruction profile changed: expected {dict(EXPECTED[index])}, "
                f"found {dict(counts[index])}"
            )
    return len(instructions)


def self_test() -> None:
    span = (AddressRange("context_switch", 0x1000, 0x1010),)
    check_instruction(Instruction(0x1000, "fxsave64", "byte ptr [r8]"), span)
    check_instruction(Instruction(0x1001, "fxrstor64", "byte ptr [r9]"), span)
    check_instruction(Instruction(0x2000, "add", "rax, rbx"), span)
    for mnemonic, operands in (
        ("addps", "xmm0, xmm1"),
        ("vaddps", "ymm0, ymm1, ymm2"),
        ("fadd", "st(0), st(1)"),
        ("emms", ""),
        ("movq", "mm0, rax"),
        ("ldmxcsr", "dword ptr [rax]"),
    ):
        try:
            check_instruction(Instruction(0x2000, mnemonic, operands), span)
        except ValueError:
            pass
        else:
            raise AssertionError(f"negative ISA control was accepted: {mnemonic}")
    records = parse_instructions(
        "1000: fxsave64 byte ptr [r8] # comment mentions xmm3\n"
        "symbol named xmm4 is not an instruction\n"
    )
    if records != [Instruction(0x1000, "fxsave64", "byte ptr [r8]")]:
        raise AssertionError(f"instruction parser treated text/comment as code: {records!r}")
    raw = parse_instructions("1000: f3 0f 51 c0 sqrtss xmm0, xmm0\n")
    if raw != [Instruction(0x1000, "sqrtss", "xmm0, xmm0", 4)]:
        raise AssertionError(f"instruction parser lost raw-byte coverage: {raw!r}")
    empty_ranges = tuple(
        AddressRange(name, 0x1000 + index * 0x10, 0x1008 + index * 0x10)
        for index, name in enumerate(RANGE_NAMES)
    )
    try:
        audit("", empty_ranges, (AddressRange(".text", 0x1000, 0x1100),))
    except ValueError as error:
        if "no instruction records" not in str(error):
            raise
    else:
        raise AssertionError("empty disassembly passed the kernel ISA audit")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", type=Path)
    parser.add_argument("--objdump")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        self_test()
        print("kernel ISA audit negative controls: PASS")
        return 0
    if not args.image or not args.objdump:
        parser.error("--image and --objdump are required unless --self-test is used")
    try:
        ranges, executable_sections = parse_pe_ranges(args.image.read_bytes())
        completed = subprocess.run(
            [args.objdump, "-d", "--x86-asm-syntax=intel", str(args.image)],
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            raise ValueError(
                f"disassembler failed with {completed.returncode}: {completed.stderr.strip()}"
            )
        count = audit(completed.stdout, ranges, executable_sections)
    except (OSError, ValueError) as error:
        print(f"kernel ISA audit: FAIL: {error}", file=sys.stderr)
        return 1
    print(
        "kernel ISA audit: PASS; "
        f"{count} real instructions checked across {len(executable_sections)} fully decoded executable sections, six trusted leaf profiles matched, "
        "ordinary x87/MMX/SSE/AVX rejected"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
