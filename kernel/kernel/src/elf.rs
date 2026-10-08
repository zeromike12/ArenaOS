//! ELF64 executable format — the ArenaOS strict-subset validator and
//! image loader (M4.1, ADR-0016).
//!
//! The container is ELF; the semantics are entirely ours. [`validate`]
//! accepts the closed fixed-address `ET_EXEC` subset and the bounded
//! static `ET_DYN` profile in ADR-0110 — page-aligned `PT_LOAD` segments
//! in the canonical lower half, W^X, entry inside executable memory, and
//! no interpreter or unvalidated dynamic mechanism — and refuses everything else with a distinct message per
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
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 0x3e;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_GNU_STACK: u32 = 0x6474_e551;

const PT_MAX: usize = 8;
const PIE_MAX_PAGES: u64 = 128;
const PIE_MAX_RELOCS: usize = 512;
const PIE_MAX_RELA_BYTES: u64 = PIE_MAX_RELOCS as u64 * 24;
const PIE_MAX_DYNAMIC_BYTES: u64 = paging::PAGE;
const PIE_MAX_METADATA_BYTES: u64 = 16 * 1024;
pub const PIE_BIAS_ALIGN: u64 = 2 * 1024 * 1024;
pub const PIE_ARENA_START: u64 = 0x0000_4000_0000_0000;
pub const PIE_ARENA_END: u64 = 0x0000_6000_0000_0000;

const DT_NULL: i64 = 0;
const DT_HASH: i64 = 4;
const DT_STRTAB: i64 = 5;
const DT_SYMTAB: i64 = 6;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_RELAENT: i64 = 9;
const DT_STRSZ: i64 = 10;
const DT_SYMENT: i64 = 11;
const DT_DEBUG: i64 = 21;
const DT_FLAGS: i64 = 30;
const DT_RELACOUNT: i64 = 0x6fff_fff9;
const DT_FLAGS_1: i64 = 0x6fff_fffb;
const DF_BIND_NOW: u64 = 0x8;
const DF_1_NOW: u64 = 0x1;
const DF_1_PIE: u64 = 0x0800_0000;
const R_X86_64_RELATIVE: u32 = 8;

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
const E_FLAGS: usize = 48;

// Elf64_Phdr field offsets, relative to the phdr's start.
const P_TYPE: usize = 0;
const P_FLAGS: usize = 4;
const P_OFFSET: usize = 8;
const P_VADDR: usize = 16;
const P_FILESZ: usize = 32;
const P_MEMSZ: usize = 40;
const P_ALIGN: usize = 48;

/// The M4 test image — a genuine cargo/rust-lld artifact
/// (`userspace/payload`, built by `tools/build.sh` before the kernel),
/// embedded at compile time. Until Phase 5 brings storage, the kernel is
/// its own only filesystem. Layout contract: `payload.ld` + `m4.rs`.
pub static TEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/payload/target/x86_64-unknown-none/release/arena-payload");

/// Real Rust/rust-lld static PIE mutation fixture (ADR-0110). The
/// reproducible source crate and build receipt are under userspace/phase14-pie.
pub static PIE_TEST_IMAGE: &[u8] = include_bytes!("../../../userspace/phase14-pie/fixture.elf");

/// The minimal shell (M4.6, ADR-0020): spawn-registry image 1, built
/// by `tools/build.sh` from `userspace/shell` with the same
/// strict-subset contract as the payload — a genuine cargo/rust-lld
/// ET_EXEC artifact at fixed addresses (0x400000 text / 0x410000
/// data+bss), embedded by `include_bytes!` and spawned by the boot
/// sequence as the initial service.
pub static SHELL_IMAGE: &[u8] =
    include_bytes!("../../../userspace/shell/target/x86_64-unknown-none/release/arena-shell");

/// The block service (M5.2, ADR-0022): spawn-registry image 2, the
/// userspace virtio-blk driver `storaged`, built by `tools/build.sh`
/// from `userspace/storaged` under the same strict-subset contract
/// (fixed 0x400000/0x410000 window, two PT_LOAD segments). Spawned at
/// boot after the m5 suite; the suite itself spawns a short-lived test
/// instance to prove the service boundary.
pub static STORAGED_IMAGE: &[u8] =
    include_bytes!("../../../userspace/storaged/target/x86_64-unknown-none/release/arena-storaged");

/// The block-service test client (M5.2): spawn-registry image 3,
/// `blktest` — built from the same crate as `storaged` (shared ABI
/// module). It allocates a buffer frame, lends it through IPC, and
/// verifies a write→read-back cycle against the scratch disk. Only the
/// m5 suite spawns it.
pub static BLKTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/storaged/target/x86_64-unknown-none/release/blktest");

