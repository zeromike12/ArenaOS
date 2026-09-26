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
//! under the process CR3, and exact frame teardown. Test 7 is IPC v1
//! (M4.4, ADR-0018): two processes, one endpoint, one notification,
//! one transferred capability — a real echo-server rendezvous driven
//! from ring 3 on both sides, with blocking, waking, and cap movement
//! asserted as counted machine events. Test 8 is the spawn protocol
//! (M4.5, ADR-0019): a ring-3 supervisor spawns the real image twice
//! through SYS_SPAWN — explicit attenuated inheritance, Process handles,
//! exit badges — and the restart is visible as the child's console
//! message appearing twice. Test 9 is the console input service (M4.6,
//! ADR-0020): the line discipline driven through the exact feed() the
//! RX ISR calls, a ring-3 reader parked and woken by it, and the shell
//! image validated as the spawn registry's image 1.
//! Markers: `m4:test:<name>`, `m4: RESULT`.

use crate::arch::x86_64::{self, paging, syscall};
use crate::cap::{self, Cap, CapObj};
use crate::console;
use crate::elf::{self, PF_R, PF_W, PF_X};
use crate::frames;
use crate::ipc;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::proc;
use crate::sched;
use crate::spawn;
use crate::sync::SyncCell;

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Result<(), &'static str>); 9] = [
        ("elf_parse", test_elf_parse),
        ("elf_reject", test_elf_reject),
        ("elf_load", test_elf_load),
        ("syscall_abi", test_syscall_abi),
        ("thread_exit_abi", test_thread_exit_abi),
        ("first_process", test_first_process),
        ("ipc_echo", test_ipc_echo),
        ("spawn_restart", test_spawn_restart),
        ("console_line", test_console_line),
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
    /// `mov r64, [rbx+disp8]` for rax(0)/rdx(2)/rsi(6); disp 0 uses the
    /// mod-00 form (rm=011 is rbx — no SIB/disp needed).
    fn mov_r64_mem_rbx(&mut self, dst: u8, disp: u8) {
        let modrm = (dst << 3) | 3;
        if disp == 0 {
            self.emit(&[0x48, 0x8B, modrm]);
        } else {
            self.emit(&[0x48, 0x8B, modrm | 0x40, disp]);
        }
    }
    /// `cmp rax, rsrc` + `jne rel32` patchable hole (src = reg number).
    fn cmp_rax_reg_jne(&mut self, src: u8) -> usize {
        self.emit(&[0x48, 0x39, 0xC8 | (src << 3)]);
        self.emit(&[0x0F, 0x85]);
        let at = self.n;
        self.emit(&[0; 4]);
        at
    }
    /// `cmp rax, rsrc` + `je rel32` patchable hole.
    fn cmp_rax_reg_je(&mut self, src: u8) -> usize {
        self.emit(&[0x48, 0x39, 0xC8 | (src << 3)]);
        self.emit(&[0x0F, 0x84]);
        let at = self.n;
        self.emit(&[0; 4]);
        at
    }
    /// `thread_exit(code)` + a belt-and-braces halt loop: the never-
    /// returned-from block every fail path ends in.
    fn exit_with(&mut self, code: u64) {
        self.mov_eax(syscall::SYS_THREAD_EXIT as u32);
        self.movabs(7, code);
        self.do_syscall();
        self.emit(&[0xEB, 0xFE]); // jmp $ (unreachable)
    }
    /// `mov [rbx+disp8], rax` (disp 0 uses the mod-00 form).
    fn mov_mem_rbx_from_rax(&mut self, disp: u8) {
        if disp == 0 {
            self.emit(&[0x48, 0x89, 0x03]);
        } else {
            self.emit(&[0x48, 0x89, 0x43, disp]);
        }
    }
    /// `cmp rax, imm8` (sign-extended) + `je rel32` patchable hole.
    fn cmp_rax_i8_je(&mut self, v: i8) -> usize {
        self.emit(&[0x48, 0x83, 0xF8, v as u8]);
        self.emit(&[0x0F, 0x84]);
        let at = self.n;
        self.emit(&[0; 4]);
        at
    }
    /// `cmp byte [rbx+disp8], imm8` + `jne rel32` patchable hole
    /// (disp 0 uses the mod-00 form; /7 is the cmp opcode extension).
    fn cmp_byte_rbx_jne(&mut self, disp: u8, imm: u8) -> usize {
        if disp == 0 {
            self.emit(&[0x80, 0x3B, imm]);
        } else {
            self.emit(&[0x80, 0x7B, disp, imm]);
        }
        self.emit(&[0x0F, 0x85]);
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

// ---- 7. IPC v1: the echo-server demo (M4.4, ADR-0018) ---------------------

/// The IPC demo window — BOTH processes use these same VAs (distinct
/// address spaces; identical layout keeps one entry fn and one region
/// set valid for server and client alike).
const I_CODE: u64 = 0x510000;
const I_DATA: u64 = 0x511000;
const I_STACK: u64 = 0x512000;
const I_STACK_TOP: u64 = 0x513000;
const I_REGIONS: [(u64, u64); 3] = [
    (I_CODE, I_CODE + 0x1000),
    (I_DATA, I_DATA + 0x1000),
    (I_STACK, I_STACK_TOP),
];

// Cap slots the payloads spell — cap::grant fills in order, and the
// test asserts the order held before the dance starts.
const IPC_EP_SLOT: u64 = 0; // both spaces: the endpoint
const IPC_NOTIF_SLOT: u64 = 1; // both spaces: the notification
const IPC_MEM_SLOT: u64 = 2; // client only: the cap it transfers

// The message identity: two recognizable words and one badge.
const IPC_PAT0: u64 = 0x434C_4945_4E54_3031; // "CLIENT01"
const IPC_PAT1: u64 = 0x574F_5244_4F4E_4531; // "WORDONE1"
const IPC_BADGE: u64 = 0xBEEF;

// Exit codes (both payloads): 42 = every ring-3 check held.
// Client diagnostics: 43 call status, 44 echoed w0, 45 echoed w1,
// 46 a reply cap arrived though none was sent, 47 wrong badge.
// Server diagnostics: 53 recv status, 54 transferred cap did not land,
// 55 reply status, 56 notify status.

/// The IPC CLIENT payload: `ipc_call` with two pattern words and its
/// Memory cap (slot 2) staged for transfer, verify the echoed words and
/// the ABSENT reply cap in ring 3, `wait` for the server's badge, and
/// exit 42 only if everything held. rbx carries the buffer VA across
/// the syscalls — callee-saved, and ABI v1 PROMISES it (ADR-0017).
fn build_ipc_client() -> Payload {
    let mut p = Payload::new();
    let mut holes: [(usize, u64); 8] = [(0, 0); 8];
    let mut nh = 0usize;
    p.movabs(3, I_DATA); // rbx = reply buffer base
    // SYS_IPC_CALL(ep 0, PAT0, PAT1, send-cap slot 2, reply buf)
    p.mov_eax(syscall::SYS_IPC_CALL as u32);
    p.movabs(7, IPC_EP_SLOT);
    p.movabs(6, IPC_PAT0);
    p.movabs(2, IPC_PAT1);
    p.movabs(10, IPC_MEM_SLOT);
    p.movabs(8, I_DATA);
    p.do_syscall();
    holes[nh] = (p.cmp_rax_i8_jne(0), 43);
    nh += 1;
    // buf[0] == PAT0
    p.mov_r64_mem_rbx(0, 0);
    p.movabs(1, IPC_PAT0);
    holes[nh] = (p.cmp_rax_reg_jne(1), 44);
    nh += 1;
    // buf[1] == PAT1
    p.mov_r64_mem_rbx(0, 8);
    p.movabs(1, IPC_PAT1);
    holes[nh] = (p.cmp_rax_reg_jne(1), 45);
    nh += 1;
    // buf[2] == CAP_NONE — the server sends no cap back
    p.mov_r64_mem_rbx(0, 16);
    p.movabs(1, u64::MAX);
    holes[nh] = (p.cmp_rax_reg_jne(1), 46);
    nh += 1;
    // SYS_WAIT(notification slot 1) must hand back the badge.
    p.mov_eax(syscall::SYS_WAIT as u32);
    p.movabs(7, IPC_NOTIF_SLOT);
    p.do_syscall();
    p.movabs(1, IPC_BADGE);
    holes[nh] = (p.cmp_rax_reg_jne(1), 47);
    nh += 1;
    p.exit_with(42);
    for i in 0..nh {
        let (h, code) = holes[i];
        let target = p.here();
        p.exit_with(code);
        p.patch_rel32(h, target);
    }
    p
}

/// The IPC SERVER payload: park in `ipc_recv` (the parked-server half
/// of the rendezvous), demand the transferred cap landed (buf[2] !=
/// CAP_NONE), echo both request words via `ipc_reply` with no cap,
/// `notify` the badge, exit 42 only if every step held.
fn build_ipc_server() -> Payload {
    let mut p = Payload::new();
    let mut holes: [(usize, u64); 8] = [(0, 0); 8];
    let mut nh = 0usize;
    p.movabs(3, I_DATA); // rbx = recv buffer base
    // SYS_IPC_RECV(ep 0, buf) — blocks until the client calls.
    p.mov_eax(syscall::SYS_IPC_RECV as u32);
    p.movabs(7, IPC_EP_SLOT);
    p.movabs(6, I_DATA);
    p.do_syscall();
    holes[nh] = (p.cmp_rax_i8_jne(0), 53);
    nh += 1;
    // The client's Memory cap must have landed: buf[2] != CAP_NONE.
    p.mov_r64_mem_rbx(0, 16);
    p.movabs(1, u64::MAX);
    holes[nh] = (p.cmp_rax_reg_je(1), 54); // je: == MAX means NOT landed
    nh += 1;
    // SYS_IPC_REPLY(ep 0, echoed w0, echoed w1, no cap).
    p.mov_eax(syscall::SYS_IPC_REPLY as u32);
    p.movabs(7, IPC_EP_SLOT);
    p.mov_r64_mem_rbx(6, 0); // rsi = buf[0]
    p.mov_r64_mem_rbx(2, 8); // rdx = buf[1]
    p.movabs(10, u64::MAX); // r10 = CAP_NONE
    p.do_syscall();
    holes[nh] = (p.cmp_rax_i8_jne(0), 55);
    nh += 1;
    // SYS_NOTIFY(notification slot 1, badge) — wakes the client's wait.
    p.mov_eax(syscall::SYS_NOTIFY as u32);
    p.movabs(7, IPC_NOTIF_SLOT);
    p.movabs(6, IPC_BADGE);
    p.do_syscall();
    holes[nh] = (p.cmp_rax_i8_jne(0), 56);
    nh += 1;
    p.exit_with(42);
    for i in 0..nh {
        let (h, code) = holes[i];
        let target = p.here();
        p.exit_with(code);
        p.patch_rel32(h, target);
    }
    p
}

/// The shared user-thread entry for both IPC processes: the per-process
/// code page (copied at setup) is what makes one a server and the other
/// a client — the VA layout, and therefore this entry, is identical.
fn ipc_thread_entry(_arg: usize) {
    crate::sync::without_interrupts(|| {
        sched::set_current_user_regions(&I_REGIONS).expect("ipc regions rejected");
        // SAFETY: the window pages are mapped U/S in this process's
        // space by ipc_proc_setup; RIP/RSP are canonical and inside the
        // registered regions; RSP0/scratch describe this thread
        // (programmed at switch-in, re-checked by enter_user).
        unsafe { syscall::enter_user(I_CODE, I_STACK_TOP) };
    });
}

/// Map the code/data/stack window into a process root, copy the payload
/// into the code page, zero data+stack. Three leaf frames — the tables
/// come with the first mapping (root + 3 tables + 3 leaves = 7 frames
/// per process, the same accounting test 6 asserts).
///
/// # Safety
/// IF=0; `root` is a live, owned PML4; every frame is freshly allocated
/// and written only through its direct-map alias.
unsafe fn ipc_proc_setup(root: u64, code: &[u8]) -> Result<(), &'static str> {
    let mut phys = [0u64; 3];
    for slot in phys.iter_mut() {
        *slot = frames::alloc().ok_or("frame exhaustion for the ipc window")?;
    }
    // SAFETY: caller contract; W^X pairs — code RX, data/stack RW+NX.
    unsafe {
        paging::map_user_page_4k(root, I_CODE, phys[0], false, true)
            .map_err(|_| "ipc code page map failed")?;
        paging::map_user_page_4k(root, I_DATA, phys[1], true, false)
            .map_err(|_| "ipc data page map failed")?;
        paging::map_user_page_4k(root, I_STACK, phys[2], true, false)
            .map_err(|_| "ipc stack page map failed")?;
        core::ptr::copy_nonoverlapping(
            code.as_ptr(),
            (phys[0] + paging::KERNEL_OFFSET) as *mut u8,
            code.len(),
        );
        core::ptr::write_bytes((phys[1] + paging::KERNEL_OFFSET) as *mut u8, 0, 4096);
        core::ptr::write_bytes((phys[2] + paging::KERNEL_OFFSET) as *mut u8, 0, 4096);
    }
    Ok(())
}

