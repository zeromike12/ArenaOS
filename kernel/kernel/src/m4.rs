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
//! accounting exact through `proc::destroy`. Tests 4-5 drive the syscall
//! ABI v1 (M4.2, ADR-0017) from real ring-3 payloads: all six argument
//! registers proven through `abi_echo6`, the six callee-saved registers
//! proven preserved, `debug_write`'s success payload and all three typed
//! refusals observed *in ring 3*, and `thread_exit` recording a
//! full-width 64-bit code through the scheduler's reap path. Test 6 is
//! the milestone capstone (M4.3): the FIRST USER PROCESS — the real
//! rust-lld image loaded into its own address space runs in ring 3
//! (`spawn_with_cr3`), verifies its META and zero-filled bss from the
//! user side, writes its pinned message through `debug_write`, and
//! exits through `thread_exit`; the kernel proves the captured message
//! byte-identical to the image file, the ring-3 bss stamp readable
//! under the process CR3, and exact frame teardown.
//! Markers: `m4:test:<name>`, `m4: RESULT`.

use crate::arch::x86_64::{self, paging, syscall};
use crate::elf::{self, PF_R, PF_W, PF_X};
use crate::frames;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::proc;
use crate::sched;
use crate::sync::SyncCell;

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Result<(), &'static str>); 6] = [
        ("elf_parse", test_elf_parse),
        ("elf_reject", test_elf_reject),
        ("elf_load", test_elf_load),
        ("syscall_abi", test_syscall_abi),
        ("thread_exit_abi", test_thread_exit_abi),
        ("first_process", test_first_process),
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
/// The linker-pinned message the running payload writes (inside the
/// file-backed text span; META describes it as a (va, len) fact).
const PAYLOAD_MSG_VA: u64 = 0x200800;
const PAYLOAD_META_VA: u64 = 0x201000;
const PAYLOAD_BSS_VA: u64 = 0x202000;
const PAYLOAD_BSS_LEN: u64 = 0x1000;
const META_MAGIC: &[u8; 16] = b"ARENAOS-PAYLOAD!";
/// META is magic + six u64 facts = 64 bytes (payload `Meta` struct).
const META_SIZE: u64 = 64;

/// The process stack for test 6 — one page above the image's bss, so
/// the loader's own page tables serve it (exactly 1 extra leaf frame).
const PROC_STACK_VA: u64 = 0x203000;
const PROC_STACK_TOP: u64 = 0x204000;
/// User regions registered for the first process's thread: text (with
/// the pinned message), data+bss, stack. `(lo, hi)` pairs — the same
/// exclusive-high convention `user_range_ok` validates against.
const PROC_REGIONS: [(u64, u64); 3] = [
    (PAYLOAD_ENTRY, PAYLOAD_ENTRY + 0x1000),
    (PAYLOAD_META_VA, PAYLOAD_META_VA + 0x2000),
    (PROC_STACK_VA, PROC_STACK_TOP),
];