/// The filesystem service (M5.3, ADR-0023): spawn-registry image 4,
/// `fsd` — AFS1 (extent data + CoW transactional metadata) served from
/// ring 3 over the block service. Built by `tools/build.sh` from
/// `userspace/fsd` under the same strict-subset contract. Spawned at
/// boot after storaged, before the shell; the m5 suite spawns its own
/// short-lived instance for the fs_service test.
pub static FSD_IMAGE: &[u8] =
    include_bytes!("../../../userspace/fsd/target/x86_64-unknown-none/release/fsd");

/// The filesystem-service test client (M5.3): spawn-registry image 5,
/// `fstest` — built from the same crate as `fsd` (shared ABI module).
/// It drives create → write → close → re-open → read → verify → ls
/// through fsd's endpoint. Only the m5 suite spawns it.
pub static FSTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/fsd/target/x86_64-unknown-none/release/fstest");

/// The network service (M6.1, ADR-0024): spawn-registry image 6,
/// `netd` — the userspace virtio-net driver, link-layer only (raw
/// Ethernet frames in/out). Built by `tools/build.sh` from
/// `userspace/netd` under the same strict-subset contract. Spawned at
/// boot after fsd when a virtio-net function exists; the m6 suite
/// spawns its own short-lived instance for the net_service test.
pub static NETD_IMAGE: &[u8] =
    include_bytes!("../../../userspace/netd/target/x86_64-unknown-none/release/arena-netd");

/// The network-service test client (M6.1): spawn-registry image 7,
/// `nettest` — built from the same crate as `netd` (shared ABI
/// module). It fetches the device MAC, hand-builds a 42-byte ARP
/// request for the slirp gateway, sends it through the service, and
/// verifies the reply's protocol fields byte-for-byte. Only the m6
/// suite spawns it.
pub static NETTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/netd/target/x86_64-unknown-none/release/nettest");

/// The entropy service (M6.2, ADR-0025): spawn-registry image 8,
/// `rngd` — the userspace virtio-rng driver, built on the SHARED
/// virtio core (`userspace/virtio.rs`). Built by `tools/build.sh`
/// from `userspace/rngd` under the same strict-subset contract.
/// Spawned at boot after netd when a virtio-rng function exists; the
/// m6 suite spawns its own short-lived instance for the rng_service
/// test.
pub static RNGD_IMAGE: &[u8] =
    include_bytes!("../../../userspace/rngd/target/x86_64-unknown-none/release/arena-rngd");

/// The entropy-service test client (M6.2): spawn-registry image 9,
/// `rngtest` — built from the same crate as `rngd` (shared ABI
/// module). It takes two 4 KiB draws into SEPARATE frames through the
/// service and asserts real variance: neither draw all-zero, neither
/// a single repeated byte, and the two draws different. Only the m6
/// suite spawns it.
pub static RNGTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/rngd/target/x86_64-unknown-none/release/rngtest");

/// The input service (M6.3, ADR-0026): spawn-registry image 10,
/// `inputd` — the userspace virtio-input keyboard driver, the fourth
/// on the SHARED virtio core (`userspace/virtio.rs`). Built by
/// `tools/build.sh` from `userspace/inputd` under the same
/// strict-subset contract. Spawned at boot after rngd when a
/// virtio-input function exists — with the ConsoleInput capability,
/// so its decoded keystrokes feed the console's line discipline; the
/// m6 suite spawns its own instance WITHOUT that cap, which puts the
/// same image in service mode.
pub static INPUTD_IMAGE: &[u8] =
    include_bytes!("../../../userspace/inputd/target/x86_64-unknown-none/release/arena-inputd");

/// The input-service test client (M6.3): spawn-registry image 11,
/// `inputtest` — built from the same crate as `inputd` (shared ABI
/// module). It reads DECODED key bytes through the service boundary
/// and verifies the sequence the harness typed on the virtual
/// keyboard, byte for byte. Only the m6 suite spawns it.
pub static INPUTTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/inputd/target/x86_64-unknown-none/release/inputtest");

/// The console channel service (M6.4, ADR-0027): spawn-registry image
/// 12, `consoled` — the userspace virtio-console driver. The boot
/// sequence spawns it with BOTH console capabilities, which is what
/// turns a byte pipe into a second console: host bytes enter the
/// kernel's line discipline, kernel console output leaves through the
/// port. The m6 suite spawns its own instance with NEITHER, which puts
/// the same image in service mode.
pub static CONSOLED_IMAGE: &[u8] =
    include_bytes!("../../../userspace/consoled/target/x86_64-unknown-none/release/arena-consoled");

/// The console-service test client (M6.4): spawn-registry image 13,
/// `contest` — built from the same crate as `consoled`. It drives a
/// round trip through the service boundary: bytes out the port that
/// the harness reads off the host socket, and bytes back from the
/// harness verified byte-for-byte. Only the m6 suite spawns it.
pub static CONTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/consoled/target/x86_64-unknown-none/release/contest");

