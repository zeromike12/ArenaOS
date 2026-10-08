#!/usr/bin/env python3
"""ADR-0048: source-extracted host/bare-metal cap and IPC table layout.

Static size is not a live frame/leak proof. No kernel/guest source is edited.
"""
from pathlib import Path
import re
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def declaration(source: str, start: str) -> str:
    at = source.index(start)
    brace = source.index("{", at)
    depth = 1
    end = brace + 1
    while depth:
        if source[end] == "{":
            depth += 1
        elif source[end] == "}":
            depth -= 1
        end += 1
    return source[at:end]


def main() -> None:
    cap = (ROOT / "kernel/kernel/src/cap.rs").read_text()
    proc = (ROOT / "kernel/kernel/src/proc.rs").read_text()
    obj = declaration(cap, "#[derive(Clone, Copy, PartialEq, Eq)]\npub enum CapObj")
    item = declaration(cap, "#[derive(Clone, Copy, PartialEq, Eq)]\npub struct Cap")
    space = declaration(cap, "#[derive(Clone, Copy)]\npub struct CapSpace")
    user_regions = re.search(r"pub const USER_REGIONS_MAX: usize = (\d+);", proc)
    if user_regions is None:
        raise ValueError("process-owned user mapping bound missing from source")
    user_regions_decl = f"pub const USER_REGIONS_MAX: usize = {user_regions[1]};"
    process = declaration(proc, "#[derive(Clone, Copy)]\npub struct Process")
    # Only substitute the length parameter and its type use. All fields,
    # discriminants and Rust repr/alignment remain the production source.
    space = space.replace("pub struct CapSpace {", "pub struct CapSpace<const N: usize> {")
    space = space.replace("CAP_SLOTS", "N")
    process = process.replace("pub struct Process {", "pub struct Process<const N: usize> {")
    process = process.replace("cap::CapSpace", "CapSpace<N>")
    source = "#![allow(dead_code)]\nuse std::mem::size_of;\n" + "\n".join(
        (obj, item, space, user_regions_decl, process)) + """
fn print_sizes<const N: usize>() {
    println!("{N}: CapSpace={} Process={} Option<Process>={} 32-table={}",
        size_of::<CapSpace<N>>(), size_of::<Process<N>>(),
        size_of::<Option<Process<N>>>(),
        32 * size_of::<Option<Process<N>>>());
}
fn main() {
    println!("CapObj={} Cap={}", size_of::<CapObj>(), size_of::<Cap>());
    print_sizes::<16>(); print_sizes::<18>(); print_sizes::<32>(); print_sizes::<64>(); print_sizes::<128>();
}
"""
    with tempfile.TemporaryDirectory(prefix="cap-layout-") as work:
        src = Path(work) / "probe.rs"
        binary = Path(work) / "probe"
        src.write_text(source)
        subprocess.run(["rustc", "--edition=2024", str(src), "-o", str(binary)], check=True)
        subprocess.run([str(binary)], check=True)

        # Compile the same *actual source definitions* against the guest's
        # no_std target. A constant array in LLVM IR exposes the target
        # layout without executing bare-metal code or altering the OS.
        sizes = """#[used]
#[unsafe(no_mangle)]
pub static ARENA_CAP_LAYOUT: [usize; 22] = [
 size_of::<CapObj>(), size_of::<Cap>(),
 size_of::<CapSpace<16>>(),size_of::<Process<16>>(),size_of::<Option<Process<16>>>(),32*size_of::<Option<Process<16>>>(),
 size_of::<CapSpace<18>>(),size_of::<Process<18>>(),size_of::<Option<Process<18>>>(),32*size_of::<Option<Process<18>>>(),
 size_of::<CapSpace<32>>(),size_of::<Process<32>>(),size_of::<Option<Process<32>>>(),32*size_of::<Option<Process<32>>>(),
 size_of::<CapSpace<64>>(),size_of::<Process<64>>(),size_of::<Option<Process<64>>>(),32*size_of::<Option<Process<64>>>(),
 size_of::<CapSpace<128>>(),size_of::<Process<128>>(),size_of::<Option<Process<128>>>(),64*size_of::<Option<Process<128>>>(),
];
"""
        target = Path(work) / "target.rs"
        ir = Path(work) / "target.ll"
        target.write_text("#![no_std]\n#![allow(dead_code)]\nuse core::mem::size_of;\n"
                          + "\n".join((obj, item, space, user_regions_decl, process)) + sizes)
        subprocess.run(["rustc", "--crate-type=lib", "--edition=2024", "-O",
                        "--target=x86_64-unknown-none", "--emit=llvm-ir",
                        str(target), "-o", str(ir)], check=True)
        match = re.search(r'@ARENA_CAP_LAYOUT = constant \[176 x i8\] c"([^"]+)"',
                          ir.read_text())
        if match is None:
            raise ValueError("guest layout constant missing from LLVM IR")
        encoded = match[1]
        data = bytearray()
        i = 0
        while i < len(encoded):
            if encoded[i] == "\\" and encoded[i + 1] == "\\":
                # LLVM prints byte 0x5C itself as an escaped backslash.
                data.append(0x5C)
                i += 2
            elif encoded[i] == "\\":
                data.append(int(encoded[i + 1:i + 3], 16))
                i += 3
            else:
                data.append(ord(encoded[i]))
                i += 1
        actual = struct.unpack("<22Q", data)
        # The historical 16/18/32/64 measurements remain in the first 18
        # fields. Phase 13 adds the process-owned user-region inventory to
        # every process record; its exact bound is extracted from proc.rs.
        # ADR-0088's 128-slot CapSpace and 64-process table remain the final
        # four fields and are measured from the actual source structs.
        expected = (16, 24, 400, 1744, 1744, 55808, 456, 1800, 1800,
                    57600, 800, 2144, 2144, 68608, 1600, 2944, 2944, 94208,
                    3200, 4544, 4544, 290816)
        if actual != expected:
            raise ValueError(f"on-target layout changed: {actual!r} != {expected!r}")
        if not re.search(r"pub const CAP_SLOTS: usize = 128;", cap):
            raise ValueError("production capability table is not the reviewed 128 slots")
        if not re.search(r"pub const MAX_PROCESSES: usize = 64;", proc):
            raise ValueError("production process table is not the reviewed 64 slots")
        print("x86_64-unknown-none target cap layout: CapSpace<128>=3200 B, "
              "Option<Process<128>>=4544 B with 80 process-owned mapping bounds, "
              "64-process table=290816 B (ADR-0088/0095) PASS")

        ipc = (ROOT / "kernel/kernel/src/ipc.rs").read_text()
        ipc_decls = [declaration(ipc, start) for start in (
            "#[derive(Clone, Copy, PartialEq, Eq)]\nenum SlotState",
            "#[derive(Clone, Copy)]\nstruct CallSlot",
            "#[derive(Clone, Copy)]\nstruct Endpoint",
            "#[derive(Clone, Copy)]\nstruct Notif",
            # ADR-0071: the endpoint's single bound-notification record.
            "#[derive(Clone, Copy, PartialEq, Eq)]\nstruct Binding",
        )]
        depth = re.search(r"const QUEUE_DEPTH: usize = (\d+);", ipc)
        width = re.search(r"const MSG_BYTES: usize = (\d+);", ipc)
        if depth is None or width is None:
            raise ValueError("IPC queue/message bounds missing from source")
        # ADR-0075 raised the queue to 16 for twelve Phase-11 clients;
        # ADR-0089 raises it to 32 for a simultaneous Phase-12 startup burst.
        # ADR-0077 set filesd's endpoint bound to 16.
        if depth[1] != '32' or width[1] != '64' or not re.search(r'pub const MAX_ENDPOINTS: usize = 16;',ipc):
            raise ValueError('production IPC bounds differ from reviewed Phase-12 32/64/16')
        # Keep the historical four-entry projection as well as measuring the
        # actual eight-entry production layout from the same source fields.
        ipc_decls[2] = ipc_decls[2].replace('struct Endpoint {','struct Endpoint<const N: usize> {').replace('QUEUE_DEPTH','N')
        ipc_source = ("#![no_std]\n#![allow(dead_code)]\n"
                      f"const QUEUE_DEPTH: usize = {depth[1]};\n"
                      f"const MSG_BYTES: usize = {width[1]};\n"
                      "use core::mem::size_of;\n"
                      + "\n".join((obj, item, *ipc_decls)) + """
#[used]
#[unsafe(no_mangle)]
pub static ARENA_IPC_LAYOUT: [usize; 8] = [
 size_of::<CallSlot>(),size_of::<Endpoint<4>>(),size_of::<Endpoint<QUEUE_DEPTH>>(),size_of::<Notif>(),
 8*size_of::<Endpoint<QUEUE_DEPTH>>(),9*size_of::<Endpoint<QUEUE_DEPTH>>(),
 16*size_of::<Endpoint<QUEUE_DEPTH>>(),64*size_of::<Notif>()
];
""")
        ipc_target = Path(work) / "ipc.rs"
        ipc_ir = Path(work) / "ipc.ll"
        ipc_target.write_text(ipc_source)
        subprocess.run(["rustc", "--crate-type=lib", "--edition=2024", "-O",
                        "--target=x86_64-unknown-none", "--emit=llvm-ir",
                        str(ipc_target), "-o", str(ipc_ir)], check=True)
        match = re.search(r'@ARENA_IPC_LAYOUT = constant \[64 x i8\] c"([^"]+)"',
                          ipc_ir.read_text())
        if match is None:
            raise ValueError("guest IPC layout constant missing from LLVM IR")
        encoded = match[1]
        data = bytearray()
        i = 0
        while i < len(encoded):
            if encoded[i] == "\\" and encoded[i + 1] == "\\":
                # LLVM prints byte 0x5C itself as an escaped backslash.
                data.append(0x5C)
                i += 2
            elif encoded[i] == "\\":
                data.append(int(encoded[i + 1:i + 3], 16))
                i += 3
            else:
                data.append(ord(encoded[i]))
                i += 1
        ipc_sizes = struct.unpack("<8Q", data)
        # ADR-0071/75/77 retain the Phase-11 endpoint and message layouts.
        # ADR-0088 raises notifications; ADR-0089 doubles only queue depth.
        if ipc_sizes != (240, 1008, 7728, 32, 61824, 69552, 123648, 2048):
            raise ValueError(f"on-target IPC layout changed: {ipc_sizes!r}")
        print("x86_64-unknown-none IPC: CallSlot=240 Endpoint<32>=7728 Notif=32; "
              "16 endpoints=123648 B, 64 notifications=2048 B (ADR-0089) PASS")
        # ADR-0051: an *additional* distinct production-fsd marker,
        # on top of ADR-0048's projection; one Notification is 24 B.
        actual = (ROOT / "kernel/kernel/src/ipc.rs").read_text()
        if not re.search(r"pub const MAX_NOTIFS: usize = 64\s*;", actual):
            raise ValueError("production notification bound not exactly 64")
        print("Phase-12 fixed notification table: 64 objects (32 client clocks plus system budget) PASS")


if __name__ == "__main__":
    main()
