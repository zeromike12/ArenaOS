//! ArenaOS filesystem service — `fsd`, the AFS1 server (M5.3,
//! ADR-0023). Spawn-registry image 4, spawned at boot after storaged
//! (and as a short-lived test instance by the m5 suite) with
//! kernel-literal grants:
//!
//! - slot 0: `Endpoint` (WRITE — the call side of storaged's block
//!   service; every fsd disk operation is one forwarded block call),
//! - slot 1: `Endpoint` (READ — the serve side of the FS service).
//!
//! AFS1 is this OS's own design (see ADR-0023 and `tools/afs1.py`,
//! the host-side mirror of every constant below):
//!
//! - **extent-based data, written in place**: a file's sectors are
//!   named by extent blocks (`start`, `len` runs); once a data sector
//!   belongs to a file offset it is overwritten in place — no CoW of
//!   file data in v1.
//! - **copy-on-write metadata**: the object table (32 × 64 B records)
//!   and the allocation bitmap (1 bit/sector) live in RAM during a
//!   transaction and are written to FRESH contiguous 4-sector runs at
//!   commit — never over the live generation.
//! - **transactional commit**: a ping-pong commit record (sectors 1/2,
//!   slot = `seq % 2`) flips the filesystem to the new generation with
//!   ONE 512-byte sector write (sector atomicity is the documented v1
//!   assumption). Data and extent blocks are written BEFORE the commit
//!   record, so a crash mid-transaction leaves the previous generation
//!   intact — leaks are possible, corruption is not. Freed metadata
//!   runs of the generation BEFORE LAST are reclaimed inside the
//!   current transaction (two-generation delay), so no sector a valid
//!   commit points at is ever overwritten.
//! - **reclamation across reboots (M5.4)**: the dead lists are RAM
//!   state, so mount additionally reclaims the SUPERSEDED ping-pong
//!   generation — the newest valid commit won, so the other slot's
//!   objtab/bitmap runs are unreferenced by definition and return to
//!   the allocator in the RAM bitmap (persisted by the next commit; a
//!   crash before it keeps them allocated — a leak, never damage).
//!   Sectors freed by an INTERRUPTED transaction's dead list stay
//!   conservatively allocated: v1 leaks honestly rather than guessing.
//! - **transactional delete (M5.4)**: UNLINK clears the object record
//!   and queues every sector of its extent chain on the dead list
//!   inside one transaction — the namespace change is atomic with the
//!   commit flip, and the sectors outlive any valid old commit that
//!   could still reference them.
//!
//! Zero-copy end to end: a client LENDS its buffer frame with the FS
//! call; fsd never maps it (lent caps cannot be mapped — by design)
//! but FORWARDS the cap to storaged, which points the device at the
//! CLIENT's page. File data flows disk ↔ client frame with no copy in
//! any ring; fsd's own metadata I/O uses one owned scratch frame
//! (lend-keep pattern, ADR-0022).
//!
//! Exit codes (the diagnostic contract with the m5 suite): 42 clean
//! shutdown, 80 mount refused (corrupt/foreign image — details on the
//! console), 81 metadata disk I/O failed, 83 recv refused, 84 reply
//! refused, 97 console refused, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

// ---- the grant layout (kernel-literal: entry.rs / m5.rs) --------------------

const SLOT_BLK: u64 = 0;
const SLOT_EP: u64 = 1;
/// Scratch slots for the one owned metadata frame — the self-map
/// CONSUMES the original; the LENT copy travels with every storaged
/// call. Client buffer caps land in the first free slot (2) and are
/// destroyed after each request.
const SLOT_SCRATCH: u64 = 8;
const SLOT_SCRATCH_LENT: u64 = 9;

// ---- failure exits (the m5 fs_service test maps each one) -------------------

const EXIT_MOUNT: u64 = 80;
const EXIT_DISK: u64 = 81;
const EXIT_RECV: u64 = 83;
const EXIT_REPLY: u64 = 84;
// (82 was the commit-failure exit; commits that lose the disk die via
// EXIT_DISK inside their writes, and a full disk is a per-request
// FS_ERR_NO_SPACE with rollback — never a server death.)

// ---- AFS1 on-disk geometry (byte-for-byte mirror of tools/afs1.py) -----------

const AFS_MAGIC: &[u8; 8] = b"ARENAFS1";
const COMMIT_MAGIC: u32 = 0x4353_4641; // b"AFSC" little-endian
const AFS_VERSION: u32 = 1;
const OBJ_COUNT: usize = 32;
const OBJ_RECORD: usize = 64;
const OBJTAB_SECTORS: u32 = 4; // 32 * 64 = 2048 bytes
const BITMAP_SECTORS: u32 = 4; // 16384 bits = the v1 ceiling
const COMMIT_BASE: u32 = 1; // slots at sectors 1 and 2
const CHECKSUM_OFF: usize = 504; // FNV-1a u64 over bytes [0, 504)
const FNV_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

// object record field offsets
const O_TYPE: usize = 0;
const O_NAMELEN: usize = 1;
const O_EHEAD: usize = 4;
const O_SIZE: usize = 8;
const O_NAME: usize = 32;
const OBJ_FREE: u8 = 0;
const OBJ_FILE: u8 = 1;

// extent block geometry
const EXT_NEXT: usize = 0;
const EXT_COUNT: usize = 4;
const EXT_ENTRIES: usize = 8;
const EXT_PER_BLOCK: u32 = 62; // (512 - 8) / 8

// commit record field offsets
const C_SEQ: usize = 8;
const C_OBJTAB: usize = 24;
const C_BITMAP: usize = 28;

/// In-RAM working copies of the CoW'd metadata (4 KiB total .bss).
/// Single-threaded server: one `&mut Fs` thread of control, raw
/// pointer access, no aliases.
static mut OBJTAB: [u8; 2048] = [0; 2048];
static mut BITMAP: [u8; 2048] = [0; 2048];

// ---- runtime limits -----------------------------------------------------------