/// The fault-injection service (M6.5, ADR-0028): spawn-registry image
/// 14, `faultd` — the smallest service in the system and the only one
/// written to be killed. Spawned only by the m6 suite.
pub static FAULTD_IMAGE: &[u8] =
    include_bytes!("../../../userspace/faultd/target/x86_64-unknown-none/release/arena-faultd");

/// The client that outlives it (M6.5): spawn-registry image 15,
/// `faulttest` — proves a destroyed server becomes a typed
/// `STATUS_SERVICE_GONE` rather than a caller blocked forever.
pub static FAULTTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/faultd/target/x86_64-unknown-none/release/faulttest");

/// The timer-facility proof (M7.0, ADR-0029): spawn-registry image 16,
/// `timertest` — arms real timers from ring 3 and measures them
/// against the monotonic clock. Only the m7 suite spawns it.
pub static TIMERTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/timertest/target/x86_64-unknown-none/release/timertest");

/// The network stack service (M7.1, ADR-0030): spawn-registry image
/// 17, `netstackd` — protocol state kept OUT of the NIC driver. ARP
/// over IPv4 and its cache, built on netd's L2 frame boundary.
pub static NETSTACKD_IMAGE: &[u8] = include_bytes!(
    "../../../userspace/netstackd/target/x86_64-unknown-none/release/arena-netstackd"
);

/// The first protocol's proof (M7.1): image 18, `arptest` — resolves
/// on the real wire, proves the cache keeps the wire quiet, and
/// proves a silent address comes back unreachable rather than hanging.
pub static ARPTEST_IMAGE: &[u8] =
    include_bytes!("../../../userspace/netstackd/target/x86_64-unknown-none/release/arptest");

/// Ring-3 service manager (ADR-0037), image 19. The bootstrap slice
/// validates live caps/readiness and spawns the initial production
/// stack. Restart/failure recovery still needs separate proof.
pub static SERVICEMGR_IMAGE: &[u8] = include_bytes!(
    "../../../userspace/servicemgr/target/x86_64-unknown-none/release/arena-servicemgr"
);

/// ADR-0045: short-lived pre-spawn dependency probe, image 20.
pub static DEPCHECK_IMAGE: &[u8] =
    include_bytes!("../../../userspace/depcheck/target/x86_64-unknown-none/release/arena-depcheck");

/// ADR-0046 Phase 8.1 read boundary: a real FS-backed config service
/// its endpoint-only reader, and separately marker-authorized updater.
pub static CONFIGD_IMAGE: &[u8] =
    include_bytes!("../../../userspace/configd/target/x86_64-unknown-none/release/arena-configd");
pub static CONFIGREAD_IMAGE: &[u8] =
    include_bytes!("../../../userspace/configd/target/x86_64-unknown-none/release/configread");
pub static CONFIGUP_IMAGE: &[u8] =
    include_bytes!("../../../userspace/configd/target/x86_64-unknown-none/release/configup");

/// ADR-0048/0049: mediated permission receiver and one-cap application.
/// The bounded PING worker reuses the audited image20 dependency probe.
pub static PERMISSIOND_IMAGE: &[u8] = include_bytes!(
    "../../../userspace/permissiond/target/x86_64-unknown-none/release/arena-permissiond"
);
pub static PERMAPP_IMAGE: &[u8] =
    include_bytes!("../../../userspace/permissiond/target/x86_64-unknown-none/release/permapp");

/// ADR-0053: verify-only stage receiver, offline pinned source closure.
/// This image never acquires an on-disk Image capability or private key.
pub static PACKAGED_IMAGE: &[u8] =
    include_bytes!("../../../userspace/packaged/target/x86_64-unknown-none/release/arena-packaged");

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
    /// Fixed-address `ET_EXEC` or bounded static `ET_DYN`.
    pub kind: ImageKind,
    /// `e_entry` — guaranteed inside a `PF_X` segment's span.
    pub entry: u64,
    /// Lowest page-aligned PT_LOAD virtual address in the file.
    pub image_start: u64,
    /// Page-rounded end of the highest PT_LOAD virtual address in the file.
    pub image_end: u64,
    /// Number of accepted `PT_LOAD` segments (1..=[`MAX_SEGMENTS`]).
    pub nsegs: usize,
    /// The accepted segments (first `nsegs` entries are meaningful).
    pub segs: [SegInfo; MAX_SEGMENTS],
    /// File offset to the validated RELA table (zero for `ET_EXEC`).
    pub rela_offset: u64,
    /// Validated `R_X86_64_RELATIVE` record count (zero for `ET_EXEC`).
    pub rela_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    FixedExec,
    StaticPie,
}