// The payload's diagnostic exit codes (mirrored in
// userspace/payload/src/main.rs — the success code itself comes from
// META.exit_ok, read out of the image file, not from here).
const PAYLOAD_EXIT_BAD_MAGIC: u64 = 43;
const PAYLOAD_EXIT_BSS_NOT_ZERO: u64 = 44;
const PAYLOAD_EXIT_BAD_WRITE: u64 = 45;
const PAYLOAD_EXIT_PANIC: u64 = 99;

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
    // The single text page holds the real program plus the pinned
    // message; both must be file-backed (memsz == filesz) and fit the
    // page the loader maps.
    if text.filesz > text.memsz || text.memsz > paging::PAGE {
        return Err("text segment sizes wrong (want file-backed within one page)");
    }
    if data.vaddr != PAYLOAD_META_VA || data.flags != PF_R | PF_W {
        return Err("data segment identity wrong (want 0x201000 R+W)");
    }
    // The data segment is file-backed for META only; the bss canary is
    // NOLOAD, so filesz MUST be strictly below memsz — that gap is the
    // loader's zero-fill contract.
    if data.filesz < META_SIZE || data.filesz >= data.memsz {
        return Err("data segment must be partly file-backed with NOLOAD bss beyond");
    }
    if data.memsz < PAYLOAD_BSS_VA + PAYLOAD_BSS_LEN - data.vaddr {
        return Err("data segment does not cover the bss canary");
    }

    // META cross-check, read straight out of the image file at the data
    // segment's offset (META is its first content by linker script).
    let mo = data.offset as usize;
    if img.len() < mo + META_SIZE as usize {
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
    // M4.3 facts: the pinned message and the success exit code. The
    // message must live inside the FILE-BACKED text span — test 6 reads
    // its expected bytes straight out of the image file.
    let m_msg_va = le64_at(img, mo + 40);
    let m_msg_len = le64_at(img, mo + 48);
    let m_exit_ok = le64_at(img, mo + 56);
    if m_msg_va != PAYLOAD_MSG_VA {
        return Err("META.msg_va disagrees with the linker-pinned message VA");
    }
    if m_msg_len == 0 || m_msg_len > 256 {
        return Err("META.msg_len outside the debug_write window (1..=256)");
    }
    if m_msg_va + m_msg_len > text.vaddr + text.filesz {
        return Err("the pinned message is not inside the file-backed text span");
    }
    if m_exit_ok == 0 {
        return Err("META.exit_ok must be a nonzero success code");
    }

    info!(
        "m4",
        "elf_parse: {}-byte rust-lld artifact — ET_EXEC entry {:#x}, {} PT_LOADs: text RX {:#x}+{:#x}, data RW {:#x}+{:#x} (filesz {:#x} < memsz: NOLOAD bss), META cross-check agrees ({}-byte message pinned at {:#x}, exit_ok {})",
        img.len(),
        parsed.entry,
        parsed.nsegs,
        text.vaddr,
        text.memsz,
        data.vaddr,
        data.memsz,
        data.filesz,
        m_msg_len,
        m_msg_va,
        m_exit_ok
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
    // the FULL file-backed text span byte-for-byte against the image
    // file (the real program plus the pinned message), META magic +
    // entry fact, and every byte of the NOLOAD bss canary zero.
    let text = elf::validate(elf::TEST_IMAGE)?.segs[0];
    // SAFETY: IF=0; `root` is the live owned PML4 whose user pages were
    // just loaded and PTE-verified; the kernel view is restored before
    // the block ends, on the only exit path.
    let seen = unsafe {
        x86_64::write_cr3(root);
        let mut text_ok = true;
        for i in 0..text.filesz {
            if stac_read(text.vaddr + i) != elf::TEST_IMAGE[(text.offset + i) as usize] {
                text_ok = false;
                break;
            }
        }
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
        (text_ok, magic_ok, m_entry, bss_zero)
    };
    if !seen.0 {
        return Err("text segment bytes wrong under the target's CR3 (file fidelity broken)");
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
        "elf_load: 3 pages into 'elfLoad' (7 frames spent: root+3 tables+3 leaves), PTEs W^X-exact (text RX, data/bss RW+NX), double-load refused at zero cost, dead target refused, {}-byte text span byte-identical to the file + META + 4 KiB zeroed NOLOAD bss read under the target's CR3, teardown exact ({after})",
        text.filesz
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

// ---- 4/5. syscall ABI v1 (ADR-0017), driven by real ring-3 payloads -------

/// The m4 user window — distinct VAs from the m3 U3 window on purpose
/// (the suites run sequentially, but distinct addresses keep every
/// region assertion unambiguous).
const M4_CODE_VA: u64 = 0x0000_0000_0050_0000;
const M4_DATA_VA: u64 = 0x0000_0000_0050_1000;
const M4_STACK_VA: u64 = 0x0000_0000_7FE0_0000;
const M4_STACK_TOP: u64 = M4_STACK_VA + 4096;
const M4_REGIONS: [(u64, u64); 3] = [
    (M4_CODE_VA, M4_CODE_VA + 4096),
    (M4_DATA_VA, M4_DATA_VA + 4096),
    (M4_STACK_VA, M4_STACK_TOP),
];

/// The debug_write message — lands in the kernel-side capture buffer and
/// on the serial console, byte for byte.
const M4_MSG: &[u8] = b"ABI-V1-DEBUG-WRITE";

/// Six distinct 64-bit arguments for the `abi_echo6` probe: all must
/// arrive at the dispatcher through RDI/RSI/RDX/R10/R8/R9 (ADR-0017).
const ECHO_ARGS: [u64; 6] = [
    0x0123_4567_89AB_CDEF,
    0xFEDC_BA98_7654_3210,
    0x0F1E_2D3C_4B5A_6978,
    0x8765_4321_FEDC_BA98,
    0xDEAD_BEEF_CAFE_F00D,
    0x5A5A_A5A5_1234_4321,
];

/// Callee-saved canaries (RBX/RBP/R12–R15): ABI v1 promises all six
/// survive every call — the payload re-checks them after the probe.
const CB: [u64; 6] = [
    0x1111_2222_3333_4444, // rbx
    0x5555_6666_7777_8888, // rbp
    0x9999_AAAA_BBBB_CCCC, // r12
    0xDDDD_EEEE_FFFF_0001, // r13
    0x0102_0304_0506_0708, // r14
    0x0A0B_0C0D_0E0F_0102, // r15
];

/// `thread_exit` test code — full 64-bit width (v0-era tests used 32-bit
/// immediates; v1 fidelity means the whole register arrives).
const EXIT_CODE_WIDE: u64 = 0x0000_BEEF_C0DE_0042;

/// Minimal cursor-based encoder for the fixed ring-3 payloads (the m3
/// builder's discipline, m4-local): every byte is real x86-64 machine
/// code the CPU executes at CPL 3.
struct Payload {
    b: [u8; 768],
    n: usize,
}

impl Payload {
    fn new() -> Self {
        Self { b: [0; 768], n: 0 }
    }
    fn emit(&mut self, bytes: &[u8]) {
        self.b[self.n..self.n + bytes.len()].copy_from_slice(bytes);
        self.n += bytes.len();
    }
    fn emit_u32(&mut self, v: u32) {
        self.emit(&v.to_le_bytes());
    }
    /// `movabs r<reg>, v` — REX.W B8+r with the full 64-bit immediate.
    fn movabs(&mut self, reg: u8, v: u64) {
        if reg >= 8 {
            self.emit(&[0x49, 0xB8 | (reg - 8)]);
        } else {
            self.emit(&[0x48, 0xB8 | reg]);
        }
        self.emit(&v.to_le_bytes());
    }
    fn do_syscall(&mut self) {
        self.emit(&[0x0F, 0x05]);
    }
    /// `mov eax, imm32` (call numbers and small immediates).
    fn mov_eax(&mut self, v: u32) {
        self.emit(&[0xB8]);
        self.emit_u32(v);
    }
    /// `mov edi, imm32`.
    fn mov_edi(&mut self, v: u32) {
        self.emit(&[0xBF]);
        self.emit_u32(v);
    }
    /// `mov esi, imm32`.
    fn mov_esi(&mut self, v: u32) {
        self.emit(&[0xBE]);
        self.emit_u32(v);
    }
    // Conditional jumps are the rel32 NEAR forms (0F 85 jne / 0F 88 js)
    // with 4-byte patch holes: this payload is ~480 bytes, so distances
    // to `fail` overflow a rel8 displacement — and a 4-byte patch into a
    // 1-byte hole silently corrupts the stream (the M4.2 bring-up
    // caught exactly that live: a residual `00 00` decoded as
    // `add [rax], al` and #GP'd on the fingerprint's non-canonical
    // value).
    /// `cmp rax, imm8` (sign-extended) + `jne rel32`, patchable hole.
    fn cmp_rax_i8_jne(&mut self, v: i8) -> usize {
        self.emit(&[0x48, 0x83, 0xF8, v as u8]);
        self.emit(&[0x0F, 0x85]); // jne rel32 (see the NEAR-forms note)
        let at = self.n;
        self.emit(&[0; 4]);
        at
    }
    /// `cmp rax, imm32` (the byte-count check) + `jne rel32` hole.
    fn cmp_rax_i32_jne(&mut self, v: u32) -> usize {
        self.emit(&[0x48, 0x3D]);
        self.emit_u32(v);
        self.emit(&[0x0F, 0x85]); // jne rel32
        let at = self.n;
        self.emit(&[0; 4]);
        at
    }
    /// `js rel32` (sign set → negative status) with a patchable hole.
    fn js_hole(&mut self) -> usize {
        self.emit(&[0x0F, 0x88]); // js rel32
        let at = self.n;
        self.emit(&[0; 4]);
        at
    }
    fn here(&self) -> usize {
        self.n
    }
    /// Emit `prefix`, then reserve a rel32 displacement hole
    /// (rip-relative lea to the payload's message tail).
    fn rel32_hole(&mut self, prefix: &[u8]) -> usize {
        self.emit(prefix);
        let at = self.n;
        self.emit(&[0; 4]);
        at
    }
    fn patch_rel32(&mut self, at: usize, target: usize) {
        let d = target as i32 - (at as i32 + 4);
        self.b[at..at + 4].copy_from_slice(&d.to_le_bytes());
    }
}

/// The ABI v1 proof payload. Contract (any violation exits 43):
/// seed six callee-saved canaries → `abi_echo6` with six distinct
/// arguments (negative status = violation; the fingerprint is stored to
/// the data page for the kernel side) → re-check every canary →
/// `debug_write` success (the returned count must equal the message
/// length) → an out-of-region buffer must answer `STATUS_BAD_ADDRESS` →
/// `len = 0` and oversized `len` must answer `STATUS_BAD_ARG` → an
/// unknown number must answer `STATUS_BAD_CALL` (-1, the v0-compatible
/// wire value) → `thread_exit(42)`.
fn build_abi_payload() -> (Payload, usize) {
    let mut p = Payload::new();
    let mut fail_jumps: [usize; 16] = [0; 16];
    let mut nj = 0usize;
    let mut lea_holes: [usize; 3] = [0; 3];
    let mut nl = 0usize;

    // (1) Callee-saved canaries: rbx, rbp, r12..r15.
    for (reg, v) in [
        (3u8, CB[0]),
        (5, CB[1]),
        (12, CB[2]),
        (13, CB[3]),
        (14, CB[4]),
        (15, CB[5]),
    ] {
        p.movabs(reg, v);
    }
    // (2) abi_echo6: six distinct arguments in RDI/RSI/RDX/R10/R8/R9.
    for (reg, v) in [
        (7u8, ECHO_ARGS[0]),
        (6, ECHO_ARGS[1]),
        (2, ECHO_ARGS[2]),
        (10, ECHO_ARGS[3]),
        (8, ECHO_ARGS[4]),
        (9, ECHO_ARGS[5]),
    ] {
        p.movabs(reg, v);
    }
    p.mov_eax(syscall::SYS_ABI_ECHO6 as u32);
    p.do_syscall();
    fail_jumps[nj] = p.js_hole(); // negative status = contract violation
    nj += 1;
    // (3) Store the fingerprint at data-page slot 0 (the kernel reads it
    //     through the owned frame's direct-map alias).
    p.movabs(7, M4_DATA_VA);
    p.emit(&[0x48, 0x89, 0x07]); // mov [rdi], rax
    // (4) Every callee-saved register must have survived the call.
    //     `cmp rN, rax` is 39 /r with reg=rax, rm=rN: REX.B (0x49)
    //     extends the RM field for r12..r15 — cmp rbx,rax = 48 39 C3,
    //     cmp rbp,rax = 48 39 C5, cmp r12..r15,rax = 49 39 C4..C7.
    //     (REX.R extends reg instead — 4C 39 E3 is `cmp rbx, r12`, a
    //     bring-up bug the canaries themselves caught live.)
    for (i, cmp_bytes) in [
        [0x48u8, 0x39, 0xC3],
        [0x48, 0x39, 0xC5],
        [0x49, 0x39, 0xC4],
        [0x49, 0x39, 0xC5],
        [0x49, 0x39, 0xC6],
        [0x49, 0x39, 0xC7],
    ]
    .iter()
    .enumerate()
    {
        p.movabs(0, CB[i]);
        p.emit(cmp_bytes);
        p.emit(&[0x0F, 0x85]); // jne rel32 fail
        fail_jumps[nj] = p.here();
        p.emit(&[0; 4]);
        nj += 1;
    }
    // (5) debug_write success: the count payload == the message length.
    lea_holes[nl] = p.rel32_hole(&[0x48, 0x8D, 0x3D]); // lea rdi, [rip+msg]
    nl += 1;
    p.mov_esi(M4_MSG.len() as u32);
    p.mov_eax(syscall::SYS_DEBUG_WRITE as u32);
    p.do_syscall();
    fail_jumps[nj] = p.cmp_rax_i32_jne(M4_MSG.len() as u32);
    nj += 1;
    // (6) Out-of-region buffer: a VA inside no registered region must
    //     answer STATUS_BAD_ADDRESS (-3) — never a fault, never a copy.
    p.movabs(7, 0x0060_0000);
    p.mov_esi(16);
    p.mov_eax(syscall::SYS_DEBUG_WRITE as u32);
    p.do_syscall();
    fail_jumps[nj] = p.cmp_rax_i8_jne(syscall::STATUS_BAD_ADDRESS as i8);
    nj += 1;
    // (7) len = 0: STATUS_BAD_ARG (-2).
    lea_holes[nl] = p.rel32_hole(&[0x48, 0x8D, 0x3D]);
    nl += 1;
    p.emit(&[0x31, 0xF6]); // xor esi, esi
    p.mov_eax(syscall::SYS_DEBUG_WRITE as u32);
    p.do_syscall();
    fail_jumps[nj] = p.cmp_rax_i8_jne(syscall::STATUS_BAD_ARG as i8);
    nj += 1;
    // (8) Oversized len: STATUS_BAD_ARG (-2).
    lea_holes[nl] = p.rel32_hole(&[0x48, 0x8D, 0x3D]);
    nl += 1;
    p.mov_esi(4096);
    p.mov_eax(syscall::SYS_DEBUG_WRITE as u32);
    p.do_syscall();
    fail_jumps[nj] = p.cmp_rax_i8_jne(syscall::STATUS_BAD_ARG as i8);
    nj += 1;
    // (9) Unknown call number: STATUS_BAD_CALL (-1) — the
    //     v0-compatible wire value, asserted from ring 3.
    p.mov_eax(0x7FFF);
    p.do_syscall();
    fail_jumps[nj] = p.cmp_rax_i8_jne(syscall::STATUS_BAD_CALL as i8);
    nj += 1;
    // (10) Success exit.
    p.mov_eax(syscall::SYS_THREAD_EXIT as u32);
    p.mov_edi(42);
    p.do_syscall();
    p.emit(&[0xEB, 0xFE]); // jmp $ (unreachable)
    let fail = p.here();
    p.mov_eax(syscall::SYS_THREAD_EXIT as u32);
    p.mov_edi(43);
    p.do_syscall();
    p.emit(&[0xEB, 0xFE]); // jmp $
    let msg_off = p.here();
    p.emit(M4_MSG);

    for j in &fail_jumps[..nj] {
        p.patch_rel32(*j, fail);
    }
    for l in &lea_holes[..nl] {
        p.patch_rel32(*l, msg_off);
    }
    (p, msg_off)
}

/// The `thread_exit` payload: exit immediately with a full-width code.
fn build_exit_payload() -> Payload {
    let mut p = Payload::new();
    p.mov_eax(syscall::SYS_THREAD_EXIT as u32);
    p.movabs(7, EXIT_CODE_WIDE); // rdi = the 64-bit exit code
    p.do_syscall();
    p.emit(&[0xEB, 0xFE]); // jmp $ (unreachable)
    p
}

/// Map the m4 three-page user window (code RX / data RW+NX / stack
/// RW+NX) into the kernel view and load the payload (+ optional message
/// tail) through the direct-map alias — the m3 `u3_setup` pattern with
/// m4-local addresses. Returns the three leaf PHYS and the post-setup
/// free-frame count.
///
/// # Safety
/// IF=0 (suite discipline); the kernel view is the live CR3.
unsafe fn m4_setup(
    payload: &[u8],
    msg_off: usize,
    msg: &[u8],
) -> Result<([u64; 3], u64), &'static str> {
    let mut phys = [0u64; 3];
    for slot in phys.iter_mut() {
        *slot = frames::alloc().ok_or("frame exhaustion for m4 user pages")?;
    }
    // SAFETY: IF=0, fresh owned frames, canonical lower-half VAs,
    // W^X-respecting permission pairs; the kernel view is an owned,
    // reachable root.
    unsafe {
        let root = paging::kernel_cr3_phys();
        paging::map_user_page_4k(root, M4_CODE_VA, phys[0], false, true)
            .map_err(|_| "m4 code page map failed")?;
        paging::map_user_page_4k(root, M4_DATA_VA, phys[1], true, false)
            .map_err(|_| "m4 data page map failed")?;
        paging::map_user_page_4k(root, M4_STACK_VA, phys[2], true, false)
            .map_err(|_| "m4 stack page map failed")?;
        let code_kv = phys[0] + paging::KERNEL_OFFSET;
        let data_kv = phys[1] + paging::KERNEL_OFFSET;
        core::ptr::copy_nonoverlapping(payload.as_ptr(), code_kv as *mut u8, payload.len());
        if !msg.is_empty() {
            core::ptr::copy_nonoverlapping(
                msg.as_ptr(),
                (code_kv + msg_off as u64) as *mut u8,
                msg.len(),
            );
        }
        core::ptr::write_bytes(data_kv as *mut u8, 0, 4096);
    }
    Ok((phys, frames::free_frames()))
}

/// Unmap the m4 window, free its three frames, and assert exact
/// accounting against the post-setup measurement.
///
/// # Safety
/// IF=0; the user thread is reaped before teardown (nothing may still
/// execute on the window's pages).
unsafe fn m4_teardown(phys: [u64; 3], after_setup_free: u64) -> Result<u64, &'static str> {
    // SAFETY: IF=0; the pages were mapped by m4_setup and nobody else
    // references them.
    unsafe {
        for va in [M4_CODE_VA, M4_DATA_VA, M4_STACK_VA] {
            let p = paging::unmap_user_page_kernel_view(va)
                .ok_or("m4 user page vanished from the kernel view")?;
            frames::free(p).map_err(|_| "m4 user page free rejected")?;
        }
    }
    let free_now = frames::free_frames();
    if free_now != after_setup_free + 3 {
        return Err("m4 user-page teardown frame accounting is not exact");
    }
    let _ = phys; // the PTE-derived addresses freed above came from our maps
    Ok(free_now)
}

/// The shared m4 user-thread entry: register the window, then iretq
/// into ring 3 (the m3 pattern — kmain armed and read-back-verified the
/// boundary MSRs in M3.3a).
fn m4_thread_entry(_arg: usize) {
    crate::sync::without_interrupts(|| {
        sched::set_current_user_regions(&M4_REGIONS).expect("m4 user regions rejected");
        // SAFETY: pages mapped U/S by m4_setup; RIP/RSP canonical and
        // inside the registered regions; RSP0/scratch describe this
        // thread (programmed at switch-in, re-checked by enter_user).
        unsafe { syscall::enter_user(M4_CODE_VA, M4_STACK_TOP) };
    });
}

/// Yield until only the bootstrap thread remains (the m3 drain
/// discipline), then one more pass so the last zombie is reaped and the
/// accounting assertions see the settled state.
fn m4_drain(max_yields: usize) -> Result<usize, &'static str> {
    let mut used = 0;
    while sched::live_threads() > 1 {
        if used >= max_yields {
            return Err("threads still live after the yield bound (scheduler stuck?)");
        }
        sched::yield_now();
        used += 1;
    }
    sched::yield_now();
    Ok(used + 1)
}

