//! Milestone 4 test suite — userspace images: the ELF64 strict-subset
//! validator and loader (M4.1, ADR-0016). Runs in `kmain` after the M3
//! RESULT line; every test is a real machine effect: the embedded
//! rust-lld artifact (`userspace/payload`) is parsed field by field and
//! cross-checked against its own self-describing META record, a
//! 25-mutation corpus proves every rejection rule in the ADR fires, and
//! a real process address space receives the image — entry-stub bytes,
//! META, and the zero-filled NOLOAD bss read back under the target's CR3
//! through STAC-bracketed accesses (SMAP is armed), PTE flags asserted
//! W^X-exact, double-load refused without allocation, and frame
//! accounting exact through `proc::destroy`.
//! Markers: `m4:test:<name>`, `m4: RESULT`.

use crate::arch::x86_64::{self, paging};
use crate::elf::{self, PF_R, PF_W, PF_X};
use crate::frames;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::proc;
use crate::sync::SyncCell;

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Result<(), &'static str>); 3] = [
        ("elf_parse", test_elf_parse),
        ("elf_reject", test_elf_reject),
        ("elf_load", test_elf_load),
    ];
    let mut passed = 0u32;
    for (name, test) in checks {
        match test() {
            Ok(()) => {
                passed += 1;
                write_marker(format_args!("m4:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("m4", "test {name} failed: {reason}");
                write_marker(format_args!("m4:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = checks.len() as u32;
    if passed == total {
        write_marker(format_args!("m4: RESULT PASS ({passed}/{total})"));
        true
    } else {
        write_marker(format_args!("m4: RESULT FAIL ({passed}/{total})"));
        false
    }
}

// ---- fixed payload layout (userspace/payload/payload.ld — the linker
// ---- script and these constants change together, ADR-0016) -----------

const PAYLOAD_ENTRY: u64 = 0x200000;
const PAYLOAD_META_VA: u64 = 0x201000;
const PAYLOAD_BSS_VA: u64 = 0x202000;
const PAYLOAD_BSS_LEN: u64 = 0x1000;
const META_MAGIC: &[u8; 16] = b"ARENAOS-PAYLOAD!";

/// The payload's `_start` stub is `jmp self` — two bytes, EB FE.
const ENTRY_STUB: [u8; 2] = [0xeb, 0xfe];

// ---- 1. parse: the real artifact, field by field ------------------------

/// Validate the embedded rust-lld artifact and cross-check the parsed
/// ELF header against the payload's own META manifest — two independent
/// descriptions of the same image that must agree.
fn test_elf_parse() -> Result<(), &'static str> {
    let img = elf::TEST_IMAGE;
    let parsed = elf::validate(img)?;

    if parsed.entry != PAYLOAD_ENTRY {
        return Err("e_entry does not match the linker script");
    }
    if parsed.nsegs != 2 {
        return Err("expected exactly two PT_LOAD segments");
    }
    let (text, data) = (&parsed.segs[0], &parsed.segs[1]);

    if text.vaddr != PAYLOAD_ENTRY || text.flags != PF_R | PF_X {
        return Err("text segment identity wrong (want 0x200000 R+X)");
    }
    if text.memsz < ENTRY_STUB.len() as u64 || text.filesz > text.memsz {
        return Err("text segment sizes wrong");
    }
    if data.vaddr != PAYLOAD_META_VA || data.flags != PF_R | PF_W {
        return Err("data segment identity wrong (want 0x201000 R+W)");
    }
    // The data segment is file-backed for META only; the bss canary is
    // NOLOAD, so filesz MUST be strictly below memsz — that gap is the
    // loader's zero-fill contract.
    if data.filesz < 40 || data.filesz >= data.memsz {
        return Err("data segment must be partly file-backed with NOLOAD bss beyond");
    }
    if data.memsz < PAYLOAD_BSS_VA + PAYLOAD_BSS_LEN - data.vaddr {
        return Err("data segment does not cover the bss canary");
    }

    // META cross-check, read straight out of the image file at the data
    // segment's offset (META is its first content by linker script).
    let mo = data.offset as usize;
    if img.len() < mo + 40 {
        return Err("data segment offset does not leave room for META");
    }
    if &img[mo..mo + 16] != META_MAGIC {
        return Err("payload META magic mismatch");
    }
    let m_entry = le64_at(img, mo + 16);
    let m_bss_va = le64_at(img, mo + 24);
    let m_bss_len = le64_at(img, mo + 32);
    if m_entry != parsed.entry {
        return Err("META.entry disagrees with the parsed e_entry");
    }
    if m_bss_va != PAYLOAD_BSS_VA || m_bss_len != PAYLOAD_BSS_LEN {
        return Err("META bss facts disagree with the linker script");
    }
    if m_bss_va < data.vaddr || m_bss_va + m_bss_len > data.vaddr + data.memsz {
        return Err("META bss span lies outside its own segment");
    }

    info!(
        "m4",
        "elf_parse: {}-byte rust-lld artifact — ET_EXEC entry {:#x}, {} PT_LOADs: text RX {:#x}+{:#x}, data RW {:#x}+{:#x} (filesz {:#x} < memsz: NOLOAD bss), META manifest cross-check agrees",
        img.len(),
        parsed.entry,
        parsed.nsegs,
        text.vaddr,
        text.memsz,
        data.vaddr,
        data.memsz,
        data.filesz
    );
    Ok(())
}

// ---- 2. reject: the mutation corpus -------------------------------------

/// Scratch space for mutated image copies (single CPU, IF=0 — a plain
/// SyncCell is the house pattern for test statics).
static SCRATCH: SyncCell<[u8; 16 * 1024]> = SyncCell::new([0; 16 * 1024]);

/// Copy the pristine image into `scratch`, apply one mutation, and
/// require that validation refuses the result.
fn mutate_reject(
    img: &[u8],
    scratch: &mut [u8],
    what: &'static str,
    mutate: impl FnOnce(&mut [u8]),
) -> Result<(), &'static str> {
    scratch[..img.len()].copy_from_slice(img);
    mutate(&mut scratch[..img.len()]);
    if elf::validate(&scratch[..img.len()]).is_ok() {
        return Err(what);
    }
    Ok(())
}

/// Every acceptance rule of ADR-0016's subset, attacked: 25 mutations of
/// the real artifact, each of which MUST be refused. A validator whose
/// rejections are untested is decoration.
fn test_elf_reject() -> Result<(), &'static str> {
    let img = elf::TEST_IMAGE;
    if img.len() > 16 * 1024 {
        return Err("scratch buffer too small for the payload image");
    }
    // Phdr positions parsed from the image itself — no assumption beyond
    // what validate already enforces.
    let ph0 = le64_at(img, 32) as usize;
    let ph1 = ph0 + 56;

    // SAFETY: single CPU, IF=0, SyncCell is the sanctioned test-static
    // pattern (crate::sync); no aliasing exists.
    let scratch: &mut [u8] = unsafe { &mut *SCRATCH.get() };

    macro_rules! rej {
        ($what:expr, $m:expr) => {
            mutate_reject(img, scratch, concat!("mutation ACCEPTED: ", $what), $m)?
        };
    }

    // -- identification block --
    rej!("bad magic", |b: &mut [u8]| b[0] = 0x00);
    rej!("ELFCLASS32", |b: &mut [u8]| b[4] = 1);
    rej!("big-endian encoding", |b: &mut [u8]| b[5] = 2);
    rej!("EI_VERSION != 1", |b: &mut [u8]| b[6] = 9);
    rej!("e_version != 1", |b: &mut [u8]| put32(b, 20, 2));
    rej!("ET_DYN (PIE)", |b: &mut [u8]| put16(b, 16, 3));
    rej!("EM_386", |b: &mut [u8]| put16(b, 18, 3));
    rej!("e_ehsize != 64", |b: &mut [u8]| put16(b, 52, 65));
    rej!("e_phentsize != 56", |b: &mut [u8]| put16(b, 54, 57));
    rej!("e_phnum = 0", |b: &mut [u8]| put16(b, 56, 0));
    rej!("e_phnum = 9 (> MAX_SEGMENTS)", |b: &mut [u8]| put16(
        b, 56, 9
    ));
    rej!("phoff past end of image", |b: &mut [u8]| put64(
        b, 32, 0x100000
    ));

    // -- truncation (a slice, not an in-place mutation) --
    scratch[..img.len()].copy_from_slice(img);
    if elf::validate(&scratch[..64]).is_ok() {
        return Err("mutation ACCEPTED: truncated to header only");
    }

    // -- segment rules --
    rej!("text segment W+X", |b: &mut [u8]| put32(b, ph0 + 4, 7));
    rej!("unknown flag bit", |b: &mut [u8]| {
        put32(b, ph0 + 4, PF_R | PF_X | 0x100)
    });
    rej!("data memsz < filesz", |b: &mut [u8]| put64(b, ph1 + 40, 1));
    rej!("data memsz = 0", |b: &mut [u8]| put64(b, ph1 + 40, 0));
    rej!("data vaddr unaligned", |b: &mut [u8]| put64(
        b,
        ph1 + 16,
        0x201001
    ));
    rej!("data vaddr reaching kernel half", |b: &mut [u8]| {
        put64(b, ph1 + 16, 0x0000_7fff_ffff_f000)
    });
    rej!("segments overlap", |b: &mut [u8]| put64(
        b,
        ph1 + 16,
        0x200000
    ));
    rej!("data file span past EOF", |b: &mut [u8]| put64(
        b,
        ph1 + 8,
        0x100000
    ));

    // -- entry + phdr-type rules --
    rej!("entry inside a non-exec segment", |b: &mut [u8]| put64(
        b, 24, 0x201000
    ));
    rej!("entry inside no segment at all", |b: &mut [u8]| put64(
        b, 24, 0x900000
    ));
    rej!("PT_INTERP present", |b: &mut [u8]| put32(b, ph1, 3));
    rej!("no PT_LOAD at all", |b: &mut [u8]| {
        put32(b, ph0, 4);
        put32(b, ph1, 4);
    });

    info!(
        "m4",
        "elf_reject: 25 mutation classes refused — identification (magic/class/endian/versions/type/machine/sizes/phnum/phoff), truncation, W+X, unknown flags, filesz>memsz, zero memsz, unaligned vaddr, kernel-half vaddr, overlap, file-span EOF, entry outside X segment (2 ways), PT_INTERP, no PT_LOAD"
    );
    Ok(())
}

// ---- 3. load: a real image in a real address space ----------------------

/// STAC-bracketed kernel read of one byte of a user page — under the
/// armed SMAP a bare access #PFs (M3's proven fault path); here we want
/// the positive path (same helper shape as m3's).
///
/// # Safety
/// IF=0; `va` mapped U/S in the live address space.
unsafe fn stac_read(va: u64) -> u8 {
    // SAFETY: caller contract; stac/clac bracket the single access.
    unsafe {
        x86_64::stac();
        let b = core::ptr::read_volatile(va as *const u8);
        x86_64::clac();
        b
    }
}

/// Eight [`stac_read`]s assembled little-endian.
///
/// # Safety
/// As [`stac_read`], for all eight bytes.
unsafe fn stac_le64(va: u64) -> u64 {
    let mut x = [0u8; 8];
    // SAFETY: caller contract forwarded per byte.
    unsafe {
        for (i, slot) in x.iter_mut().enumerate() {
            *slot = stac_read(va + i as u64);
        }
    }
    u64::from_le_bytes(x)
}

/// Load the embedded image into a fresh process and verify the address
/// space it produced: PTE flags W^X-exact, contents read back under the
/// target's CR3 (entry stub, META manifest, 4 KiB of zeroed NOLOAD bss),
/// double-load refused without spending a single frame, dead-target
/// refused, and frame accounting exact across create+load+destroy
/// (1 root + 3 page-table frames + 3 leaf frames = 7, all reclaimed by
/// the destroy sweep).
fn test_elf_load() -> Result<(), &'static str> {
    let baseline = frames::free_frames();

    // Dead-target refusal: create + destroy, then load into the ghost.
    let ghost = proc::create("elfGhost")?;
    proc::destroy(ghost)?;
    if elf::load(elf::TEST_IMAGE, ghost).is_ok() {
        return Err("load into a dead process was not refused");
    }
    if frames::free_frames() != baseline {
        return Err("ghost create/destroy did not round-trip frames");
    }

    let pid = proc::create("elfLoad")?;
    let loaded = elf::load(elf::TEST_IMAGE, pid)?;
    if loaded.entry != PAYLOAD_ENTRY {
        return Err("LoadInfo.entry is not the image entry");
    }
    if loaded.pages != 3 {
        return Err("expected exactly 3 mapped pages (text + meta + bss)");
    }
    let spent = baseline - frames::free_frames();
    if spent != 7 {
        return Err("create+load did not spend exactly 7 frames (root + 3 tables + 3 leaves)");
    }

    let root = proc::pml4_of(pid).ok_or("target lost its pml4")?;
    // W^X at the PTE level (introspection walks physical tables — no
    // CR3 switch needed for this part).
    // SAFETY: ring 0, IF=0; `root` is the live process's owned PML4.
    let tpte = unsafe { paging::user_pte_flags(root, PAYLOAD_ENTRY) }
        .ok_or("text pte absent after load")?;
    if tpte & (paging::PTE_PRESENT | paging::PTE_USER) != paging::PTE_PRESENT | paging::PTE_USER {
        return Err("text pte not present+user");
    }
    if tpte & paging::PTE_WRITE != 0 {
        return Err("text pte is WRITABLE — W^X violation");
    }
    if tpte & paging::PTE_NX != 0 {
        return Err("text pte is NX — the entry would not be executable");
    }
    for va in [PAYLOAD_META_VA, PAYLOAD_BSS_VA] {
        // SAFETY: as above.
        let d =
            unsafe { paging::user_pte_flags(root, va) }.ok_or("data/bss pte absent after load")?;
        let want = paging::PTE_PRESENT | paging::PTE_USER | paging::PTE_WRITE | paging::PTE_NX;
        if d & want != want {
            return Err("data/bss pte flags wrong (want present+user+write+NX)");
        }
    }

    // Double-load refusal — and it must refuse BEFORE allocating, so a
    // refused load costs exactly zero frames.
    let before = frames::free_frames();
    if elf::load(elf::TEST_IMAGE, pid).is_ok() {
        return Err("double load into the same space was not refused");
    }
    if frames::free_frames() != before {
        return Err("refused double-load leaked frames");
    }

    // Contents under the target's CR3 through STAC-bracketed reads:
    // entry stub EB FE, META magic + entry fact, and every byte of the
    // NOLOAD bss canary zero.
    // SAFETY: IF=0; `root` is the live owned PML4 whose user pages were
    // just loaded and PTE-verified; the kernel view is restored before
    // the block ends, on the only exit path.
    let seen = unsafe {
        x86_64::write_cr3(root);
        let stub = [stac_read(PAYLOAD_ENTRY), stac_read(PAYLOAD_ENTRY + 1)];
        let mut magic_ok = true;
        for (i, want) in META_MAGIC.iter().enumerate() {
            if stac_read(PAYLOAD_META_VA + i as u64) != *want {
                magic_ok = false;
            }
        }
        let m_entry = stac_le64(PAYLOAD_META_VA + 16);
        let mut bss_zero = true;
        for i in 0..PAYLOAD_BSS_LEN {
            if stac_read(PAYLOAD_BSS_VA + i) != 0 {
                bss_zero = false;
                break;
            }
        }
        x86_64::write_cr3(paging::kernel_cr3_phys());
        (stub, magic_ok, m_entry, bss_zero)
    };
    if seen.0 != ENTRY_STUB {
        return Err("entry stub bytes wrong under the target's CR3 (want EB FE)");
    }
    if !seen.1 {
        return Err("META magic wrong under the target's CR3");
    }
    if seen.2 != PAYLOAD_ENTRY {
        return Err("META.entry wrong under the target's CR3");
    }
    if !seen.3 {
        return Err("bss canary not fully zero under the target's CR3");
    }

    // Teardown: the destroy sweep must reclaim every frame the image
    // brought into the space — back to the exact baseline.
    proc::destroy(pid)?;
    let after = frames::free_frames();
    if after != baseline {
        return Err("destroy did not reclaim the loaded image exactly");
    }

    info!(
        "m4",
        "elf_load: 3 pages into 'elfLoad' (7 frames spent: root+3 tables+3 leaves), PTEs W^X-exact (text RX, data/bss RW+NX), double-load refused at zero cost, dead target refused, EB FE + META + 4 KiB zeroed NOLOAD bss read under the target's CR3, teardown exact ({after})"
    );
    Ok(())
}

// ---- little-endian scratch helpers (host order is LE; explicit anyway) --

fn le64_at(b: &[u8], at: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(x)
}

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