/// What [`load`] placed into the target address space.
pub struct LoadInfo {
    /// Initial RIP for the image (the ELF's `e_entry`).
    pub entry: u64,
    /// Lowest PT_LOAD address after applying this launch's load bias.
    pub image_base: u64,
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

fn file_offset_for_vaddr(info: &ImageInfo, address: u64, size: u64) -> Option<usize> {
    let end = address.checked_add(size)?;
    for seg in &info.segs[..info.nsegs] {
        let file_end = seg.vaddr.checked_add(seg.filesz)?;
        if address >= seg.vaddr && end <= file_end {
            let offset = seg.offset.checked_add(address - seg.vaddr)?;
            let file_end = offset.checked_add(size)?;
            if file_end <= usize::MAX as u64 {
                return Some(offset as usize);
            }
        }
    }
    None
}

#[derive(Clone, Copy)]
struct DynamicHeader {
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    flags: u32,
}

/// Validate the intentionally small `rust-lld` static-PIE profile in
/// ADR-0110. All file and virtual-address ranges are checked before any
/// relocation record is read. This validator is also used at Image
/// registration, so malformed metadata never reaches process mapping.
fn validate_pie(bytes: &[u8]) -> Result<ImageInfo, &'static str> {
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
    if bytes[6] != EI_VERSION_ONE || le32(bytes, E_VERSION) != 1 {
        return Err("elf: bad ELF version");
    }
    if bytes[7] != 0 || bytes[8] != 0 || bytes[9..16].iter().any(|b| *b != 0) {
        return Err("elf: unsupported ET_DYN ABI identification");
    }
    if le16(bytes, E_TYPE) != ET_DYN || le16(bytes, E_MACHINE) != EM_X86_64 {
        return Err("elf: static PIE requires ET_DYN EM_X86_64");
    }
    if le32(bytes, E_FLAGS) != 0 {
        return Err("elf: static PIE e_flags must be zero");
    }
    if le16(bytes, E_EHSIZE) != 64 || le16(bytes, E_PHENTSIZE) != 56 {
        return Err("elf: static PIE header sizes are invalid");
    }

    let phnum = le16(bytes, E_PHNUM) as usize;
    if phnum == 0 || phnum > PT_MAX {
        return Err("elf: static PIE program-header count outside 1..=8");
    }
    let phoff = le64(bytes, E_PHOFF);
    if phoff < 64 || !phoff.is_multiple_of(8) {
        return Err("elf: static PIE program-header offset is invalid");
    }
    let table_bytes = (phnum as u64) * 56;
    let Some(phend) = phoff.checked_add(table_bytes) else {
        return Err("elf: phdr table bounds overflow");
    };
    if phend > bytes.len() as u64 {
        return Err("elf: phdr table past end of image");
    }

    let phoff = phoff as usize;
    let entry = le64(bytes, E_ENTRY);
    let mut info = ImageInfo {
        kind: ImageKind::StaticPie,
        entry,
        image_start: u64::MAX,
        image_end: 0,
        nsegs: 0,
        segs: [SegInfo::EMPTY; MAX_SEGMENTS],
        rela_offset: 0,
        rela_count: 0,
    };
    let mut dynamic = None;
    let mut stack_seen = false;
    let mut load_pages = 0u64;

