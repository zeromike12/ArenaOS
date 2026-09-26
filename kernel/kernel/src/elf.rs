//! ELF64 executable format — the ArenaOS strict-subset validator and
//! image loader (M4.1, ADR-0016).
//!
//! The container is ELF; the semantics are entirely ours. [`validate`]
//! accepts only the closed subset the ADR defines — `ET_EXEC` static
//! images, page-aligned `PT_LOAD` segments in the canonical lower half,
//! W^X per segment (ADR-0008), entry inside an executable segment, no
//! `PT_INTERP` — and refuses everything else with a distinct message per
//! rule. [`load`] turns a validated image into populated pages of a
//! [`crate::proc`] address space: fresh frames, zero-filled, file-backed
//! prefixes copied, mapped with the segment's own flags.
//!
//! All multi-byte fields are read manually, little-endian, at fixed
//! offsets — no `repr(C)` casts over untrusted bytes, so alignment and
//! padding are never the validator's problem. Bounds are proven before
//! any read that could go out of range; the helpers below index directly
//! and rely on that ordering.
//!
//! The loader is source-agnostic: today the bytes are a compiled-in test
//! artifact ([`TEST_IMAGE`]); in Phase 5 the same entry point receives
//! bytes from the filesystem, and in M4.5 spawn is capability-gated.
//!
//! Failure semantics: if [`load`] errors mid-way, pages already mapped
//! stay in the target's address space — destroying the process reclaims
//! them (its user-half sweep frees leaves and tables, ADR-0014), so no
//! partial load leaks outside the address space.

use crate::arch::x86_64::paging;
use crate::frames;
use crate::proc;
use crate::sync::without_interrupts;

/// Cap on `PT_LOAD` segments accepted in one image (ADR-0016).
pub const MAX_SEGMENTS: usize = 8;

/// Canonical lower-half limit: user VAs live strictly below this.
const USER_HALF_LIMIT: u64 = 0x0000_8000_0000_0000;

const EI_MAG: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const EI_CLASS64: u8 = 2;
const EI_DATA_LSB: u8 = 1;
const EI_VERSION_ONE: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 0x3e;
const PT_LOAD: u32 = 1;
const PT_INTERP: u32 = 3;

/// Segment permission bits (ELF `p_flags`).
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;
const PF_KNOWN: u32 = PF_R | PF_W | PF_X;

/// ELF64 header (64 bytes) plus at least one program header (56).
const MIN_IMAGE: usize = 64 + 56;

// Elf64_Ehdr field offsets.
const E_TYPE: usize = 16;
const E_MACHINE: usize = 18;
const E_VERSION: usize = 20;
const E_ENTRY: usize = 24;
const E_PHOFF: usize = 32;
const E_EHSIZE: usize = 52;
const E_PHENTSIZE: usize = 54;
const E_PHNUM: usize = 56;

// Elf64_Phdr field offsets, relative to the phdr's start.
const P_TYPE: usize = 0;
const P_FLAGS: usize = 4;
const P_OFFSET: usize = 8;
const P_VADDR: usize = 16;
const P_FILESZ: usize = 32;
const P_MEMSZ: usize = 40;

/// The M4 test image — a genuine cargo/rust-lld artifact
/// (`userspace/payload`, built by `tools/build.sh` before the kernel),
/// embedded at compile time. Until Phase 5 brings storage, the kernel is
/// its own only filesystem. Layout contract: `payload.ld` + `m4.rs`.
pub static TEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/payload/target/x86_64-unknown-none/release/arena-payload");

/// The minimal shell (M4.6, ADR-0020): spawn-registry image 1, built
/// by `tools/build.sh` from `userspace/shell` with the same
/// strict-subset contract as the payload — a genuine cargo/rust-lld
/// ET_EXEC artifact at fixed addresses (0x400000 text / 0x410000
/// data+bss), embedded by `include_bytes!` and spawned by the boot
/// sequence as the initial service.
pub static SHELL_IMAGE: &[u8] =
    include_bytes!("../../../userspace/shell/target/x86_64-unknown-none/release/arena-shell");

/// One accepted `PT_LOAD` segment.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SegInfo {
    /// Load address (page-aligned, canonical lower half).
    pub vaddr: u64,
    /// Span in memory (>= `filesz`; the excess is zero-filled BSS).
    pub memsz: u64,
    /// File-backed prefix length.
    pub filesz: u64,
    /// Offset of the file-backed prefix within the image.
    pub offset: u64,
    /// `PF_*` permission bits (never `PF_W|PF_X` together).
    pub flags: u32,
}

