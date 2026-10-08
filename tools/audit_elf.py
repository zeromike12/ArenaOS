#!/usr/bin/env python3
"""Deterministic ELF program header and loader budget audit tool for ArenaOS.

Validates executables against production kernel loader rules defined in:
  - kernel/kernel/src/elf.rs
  - kernel/kernel/src/image_registry.rs

Enforces:
  1. File size <= MAX_BYTES (262,144 bytes / 256 KiB)
  2. Executable format: ELF64, Little-Endian, ET_EXEC, EM_X86_64
  3. No dynamic interpreter: PT_INTERP rejected
  4. W^X compliance: No PT_LOAD segment with both W and X permissions
  5. Segment alignment: p_vaddr must be 4096-byte aligned
  6. Address limits: Segments within user half (0x0000_8000_0000_0000)
  7. Entry point: Must be strictly inside an executable (PF_X) segment
  8. Total load pages <= MAX_LOAD_PAGES (128 pages / 512 KiB)
     calculated using the kernel's exact rounding algorithm.
  9. Segments: Distinguishes file-backed data from static .bss.

Usage:
  python3 tools/audit_elf.py <elf_path> [--json]
"""

import argparse
import hashlib
import json
import struct
import sys
from pathlib import Path

MAX_BYTES = 262144
MAX_LOAD_PAGES = 128
PAGE_SIZE = 4096
USER_HALF_LIMIT = 0x0000800000000000

ET_EXEC = 2
EM_X86_64 = 62
PT_LOAD = 1
PT_INTERP = 3

PF_X = 1
PF_W = 2
PF_R = 4
PF_KNOWN = PF_R | PF_W | PF_X


def audit_elf(file_path: Path) -> dict:
    with open(file_path, "rb") as f:
        data = f.read()

    file_size = len(data)
    digest = hashlib.sha256(data).hexdigest()

    errors = []

    if file_size > MAX_BYTES:
        errors.append(f"Image size {file_size} bytes exceeds MAX_BYTES ({MAX_BYTES})")

    if file_size < 64:
        errors.append("File shorter than ELF header")
        return {"file": str(file_path), "pass": False, "errors": errors}

    if data[:4] != b"\x7fELF":
        errors.append("Invalid ELF magic")
        return {"file": str(file_path), "pass": False, "errors": errors}

    ei_class = data[4]
    ei_data = data[5]
    ei_version = data[6]

    if ei_class != 2:
        errors.append(f"Not ELFCLASS64 (ei_class={ei_class})")
    if ei_data != 1:
        errors.append(f"Not little-endian (ei_data={ei_data})")
    if ei_version != 1:
        errors.append(f"Bad ei_version ({ei_version})")

    e_type, e_machine, e_version, e_entry, e_phoff, e_shoff, e_flags, e_ehsize, e_phentsize, e_phnum = struct.unpack_from(
        "<HHIQQQIHHH", data, 16
    )

    if e_type != ET_EXEC:
        errors.append(f"Only ET_EXEC static images accepted (e_type={e_type})")
    if e_machine != EM_X86_64:
        errors.append(f"Not EM_X86_64 (e_machine={e_machine})")
    if e_version != 1:
        errors.append(f"Bad e_version ({e_version})")
    if e_ehsize != 64:
        errors.append(f"e_ehsize must be 64 (got {e_ehsize})")
    if e_phentsize != 56:
        errors.append(f"e_phentsize must be 56 (got {e_phentsize})")
    if not (1 <= e_phnum <= 8):
        errors.append(f"e_phnum outside 1..=8 (got {e_phnum})")

    phend = e_phoff + e_phnum * 56
    if phend > file_size:
        errors.append("Program header table extends past end of file")

    segments = []
    load_segments = []
    total_load_pages = 0
    entry_in_x = False

    for i in range(e_phnum):
        off = e_phoff + i * 56
        p_type, p_flags, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz, p_align = struct.unpack_from(
            "<IIQQQQQQ", data, off
        )

        seg_info = {
            "index": i,
            "type": p_type,
            "flags": p_flags,
            "offset": p_offset,
            "vaddr": p_vaddr,
            "filesz": p_filesz,
            "memsz": p_memsz,
            "align": p_align,
        }
        segments.append(seg_info)

        if p_type == PT_INTERP:
            errors.append("PT_INTERP rejected: no dynamic images allowed")
            continue

        if p_type != PT_LOAD:
            continue

        # W^X audit
        if (p_flags & PF_W) and (p_flags & PF_X):
            errors.append(f"Segment {i} violates W^X: both writable and executable")

        if p_flags & ~PF_KNOWN:
            errors.append(f"Segment {i} contains unknown flags: 0x{p_flags:x}")

        if p_memsz == 0:
            errors.append(f"Segment {i} has zero memsz")

        if p_filesz > p_memsz:
            errors.append(f"Segment {i} filesz ({p_filesz}) exceeds memsz ({p_memsz})")

        if p_vaddr % PAGE_SIZE != 0:
            errors.append(f"Segment {i} vaddr 0x{p_vaddr:x} is not page aligned")

        if p_vaddr >= USER_HALF_LIMIT or (p_vaddr + p_memsz) > USER_HALF_LIMIT:
            errors.append(f"Segment {i} spans into kernel space")

        if (p_offset + p_filesz) > file_size:
            errors.append(f"Segment {i} file data extends past end of image")

        # Kernel load page calculation algorithm (image_registry.rs):
        # end = vaddr + memsz
        # round = (end + 4095) & !4095
        # span = round - vaddr
        # pages += span / 4096
        end = p_vaddr + p_memsz
        round_up = (end + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1)
        span = round_up - p_vaddr
        if span % PAGE_SIZE != 0:
            errors.append(f"Segment {i} span not page aligned")
        seg_pages = span // PAGE_SIZE
        total_load_pages += seg_pages

        flag_str = ("R" if p_flags & PF_R else "") + ("W" if p_flags & PF_W else "") + ("X" if p_flags & PF_X else "")
        load_seg = {
            "index": i,
            "vaddr": hex(p_vaddr),
            "filesz": p_filesz,
            "memsz": p_memsz,
            "bss": p_memsz - p_filesz,
            "flags": flag_str,
            "pages": seg_pages,
        }
        load_segments.append(load_seg)

        if (p_flags & PF_X) and (p_vaddr <= e_entry < p_vaddr + p_memsz):
            entry_in_x = True

    if not load_segments:
        errors.append("No PT_LOAD segments found")

    if not entry_in_x:
        errors.append(f"Entry point 0x{e_entry:x} is not inside an executable (PF_X) segment")

    if total_load_pages > MAX_LOAD_PAGES:
        errors.append(f"Total load pages {total_load_pages} exceeds MAX_LOAD_PAGES ({MAX_LOAD_PAGES})")

    # Segment overlap check
    for a in range(len(load_segments)):
        for b in range(a + 1, len(load_segments)):
            va_a = int(load_segments[a]["vaddr"], 16)
            end_a = va_a + load_segments[a]["memsz"]
            va_b = int(load_segments[b]["vaddr"], 16)
            end_b = va_b + load_segments[b]["memsz"]
            if va_a < end_b and va_b < end_a:
                errors.append(f"Load segments {a} and {b} overlap in virtual address space")

    is_pass = len(errors) == 0

    return {
        "file": str(file_path),
        "sha256": digest,
        "file_size": file_size,
        "file_size_budget": MAX_BYTES,
        "file_size_headroom": MAX_BYTES - file_size,
        "entry_point": hex(e_entry),
        "total_load_pages": total_load_pages,
        "load_page_budget": MAX_LOAD_PAGES,
        "load_page_headroom": MAX_LOAD_PAGES - total_load_pages,
        "load_segments": load_segments,
        "pass": is_pass,
        "errors": errors,
    }