/// M4.4 — IPC v1 end to end (ADR-0018): two processes, one endpoint,
/// one notification, one transferred capability. The server parks in
/// `ipc_recv` first; the client's `ipc_call` delivers two words plus
/// its Memory cap and blocks; the server echoes the words via
/// `ipc_reply` and badges the client via `notify`; both exit 42 only if
/// every check held *in ring 3*. The kernel side independently asserts:
/// dispatcher and IPC counters exact, the cap landed in the server's
/// space with rights intact while the sender KEPT its original (copy,
/// never move), both threads reaped, objects destroyed without refusal,
/// frames exact.
fn test_ipc_echo() -> Result<(), &'static str> {
    let baseline = frames::free_frames();

    let srv = proc::create("ipcServer")?;
    let cli = proc::create("ipcClient")?;
    let sroot = proc::pml4_of(srv).ok_or("server lost its pml4")?;
    let croot = proc::pml4_of(cli).ok_or("client lost its pml4")?;
    let server = build_ipc_server();
    let client = build_ipc_client();
    // SAFETY: IF=0 suite discipline; owned live roots; fresh frames.
    unsafe {
        ipc_proc_setup(sroot, &server.b[..server.n])?;
        ipc_proc_setup(croot, &client.b[..client.n])?;
    }

    // The IPC objects and the caps that reach them (WRITE = call side,
    // READ = serve side, ADR-0018). The transferred cap needs COPY —
    // transfer is a copy under the attenuation rule (ADR-0015).
    let eid = ipc::create_endpoint().map_err(|_| "endpoint table full")?;
    let nid = ipc::create_notification().map_err(|_| "notification table full")?;
    let mem_frame = frames::alloc().ok_or("no frame for the transfer cap")?;
    let s_ep = cap::grant(
        srv,
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: cap::RIGHTS_READ,
        },
    )
    .map_err(|_| "server endpoint grant failed")?;
    let s_nf = cap::grant(
        srv,
        Cap {
            obj: CapObj::Notification { nid },
            rights: cap::RIGHTS_WRITE,
        },
    )
    .map_err(|_| "server notification grant failed")?;
    let c_ep = cap::grant(
        cli,
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: cap::RIGHTS_WRITE,
        },
    )
    .map_err(|_| "client endpoint grant failed")?;
    let c_nf = cap::grant(
        cli,
        Cap {
            obj: CapObj::Notification { nid },
            rights: cap::RIGHTS_READ,
        },
    )
    .map_err(|_| "client notification grant failed")?;
    let c_mem = cap::grant(
        cli,
        Cap {
            obj: CapObj::Memory {
                phys: mem_frame,
                pages: 1,
            },
            rights: cap::RIGHTS_READ | cap::RIGHTS_COPY,
        },
    )
    .map_err(|_| "client memory grant failed")?;
    if (s_ep, s_nf, c_ep, c_nf, c_mem) != (0, 1, 0, 1, 2) {
        return Err("cap slots did not fill in the order the payloads spell");
    }

    let st0 = syscall::stats();
    let i0 = ipc::stats();
    let threads0 = sched::live_threads();
    let sid = sched::spawn_in_proc("ipc-serv", ipc_thread_entry, 0, srv)?;
    sched::yield_now(); // the server parks in recv FIRST (the parked-server path)
    let cid = sched::spawn_in_proc("ipc-cli", ipc_thread_entry, 0, cli)?;
    sched::yield_now();
    m4_drain(256)?;

    // Verdicts — the payloads' exit codes are their diagnostic channel.
    let Some(ss) = syscall::exit_status_of(sid) else {
        return Err("the server never exited");
    };
    if ss != 42 {
        return Err(match ss {
            53 => "server: ipc_recv returned a nonzero status (53)",
            54 => "server: the transferred cap did not land (54)",
            55 => "server: ipc_reply returned a nonzero status (55)",
            56 => "server: notify returned a nonzero status (56)",
            _ => "server exited with a code from nowhere in the contract",
        });
    }
    let Some(cs) = syscall::exit_status_of(cid) else {
        return Err("the client never exited");
    };
    if cs != 42 {
        return Err(match cs {
            43 => "client: ipc_call returned a nonzero status (43)",
            44 => "client: echoed word 0 mismatch (44)",
            45 => "client: echoed word 1 mismatch (45)",
            46 => "client: a reply cap arrived though none was sent (46)",
            47 => "client: the waited badge was wrong (47)",
            _ => "client exited with a code from nowhere in the contract",
        });
    }

    // Counters: client syscalls = call, wait, exit; server = recv,
    // reply, notify, exit → exactly 7 dispatches, none invalid.
    let st = syscall::stats();
    if st.calls - st0.calls != 7 {
        return Err("ipc demo dispatch count wrong (want 7)");
    }
    if st.invalid_nr != st0.invalid_nr {
        return Err("a demo syscall number was invalid");
    }
    let i = ipc::stats();
    if (
        i.calls - i0.calls,
        i.recvs - i0.recvs,
        i.replies - i0.replies,
    ) != (1, 1, 1)
    {
        return Err("endpoint operation counters wrong");
    }
    if (i.notifies - i0.notifies, i.waits - i0.waits) != (1, 1) {
        return Err("notification counters wrong");
    }
    if i.cap_transfers - i0.cap_transfers != 1 || i.cap_drops != i0.cap_drops {
        return Err("cap-transfer accounting wrong (want 1 transfer, 0 drops)");
    }
    // Two blocks, not three: the server parks in recv and the client
    // parks in call — but the client's wait finds the badge ALREADY
    // pending. That ordering is inherent: reply's wake enqueues the
    // client without switching, so the server runs notify + exit before
    // the client resumes. The immediate path of wait gets the coverage;
    // the parked path is proven by the two blocks that did happen.
    if i.blocks - i0.blocks != 2 {
        return Err("blocking events wrong (want server recv + client call)");
    }
    if sched::live_threads() != threads0 {
        return Err("demo threads were not reaped — live count off");
    }

    // The transferred cap landed in the server's space (first free slot
    // = 2) describing the same frame with the same rights — and the
    // sender KEPT its original: transfer is a copy, never a move.
    let landed = cap::read(srv, 2).map_err(|_| "no cap landed in the server's space")?;
    match landed.obj {
        CapObj::Memory { phys, pages } if phys == mem_frame && pages == 1 => {}
        _ => return Err("the landed cap is not the client's Memory cap"),
    }
    if landed.rights != cap::RIGHTS_READ | cap::RIGHTS_COPY {
        return Err("the transferred cap's rights changed in flight");
    }
    let kept = cap::read(cli, c_mem as usize)
        .map_err(|_| "the sender LOST its original cap (move, not copy)")?;
    if !matches!(kept.obj, CapObj::Memory { .. }) {
        return Err("the sender's slot no longer holds the Memory cap");
    }

    // Teardown: objects must destroy without refusal (no stranded
    // state), both address spaces reclaim exactly, the described frame
    // returns to the allocator.
    ipc::destroy_endpoint(eid).map_err(|_| "endpoint teardown refused (state left behind?)")?;
    ipc::destroy_notification(nid).map_err(|_| "notification teardown refused")?;
    proc::destroy(srv)?;
    proc::destroy(cli)?;
    frames::free(mem_frame).map_err(|_| "transfer frame free rejected")?;
    let after = frames::free_frames();
    if after != baseline {
        return Err("ipc demo teardown is not frame-exact");
    }
    info!(
        "m4",
        "ipc_echo: two processes talked — the server parked in recv, the client's call delivered 2 words + a Memory cap (landed rights-intact at the server's first free slot, sender kept its copy), the reply echoed both words (verified IN RING 3), notify/wait carried badge {:#x}; 7 dispatches + 3 blocking events counted, threads reaped, frames {} (teardown exact)",
        IPC_BADGE,
        after
    );
    Ok(())
}