impl SegInfo {
    const EMPTY: Self = Self {
        vaddr: 0,
        memsz: 0,
        filesz: 0,
        offset: 0,
        flags: 0,
    };
}

/// A validated image's identity: entry point plus its load segments.
pub struct ImageInfo {
    /// `e_entry` — guaranteed inside a `PF_X` segment's span.
    pub entry: u64,
    /// Number of accepted `PT_LOAD` segments (1..=[`MAX_SEGMENTS`]).
    pub nsegs: usize,
    /// The accepted segments (first `nsegs` entries are meaningful).
    pub segs: [SegInfo; MAX_SEGMENTS],
}

/// What [`load`] placed into the target address space.
pub struct LoadInfo {
    /// Initial RIP for the image (the ELF's `e_entry`).
    pub entry: u64,
    /// Number of 4 KiB pages mapped.
    pub pages: u64,
}

// --- little-endian field reads -------------------------------------------
// Direct indexing on purpose: `validate` proves every bound BEFORE the
// reads that depend on it, so these cannot panic on accepted input, and
// rejected input fails at the bound check, not here.

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le64(b: &[u8], at: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(x)
}

/// Validate `bytes` against the ArenaOS ELF subset (ADR-0016). Every
/// rule refusal carries its own message; the m4 rejection corpus proves
/// each one fires.
pub fn validate(bytes: &[u8]) -> Result<ImageInfo, &'static str> {
    if bytes.len() < MIN_IMAGE {
        return Err("elf: image shorter than ELF header + one phdr");
    }
    if bytes[0..4] != EI_MAG {
        return Err("elf: bad magic");
    }
    if bytes[4] != EI_CLASS64 {
        return Err("elf: not ELFCLASS64");
    }
    if bytes[5] != EI_DATA_LSB {
        return Err("elf: not little-endian");
    }
    if bytes[6] != EI_VERSION_ONE {
        return Err("elf: bad EI_VERSION");
    }
    // EI_OSABI (bytes[7]) is deliberately unchecked: it carries no
    // ArenaOS semantics — toolchains stamp whatever they like and the
    // loader ignores it (ADR-0016).
    if le32(bytes, E_VERSION) != 1 {
        return Err("elf: bad e_version");
    }
    if le16(bytes, E_TYPE) != ET_EXEC {
        return Err("elf: only ET_EXEC static images are accepted");
    }
    if le16(bytes, E_MACHINE) != EM_X86_64 {
        return Err("elf: not EM_X86_64");
    }
    if le16(bytes, E_EHSIZE) != 64 {
        return Err("elf: e_ehsize must be 64");
    }
    if le16(bytes, E_PHENTSIZE) != 56 {
        return Err("elf: e_phentsize must be 56");
    }
    let phnum = le16(bytes, E_PHNUM) as usize;
    if phnum == 0 || phnum > MAX_SEGMENTS {
        return Err("elf: e_phnum outside 1..=MAX_SEGMENTS");
    }
    let phoff = le64(bytes, E_PHOFF);
    let table_bytes = (phnum as u64) * 56; // phnum <= 8: cannot overflow
    let Some(phend) = phoff.checked_add(table_bytes) else {
        return Err("elf: phdr table bounds overflow");
    };
    if phend > bytes.len() as u64 {
        return Err("elf: phdr table past end of image");
    }

    // Bounds proven: every phdr field read below is inside the image.
    let phoff = phoff as usize;
    let entry = le64(bytes, E_ENTRY);
    let mut info = ImageInfo {
        entry,
        nsegs: 0,
        segs: [SegInfo::EMPTY; MAX_SEGMENTS],
    };

    for i in 0..phnum {
        let ph = phoff + i * 56;
        let ptype = le32(bytes, ph + P_TYPE);
        if ptype == PT_INTERP {
            return Err("elf: PT_INTERP rejected — no dynamic images");
        }
        if ptype != PT_LOAD {
            // Inert per the ELF rule that unknown phdr types may be
            // ignored: only PT_LOAD consumes any loader action.
            continue;
        }
        let flags = le32(bytes, ph + P_FLAGS);
        let offset = le64(bytes, ph + P_OFFSET);
        let vaddr = le64(bytes, ph + P_VADDR);
        let filesz = le64(bytes, ph + P_FILESZ);
        let memsz = le64(bytes, ph + P_MEMSZ);

        if flags & !PF_KNOWN != 0 {
            return Err("elf: unknown segment flags");
        }
        if flags & (PF_W | PF_X) == PF_W | PF_X {
            return Err("elf: W+X segment violates W^X (ADR-0008)");
        }
        if memsz == 0 {
            return Err("elf: PT_LOAD with zero memsz");
        }
        if filesz > memsz {
            return Err("elf: filesz exceeds memsz");
        }
        if vaddr % paging::PAGE != 0 {
            return Err("elf: p_vaddr not page-aligned");
        }
        if vaddr >= USER_HALF_LIMIT {
            return Err("elf: segment starts in the kernel half");
        }
        let Some(vend) = vaddr.checked_add(memsz) else {
            return Err("elf: segment VA span overflows");
        };
        if vend > USER_HALF_LIMIT {
            return Err("elf: segment reaches the kernel half");
        }
        let Some(fend) = offset.checked_add(filesz) else {
            return Err("elf: segment file span overflows");
        };
        if fend > bytes.len() as u64 {
            return Err("elf: segment data past end of image");
        }
        for prev in &info.segs[..info.nsegs] {
            let prev_end = prev.vaddr + prev.memsz; // validated: no overflow
            if vaddr < prev_end && prev.vaddr < vend {
                return Err("elf: segments overlap");
            }
        }
        info.segs[info.nsegs] = SegInfo {
            vaddr,
            memsz,
            filesz,
            offset,
            flags,
        };
        info.nsegs += 1;
    }

    if info.nsegs == 0 {
        return Err("elf: no PT_LOAD segments");
    }
    let entry_in_x = info.segs[..info.nsegs].iter().any(|s| {
        s.flags & PF_X != 0 && entry >= s.vaddr && entry < s.vaddr + s.memsz // no overflow: validated
    });
    if !entry_in_x {
        return Err("elf: entry not inside an executable segment");
    }
    Ok(info)
}