def main():
    parser = argparse.ArgumentParser(description="Deterministic ELF program header audit for ArenaOS.")
    parser.add_argument("elf", type=Path, help="Path to ELF executable")
    parser.add_argument("--json", action="store_true", help="Output audit report as JSON")
    args = parser.parse_args()

    if not args.elf.exists():
        print(f"Error: File '{args.elf}' not found.", file=sys.stderr)
        sys.exit(1)

    result = audit_elf(args.elf)

    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print("=" * 72)
        print(f"ArenaOS ELF Loader Audit: {result['file']}")
        print("=" * 72)
        print(f"SHA-256 Digest:       {result['sha256']}")
        print(f"File Size:            {result['file_size']} bytes (Budget: {result['file_size_budget']} B, Headroom: {result['file_size_headroom']} B)")
        print(f"Entry Point:          {result['entry_point']}")
        print(f"Total Load Pages:     {result['total_load_pages']} pages (Budget: {result['load_page_budget']} pages, Headroom: {result['load_page_headroom']} pages)")
        print(f"Load Segments ({len(result['load_segments'])}):")
        for seg in result["load_segments"]:
            print(f"  [{seg['index']}] VA={seg['vaddr']} FileSiz={seg['filesz']:6d} MemSiz={seg['memsz']:6d} ({seg['flags']:<3}) BSS={seg['bss']:6d} -> {seg['pages']:2d} pages")
        print("-" * 72)
        if result["pass"]:
            print("AUDIT VERDICT: PASS (Conforms to ArenaOS kernel loader constraints)")
        else:
            print("AUDIT VERDICT: FAIL")
            for err in result["errors"]:
                print(f"  ERROR: {err}")
        print("=" * 72)

    sys.exit(0 if result["pass"] else 1)


if __name__ == "__main__":
    main()
