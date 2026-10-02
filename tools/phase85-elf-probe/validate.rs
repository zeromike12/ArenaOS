// Host-only adapter for the UNMODIFIED production kernel ELF validator.
// The actual elf.rs include_bytes! registry needs the existing built
// userspace images; `tools/build.sh --image` produces them first. Stubs are
// never invoked: the test calls elf::validate only, not load/spawn.
#![allow(dead_code)]
use std::path::PathBuf;
mod arch { pub mod x86_64 { pub mod paging {
    pub const PAGE: u64 = 4096;
    pub const KERNEL_OFFSET: u64 = 0xffff_8000_0000_0000;
    pub unsafe fn user_pte_flags(_: u64, _: u64) -> Option<u64> { panic!("not called") }
    pub unsafe fn map_user_page_4k(_: u64, _: u64, _: u64, _: bool, _: bool)
        -> Result<(), &'static str> { panic!("not called") }
} } }
mod frames {
    pub fn alloc() -> Option<u64> { panic!("not called") }
    pub fn free(_: u64) -> Result<(), &'static str> { panic!("not called") }
}
mod proc { pub fn pml4_of(_: u64) -> Option<u64> { panic!("not called") } }
mod sync { pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R { f() } }
#[path = "../../kernel/kernel/src/elf.rs"]
mod elf;
fn main() {
    let p = PathBuf::from(std::env::args_os().nth(1).expect("ELF path"));
    let bytes = std::fs::read(&p).expect("compiled ELF file");
    assert!(!bytes.is_empty() && bytes.len() <= 4096,
            "APKG v1 payload bound: {} bytes", bytes.len());
    let info = elf::validate(&bytes).expect("production elf::validate refused probe");
    assert_eq!(info.nsegs, 1, "single RX segment, no RW image authority");
    let text = info.segs[0];
    assert_eq!(text.flags, elf::PF_R | elf::PF_X);
    assert_eq!(text.offset, 0, "ELF headers included in bounded PT_LOAD");
    assert!(info.entry >= text.vaddr && info.entry < text.vaddr + text.memsz);
    assert_eq!(text.filesz, text.memsz);
    let mut bad = bytes.clone();
    bad[68..72].copy_from_slice(&7u32.to_le_bytes()); // W+X PT_LOAD
    assert!(elf::validate(&bad).is_err(), "W+X was accepted");
    bad = bytes.clone();
    bad[24..32].copy_from_slice(&0x500000u64.to_le_bytes()); // entry outside RX
    assert!(elf::validate(&bad).is_err(), "foreign entry was accepted");
    assert!(elf::validate(&bytes[..120]).is_err(), "truncated segment was accepted");
    println!("production elf::validate: PASS; payload={} bytes <=4096, entry={:#x}, PT_LOAD offset={} filesz={} memsz={} RX; W+X/entry/truncation refused",
             bytes.len(), info.entry, text.offset, text.filesz, text.memsz);
}