// ---- 8. the spawn protocol: supervisor restart demo (M4.5, ADR-0019) -------

/// The badge the supervisor lends to its children's exit notifications
/// (distinct from the IPC demo's badge — cosmetic, keeps the two tests'
/// console stories unambiguous).
const SPAWN_BADGE: u64 = 0xE7E7;

/// The SUPERVISOR payload — the root task of its own little subtree.
/// Cap slots (granted in order by the test): 0 = Image (READ),
/// 1 = Notification (READ|WRITE), 2 = Memory (READ|COPY — the handle it
/// delegates). Program: write the inheritance spec [(slot 2, READ|COPY)]
/// to its data page, SYS_SPAWN the real image with the spec + its
/// notification + a badge, wait for the child's exit badge, then RESTART:
/// spawn again and wait again. Exit 42 only if both lives came back
/// badged; 73/74/75/76 name the failed step.
fn build_spawn_supervisor() -> Payload {
    let mut p = Payload::new();
    let mut holes: [(usize, u64); 8] = [(0, 0); 8];
    let mut nh = 0usize;
    p.movabs(3, I_DATA); // rbx = spec buffer (callee-saved across syscalls)
    // spec[0] = (src slot 2, rights READ|COPY) — explicit, attenuated.
    p.movabs(0, IPC_MEM_SLOT);
    p.mov_mem_rbx_from_rax(0);
    p.movabs(0, (cap::RIGHTS_READ | cap::RIGHTS_COPY) as u64);
    p.mov_mem_rbx_from_rax(8);
    // --- spawn worker #1 ---
    p.mov_eax(syscall::SYS_SPAWN as u32);
    p.movabs(7, 0); // image cap slot
    p.movabs(6, I_DATA); // spec buffer
    p.movabs(2, 1); // one entry
    p.movabs(10, IPC_NOTIF_SLOT); // exit-badge target
    p.movabs(8, SPAWN_BADGE);
    p.do_syscall();
    holes[nh] = (p.js_hole(), 73); // negative status
    nh += 1;
    holes[nh] = (p.cmp_rax_i8_je(0), 73); // pid must be positive
    nh += 1;
    // --- wait for its exit badge ---
    p.mov_eax(syscall::SYS_WAIT as u32);
    p.movabs(7, IPC_NOTIF_SLOT);
    p.do_syscall();
    p.movabs(1, SPAWN_BADGE);
    holes[nh] = (p.cmp_rax_reg_jne(1), 74);
    nh += 1;
    // --- RESTART: spawn worker #2 (same spec, same badge) ---
    p.mov_eax(syscall::SYS_SPAWN as u32);
    p.movabs(7, 0);
    p.movabs(6, I_DATA);
    p.movabs(2, 1);
    p.movabs(10, IPC_NOTIF_SLOT);
    p.movabs(8, SPAWN_BADGE);
    p.do_syscall();
    holes[nh] = (p.js_hole(), 75);
    nh += 1;
    holes[nh] = (p.cmp_rax_i8_je(0), 75);
    nh += 1;
    // --- and its badge ---
    p.mov_eax(syscall::SYS_WAIT as u32);
    p.movabs(7, IPC_NOTIF_SLOT);
    p.do_syscall();
    p.movabs(1, SPAWN_BADGE);
    holes[nh] = (p.cmp_rax_reg_jne(1), 76);
    nh += 1;
    p.exit_with(42);
    for i in 0..nh {
        let (h, code) = holes[i];
        let target = p.here();
        p.exit_with(code);
        p.patch_rel32(h, target);
    }
    p
}