/// Validate `bytes` and load them into process `pid`'s address space:
/// per segment page — refuse an occupied VA (checked before allocating,
/// so a refused double-load leaks nothing), allocate a frame, zero it,
/// copy the page's file-backed prefix, map it with the segment's flags
/// (W^X re-enforced by the mapper). The frames belong to the address
/// space; `proc::destroy` reclaims them.
pub fn load(bytes: &[u8], pid: u64) -> Result<LoadInfo, &'static str> {
    let info = validate(bytes)?;
    without_interrupts(|| {
        let root = proc::pml4_of(pid).ok_or("elf: target process is dead")?;
        let mut pages = 0u64;
        for seg in &info.segs[..info.nsegs] {
            let writable = seg.flags & PF_W != 0;
            let exec = seg.flags & PF_X != 0;
            let npages = seg.memsz.div_ceil(paging::PAGE);
            for k in 0..npages {
                let va = seg.vaddr + k * paging::PAGE;
                // SAFETY: ring 0, IF=0 (without_interrupts); `root` is
                // the live process's owned PML4 (checked above); this is
                // introspection only.
                let occupied = unsafe { paging::user_pte_flags(root, va).is_some() };
                if occupied {
                    return Err("elf: va already mapped in target (overlapping load?)");
                }
                let frame = frames::alloc().ok_or("elf: out of frames")?;
                // SAFETY: `frame` was just allocated (exclusive owner:
                // us); RAM frames are direct-mapped at
                // phys+KERNEL_OFFSET. The copy source is in-bounds:
                // validate proved offset+filesz <= len, and
                // file_pos + n <= offset + filesz by construction.
                unsafe {
                    let dst = (frame + paging::KERNEL_OFFSET) as *mut u8;
                    core::ptr::write_bytes(dst, 0, paging::PAGE as usize);
                    let file_pos = seg.offset + k * paging::PAGE;
                    let avail = seg.filesz.saturating_sub(k * paging::PAGE);
                    let n = avail.min(paging::PAGE) as usize;
                    if n > 0 {
                        core::ptr::copy_nonoverlapping(
                            bytes.as_ptr().add(file_pos as usize),
                            dst,
                            n,
                        );
                    }
                }
                // SAFETY: ring 0, IF=0; `root` live and owned; `frame`
                // exclusive; `va` canonical lower half and page-aligned
                // (validated); W^X enforced twice (validate + mapper).
                unsafe {
                    paging::map_user_page_4k(root, va, frame, writable, exec).map_err(|e| {
                        // The frame never became the space's: free it so
                        // a mapping refusal does not leak.
                        let _ = frames::free(frame);
                        e
                    })?;
                }
                pages += 1;
            }
        }
        Ok(LoadInfo {
            entry: info.entry,
            pages,
        })
    })
}
