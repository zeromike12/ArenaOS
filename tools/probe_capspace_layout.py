#!/usr/bin/env python3
"""ADR-0048: measure real Cap/CapSpace/Process declarations at 16/18/32 slots.

Host x86_64 Rust layout estimate only; guest frame accounting and full-table
proofs are separate implementation gates. No kernel or guest file is edited.
"""
from pathlib import Path
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
    process = declaration(proc, "#[derive(Clone, Copy)]\npub struct Process")
    # Only substitute the length parameter and its type use. All fields,
    # discriminants and Rust repr/alignment remain the production source.
    space = space.replace("pub struct CapSpace {", "pub struct CapSpace<const N: usize> {")
    space = space.replace("CAP_SLOTS", "N")
    process = process.replace("pub struct Process {", "pub struct Process<const N: usize> {")
    process = process.replace("cap::CapSpace", "CapSpace<N>")
    source = "#![allow(dead_code)]\nuse std::mem::size_of;\n" + "\n".join(
        (obj, item, space, process)) + """
fn print_sizes<const N: usize>() {
    println!("{N}: CapSpace={} Process={} Option<Process>={} 32-table={}",
        size_of::<CapSpace<N>>(), size_of::<Process<N>>(),
        size_of::<Option<Process<N>>>(),
        32 * size_of::<Option<Process<N>>>());
}
fn main() {
    println!("CapObj={} Cap={}", size_of::<CapObj>(), size_of::<Cap>());
    print_sizes::<16>(); print_sizes::<18>(); print_sizes::<32>();
}
"""
    with tempfile.TemporaryDirectory(prefix="cap-layout-") as work:
        src = Path(work) / "probe.rs"
        binary = Path(work) / "probe"
        src.write_text(source)
        subprocess.run(["rustc", "--edition=2024", str(src), "-o", str(binary)], check=True)
        subprocess.run([str(binary)], check=True)


if __name__ == "__main__":
    main()