/// M4.5 — the spawn protocol end to end (ADR-0019): a ring-3 supervisor
/// creates processes from an IMAGE CAPABILITY with EXPLICIT HANDLE
/// INHERITANCE, and restarts a worker through its exit badge. The child
/// is the untouched M4.3 image — its console message appearing TWICE is
/// the restart, visible on the wire. Kernel-side: both children exited
/// with the image's own success code (via spawn-registry thread ids),
/// the inherited cap landed attenuated in each child's first slot, the
/// parent's Process handles landed in order, the parent kept its
/// original (copy, not move), counters exact, threads reaped, frames
/// exact across three address spaces.
fn test_spawn_restart() -> Result<(), &'static str> {
    let baseline = frames::free_frames();

    // The child's message length — derived from the image's own META
    // (self-describing, as in test 6), for the byte-accounting verdict.
    let parsed_img = elf::validate(elf::TEST_IMAGE)?;
    let mo = parsed_img.segs[1].offset as usize;
    let msg_len = le64_at(elf::TEST_IMAGE, mo + 48) as u64;

    let sup = proc::create("supervisor")?;
    let sroot = proc::pml4_of(sup).ok_or("supervisor lost its pml4")?;
    let p = build_spawn_supervisor();
    // SAFETY: IF=0 suite discipline; owned live root; fresh frames.
    unsafe {
        ipc_proc_setup(sroot, &p.b[..p.n])?;
    }

    // The supervisor's initial handles: an image to spawn from, a
    // notification to lend its children's exits to, and one COPY-able
    // Memory cap to delegate.
    let nid = ipc::create_notification().map_err(|_| "notification table full")?;
    let mem_frame = frames::alloc().ok_or("no frame for the inheritable cap")?;
    let s_img = cap::grant(
        sup,
        Cap {
            obj: CapObj::Image { img_id: 0 },
            rights: cap::RIGHTS_READ,
        },
    )
    .map_err(|_| "image grant failed")?;
    let s_nf = cap::grant(
        sup,
        Cap {
            obj: CapObj::Notification { nid },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
    )
    .map_err(|_| "notification grant failed")?;
    let s_mem = cap::grant(
        sup,
        Cap {
            obj: CapObj::Memory {
                phys: mem_frame,
                pages: 1,
            },
            rights: cap::RIGHTS_READ | cap::RIGHTS_COPY,
        },
    )
    .map_err(|_| "memory grant failed")?;
    if (s_img, s_nf, s_mem) != (0, 1, 2) {
        return Err("supervisor cap slots did not fill in the order the payload spells");
    }

    let st0 = syscall::stats();
    let i0 = ipc::stats();
    let threads0 = sched::live_threads();
    let sup_tid = sched::spawn_in_proc("supervisor", ipc_thread_entry, 0, sup)?;
    sched::yield_now();
    m4_drain(512)?;

    // The supervisor's exit code is its diagnostic channel.
    let Some(ss) = syscall::exit_status_of(sup_tid) else {
        return Err("the supervisor never exited");
    };
    if ss != 42 {
        return Err(match ss {
            73 => "supervisor: first spawn refused or returned a non-positive pid (73)",
            74 => "supervisor: the first exit badge never arrived intact (74)",
            75 => "supervisor: the restart spawn was refused (75)",
            76 => "supervisor: the second exit badge never arrived intact (76)",
            _ => "supervisor exited with a code from nowhere in the contract",
        });
    }

    // The spawn registry is the machine-state witness: exactly two
    // children, both exited with the image's own success code, both
    // holding the inherited cap at their first free slot.
    let recs = spawn::records_snapshot();
    let live: [(u64, u64); 8] = {
        let mut out = [(0u64, 0u64); 8];
        let mut n = 0;
        for r in recs.iter().flatten() {
            out[n] = *r;
            n += 1;
        }
        out
    };
    let nkids = recs.iter().flatten().count();
    if nkids != 2 {
        return Err("the spawn registry does not hold exactly the two children");
    }
    for &(cpid, ctid) in live[..nkids].iter() {
        if syscall::exit_status_of(ctid) != Some(42) {
            return Err("a spawned child did not exit with the image's success code");
        }
        let landed = cap::read(cpid, 0).map_err(|_| "no inherited cap in the child's space")?;
        match landed.obj {
            CapObj::Memory { phys, pages } if phys == mem_frame && pages == 1 => {}
            _ => return Err("the inherited cap is not the supervisor's Memory cap"),
        }
        if landed.rights != cap::RIGHTS_READ | cap::RIGHTS_COPY {
            return Err("inherited rights wrong (want exactly READ|COPY)");
        }
    }
    // The parent's Process handles landed in spawn order (first free
    // slots after its own three), each naming its child.
    for (i, &(cpid, _)) in live[..nkids].iter().enumerate() {
        let h = cap::read(sup, 3 + i)
            .map_err(|_| "a Process handle did not land in the supervisor's space")?;
        match h.obj {
            CapObj::Process { pid } if pid == cpid => {}
            _ => return Err("a Process handle does not name the spawned child"),
        }
    }
    if cap::read(sup, s_mem as usize).is_err() {
        return Err("the supervisor LOST its original Memory cap (move, not copy)");
    }

    // Counters: supervisor = spawn, wait, spawn, wait, exit (5); each
    // child = debug_write + exit (2 × 2) → 9 dispatches, 3 exits, and
    // the child's message hit the console once per life. Both waits
    // parked: without preemption a worker runs only while the
    // supervisor sleeps, so the badges could not have been pending.
    let st = syscall::stats();
    if st.calls - st0.calls != 9 {
        return Err("spawn demo dispatch count wrong (want 9)");
    }
    if st.exit_calls - st0.exit_calls != 3 {
        return Err("exit count wrong (want supervisor + two children)");
    }
    if st.write_calls - st0.write_calls != 2 {
        return Err("child debug_write count wrong (want 2 — one per life)");
    }
    if st.write_bytes - st0.write_bytes != 2 * msg_len {
        return Err("child message byte accounting wrong (want two lives' worth)");
    }
    let i = ipc::stats();
    if i.notifies - i0.notifies != 2 {
        return Err("exit notifications not fired exactly twice");
    }
    if i.waits - i0.waits != 2 {
        return Err("supervisor wait count wrong");
    }
    if i.blocks - i0.blocks != 2 {
        return Err("both supervisor waits should have parked (no preemption)");
    }
    if sched::live_threads() != threads0 {
        return Err("demo threads were not reaped — live count off");
    }

    // Teardown: forget the records, reclaim the children (dead, reaped)
    // and the supervisor, retire the notification, free the described
    // frame — back to the exact baseline.
    for &(cpid, _) in live[..nkids].iter() {
        spawn::forget(cpid).map_err(|_| "spawn record forget refused")?;
        proc::destroy(cpid)?;
    }
    ipc::destroy_notification(nid).map_err(|_| "notification teardown refused")?;
    proc::destroy(sup)?;
    frames::free(mem_frame).map_err(|_| "inheritable frame free rejected")?;
    let after = frames::free_frames();
    if after != baseline {
        return Err("spawn demo teardown is not frame-exact");
    }
    info!(
        "m4",
        "spawn_restart: a ring-3 supervisor spawned the real image TWICE through SYS_SPAWN — each child inherited the attenuated Memory cap at its slot 0, ran the M4.3 program (its {}-byte message on the console twice: the restart, visible), and badged the supervisor at exit; both children exited 42, Process handles landed in order, the supervisor kept its original, counters exact (9 dispatches / 2 notifies / 2 parked waits), frames {} (teardown exact)",
        msg_len,
        after
    );
    Ok(())
}