/// ABI v1 end to end from ring 3 (ADR-0017): the payload proves every
/// user-visible promise *in ring 3* (non-negative echo fingerprint, six
/// callee-saved canaries, three typed refusals, the -1 unknown-call
/// value, the success count) and exits 42 only if all held; the kernel
/// side proves the dispatcher-visible facts (call/rejection/byte
/// accounting, the captured message bytes, the stored fingerprint
/// against `echo6_fingerprint` itself, exact frame teardown).
fn test_syscall_abi() -> Result<(), &'static str> {
    let (p, msg_off) = build_abi_payload();
    // SAFETY: m4_setup's contract (IF=0, fresh frames, kernel view live).
    let (phys, after) = unsafe { m4_setup(&p.b[..msg_off], msg_off, M4_MSG) }?;
    let st0 = syscall::stats();
    let id = sched::spawn("abi", m4_thread_entry, 0)?;
    sched::yield_now();
    m4_drain(64)?;

    // Verdicts: every claim is dispatcher-recorded machine state.
    let Some(status) = syscall::exit_status_of(id) else {
        return Err("no thread_exit recorded for the ABI payload thread");
    };
    if status != 42 {
        return Err("ABI payload took the failure exit (a v1 contract check failed in ring 3)");
    }
    let st = syscall::stats();
    if st.echo_calls - st0.echo_calls != 1 {
        return Err("abi_echo6 not dispatched exactly once");
    }
    if st.write_calls - st0.write_calls != 4 {
        return Err("debug_write call accounting wrong (want 4)");
    }
    if st.write_rejected - st0.write_rejected != 3 {
        return Err("debug_write typed-refusal accounting wrong (want 3)");
    }
    if st.write_bytes - st0.write_bytes != M4_MSG.len() as u64 {
        return Err("debug_write byte accounting wrong");
    }
    if st.invalid_nr - st0.invalid_nr != 1 {
        return Err("unknown call number not rejected exactly once");
    }
    if st.exit_calls - st0.exit_calls != 1 {
        return Err("thread_exit accounting wrong");
    }
    let (buf, n) = syscall::last_write();
    if n != M4_MSG.len() || buf[..n] != M4_MSG[..] {
        return Err("debug_write kernel-side copy mismatch");
    }
    // The fingerprint the payload stored at data-page slot 0 — the
    // expectation comes from the dispatcher's own function (no
    // duplicated arithmetic across the ABI's two sides).
    // SAFETY: IF=0; phys[1] is m4_setup's owned, direct-mapped frame.
    let fp = unsafe { core::ptr::read_volatile((phys[1] + paging::KERNEL_OFFSET) as *const u64) };
    let want = syscall::echo6_fingerprint(ECHO_ARGS);
    if fp != want {
        return Err("abi_echo6 fingerprint mismatch — six-register marshalling broken");
    }

    let free_now = unsafe { m4_teardown(phys, after) }?;
    info!(
        "m4",
        "syscall_abi: ring-3 payload proved ABI v1 end to end — echo6 fingerprint {:#x} across RDI/RSI/RDX/R10/R8/R9, six callee-saved canaries survived, debug_write copied {} bytes with 3 typed refusals observed in ring 3 (-3/-2/-2), unknown nr -> -1, thread_exit(42); frames {} (teardown exact)",
        fp,
        n,
        free_now
    );
    Ok(())
}