    for i in 0..phnum {
        let ph = phoff + i * 56;
        let ptype = le32(bytes, ph + P_TYPE);
        let flags = le32(bytes, ph + P_FLAGS);
        let offset = le64(bytes, ph + P_OFFSET);
        let vaddr = le64(bytes, ph + P_VADDR);
        let filesz = le64(bytes, ph + P_FILESZ);
        let memsz = le64(bytes, ph + P_MEMSZ);
        let align = le64(bytes, ph + P_ALIGN);

        match ptype {
            PT_LOAD => {
                if flags & !PF_KNOWN != 0 || flags & PF_R == 0 {
                    return Err("elf: static PIE PT_LOAD flags must include R and use only R/W/X");
                }
                if flags & (PF_W | PF_X) == (PF_W | PF_X) {
                    return Err("elf: W+X segment violates W^X (ADR-0008)");
                }
                if memsz == 0 || filesz > memsz {
                    return Err("elf: static PIE PT_LOAD sizes are invalid");
                }
                if align != paging::PAGE
                    || !vaddr.is_multiple_of(paging::PAGE)
                    || !offset.is_multiple_of(paging::PAGE)
                    || vaddr % align != offset % align
                {
                    return Err("elf: static PIE PT_LOAD alignment is invalid");
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
                let Some(page_end) = vend
                    .checked_add(paging::PAGE - 1)
                    .map(|end| end & !(paging::PAGE - 1))
                else {
                    return Err("elf: segment page rounding overflows");
                };
                for prev in &info.segs[..info.nsegs] {
                    let prev_end = prev
                        .vaddr
                        .checked_add(prev.memsz)
                        .and_then(|end| end.checked_add(paging::PAGE - 1))
                        .map(|end| end & !(paging::PAGE - 1))
                        .ok_or("elf: prior segment page rounding overflows")?;
                    if vaddr < prev_end && prev.vaddr < page_end {
                        return Err("elf: PT_LOAD page-rounded overlap");
                    }
                }
                if info.nsegs == MAX_SEGMENTS {
                    return Err("elf: too many PT_LOAD segments");
                }
                let pages = (page_end - vaddr) / paging::PAGE;
                load_pages = load_pages
                    .checked_add(pages)
                    .ok_or("elf: static PIE page-count overflow")?;
                if load_pages > PIE_MAX_PAGES {
                    return Err("elf: static PIE exceeds the 128-page load budget");
                }
                info.image_start = info.image_start.min(vaddr);
                info.image_end = info.image_end.max(page_end);
                info.segs[info.nsegs] = SegInfo {
                    vaddr,
                    memsz,
                    filesz,
                    offset,
                    flags,
                };
                info.nsegs += 1;
            }
            PT_DYNAMIC => {
                if dynamic.is_some() {
                    return Err("elf: multiple PT_DYNAMIC headers");
                }
                if flags & !PF_KNOWN != 0 || flags & PF_R == 0 || flags & PF_X != 0 {
                    return Err("elf: PT_DYNAMIC permissions are unsupported");
                }
                if filesz == 0
                    || filesz != memsz
                    || filesz > PIE_MAX_DYNAMIC_BYTES
                    || !filesz.is_multiple_of(16)
                    || align != 8
                    || vaddr % align != offset % align
                {
                    return Err("elf: PT_DYNAMIC size/alignment is invalid");
                }
                let Some(fend) = offset.checked_add(filesz) else {
                    return Err("elf: PT_DYNAMIC file span overflows");
                };
                let Some(vend) = vaddr.checked_add(memsz) else {
                    return Err("elf: PT_DYNAMIC VA span overflows");
                };
                if fend > bytes.len() as u64 || vend > USER_HALF_LIMIT {
                    return Err("elf: PT_DYNAMIC lies outside the bounded image");
                }
                dynamic = Some(DynamicHeader {
                    offset,
                    vaddr,
                    filesz,
                    memsz,
                    flags,
                });
            }
            PT_GNU_STACK => {
                if stack_seen {
                    return Err("elf: multiple PT_GNU_STACK headers");
                }
                stack_seen = true;
                if flags & !PF_KNOWN != 0
                    || flags & PF_X != 0
                    || offset != 0
                    || vaddr != 0
                    || filesz != 0
                    || memsz != 0
                    || align != 0
                {
                    return Err("elf: PT_GNU_STACK must be empty and non-executable");
                }
            }
            PT_INTERP => return Err("elf: PT_INTERP rejected — no dynamic linker"),
            _ => return Err("elf: unsupported ET_DYN program-header type"),
        }
    }

    if info.nsegs == 0 {
        return Err("elf: no PT_LOAD segments");
    }
    if info
        .image_end
        .checked_sub(info.image_start)
        .is_none_or(|span| span > PIE_MAX_PAGES * paging::PAGE)
    {
        return Err("elf: static PIE image span exceeds 128 pages");
    }
    let entry_in_x = info.segs[..info.nsegs]
        .iter()
        .any(|seg| seg.flags & PF_X != 0 && entry >= seg.vaddr && entry < seg.vaddr + seg.memsz);
    if !entry_in_x {
        return Err("elf: entry not inside an executable segment");
    }

    let dynamic = dynamic.ok_or("elf: static PIE requires exactly one PT_DYNAMIC")?;
    if dynamic.memsz != dynamic.filesz
        || dynamic.flags & PF_R == 0
        || file_offset_for_vaddr(&info, dynamic.vaddr, dynamic.filesz)
            != Some(dynamic.offset as usize)
    {
        return Err("elf: PT_DYNAMIC is not backed by readable PT_LOAD file bytes");
    }

    let dynamic_file = dynamic.offset as usize;
    let dynamic_count = (dynamic.filesz / 16) as usize;
    let mut tags = [None; 12];
    let mut null_seen = false;
    for index in 0..dynamic_count {
        let at = dynamic_file + index * 16;
        let tag = le64(bytes, at) as i64;
        let value = le64(bytes, at + 8);
        if tag == DT_NULL {
            if index + 1 != dynamic_count {
                return Err("elf: DT_NULL must be the final dynamic record");
            }
            if value != 0 {
                return Err("elf: DT_NULL value must be zero");
            }
            null_seen = true;
            break;
        }
        let slot = match tag {
            DT_FLAGS => 0,
            DT_FLAGS_1 => 1,
            DT_DEBUG => 2,
            DT_RELA => 3,
            DT_RELASZ => 4,
            DT_RELAENT => 5,
            DT_RELACOUNT => 6,
            DT_SYMTAB => 7,
            DT_SYMENT => 8,
            DT_STRTAB => 9,
            DT_STRSZ => 10,
            DT_HASH => 11,
            _ => return Err("elf: unsupported PT_DYNAMIC tag"),
        };
        if tags[slot].replace(value).is_some() {
            return Err("elf: duplicate PT_DYNAMIC tag");
        }
    }
    if !null_seen || tags.iter().any(Option::is_none) || dynamic_count != 13 {
        return Err("elf: PT_DYNAMIC does not match the bounded PIE tag profile");
    }
    let flags = tags[0].unwrap();
    let flags1 = tags[1].unwrap();
    let debug = tags[2].unwrap();
    let rela_vaddr = tags[3].unwrap();
    let rela_size = tags[4].unwrap();
    let rela_ent = tags[5].unwrap();
    let rela_count_tag = tags[6].unwrap();
    let symtab_vaddr = tags[7].unwrap();
    let syment = tags[8].unwrap();
    let strtab_vaddr = tags[9].unwrap();
    let strsz = tags[10].unwrap();
    let hash_vaddr = tags[11].unwrap();
    if flags != DF_BIND_NOW || flags1 != (DF_1_NOW | DF_1_PIE) || debug != 0 {
        return Err("elf: PT_DYNAMIC flags/debug are unsupported");
    }
    if rela_ent != 24 || rela_size % 24 != 0 || rela_size > PIE_MAX_RELA_BYTES {
        return Err("elf: RELA entry-size/table bounds are invalid");
    }
    let rela_count = usize::try_from(rela_size / 24)
        .map_err(|_| "elf: RELA count does not fit the kernel bound")?;
    if rela_count > PIE_MAX_RELOCS || rela_count_tag != rela_count as u64 {
        return Err("elf: DT_RELACOUNT does not match DT_RELASZ");
    }
    if syment != 24 || strsz != 1 {
        return Err("elf: static PIE symbol/string metadata size is unsupported");
    }
    let symtab = file_offset_for_vaddr(&info, symtab_vaddr, 24)
        .ok_or("elf: DT_SYMTAB is outside file-backed PT_LOAD bytes")?;
    if !symtab_vaddr.is_multiple_of(8) || bytes[symtab..symtab + 24].iter().any(|b| *b != 0) {
        return Err("elf: DT_SYMTAB must contain only the null symbol");
    }
    let strtab = file_offset_for_vaddr(&info, strtab_vaddr, 1)
        .ok_or("elf: DT_STRTAB is outside file-backed PT_LOAD bytes")?;
    if bytes[strtab] != 0 {
        return Err("elf: DT_STRTAB must be the one-byte empty string table");
    }
    let hash = file_offset_for_vaddr(&info, hash_vaddr, 16)
        .ok_or("elf: DT_HASH is outside file-backed PT_LOAD bytes")?;
    if !hash_vaddr.is_multiple_of(4)
        || le32(bytes, hash) != 1
        || le32(bytes, hash + 4) != 1
        || le32(bytes, hash + 8) != 0
        || le32(bytes, hash + 12) != 0
    {
        return Err("elf: DT_HASH must describe the null-only symbol table");
    }

    let rela_offset = if rela_count == 0 {
        if rela_size != 0 {
            return Err("elf: empty RELA count has a non-empty table");
        }
        0
    } else {
        if !rela_vaddr.is_multiple_of(8) {
            return Err("elf: DT_RELA address is unaligned");
        }
        file_offset_for_vaddr(&info, rela_vaddr, rela_size)
            .ok_or("elf: DT_RELA is outside file-backed PT_LOAD bytes")? as u64
    };
    let metadata_bytes = dynamic
        .filesz
        .checked_add(rela_size)
        .and_then(|n| n.checked_add(24 + 1 + 16))
        .ok_or("elf: PIE metadata size overflow")?;
    if metadata_bytes > PIE_MAX_METADATA_BYTES {
        return Err("elf: PIE dynamic/relocation metadata exceeds 16 KiB");
    }

    if rela_count != 0 {
        let mut targets = [0u64; PIE_MAX_RELOCS];
        let rela_file = rela_offset as usize;
        for index in 0..rela_count {
            let at = rela_file + index * 24;
            let target = le64(bytes, at);
            let info_word = le64(bytes, at + 8);
            let symbol = (info_word >> 32) as u32;
            let kind = info_word as u32;
            let addend = le64(bytes, at + 16) as i64;
            if kind != R_X86_64_RELATIVE {
                return Err("elf: unsupported x86-64 relocation type");
            }
            if symbol != 0 {
                return Err("elf: symbolic relocation is unsupported");
            }
            if !target.is_multiple_of(8) || addend < 0 {
                return Err("elf: RELATIVE target/addend alignment or sign is invalid");
            }
            let target_end = target
                .checked_add(8)
                .ok_or("elf: RELATIVE target span overflows")?;
            let destination = info.segs[..info.nsegs].iter().any(|seg| {
                seg.flags & PF_W != 0
                    && seg.flags & PF_X == 0
                    && target >= seg.vaddr
                    && target_end <= seg.vaddr + seg.memsz
            });
            if !destination {
                return Err("elf: RELATIVE destination is outside writable non-X PT_LOAD memory");
            }
            let addend = addend as u64;
            let addend_in_image = info.segs[..info.nsegs]
                .iter()
                .any(|seg| addend >= seg.vaddr && addend < seg.vaddr + seg.memsz);
            if !addend_in_image {
                return Err("elf: RELATIVE addend is outside loaded image memory");
            }
            if targets[..index].contains(&target) {
                return Err("elf: duplicate RELATIVE relocation destination");
            }
            targets[index] = target;
        }
    }

    info.rela_offset = rela_offset;
    info.rela_count = rela_count;
    Ok(info)
}

/// Validate `bytes` against the ArenaOS ELF subset (ADR-0016). Every
/// rule refusal carries its own message; the m4 rejection corpus proves
/// each one fires.
pub fn validate(bytes: &[u8]) -> Result<ImageInfo, &'static str> {
    if bytes.len() < MIN_IMAGE {
        return Err("elf: image shorter than ELF header + one phdr");
    }
    if le16(bytes, E_TYPE) == ET_DYN {
        return validate_pie(bytes);
    }
    validate_exec(bytes)
}