// ---- 9. the console input service (M4.6, ADR-0020) -------------------------

/// The ring-3 CONSOLE READER: one `SYS_CONSOLE_READ` into its data page,
/// then verify in ring 3 that the line arrived byte-exact ("hi m4" — 5
/// bytes, first 'h', last '4'). Exit 42 on success; 81 = wrong length,
/// 82 = wrong bytes. The read parks the thread (the queue is empty when
/// it starts) — the bootstrap's `feed()` below wakes it, which is the
/// exact scheduler dance the vector-33 UART ISR performs for the shell.
fn build_console_reader() -> Payload {
    let mut p = Payload::new();
    let buf = I_DATA + 0x100;
    p.movabs(3, buf); // rbx = line buffer (callee-saved across syscalls)
    p.mov_eax(syscall::SYS_CONSOLE_READ as u32);
    p.movabs(7, buf); // RDI = buffer
    p.movabs(6, 64); // RSI = max
    p.do_syscall();
    let h_len = p.cmp_rax_i8_jne(5); // "hi m4" = 5 bytes
    let h_b0 = p.cmp_byte_rbx_jne(0, b'h');
    let h_b4 = p.cmp_byte_rbx_jne(4, b'4');
    p.exit_with(42);
    let t_len = p.here();
    p.exit_with(81);
    p.patch_rel32(h_len, t_len);
    let t_bytes = p.here();
    p.exit_with(82);
    p.patch_rel32(h_b0, t_bytes);
    p.patch_rel32(h_b4, t_bytes);
    p
}