/// `thread_exit` on its own marker: a full-width 64-bit code recorded
/// faithfully, the thread reaped through the scheduler (live count back
/// to baseline), dispatch accounted exactly once, frames exact.
fn test_thread_exit_abi() -> Result<(), &'static str> {
    let p = build_exit_payload();
    // SAFETY: m4_setup's contract; no message tail.
    let (phys, after) = unsafe { m4_setup(&p.b[..p.n], p.n, &[]) }?;
    let threads0 = sched::live_threads();
    let st0 = syscall::stats();
    let id = sched::spawn("wide-exit", m4_thread_entry, 0)?;
    sched::yield_now();
    m4_drain(64)?;

    if syscall::exit_status_of(id) != Some(EXIT_CODE_WIDE) {
        return Err("thread_exit did not record the full 64-bit code faithfully");
    }
    let st = syscall::stats();
    if st.exit_calls - st0.exit_calls != 1 {
        return Err("thread_exit dispatch accounting wrong");
    }
    if sched::live_threads() != threads0 {
        return Err("exited thread was not reaped — live count off");
    }
    let free_now = unsafe { m4_teardown(phys, after) }?;
    info!(
        "m4",
        "thread_exit_abi: full-width code {:#x} recorded faithfully, thread reaped through the scheduler (live count back to {}), frames {} (teardown exact)",
        EXIT_CODE_WIDE,
        threads0,
        free_now
    );
    Ok(())
}