/// Open-file table slots (fh = index + 1; 0 is never a handle).
const OPEN_MAX: usize = 8;
/// Dead-list bound per generation. Overflow LEAKS sectors (logged) —
/// leaks are the honest failure mode; corruption is not an option.
const DEAD_MAX: usize = 64;
// v1 files hold at most one extent block's worth of runs (62); the
// on-disk chain format (`next`) is future-proof, the server is not.

// ---- diagnostics --------------------------------------------------------------

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("fsd: FATAL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m5 contract.
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write_str("fsd: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

// ---- unaligned-safe little-endian field access --------------------------------

unsafe fn rd32(p: *const u8, off: usize) -> u32 {
    // SAFETY: caller passes a valid 4-byte span.
    unsafe { core::ptr::read_unaligned(p.add(off) as *const u32) }
}

unsafe fn rd64(p: *const u8, off: usize) -> u64 {
    // SAFETY: as rd32.
    unsafe { core::ptr::read_unaligned(p.add(off) as *const u64) }
}

unsafe fn wr32(p: *mut u8, off: usize, v: u32) {
    // SAFETY: as rd32.
    unsafe { core::ptr::write_unaligned(p.add(off) as *mut u32, v) }
}

unsafe fn wr64(p: *mut u8, off: usize, v: u64) {
    // SAFETY: as rd32.
    unsafe { core::ptr::write_unaligned(p.add(off) as *mut u64, v) }
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h = FNV_BASIS;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Verify a 512-byte scratch sector's trailing FNV-1a checksum.
unsafe fn checksum_ok(p: *const u8) -> bool {
    // SAFETY: p names a full sector in fsd's own mapped scratch frame.
    unsafe { fnv1a64(core::slice::from_raw_parts(p, CHECKSUM_OFF)) == rd64(p, CHECKSUM_OFF) }
}

// ---- the filesystem state -------------------------------------------------------

struct Fs {
    total_sectors: u32,
    seq: u64,
    cur_ot: u32,
    cur_bm: u32,
    /// The mapped scratch frame (fsd's own — metadata I/O only).
    scratch: u64,
    /// Lifetime count of block-service calls (meta + forwarded).
    disk_ops: u64,
    /// Open-file table: object index + 1 per handle; 0 = closed.
    open: [u16; OPEN_MAX],
    /// Sectors of the generation before last — freed at the NEXT
    /// transaction's start (two-generation delay, ADR-0023).
    dead_now: [u32; DEAD_MAX],
    dead_now_n: usize,
    /// Sectors dying in the CURRENT transaction (old cur runs + CoW'd
    /// extent blocks) — become `dead_now` at commit.
    dead_next: [u32; DEAD_MAX],
    dead_next_n: usize,
}

impl Fs {
    fn scratch_ptr(&self) -> *mut u8 {
        self.scratch as *mut u8
    }
}

/// One block-service call: forward `op` on `sector` with the buffer at
/// `buf_off` inside the frame named by `cap_slot` (`CAP_NONE` = no
/// buffer, shutdown-style). Returns the device status word; Err means
/// the TRANSPORT refused (endpoint dead/queue full) — a machine fault
/// for metadata I/O, an `FS_ERR_IO` for client I/O.
unsafe fn block_call(
    fs: &mut Fs,
    op: u64,
    sector: u64,
    buf_off: u64,
    cap_slot: u64,
) -> Result<u64, ()> {
    let mut reply = [0u64; 3];
    // SAFETY: `reply` is on this thread's own (registered) stack; the
    // endpoint cap is the granted slot 0; the buffer cap is fsd's own
    // lent scratch copy or a forwarded client cap.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_BLK,
            sector,
            block_req_w1(op, buf_off),
            cap_slot,
            reply.as_mut_ptr() as u64,
            0, // the block protocol carries no inline message
        )
    };
    if r < 0 {
        log_line(|o| {
            o.str("fsd: block call(op ");
            o.u64(op);
            o.str(" sector ");
            o.u64(sector);
            o.str(") transport refused: ");
            o.i64(r);
        });
        return Err(());
    }
    fs.disk_ops += 1;
    Ok(reply[0])
}

/// Metadata disk read: sector → scratch frame. Fatal on any failure —
/// fsd cannot serve a filesystem it cannot read.
unsafe fn disk_read(fs: &mut Fs, sector: u32) {
    let st = unsafe { block_call(fs, OP_READ, u64::from(sector), 0, SLOT_SCRATCH_LENT) };
    match st {
        Ok(VIRTIO_BLK_S_OK) => {}
        Ok(s) => {
            log_line(|o| {
                o.str("fsd: READ sector ");
                o.u64(u64::from(sector));
                o.str(" device status ");
                o.u64(s);
            });
            fail(EXIT_DISK, "a metadata read failed at the device");
        }
        Err(()) => fail(EXIT_DISK, "a metadata read lost the block service"),
    }
}

/// Metadata disk write: scratch frame → sector. Fatal on any failure.
unsafe fn disk_write(fs: &mut Fs, sector: u32) {
    let st = unsafe { block_call(fs, OP_WRITE, u64::from(sector), 0, SLOT_SCRATCH_LENT) };
    match st {
        Ok(VIRTIO_BLK_S_OK) => {}
        Ok(s) => {
            log_line(|o| {
                o.str("fsd: WRITE sector ");
                o.u64(u64::from(sector));
                o.str(" device status ");
                o.u64(s);
            });
            fail(EXIT_DISK, "a metadata write failed at the device");
        }
        Err(()) => fail(EXIT_DISK, "a metadata write lost the block service"),
    }
}

// ---- bitmap (RAM copy; persisted by commit) --------------------------------------

unsafe fn bit_get(s: u32) -> bool {
    // SAFETY: s < 16384 is checked by every caller path (allocation
    // scans bound by total_sectors <= BITMAP coverage).
    unsafe {
        let bm = core::ptr::addr_of!(BITMAP);
        (*bm)[(s / 8) as usize] & (1 << (s % 8)) != 0
    }
}

unsafe fn bit_set(s: u32, v: bool) {
    // SAFETY: as bit_get; single writer.
    unsafe {
        let bm = core::ptr::addr_of_mut!(BITMAP);
        let byte = &mut (*bm)[(s / 8) as usize];
        if v {
            *byte |= 1 << (s % 8);
        } else {
            *byte &= !(1 << (s % 8));
        }
    }
}

unsafe fn free_count(fs: &Fs) -> u32 {
    let mut n = 0u32;
    for s in 0..fs.total_sectors {
        // SAFETY: bounded scan.
        if unsafe { !bit_get(s) } {
            n += 1;
        }
    }
    n
}

unsafe fn alloc_sector(fs: &Fs) -> Option<u32> {
    for s in 0..fs.total_sectors {
        // SAFETY: bounded scan; single writer.
        unsafe {
            if !bit_get(s) {
                bit_set(s, true);
                return Some(s);
            }
        }
    }
    None
}

unsafe fn alloc_run(fs: &Fs, n: u32) -> Option<u32> {
    let mut start: Option<u32> = None;
    let mut run = 0u32;
    for s in 0..fs.total_sectors {
        // SAFETY: bounded scan.
        let free = unsafe { !bit_get(s) };
        if free {
            if run == 0 {
                start = Some(s);
            }
            run += 1;
            if run == n {
                let base = start.unwrap();
                for k in 0..n {
                    // SAFETY: base+k < total_sectors (scan bound).
                    unsafe { bit_set(base + k, true) };
                }
                return Some(base);
            }
        } else {
            run = 0;
        }
    }
    None
}

unsafe fn push_dead(fs: &mut Fs, s: u32) {
    if fs.dead_next_n < DEAD_MAX {
        fs.dead_next[fs.dead_next_n] = s;
        fs.dead_next_n += 1;
    } else {
        // The honest degradation: a bounded dead list means an
        // overflowing transaction LEAKS sectors (they stay marked
        // used forever). Logged, never fatal, never corruption.
        log_line(|o| {
            o.str("fsd: WARNING — dead list full, leaking sector ");
            o.u64(u64::from(s));
        });
    }
}

// ---- object table (RAM copy; persisted by commit) --------------------------------

unsafe fn obj_ptr(i: usize) -> *mut u8 {
    // SAFETY: i < OBJ_COUNT enforced by callers; 64-byte stride.
    unsafe {
        core::ptr::addr_of_mut!(OBJTAB)
            .cast::<u8>()
            .add(i * OBJ_RECORD)
    }
}

unsafe fn obj_type(i: usize) -> u8 {
    // SAFETY: as obj_ptr.
    unsafe { *obj_ptr(i).add(O_TYPE) }
}

unsafe fn obj_ehead(i: usize) -> u32 {
    // SAFETY: as obj_ptr.
    unsafe { rd32(obj_ptr(i), O_EHEAD) }
}

unsafe fn obj_set_ehead(i: usize, v: u32) {
    // SAFETY: as obj_ptr.
    unsafe { wr32(obj_ptr(i), O_EHEAD, v) }
}

unsafe fn obj_size(i: usize) -> u64 {
    // SAFETY: as obj_ptr.
    unsafe { rd64(obj_ptr(i), O_SIZE) }
}

unsafe fn obj_set_size(i: usize, v: u64) {
    // SAFETY: as obj_ptr.
    unsafe { wr64(obj_ptr(i), O_SIZE, v) }
}

/// Compare object `i`'s name against `name`; equal?
unsafe fn obj_name_eq(i: usize, name: &[u8]) -> bool {
    // SAFETY: as obj_ptr; record is 64 bytes, name field 32, and the
    // stored length is validated against the field width first.
    unsafe {
        let p = obj_ptr(i);
        let nlen = *p.add(O_NAMELEN) as usize;
        nlen == name.len()
            && nlen <= FS_NAME_MAX
            && core::slice::from_raw_parts(p.add(O_NAME), nlen) == name
    }
}

unsafe fn obj_find_name(name: &[u8]) -> Option<usize> {
    // SAFETY: bounded indices; reads go through the checked accessors.
    unsafe { (0..OBJ_COUNT).find(|&i| obj_type(i) == OBJ_FILE && obj_name_eq(i, name)) }
}

/// Clear record `i` back to free (CREATE rollback).
unsafe fn obj_create_free(i: usize) {
    // SAFETY: as obj_ptr.
    unsafe {
        let p = obj_ptr(i);
        core::ptr::write_bytes(p, 0, OBJ_RECORD);
    }
}

unsafe fn obj_find_free() -> Option<usize> {
    // SAFETY: bounded indices.
    unsafe { (0..OBJ_COUNT).find(|&i| obj_type(i) == OBJ_FREE) }
}

unsafe fn obj_create(i: usize, name: &[u8]) {
    // SAFETY: as obj_ptr; caller holds a free record.
    unsafe {
        let p = obj_ptr(i);
        *p.add(O_TYPE) = OBJ_FILE;
        *p.add(O_NAMELEN) = name.len() as u8;
        *p.add(2) = 0; // flags
        *p.add(3) = 0; // pad
        wr32(p, O_EHEAD, 0);
        wr64(p, O_SIZE, 0);
        wr64(p, 16, 0); // mtime — no clock in v1
        wr64(p, 24, 0); // reserved
        for k in 0..FS_NAME_MAX {
            *p.add(O_NAME + k) = if k < name.len() { name[k] } else { 0 };
        }
    }
}

// ---- extents -----------------------------------------------------------------------

/// Map a file sector to its physical sector by walking the extent
/// chain (generic: follows `next`, though the v1 server keeps depth 1).
///
/// An extent entry is `(phys_start, len)`; the FILE position of an
/// entry is the cumulative sum of the lengths before it (extents are
/// ordered by file position — the classic implicit-offset extent tree).
/// v1 appends only at EOF (gap writes are refused), so the cumulative
/// mapping always agrees with the object's size.
unsafe fn ext_lookup(fs: &mut Fs, obj: usize, file_sector: u32) -> Option<u32> {
    let mut cur = unsafe { obj_ehead(obj) };
    let mut guard = 0u32;
    let mut fpos = 0u32; // file sectors covered by the extents so far
    while cur != 0 {
        guard += 1;
        if guard > 64 {
            return None; // a runaway chain is corruption; treat as absent
        }
        unsafe { disk_read(fs, cur) };
        // SAFETY: scratch holds the extent block.
        let (next, count) = unsafe {
            (
                rd32(fs.scratch_ptr(), EXT_NEXT),
                rd32(fs.scratch_ptr(), EXT_COUNT),
            )
        };
        if count > EXT_PER_BLOCK {
            return None;
        }
        for k in 0..count {
            // SAFETY: k < 62; entries at 8 + k*8.
            let (st, len) = unsafe {
                (
                    rd32(fs.scratch_ptr(), EXT_ENTRIES + (k as usize) * 8),
                    rd32(fs.scratch_ptr(), EXT_ENTRIES + (k as usize) * 8 + 4),
                )
            };
            if len == 0 {
                return None; // a zero-length extent is corruption
            }
            if file_sector >= fpos && file_sector < fpos + len {
                return Some(st + (file_sector - fpos));
            }
            fpos += len;
        }
        cur = next;
    }
    None
}

/// Append the physical run `(start, len)` at the file's EOF: CoW the
/// file's extent block (merge into the last run when PHYSICALLY
/// contiguous — the file position is contiguous by construction, since
/// v1 only appends at EOF) and push the replaced block onto the dead
/// list. Err = out of space or the v1 per-file extent limit (62 runs).
unsafe fn ext_add(fs: &mut Fs, obj: usize, start: u32, len: u32) -> Result<(), ()> {
    let head = unsafe { obj_ehead(obj) };
    if head == 0 {
        let Some(nb) = (unsafe { alloc_sector(fs) }) else {
            return Err(());
        };
        // SAFETY: build the fresh one-entry block in scratch.
        unsafe {
            let p = fs.scratch_ptr();
            core::ptr::write_bytes(p, 0, SECTOR_BYTES);
            wr32(p, EXT_NEXT, 0);
            wr32(p, EXT_COUNT, 1);
            wr32(p, EXT_ENTRIES, start);
            wr32(p, EXT_ENTRIES + 4, len);
            disk_write(fs, nb);
            obj_set_ehead(obj, nb);
        }
        return Ok(());
    }
    // SAFETY: read the (single) extent block, amend, CoW to a new
    // sector; the old one dies with the next-next generation.
    unsafe {
        disk_read(fs, head);
        let p = fs.scratch_ptr();
        let count = rd32(p, EXT_COUNT);
        if count >= EXT_PER_BLOCK {
            return Err(()); // v1: one extent block per file
        }
        let last = EXT_ENTRIES + ((count - 1) as usize) * 8;
        let (ls, ll) = (rd32(p, last), rd32(p, last + 4));
        if ls + ll == start {
            wr32(p, last + 4, ll + len); // merge the contiguous run
        } else {
            let e = EXT_ENTRIES + (count as usize) * 8;
            wr32(p, e, start);
            wr32(p, e + 4, len);
            wr32(p, EXT_COUNT, count + 1);
        }
        let Some(nb) = alloc_sector(fs) else {
            return Err(());
        };
        disk_write(fs, nb);
        push_dead(fs, head);
        obj_set_ehead(obj, nb);
    }
    Ok(())
}

// ---- transactions ------------------------------------------------------------------

/// Start a transaction: reclaim the generation-before-last's dead
/// sectors into the RAM bitmap (they become free in the NEXT commit —
/// a crash before it simply keeps them allocated: leak, not damage).
unsafe fn begin_tx(fs: &mut Fs) {
    for k in 0..fs.dead_now_n {
        // SAFETY: dead sectors are < total_sectors by construction.
        unsafe { bit_set(fs.dead_now[k], false) };
    }
    if fs.dead_now_n > 0 {
        let n = fs.dead_now_n;
        log_line(|o| {
            o.str("fsd: reclaiming ");
            o.u64(n as u64);
            o.str(" sector(s) of the generation before last");
        });
    }
    fs.dead_now_n = 0;
}

/// Commit: CoW the object table and bitmap to fresh runs, then flip
/// the ping-pong commit record (ONE sector write) to the new
/// generation. Err = no space for the runs (the caller answers
/// FS_ERR_NO_SPACE; the transaction's earlier writes leak honestly).
unsafe fn commit(fs: &mut Fs, why: &str) -> Result<(), ()> {
    let Some(new_ot) = (unsafe { alloc_run(fs, OBJTAB_SECTORS) }) else {
        return Err(());
    };
    let Some(new_bm) = (unsafe { alloc_run(fs, BITMAP_SECTORS) }) else {
        // The objtab run just allocated is now a leak — acceptable:
        // a full disk mid-commit is already the degraded path.
        return Err(());
    };
    // SAFETY: copy the RAM generations out through scratch.
    unsafe {
        let p = fs.scratch_ptr();
        for k in 0..OBJTAB_SECTORS {
            let src = core::ptr::addr_of!(OBJTAB)
                .cast::<u8>()
                .add((k as usize) * SECTOR_BYTES);
            core::ptr::copy_nonoverlapping(src, p, SECTOR_BYTES);
            disk_write(fs, new_ot + k);
        }
        for k in 0..BITMAP_SECTORS {
            let src = core::ptr::addr_of!(BITMAP)
                .cast::<u8>()
                .add((k as usize) * SECTOR_BYTES);
            core::ptr::copy_nonoverlapping(src, p, SECTOR_BYTES);
            disk_write(fs, new_bm + k);
        }
        // The commit record — the transaction's single atomic flip.
        fs.seq += 1;
        core::ptr::write_bytes(p, 0, SECTOR_BYTES);
        wr32(p, 0, COMMIT_MAGIC);
        wr32(p, 4, AFS_VERSION);
        wr64(p, C_SEQ, fs.seq);
        wr32(p, C_OBJTAB, new_ot);
        wr32(p, C_BITMAP, new_bm);
        wr64(
            p,
            CHECKSUM_OFF,
            fnv1a64(core::slice::from_raw_parts(p as *const u8, CHECKSUM_OFF)),
        );
        disk_write(fs, COMMIT_BASE + (fs.seq % 2) as u32);
        // Rotate the dead lists: what died last transaction frees in
        // the NEXT one; this generation's replaced runs start dying.
        fs.dead_now_n = fs.dead_next_n;
        for k in 0..fs.dead_next_n {
            fs.dead_now[k] = fs.dead_next[k];
        }
        fs.dead_next_n = 0;
        for s in fs.cur_ot..fs.cur_ot + OBJTAB_SECTORS {
            push_dead(fs, s);
        }
        for s in fs.cur_bm..fs.cur_bm + BITMAP_SECTORS {
            push_dead(fs, s);
        }
        fs.cur_ot = new_ot;
        fs.cur_bm = new_bm;
    }
    log_line(|o| {
        o.str("fsd: commit seq ");
        o.u64(fs.seq);
        o.str(" (");
        o.str(why);
        o.str("): objtab@");
        o.u64(u64::from(new_ot));
        o.str(" bitmap@");
        o.u64(u64::from(new_bm));
    });
    Ok(())
}

// ---- mount ---------------------------------------------------------------------------

/// Read and verify the superblock, pick the newest valid commit, and
/// load that generation's object table + bitmap into RAM.
unsafe fn mount(fs: &mut Fs) {
    // SAFETY: mount is the whole function's unsafe contract — every
    // read is fsd's own scratch frame, every field verified.
    unsafe {
        disk_read(fs, 0);
        let p = fs.scratch_ptr();
        if core::slice::from_raw_parts(p as *const u8, 8) != AFS_MAGIC {
            fail(EXIT_MOUNT, "sector 0 is not an AFS1 superblock (bad magic)");
        }
        if !checksum_ok(p) {
            fail(EXIT_MOUNT, "the superblock checksum failed");
        }
        let version = rd32(p, 8);
        let block = rd32(p, 12);
        let total = rd32(p, 16);
        let ocount = rd32(p, 20);
        let orec = rd32(p, 24);
        let otabs = rd32(p, 28);
        let bms = rd32(p, 32);
        let cbase = rd32(p, 36);
        if version != AFS_VERSION
            || block != SECTOR_BYTES as u32
            || ocount != OBJ_COUNT as u32
            || orec != OBJ_RECORD as u32
            || otabs != OBJTAB_SECTORS
            || bms != BITMAP_SECTORS
            || cbase != COMMIT_BASE
            || total == 0
            || total > BITMAP_SECTORS * SECTOR_BYTES as u32 * 8
        {
            log_line(|o| {
                o.str("fsd: superblock geometry ");
                o.u64(u64::from(version));
                o.str("/");
                o.u64(u64::from(block));
                o.str("/");
                o.u64(u64::from(total));
                o.str("/");
                o.u64(u64::from(ocount));
            });
            fail(
                EXIT_MOUNT,
                "the superblock geometry is not this fsd's AFS1 v1",
            );
        }
        fs.total_sectors = total;

        // The newest valid commit wins the ping-pong.
        let mut best: Option<(u64, u32, u32)> = None;
        for slot in 0..2u32 {
            disk_read(fs, COMMIT_BASE + slot);
            let p = fs.scratch_ptr();
            if rd32(p, 0) != COMMIT_MAGIC || rd32(p, 4) != AFS_VERSION || !checksum_ok(p) {
                continue;
            }
            let cand = (rd64(p, C_SEQ), rd32(p, C_OBJTAB), rd32(p, C_BITMAP));
            if best.is_none_or(|b: (u64, u32, u32)| cand.0 > b.0) {
                best = Some(cand);
            }
        }
        let Some((seq, ot, bm)) = best else {
            fail(
                EXIT_MOUNT,
                "no valid commit record (both ping-pong slots dead)",
            );
        };
        if ot + OBJTAB_SECTORS > total || bm + BITMAP_SECTORS > total {
            fail(EXIT_MOUNT, "the newest commit points outside the disk");
        }
        fs.seq = seq;
        fs.cur_ot = ot;
        fs.cur_bm = bm;

        for k in 0..OBJTAB_SECTORS {
            disk_read(fs, ot + k);
            let dst = core::ptr::addr_of_mut!(OBJTAB)
                .cast::<u8>()
                .add((k as usize) * SECTOR_BYTES);
            core::ptr::copy_nonoverlapping(fs.scratch_ptr(), dst, SECTOR_BYTES);
        }
        for k in 0..BITMAP_SECTORS {
            disk_read(fs, bm + k);
            let dst = core::ptr::addr_of_mut!(BITMAP)
                .cast::<u8>()
                .add((k as usize) * SECTOR_BYTES);
            core::ptr::copy_nonoverlapping(fs.scratch_ptr(), dst, SECTOR_BYTES);
        }

        // The live generation's own sectors must be marked used —
        // otherwise the bitmap and the commit disagree (corruption).
        for s in [0u32, 1, 2] {
            if !bit_get(s) {
                fail(EXIT_MOUNT, "the bitmap lost a fixed metadata sector");
            }
        }
        for s in ot..ot + OBJTAB_SECTORS {
            if !bit_get(s) {
                fail(EXIT_MOUNT, "the bitmap lost the live object table");
            }
        }
        for s in bm..bm + BITMAP_SECTORS {
            if !bit_get(s) {
                fail(EXIT_MOUNT, "the bitmap lost the live allocation bitmap");
            }
        }

        // M5.4: reclaim the SUPERSEDED ping-pong generation. The
        // newest valid commit won the mount, so whatever record sits
        // in the other slot is unreferenced by definition — its
        // objtab/bitmap runs return to the allocator in the RAM
        // bitmap, and the NEXT commit persists that. A crash before
        // the next commit leaves them allocated: a leak, never
        // damage. The live generation's own sectors are guarded —
        // a stale record can never free what the mount just chose.
        disk_read(fs, COMMIT_BASE + (1 - fs.seq % 2) as u32);
        let p = fs.scratch_ptr();
        if rd32(p, 0) == COMMIT_MAGIC && rd32(p, 4) == AFS_VERSION && checksum_ok(p) {
            let oseq = rd64(p, C_SEQ);
            let (oot, obm) = (rd32(p, C_OBJTAB), rd32(p, C_BITMAP));
            let in_live = |s: u32| -> bool {
                s < 3
                    || (fs.cur_ot..fs.cur_ot + OBJTAB_SECTORS).contains(&s)
                    || (fs.cur_bm..fs.cur_bm + BITMAP_SECTORS).contains(&s)
            };
            if oseq < fs.seq
                && oot + OBJTAB_SECTORS <= fs.total_sectors
                && obm + BITMAP_SECTORS <= fs.total_sectors
            {
                let mut reclaimed = 0u32;
                for s in oot..oot + OBJTAB_SECTORS {
                    if !in_live(s) && bit_get(s) {
                        bit_set(s, false);
                        reclaimed += 1;
                    }
                }
                for s in obm..obm + BITMAP_SECTORS {
                    if !in_live(s) && bit_get(s) {
                        bit_set(s, false);
                        reclaimed += 1;
                    }
                }
                log_line(|o| {
                    o.str("fsd: reclaimed ");
                    o.u64(u64::from(reclaimed));
                    o.str(" superseded metadata sector(s) from commit seq ");
                    o.u64(oseq);
                });
            }
        }
    }
    let free = unsafe { free_count(fs) };
    let files = unsafe { (0..OBJ_COUNT).filter(|&i| obj_type(i) == OBJ_FILE).count() };
    log_line(|o| {
        o.str("fsd: mounted AFS1 — commit seq ");
        o.u64(fs.seq);
        o.str(", objtab@");
        o.u64(u64::from(fs.cur_ot));
        o.str(", bitmap@");
        o.u64(u64::from(fs.cur_bm));
        o.str(", ");
        o.u64(u64::from(fs.total_sectors));
        o.str(" sectors, ");
        o.u64(u64::from(free));
        o.str(" free, ");
        o.u64(files as u64);
        o.str(" file(s)");
    });
}

// ---- the FS operations ----------------------------------------------------------------

/// Parse a request name out of the inline message: the bytes up to the
/// first NUL, NUL-padded to at most FS_NAME_MAX. Rejects empty names,
/// names with no terminator inside the field, and '/' (v1 is
/// flat-namespaced: one object table, no directories).
fn parse_name(msg: &[u8; MSG_BYTES]) -> Option<usize> {
    let mut n = 0usize;
    while n < FS_NAME_MAX && msg[n] != 0 {
        n += 1;
    }
    // A usable name is 1..=31 bytes with its NUL terminator inside the
    // 32-byte field; an empty or unterminated name is refused.
    if n == 0 || n >= FS_NAME_MAX {
        return None;
    }
    if msg[..n].contains(&b'/') {
        return None;
    }
    Some(n)
}

unsafe fn open_alloc(fs: &mut Fs, obj: usize) -> Option<u64> {
    for i in 0..OPEN_MAX {
        if fs.open[i] == 0 {
            fs.open[i] = (obj + 1) as u16;
            return Some(i as u64 + 1); // fh is 1-based
        }
    }
    None
}

unsafe fn open_obj(fs: &Fs, fh: u64) -> Option<usize> {
    if fh == 0 || fh > OPEN_MAX as u64 {
        return None;
    }
    let v = fs.open[(fh - 1) as usize];
    if v == 0 {
        None
    } else {
        Some(usize::from(v) - 1)
    }
}

/// Serve one request. Returns the reply words; `omsg` carries the
/// reply's inline message (LS dirents; zero otherwise).
///
/// # Safety
/// `fs` is the single-threaded server state; `imsg`/`omsg` are this
/// thread's own buffers; `landed` is the client cap slot (or
/// `CAP_NONE`) and is destroyed here after any forwarding.
unsafe fn serve(
    fs: &mut Fs,
    op: u64,
    w1: u64,
    landed: u64,
    imsg: &[u8; MSG_BYTES],
    omsg: &mut [u8; MSG_BYTES],
) -> (u64, u64) {
    // SAFETY: the whole function body operates on fsd's own state and
    // validated message bytes; every disk path goes through the
    // checked helpers.
    unsafe {
        match op {
            FS_OP_CREATE | FS_OP_OPEN => {
                let Some(n) = parse_name(imsg) else {
                    return (FS_ERR_BAD_NAME, 0);
                };
                let name = &imsg[..n];
                let found = obj_find_name(name);
                if op == FS_OP_CREATE {
                    if found.is_some() {
                        return (FS_ERR_EXISTS, 0);
                    }
                    let Some(i) = obj_find_free() else {
                        return (FS_ERR_TABLE_FULL, 0);
                    };
                    obj_create(i, name);
                    begin_tx(fs);
                    if commit(fs, "create").is_err() {
                        // Roll the record back: a refused CREATE must
                        // not leave a phantom that a LATER commit would
                        // persist (honest failure semantics).
                        obj_create_free(i);
                        return (FS_ERR_NO_SPACE, 0);
                    }
                    let Some(fh) = open_alloc(fs, i) else {
                        return (FS_ERR_TABLE_FULL, 0);
                    };
                    log_line(|o| {
                        o.str("fsd: CREATE obj ");
                        o.u64(i as u64);
                        o.str(" → fh ");
                        o.u64(fh);
                    });
                    (FS_OK, fh)
                } else {
                    let Some(i) = found else {
                        return (FS_ERR_NOT_FOUND, 0);
                    };
                    let Some(fh) = open_alloc(fs, i) else {
                        return (FS_ERR_TABLE_FULL, 0);
                    };
                    log_line(|o| {
                        o.str("fsd: OPEN obj ");
                        o.u64(i as u64);
                        o.str(" → fh ");
                        o.u64(fh);
                    });
                    (FS_OK, fh)
                }
            }
            FS_OP_READ | FS_OP_WRITE => {
                let fh = w1 & 0xFF;
                let off = w1 >> 8;
                let len = rd64(imsg.as_ptr(), 0);
                let Some(obj) = open_obj(fs, fh) else {
                    return (FS_ERR_BAD_FH, 0);
                };
                if len == 0 || len > FS_XFER_MAX || landed == CAP_NONE {
                    return (FS_ERR_RANGE, 0);
                }
                let size = obj_size(obj);
                if op == FS_OP_READ {
                    if off >= size {
                        return (FS_OK, 0); // empty read at/after EOF
                    }
                    let actual = if size - off < len { size - off } else { len };
                    let first = off / SECTOR_BYTES as u64;
                    let last = (off + actual).div_ceil(SECTOR_BYTES as u64);
                    for j in first..last {
                        let Some(phys) = ext_lookup(fs, obj, j as u32) else {
                            return (FS_ERR_CORRUPT, 0);
                        };
                        let st = block_call(
                            fs,
                            OP_READ,
                            u64::from(phys),
                            j * SECTOR_BYTES as u64 - off,
                            landed,
                        );
                        if !matches!(st, Ok(VIRTIO_BLK_S_OK)) {
                            return (FS_ERR_IO, 0);
                        }
                    }
                    (FS_OK, actual)
                } else {
                    // WRITE: v1 refuses gap writes (off > size) — the
                    // cumulative extent mapping only stays honest when
                    // new sectors append at EOF. Overwriting existing
                    // sectors is an in-place data write; extending past
                    // EOF appends extents in file order.
                    if off > size {
                        return (FS_ERR_RANGE, 0);
                    }
                    // Pre-check space honestly — data sectors + one
                    // extent block + the commit's two 4-sector runs.
                    let end = off + len;
                    let need = end.div_ceil(SECTOR_BYTES as u64);
                    let have = size.div_ceil(SECTOR_BYTES as u64);
                    let fresh = need.saturating_sub(have);
                    if free_count(fs)
                        < (fresh + 1 + OBJTAB_SECTORS as u64 + BITMAP_SECTORS as u64) as u32
                    {
                        return (FS_ERR_NO_SPACE, 0);
                    }
                    begin_tx(fs);
                    let old_size = size;
                    let first = off / SECTOR_BYTES as u64;
                    for j in first..need {
                        let phys = match ext_lookup(fs, obj, j as u32) {
                            Some(p) => p,
                            None => {
                                let Some(s) = alloc_sector(fs) else {
                                    return (FS_ERR_NO_SPACE, 0);
                                };
                                if ext_add(fs, obj, s, 1).is_err() {
                                    return (FS_ERR_NO_SPACE, 0);
                                }
                                s
                            }
                        };
                        let st = block_call(
                            fs,
                            OP_WRITE,
                            u64::from(phys),
                            j * SECTOR_BYTES as u64 - off,
                            landed,
                        );
                        if !matches!(st, Ok(VIRTIO_BLK_S_OK)) {
                            return (FS_ERR_IO, 0);
                        }
                    }
                    if end > size {
                        obj_set_size(obj, end);
                    }
                    if commit(fs, "write").is_err() {
                        // Roll the size back: any extents already CoW'd
                        // sit beyond the (restored) EOF — unreachable,
                        // persisted-as-allocated at the next commit.
                        // The honest v1 degradation: a leak, never a
                        // half-visible write.
                        obj_set_size(obj, old_size);
                        log_line(|o| {
                            o.str(
                                "fsd: WARNING — write commit refused (disk full); size rolled back",
                            );
                        });
                        return (FS_ERR_NO_SPACE, 0);
                    }
                    log_line(|o| {
                        o.str("fsd: WRITE fh ");
                        o.u64(fh);
                        o.str(" off ");
                        o.u64(off);
                        o.str(" len ");
                        o.u64(len);
                        o.str(" → committed, size ");
                        o.u64(obj_size(obj));
                    });
                    (FS_OK, len)
                }
            }
            FS_OP_CLOSE => {
                let fh = w1 & 0xFF;
                if open_obj(fs, fh).is_none() {
                    return (FS_ERR_BAD_FH, 0);
                }
                fs.open[(fh - 1) as usize] = 0;
                (FS_OK, 0)
            }
            FS_OP_LS => {
                let cursor = w1 as u32;
                let mut idx = cursor as usize;
                while idx < OBJ_COUNT && obj_type(idx) != OBJ_FILE {
                    idx += 1;
                }
                if idx >= OBJ_COUNT {
                    wr32(omsg.as_mut_ptr(), 0, FS_CURSOR_END);
                    wr64(omsg.as_mut_ptr(), 4, 0);
                    wr32(omsg.as_mut_ptr(), 12, 0);
                    return (FS_OK, u64::from(FS_CURSOR_END));
                }
                let p = obj_ptr(idx);
                let nlen = usize::from(*p.add(O_NAMELEN));
                wr32(omsg.as_mut_ptr(), 0, (idx + 1) as u32);
                wr64(omsg.as_mut_ptr(), 4, obj_size(idx));
                wr32(omsg.as_mut_ptr(), 12, nlen as u32);
                for k in 0..nlen {
                    omsg[16 + k] = *p.add(O_NAME + k);
                }
                (FS_OK, idx as u64)
            }
            FS_OP_UNLINK => {
                // M5.4: transactional delete. The record and every
                // sector of the extent chain die with two-generation
                // delay (a valid superseded commit may still point at
                // them); the commit flip removes the name atomically.
                let Some(n) = parse_name(imsg) else {
                    return (FS_ERR_BAD_NAME, 0);
                };
                let name = &imsg[..n];
                let Some(i) = obj_find_name(name) else {
                    return (FS_ERR_NOT_FOUND, 0);
                };
                if (0..OPEN_MAX).any(|k| fs.open[k] == (i + 1) as u16) {
                    // v1: no unlink-at-last-close — deleting under a
                    // live handle would surprise honest clients.
                    return (FS_ERR_BUSY, 0);
                }
                // Snapshot the record for rollback: a commit refused
                // for space must not persist as a deletion LATER.
                let mut rec = [0u8; OBJ_RECORD];
                core::ptr::copy_nonoverlapping(obj_ptr(i), rec.as_mut_ptr(), OBJ_RECORD);
                let dead_mark = fs.dead_next_n;
                begin_tx(fs);
                // Walk the chain: every data run and every extent
                // block dies. Bounded — a runaway chain stops early
                // and the tail leaks honestly (never corruption).
                let mut cur = obj_ehead(i);
                let mut guard = 0u32;
                let mut freed = 0u32;
                while cur != 0 && cur < fs.total_sectors {
                    guard += 1;
                    if guard > 64 {
                        break;
                    }
                    disk_read(fs, cur);
                    let (next, count) = (
                        rd32(fs.scratch_ptr(), EXT_NEXT),
                        rd32(fs.scratch_ptr(), EXT_COUNT),
                    );
                    if count > EXT_PER_BLOCK {
                        break;
                    }
                    for k in 0..count {
                        let (st, len) = (
                            rd32(fs.scratch_ptr(), EXT_ENTRIES + (k as usize) * 8),
                            rd32(fs.scratch_ptr(), EXT_ENTRIES + (k as usize) * 8 + 4),
                        );
                        for s in st..st + len {
                            if s < fs.total_sectors {
                                push_dead(fs, s);
                                freed += 1;
                            }
                        }
                    }
                    push_dead(fs, cur);
                    cur = next;
                }
                obj_create_free(i);
                if commit(fs, "unlink").is_err() {
                    // Roll back: restore the record, un-die the chain
                    // (truncate the dead list to the pre-tx mark).
                    // begin_tx's own reclamation stays — it was due
                    // regardless of THIS transaction's outcome.
                    core::ptr::copy_nonoverlapping(rec.as_ptr(), obj_ptr(i), OBJ_RECORD);
                    fs.dead_next_n = dead_mark;
                    return (FS_ERR_NO_SPACE, 0);
                }
                log_line(|o| {
                    o.str("fsd: UNLINK obj ");
                    o.u64(i as u64);
                    o.str(" → committed (");
                    o.u64(u64::from(freed));
                    o.str(" data sector(s) + the chain free at +2 generations)");
                });
                (FS_OK, 0)
            }
            FS_OP_SHUTDOWN => {
                log_line(|o| {
                    o.str("fsd: shutdown requested after ");
                    o.u64(fs.disk_ops);
                    o.str(" disk operation(s) — replying and exiting");
                });
                (FS_OK, fs.disk_ops)
            }
            _ => (FS_ERR_BAD_OP, 0),
        }
    }
}

/// The server entry: the spawn protocol's first thread lands here at
/// ring 3 with the two endpoint grants in slots 0/1.
///
/// # Safety
/// As every image's `_start`: ring 3, derived stack top, kernel-loaded
/// address space, the documented grants. The body is ABI v1 wrappers
/// over this image's own statics, stack, and allocated window.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: single-threaded program; every unsafe block documents
    // its own contract (own memory, validated inputs, checked helper
    // paths).
    unsafe {
        log_line(|o| o.str("fsd: AFS1 filesystem service starting (M5.3/5.4, ADR-0023)"));

        // The metadata scratch frame: owned, copied (the LENT copy
        // travels with every storaged call), self-mapped (the map
        // CONSUMES the original — the lend-keep pattern, ADR-0022).
        let phys = syscall1(SYS_ALLOC_FRAME, SLOT_SCRATCH);
        if phys <= 0 {
            fail(EXIT_MOUNT, "scratch frame allocation refused");
        }
        if syscall3(SYS_CAP_COPY, SLOT_SCRATCH, SLOT_SCRATCH_LENT, RIGHTS_ALL) < 0 {
            fail(EXIT_MOUNT, "the scratch cap copy refused");
        }
        let win = syscall2(SYS_MAP_MEMORY, SLOT_SCRATCH, 1);
        if win <= 0 {
            fail(EXIT_MOUNT, "the scratch self-map refused");
        }

        let mut fs = Fs {
            total_sectors: 0,
            seq: 0,
            cur_ot: 0,
            cur_bm: 0,
            scratch: win as u64,
            disk_ops: 0,
            open: [0; OPEN_MAX],
            dead_now: [0; DEAD_MAX],
            dead_now_n: 0,
            dead_next: [0; DEAD_MAX],
            dead_next_n: 0,
        };
        mount(&mut fs);

        let mut ibuf = [0u64; 3];
        let mut imsg = [0u8; MSG_BYTES];
        let mut omsg: [u8; MSG_BYTES];
        loop {
            let r = syscall3(
                SYS_IPC_RECV,
                SLOT_EP,
                ibuf.as_mut_ptr() as u64,
                imsg.as_mut_ptr() as u64,
            );
            if r < 0 {
                log_line(|o| {
                    o.str("fsd: recv returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "SYS_IPC_RECV refused");
            }
            let (op, w1, landed) = (ibuf[0], ibuf[1], ibuf[2]);
            omsg = [0u8; MSG_BYTES];
            let (rw0, rw1) = serve(&mut fs, op, w1, landed, &imsg, &mut omsg);
            // The forwarded client cap is done with: discard fsd's
            // reference (frees nothing — the client keeps its window).
            if landed != CAP_NONE {
                let _ = syscall1(SYS_CAP_DESTROY, landed);
            }
            let rr = syscall5(
                SYS_IPC_REPLY,
                SLOT_EP,
                rw0,
                rw1,
                CAP_NONE,
                omsg.as_ptr() as u64,
            );
            if rr < 0 {
                log_line(|o| {
                    o.str("fsd: reply returned ");
                    o.i64(rr);
                });
                fail(EXIT_REPLY, "SYS_IPC_REPLY refused");
            }
            if op == FS_OP_SHUTDOWN && rw0 == FS_OK {
                // The poison discipline (ADR-0022): reply first, then
                // exit by fsd's own hand — a parked server is never
                // destroyed out from under its recv.
                syscall1(SYS_THREAD_EXIT, EXIT_OK);
            }
        }
    }
}