/// M4.6 — the console input service end to end (ADR-0020). The line
/// discipline is driven through `console::feed` — the SAME function the
/// vector-33 RX ISR calls per received byte, so there is no test-only
/// twin of the input path: edit/commit/queue/wake semantics here are
/// the live ones, and the harness's shell session exercises the real
/// UART in front of them. Kernel-side: backspace editing, empty lines
/// never queued, truncation counted, queue overflow drops the OLDEST
/// line. Ring-3 side: a reader parks on the empty queue and is woken by
/// a fed line, verifying length and bytes in ring 3. Plus the shell
/// image itself — the spawn registry's image 1, which the boot sequence
/// spawns as the initial service the moment this suite passes.
///
/// Counter deltas (not absolutes): the suite assumes no EXTERNAL console
/// input arrives while it runs — true by construction in the harness,
/// where every feed is paced on the shell prompt, which only exists
/// after this suite.
fn test_console_line() -> Result<(), &'static str> {
    let baseline = frames::free_frames();
    let c0 = console::stats();
    let mut kbuf = [0u8; console::LINE_MAX];

    // 1. The discipline: backspace edit, commit on CR; an empty line is
    //    consumed, never queued.
    for &b in b"ab\x7fc\r" {
        console::feed(b);
    }
    let n = console::read_line(&mut kbuf).map_err(|_| "read refused a queued line")?;
    if n != 2 || &kbuf[..n] != b"ac" {
        return Err("backspace edit did not produce 'ac'");
    }
    console::feed(b'\r');
    if console::pending_lines() != 0 {
        return Err("an empty line reached the queue");
    }

    // 2. Truncation: a short `max` cuts the line and is counted (the
    //    remainder is discarded, never re-queued).
    for &b in b"uvwxyz\r" {
        console::feed(b);
    }
    let n = console::read_line(&mut kbuf[..3]).map_err(|_| "truncating read refused")?;
    if n != 3 || &kbuf[..3] != b"uvw" {
        return Err("truncated read wrong");
    }

    // 3. Overflow: five committed lines into a four-deep queue — the
    //    OLDEST (l1) is dropped, the rest come out in order.
    for &b in b"l1\rl2\rl3\rl4\rl5\r" {
        console::feed(b);
    }
    for want in [b"l2", b"l3", b"l4", b"l5"] {
        let n = console::read_line(&mut kbuf).map_err(|_| "queued read refused")?;
        if &kbuf[..n] != *want {
            return Err("queue order wrong after oldest-drop");
        }
    }

    // 4. The ring-3 leg: the reader parks on the empty queue; the fed
    //    line wakes it (feed() from this kernel thread == feed() from
    //    the ISR: same function, same wake, enqueue-only).
    let cproc = proc::create("console")?;
    let croot = proc::pml4_of(cproc).ok_or("console proc lost its pml4")?;
    let p = build_console_reader();
    // SAFETY: IF=0 suite discipline; owned live root; fresh frames.
    unsafe {
        ipc_proc_setup(croot, &p.b[..p.n])?;
    }
    let threads0 = sched::live_threads();
    let tid = sched::spawn_in_proc("console", ipc_thread_entry, 0, cproc)?;
    sched::yield_now(); // the reader reaches SYS_CONSOLE_READ and parks
    sched::yield_now();
    for &b in b"hi m4\r" {
        console::feed(b);
    }
    m4_drain(64)?;
    let Some(ss) = syscall::exit_status_of(tid) else {
        return Err("the console reader never exited");
    };
    if ss != 42 {
        return Err(match ss {
            81 => "ring-3 console read returned the wrong length (81)",
            82 => "ring-3 console read returned wrong bytes (82)",
            _ => "console reader exited with a code from nowhere in the contract",
        });
    }

    // 5. The shell image: registry image 1, a loadable ADR-0016
    //    artifact — the boot sequence spawns it as the initial service
    //    the moment this suite passes.
    let sh = elf::validate(elf::SHELL_IMAGE)?;
    if spawn::image_bytes(1) != Some(elf::SHELL_IMAGE) {
        return Err("registry image 1 is not the embedded shell");
    }
    if sh.nsegs != 2 {
        return Err("shell image is not exactly two PT_LOAD segments (W^X text + data/bss)");
    }

    // 6. Counter deltas: 8 committed lines (ac, uvwxyz, l1..l5, hi m4),
    //    1 dropped to overflow (l1), 7 reads (1+1+4 kernel-side, 1 from
    //    ring 3), exactly 1 parked read (the ring-3 leg), 1 truncation.
    let c1 = console::stats();
    if c1.lines_in - c0.lines_in != 8 {
        return Err("committed-line count wrong (want 8)");
    }
    if c1.dropped_lines - c0.dropped_lines != 1 {
        return Err("overflow drop count wrong (want exactly l1)");
    }
    if c1.reads - c0.reads != 7 {
        return Err("read count wrong (want 7)");
    }
    if c1.blocks - c0.blocks != 1 {
        return Err("exactly the ring-3 read should have parked");
    }
    if c1.truncations - c0.truncations != 1 {
        return Err("truncation count wrong (want 1)");
    }
    if console::pending_lines() != 0 {
        return Err("console queue not drained — the shell would inherit stale lines");
    }

    proc::destroy(cproc)?;
    if sched::live_threads() != threads0 {
        return Err("console reader was not reaped");
    }
    let after = frames::free_frames();
    if after != baseline {
        return Err("console test teardown is not frame-exact");
    }
    info!(
        "m4",
        "console_line: the line discipline (fed through the RX ISR's own entry point) did backspace edits, refused empty lines, truncated and counted, dropped the OLDEST of five queued lines; a ring-3 reader parked on the empty queue and a fed line woke it — length and bytes verified IN RING 3; the shell image validated as registry image 1 ({} bytes, entry {:#x}, {} segments); counters exact, frames {} (teardown exact)",
        elf::SHELL_IMAGE.len(),
        sh.entry,
        sh.nsegs,
        after
    );
    Ok(())
}