// ---- 6. the first user process (M4.3) ------------------------------------

/// The first process's user-thread entry: the scheduler already put us
/// on the process's CR3 (`spawn_with_cr3` → `plan_switch`), so this only
/// registers the user regions the dispatcher validates syscall buffers
/// against, then hands the machine to the loaded image's `_start`.
fn first_proc_entry(_arg: usize) {
    crate::sync::without_interrupts(|| {
        sched::set_current_user_regions(&PROC_REGIONS).expect("first-process regions rejected");
        // SAFETY: pages mapped U/S in the process address space (text/
        // data/bss by the loader, the stack page by the test); RIP is
        // the image entry and RSP the stack top — both canonical and
        // inside the registered regions; RSP0/scratch describe this
        // thread (programmed at switch-in, re-checked by enter_user).
        unsafe { syscall::enter_user(PAYLOAD_ENTRY, PROC_STACK_TOP) };
    });
}

/// M4.3 — the milestone capstone: the real rust-lld image, loaded into
/// its own address space, RUNS in ring 3. The payload proves the
/// user-side facts (META reads back with its linked content, NOLOAD bss
/// reads zero, the stamp lands, `debug_write` returns the exact count)
/// and reports through its exit code — META.exit_ok on success, a
/// diagnostic code otherwise. The kernel proves the dispatcher-side
/// facts: the captured message is byte-identical to the bytes pinned in
/// the image FILE (expectations derived from META in the file — nothing
/// duplicated across the boundary), the ring-3 bss stamp reads back
/// under the process CR3, call accounting is exact, the thread is
/// reaped, and `proc::destroy` reclaims image+stack+tables to the exact
/// baseline.
fn test_first_process() -> Result<(), &'static str> {
    let baseline = frames::free_frames();
    let img = elf::TEST_IMAGE;

    // The image's self-description: text span (to locate the message's
    // file bytes) and the META facts the verdicts are derived from.
    let parsed = elf::validate(img)?;
    let text = parsed.segs[0];
    let mo = parsed.segs[1].offset as usize;
    if img.len() < mo + META_SIZE as usize {
        return Err("image too short for META");
    }
    let msg_va = le64_at(img, mo + 40);
    let msg_len = le64_at(img, mo + 48) as usize;
    let exit_ok = le64_at(img, mo + 56);
    if msg_va < text.vaddr || msg_va + msg_len as u64 > text.vaddr + text.filesz {
        return Err("META message span outside the file-backed text segment");
    }
    let msg_off = (text.offset + (msg_va - text.vaddr)) as usize;

    let pid = proc::create("m4First")?;
    let loaded = elf::load(img, pid)?;
    if loaded.entry != PAYLOAD_ENTRY || loaded.pages != 3 {
        return Err("loader did not place the image at its linked identity");
    }
    let root = proc::pml4_of(pid).ok_or("process lost its pml4")?;

    // The stack page: one leaf frame above the bss — the loader's own
    // PT serves it (0x200000..0x203000 share PDPT[0]/PD[1]/PT), so the
    // whole address space costs exactly 8 frames.
    let stack_phys = frames::alloc().ok_or("no frame for the process stack")?;
    // SAFETY: IF=0; `stack_phys` was just allocated (exclusive owner);
    // RAM frames are direct-mapped at phys+KERNEL_OFFSET.
    unsafe {
        core::ptr::write_bytes((stack_phys + paging::KERNEL_OFFSET) as *mut u8, 0, 4096);
        // SAFETY: the process root is live and owned; the VA is free
        // (the image ends at 0x203000); RW+NX is the stack's W^X pair.
        paging::map_user_page_4k(root, PROC_STACK_VA, stack_phys, true, false)
            .map_err(|_| "stack page map failed")?;
    }
    let spent = baseline - frames::free_frames();
    if spent != 8 {
        return Err("create+load+stack did not spend exactly 8 frames (root+3 tables+4 leaves)");
    }

    let st0 = syscall::stats();
    let threads0 = sched::live_threads();
    let id = sched::spawn_with_cr3("first-proc", first_proc_entry, 0, root)?;
    sched::yield_now();
    m4_drain(64)?;

    // The exit code IS the payload's diagnostic channel: anything other
    // than META.exit_ok names the ring-3 check that failed.
    let Some(status) = syscall::exit_status_of(id) else {
        return Err("no thread_exit recorded — the process never reached its exit");
    };
    if status != exit_ok {
        return Err(match status {
            PAYLOAD_EXIT_BAD_MAGIC => "payload: META magic read back wrong in ring 3 (43)",
            PAYLOAD_EXIT_BSS_NOT_ZERO => "payload: NOLOAD bss not zero in ring 3 (44)",
            PAYLOAD_EXIT_BAD_WRITE => "payload: debug_write returned the wrong count (45)",
            PAYLOAD_EXIT_PANIC => "payload: its panic handler ran (99)",
            _ => "payload exited with a code from nowhere in the contract",
        });
    }
    let st = syscall::stats();
    if st.write_calls - st0.write_calls != 1 {
        return Err("debug_write not dispatched exactly once");
    }
    if st.write_rejected != st0.write_rejected {
        return Err("the process's debug_write was rejected — but it succeeded");
    }
    if st.write_bytes - st0.write_bytes != msg_len as u64 {
        return Err("debug_write byte accounting wrong");
    }
    if st.exit_calls - st0.exit_calls != 1 {
        return Err("thread_exit accounting wrong");
    }
    if sched::live_threads() != threads0 {
        return Err("the process thread was not reaped — live count off");
    }
    // The captured console bytes must equal the message AS PINNED IN THE
    // IMAGE FILE — the expectation never passed through the payload.
    let (buf, n) = syscall::last_write();
    if n != msg_len || buf[..n] != img[msg_off..msg_off + msg_len] {
        return Err("captured message != the message pinned in the image file");
    }

    // The ring-3 bss stamp, read back under the process CR3: slot 0 =
    // the META magic's first 8 bytes as a LE u64; the rest of the 4 KiB
    // canary must still be zero (the program wrote exactly one slot).
    let mut want_bytes = [0u8; 8];
    want_bytes.copy_from_slice(&META_MAGIC[..8]);
    let want_stamp = u64::from_le_bytes(want_bytes);
    // SAFETY: IF=0; `root` is the live process's owned PML4 whose user
    // pages were just written by the (now reaped) payload thread; the
    // kernel view is restored on the block's only exit path.
    let (stamp, rest_zero) = unsafe {
        x86_64::write_cr3(root);
        let s = stac_le64(PAYLOAD_BSS_VA);
        let mut z = true;
        for i in 8..PAYLOAD_BSS_LEN {
            if stac_read(PAYLOAD_BSS_VA + i) != 0 {
                z = false;
                break;
            }
        }
        x86_64::write_cr3(paging::kernel_cr3_phys());
        (s, z)
    };
    if stamp != want_stamp {
        return Err("the payload's ring-3 bss stamp did not land (want the magic's LE u64)");
    }
    if !rest_zero {
        return Err("the payload wrote outside its stamp slot");
    }

    proc::destroy(pid)?;
    let after = frames::free_frames();
    if after != baseline {
        return Err("destroy did not reclaim image+stack+tables exactly");
    }
    info!(
        "m4",
        "first_process: the {}-byte rust-lld image RAN in ring 3 inside 'm4First' (spawn_with_cr3, 8 frames) — META verified and bss found zero FROM the user side, the {}-byte pinned message arrived byte-identical to the file, stamp {:#x} read back under the process CR3, thread_exit({}) recorded, thread reaped, teardown exact ({after})",
        img.len(),
        msg_len,
        stamp,
        status
    );
    Ok(())
}