/// Historical fixed-address parser. Preserve the accepted `ET_EXEC`
/// rules, including ignored unknown program-header types. Only the new
/// `ET_DYN` path assigns semantics to `PT_DYNAMIC`.
fn validate_exec(bytes: &[u8]) -> Result<ImageInfo, &'static str> {
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
        kind: ImageKind::FixedExec,
        entry,
        image_start: u64::MAX,
        image_end: 0,
        nsegs: 0,
        segs: [SegInfo::EMPTY; MAX_SEGMENTS],
        rela_offset: 0,
        rela_count: 0,
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
        info.image_start = info.image_start.min(vaddr);
        let page_end = vend
            .checked_add(paging::PAGE - 1)
            .ok_or("elf: segment page rounding overflows")?
            & !(paging::PAGE - 1);
        info.image_end = info.image_end.max(page_end);
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
    if info.kind != ImageKind::FixedExec {
        return Err("elf: static PIE requires randomized load_pie placement");
    }
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
            image_base: info.image_start,
            pages,
        })
    })
}

/// Populate a validated static PIE at the trusted kernel-selected bias.
/// Every page starts RW/NX while the process has no runnable threads;
/// checked relative relocations are written through the physical direct
/// map, then each page receives its final segment permissions.
pub fn load_pie(bytes: &[u8], pid: u64, bias: u64) -> Result<LoadInfo, &'static str> {
    let info = validate(bytes)?;
    if info.kind != ImageKind::StaticPie {
        return Err("elf: load_pie requires ET_DYN");
    }
    if !(PIE_ARENA_START..PIE_ARENA_END).contains(&bias) || !bias.is_multiple_of(PIE_BIAS_ALIGN) {
        return Err("elf: selected PIE bias is outside the aligned placement arena");
    }
    let image_base = bias
        .checked_add(info.image_start)
        .ok_or("elf: relocated image base overflow")?;
    let image_end = bias
        .checked_add(info.image_end)
        .ok_or("elf: relocated image end overflow")?;
    let entry = bias
        .checked_add(info.entry)
        .ok_or("elf: relocated entry overflow")?;
    if image_base < PIE_ARENA_START
        || image_end > PIE_ARENA_END
        || image_end > USER_HALF_LIMIT
        || entry < image_base
        || entry >= image_end
    {
        return Err("elf: relocated PIE image is outside the placement arena");
    }

    without_interrupts(|| {
        let root = proc::pml4_of(pid).ok_or("elf: target process is dead")?;
        // Preflight every PT_LOAD leaf before allocating any frame. The
        // whole selected envelope is checked by spawn.rs; here we repeat
        // the actual leaf check at the loader boundary.
        for seg in &info.segs[..info.nsegs] {
            let start = bias
                .checked_add(seg.vaddr)
                .ok_or("elf: relocated segment start overflow")?;
            let end = bias
                .checked_add(seg.vaddr + seg.memsz)
                .ok_or("elf: relocated segment end overflow")?;
            let page_end = end
                .checked_add(paging::PAGE - 1)
                .ok_or("elf: relocated segment page rounding overflow")?
                & !(paging::PAGE - 1);
            let mut va = start;
            while va < page_end {
                // SAFETY: IF=0 and this root belongs to the unpublished child.
                if unsafe { paging::user_pte_flags(root, va).is_some() } {
                    return Err("elf: selected PIE page already mapped");
                }
                va += paging::PAGE;
            }
        }

        let mut pages = 0u64;
        for seg in &info.segs[..info.nsegs] {
            let start = bias
                .checked_add(seg.vaddr)
                .ok_or("elf: relocated segment start overflow")?;
            let end = bias
                .checked_add(seg.vaddr + seg.memsz)
                .ok_or("elf: relocated segment end overflow")?;
            let page_end = end
                .checked_add(paging::PAGE - 1)
                .ok_or("elf: relocated segment page rounding overflow")?
                & !(paging::PAGE - 1);
            let npages = (page_end - start) / paging::PAGE;
            for page_index in 0..npages {
                let va = start + page_index * paging::PAGE;
                let frame = frames::alloc().ok_or("elf: out of frames")?;
                // SAFETY: fresh private frame and validated file prefix.
                unsafe {
                    let dst = (frame + paging::KERNEL_OFFSET) as *mut u8;
                    core::ptr::write_bytes(dst, 0, paging::PAGE as usize);
                    let file_pos = seg.offset + page_index * paging::PAGE;
                    let avail = seg.filesz.saturating_sub(page_index * paging::PAGE);
                    let count = avail.min(paging::PAGE) as usize;
                    if count > 0 {
                        core::ptr::copy_nonoverlapping(
                            bytes.as_ptr().add(file_pos as usize),
                            dst,
                            count,
                        );
                    }
                    // Private construction mapping is writable and NX. No
                    // user thread can run until after the final protection pass.
                    paging::map_user_page_4k(root, va, frame, true, false).map_err(|e| {
                        let _ = frames::free(frame);
                        e
                    })?;
                }
                pages += 1;
            }
        }

        // ELF64 R_X86_64_RELATIVE semantics: *(B + r_offset) = B + A.
        // Validation already proved the file table and destination segment;
        // retain checked arithmetic and verify the private leaf again here.
        for index in 0..info.rela_count {
            let at = info.rela_offset as usize + index * 24;
            let target = bias
                .checked_add(le64(bytes, at))
                .ok_or("elf: relocated RELATIVE target overflow")?;
            let addend = le64(bytes, at + 16) as i64;
            if addend < 0 {
                return Err("elf: negative RELATIVE addend refused");
            }
            let value = bias
                .checked_add(addend as u64)
                .ok_or("elf: relocated RELATIVE value overflow")?;
            if target.checked_add(8).is_none_or(|end| end > image_end)
                || !(image_base..image_end).contains(&value)
                || value >= USER_HALF_LIMIT
            {
                return Err("elf: relocated RELATIVE value/target outside image");
            }
            // SAFETY: IF=0, live unpublished root; the target was validated
            // writable/non-X and mapped private RW/NX above.
            let phys = unsafe { paging::user_pte_phys(root, target & !(paging::PAGE - 1)) }
                .ok_or("elf: RELATIVE destination page is missing")?;
            let cell = (phys + paging::KERNEL_OFFSET + (target & (paging::PAGE - 1))) as *mut u64;
            unsafe { core::ptr::write_volatile(cell, value) };
        }

        // Final page permissions are the ELF segment permissions. Page
        // rounded overlap was rejected by validation, so every leaf has one
        // unambiguous protection owner.
        for seg in &info.segs[..info.nsegs] {
            let start = bias + seg.vaddr;
            let end = bias + seg.vaddr + seg.memsz;
            let page_end = end.div_ceil(paging::PAGE) * paging::PAGE;
            let writable = seg.flags & PF_W != 0;
            let executable = seg.flags & PF_X != 0;
            let mut va = start;
            while va < page_end {
                // SAFETY: IF=0 and leaf was just mapped in this child.
                unsafe {
                    paging::protect_user_page_4k(root, va, writable, executable)?;
                    let flags = paging::user_pte_flags(root, va)
                        .ok_or("elf: final PIE page protection lost its leaf")?;
                    let expected = paging::PTE_PRESENT
                        | paging::PTE_USER
                        | if writable { paging::PTE_WRITE } else { 0 }
                        | if executable { 0 } else { paging::PTE_NX };
                    if flags
                        & (paging::PTE_PRESENT
                            | paging::PTE_USER
                            | paging::PTE_WRITE
                            | paging::PTE_NX)
                        != expected
                        || (flags & paging::PTE_WRITE != 0 && flags & paging::PTE_NX == 0)
                    {
                        return Err(
                            "elf: final PIE page protections differ from the validated segment",
                        );
                    }
                }
                va += paging::PAGE;
            }
        }
        Ok(LoadInfo {
            entry,
            image_base,
            pages,
        })
    })
}
