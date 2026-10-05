//! filesd: the AFS2 file service (Phase 11.5/11.6, ADR-0076/0077).
//!
//! Authority. Every request arrives through a badged endpoint capability
//! (ADR-0074). The badge names one capability record here: an AFS2 object
//! id (generation-exact), a rights mask and the caller's registered I/O
//! page. Requests name children relative to that object; there are no
//! paths, no `..`, and nothing in a request grants anything. The desktop
//! broker's kernel-minted badge names `/Users/user`; `/System` (the AFS1
//! import) has no record anyone can hold. A deleted object makes every
//! record naming it stale (the object generation no longer matches).
//!
//! Storage. The AFS2 volume is the disk from sector 16384 (8 MiB) on, in
//! 4 KiB blocks through storaged's block operations and one lent frame.
//! On a blank region filesd formats a volume and imports AFS1 read-only
//! through fsd; the import is complete only when its final marker commits,
//! and an interrupted import is redone from scratch. A damaged volume is
//! never repaired or reformatted: filesd stays offline (fail closed).
#![no_std]
#![no_main]
#![allow(static_mut_refs, clippy::deref_addrof)]

use arena_afs2::{self as afs, BLOCK, Device, Error as E, SECTOR, Volume};
use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;
#[path = "../../filesd_wire.rs"]
mod wire;
use wire::*;

const SLOT_BLK: u64 = 0;
const SLOT_EP: u64 = 1;
const SLOT_RTC: u64 = 2;
const SLOT_FSD: u64 = 3;
const SLOT_FRAME: u64 = 8;
const SLOT_FRAME_LENT: u64 = 9;
/// The AFS2 region: sectors 16384.. (after the 8 MiB AFS1 legacy area).
const BASE_SECTOR: u64 = 16384;
const VOLUME_BLOCKS: u64 = 16384;
const GRANTS: usize = 256;
/// Live records one grant lineage may hold (the grant itself included):
/// one application cannot exhaust the table for everyone else.
const LINEAGE_QUOTA: usize = 16;

// ---- entry with a guarded dedicated stack (see filesd.ld) ------------------
#[repr(C, align(4096))]
struct Stack([u8; 128 * 1024]);
#[unsafe(link_section = ".stack")]
static mut STACK: Stack = Stack([0; 128 * 1024]);
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "lea rsp, [rip + {stack} + {size}]",
        "call {main}",
        "ud2",
        stack = sym STACK,
        size = const 128 * 1024,
        main = sym main,
    )
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    log(b"filesd: PANIC\n");
    exit(99)
}

fn exit(code: u64) -> ! {
    unsafe {
        syscall1(SYS_THREAD_EXIT, code);
    }
    loop {
        core::hint::spin_loop()
    }
}

fn log(s: &[u8]) {
    unsafe {
        syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64);
    }
}

fn log_num(mut v: u64) {
    let mut d = [0u8; 20];
    let mut n = 0;
    loop {
        d[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    d[..n].reverse();
    log(&d[..n]);
}

// ---- block device over storaged ---------------------------------------------

static mut FRAME: u64 = 0;

struct Blk;

fn block_call(op: u64, sector: u64) -> Result<(), E> {
    let mut reply = [0u64; 3];
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_BLK,
            sector,
            block_req_w1(op, 0),
            SLOT_FRAME_LENT,
            reply.as_mut_ptr() as u64,
            0,
        )
    };
    if r < 0 || reply[0] != VIRTIO_BLK_S_OK {
        return Err(E::Io);
    }
    Ok(())
}

impl Device for Blk {
    fn read(&mut self, block: u64, buf: &mut [u8; BLOCK]) -> afs::Result<()> {
        block_call(OP_READ_BLOCK, BASE_SECTOR + block * 8)?;
        unsafe { core::ptr::copy_nonoverlapping(FRAME as *const u8, buf.as_mut_ptr(), BLOCK) };
        Ok(())
    }
    fn write(&mut self, block: u64, buf: &[u8; BLOCK]) -> afs::Result<()> {
        unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), FRAME as *mut u8, BLOCK) };
        block_call(OP_WRITE_BLOCK, BASE_SECTOR + block * 8)
    }
    /// The commit record: sector 0 of a commit-slot block; the rest of
    /// that block is unused by the format, so a whole-block write carrying
    /// the record and zeros is the same commit.
    fn write_sector(&mut self, block: u64, buf: &[u8; SECTOR]) -> afs::Result<()> {
        unsafe {
            core::ptr::write_bytes(FRAME as *mut u8, 0, BLOCK);
            core::ptr::copy_nonoverlapping(buf.as_ptr(), FRAME as *mut u8, SECTOR);
        }
        block_call(OP_WRITE_BLOCK, BASE_SECTOR + block * 8)
    }
}

static mut VOL: Volume<Blk> = Volume::empty();

// ---- wall time (ADR-0076 "Time": data, never authority) -------------------

static mut WALL_BASE_US: u64 = 0;
static mut MONO_BASE_US: u64 = 0;

fn mono_us() -> u64 {
    unsafe { syscall0(SYS_CLOCK_NOW) }.max(0) as u64
}

/// Wall microseconds since 1970, or 0 = unknown (no or invalid RTC).
fn wall_us() -> u64 {
    let base = unsafe { WALL_BASE_US };
    if base == 0 {
        return 0;
    }
    base + mono_us().saturating_sub(unsafe { MONO_BASE_US })
}

// ---- capability records -------------------------------------------------------

#[derive(Clone, Copy)]
struct Grant {
    live: bool,
    generation: u16,
    object: u64,
    rights: u8,
    /// The registered I/O page of a lineage head (0 = no session yet);
    /// every record of the lineage uses its head's page.
    io: u64,
    /// The grant this record descends from: a record opened from the
    /// broker's /Users/user record (1) is its own lineage; anything opened
    /// from it inherits it. Revoking a lineage retires all of it.
    lineage: u8,
}
const NO_GRANT: Grant = Grant {
    live: false,
    generation: 0,
    object: 0,
    rights: 0,
    io: 0,
    lineage: 0,
};
static mut GRANT: [Grant; GRANTS] = [NO_GRANT; GRANTS];

fn record(badge: u32) -> Option<usize> {
    let i = (badge & 0xffff) as usize;
    let generation = (badge >> 16) as u16;
    let g = unsafe { GRANT.get(i)? };
    (i != 0 && g.live && g.generation == generation).then_some(i)
}

fn release(i: usize) {
    let g = unsafe { &mut GRANT[i] };
    // Only a lineage head maps a page (its records share it).
    if g.io != 0 {
        unsafe {
            syscall6(SYS_SHARED_UNMAP, g.io, 0, 0, 0, 0, 0);
        }
    }
    g.live = false;
    g.io = 0;
    g.object = 0;
    g.rights = 0;
    g.lineage = 0;
}

/// Retire record `i` and, when it heads a lineage, everything derived
/// from it: copies the application passed on go stale with it.
fn revoke(i: usize) {
    let head = usize::from(unsafe { GRANT[i].lineage }) == i;
    release(i);
    if head {
        for j in 2..GRANTS {
            if unsafe { GRANT[j].live && usize::from(GRANT[j].lineage) == i } {
                release(j);
            }
        }
    }
}

/// A new record and its badged capability (in filesd's own slot),
/// derived from record `parent`, placed in the lineage of record `place`
/// (normally `parent` itself).
fn mint(parent: usize, place: usize, object: u64, rights: u8) -> Result<(u64, usize), u64> {
    // A record index whose generation is exhausted retires for good: a
    // badge is never valid twice.
    let i = (2..GRANTS)
        .find(|i| unsafe { !GRANT[*i].live && GRANT[*i].generation < u16::MAX })
        .ok_or(S_FULL)?;
    let lineage = if parent == 1 && place == 1 {
        i
    } else {
        let l = usize::from(unsafe { GRANT[place].lineage });
        let held = (2..GRANTS)
            .filter(|j| unsafe { GRANT[*j].live && usize::from(GRANT[*j].lineage) == l })
            .count();
        if held >= LINEAGE_QUOTA {
            return Err(S_FULL);
        }
        l
    };
    let g = unsafe { &mut GRANT[i] };
    g.generation += 1;
    let badge = i as u32 | u32::from(g.generation) << 16;
    let slot = unsafe {
        syscall6(
            SYS_ENDPOINT_MINT,
            SLOT_EP,
            u64::from(badge),
            RIGHTS_WRITE | RIGHTS_COPY,
            0,
            0,
            0,
        )
    };
    if slot < 0 {
        return Err(S_FULL);
    }
    *g = Grant {
        live: true,
        generation: g.generation,
        object,
        rights,
        io: 0,
        lineage: lineage as u8,
    };
    Ok((slot as u64, i))
}

fn status(e: E) -> u64 {
    match e {
        E::NoEnt => S_NOENT,
        E::Exist => S_EXIST,
        E::NotDir => S_NOTDIR,
        E::IsDir => S_ISDIR,
        E::NotEmpty => S_NOTEMPTY,
        E::Inval => S_INVAL,
        E::NoSpc => S_NOSPC,
        E::FBig => S_FBIG,
        E::Loop => S_LOOP,
        E::Corrupt => S_CORRUPT,
        E::Io | E::Unsupported => S_IO,
    }
}

// ---- requests -------------------------------------------------------------------

struct Reply {
    status: u64,
    value: u64,
    cap: u64,
    /// The record the transferred `cap` names (retired if undelivered).
    minted: usize,
    bytes: [u8; BYTES],
}
fn reply(status: u64, value: u64) -> Reply {
    Reply {
        status,
        value,
        cap: CAP_NONE,
        minted: 0,
        bytes: [0; BYTES],
    }
}

static mut NAME: [u8; 512] = [0; 512];
static mut DATA: [u8; PAGE] = [0; PAGE];

/// Copy `len` bytes of the caller's I/O page at `at` into private memory
/// first: the caller cannot change a name after it is checked.
fn take(io: u64, at: usize, len: usize) -> Option<&'static [u8]> {
    if io == 0 || at + len > PAGE || len > 255 {
        return None;
    }
    unsafe {
        let dst = &mut NAME[at..at + len];
        for (k, b) in dst.iter_mut().enumerate() {
            *b = core::ptr::read_volatile((io as *const u8).add(at + k));
        }
        Some(&NAME[at..at + len])
    }
}

fn handle(badge: u32, req: Request, landed: u64) -> Reply {
    let vol = unsafe { &mut *(&raw mut VOL) };
    let Some(i) = record(badge) else {
        return reply(S_DENIED, 0);
    };
    let mut g = unsafe { GRANT[i] };
    let head = usize::from(g.lineage);
    g.io = unsafe { GRANT[head].io };
    let has = |r: u8| g.rights & r == r;
    let need_io = !matches!(req.op, OP_SESSION | OP_RELEASE | OP_STATFS | OP_REVOKE | OP_TRUNCATE)
        && !(req.op == OP_STAT && req.name_len == 0)
        && !(req.op == OP_OPEN && req.name_len == 0);
    if need_io && g.io == 0 {
        return reply(S_NO_SESSION, 0);
    }
    let name = || take(g.io, 0, usize::from(req.name_len));
    let wall = wall_us();
    match req.op {
        OP_SESSION => {
            let mut d = [0u64; 3];
            if landed == CAP_NONE
                || unsafe { syscall2(SYS_CAP_DESCRIBE, landed, d.as_mut_ptr() as u64) } != 0
                || d[0] != 7
                || d[2] & (RIGHTS_READ | RIGHTS_WRITE) != RIGHTS_READ | RIGHTS_WRITE
            {
                return reply(S_INVAL, 0);
            }
            let va = unsafe { syscall2(SYS_SHARED_MAP, landed, 1) };
            if va <= 0 {
                return reply(S_FULL, 0);
            }
            if g.io != 0 {
                unsafe {
                    syscall6(SYS_SHARED_UNMAP, g.io, 0, 0, 0, 0, 0);
                }
            }
            unsafe { GRANT[head].io = va as u64 };
            reply(S_OK, 0)
        }
        OP_STAT => {
            let object = if req.name_len == 0 {
                g.object
            } else {
                if !has(R_LIST) {
                    return reply(S_DENIED, 0);
                }
                let Some(n) = name() else {
                    return reply(S_INVAL, 0);
                };
                match vol.lookup(g.object, n) {
                    Ok((o, _)) => o,
                    Err(e) => return reply(status(e), 0),
                }
            };
            match vol.stat(object) {
                Ok(st) => {
                    let mut r = reply(S_OK, st.size);
                    r.bytes = StatReply {
                        typ: st.typ,
                        size: st.size,
                        mtime: st.mtime,
                        ctime: st.ctime,
                        entries: st.entries,
                        version: st.version,
                    }
                    .encode();
                    r
                }
                Err(e) => reply(status(e), 0),
            }
        }
        OP_LIST => {
            if !has(R_LIST) {
                return reply(S_DENIED, 0);
            }
            let Some(cursor) = name() else {
                return reply(S_INVAL, 0);
            };
            let mut after = [0u8; 255];
            let after_len = cursor.len();
            after[..after_len].copy_from_slice(cursor);
            let mut entries = [afs::Entry::EMPTY; 8];
            let mut at = 0usize;
            let mut count = 0u64;
            let mut more = false;
            let mut cur = after;
            let mut cur_len = after_len;
            'outer: loop {
                let n = match vol.list(g.object, &cur[..cur_len], &mut entries) {
                    Ok(n) => n,
                    Err(e) => return reply(status(e), 0),
                };
                if n == 0 {
                    break;
                }
                for e in &entries[..n] {
                    let need = LIST_HEAD + usize::from(e.len);
                    if at + need > PAGE {
                        more = true;
                        break 'outer;
                    }
                    let (size, mtime) = match vol.stat(e.object) {
                        Ok(s) => (s.size, s.mtime),
                        Err(err) => return reply(status(err), 0),
                    };
                    unsafe {
                        DATA[at] = e.typ;
                        DATA[at + 1] = e.len;
                        DATA[at + 2..at + 10].copy_from_slice(&size.to_le_bytes());
                        DATA[at + 10..at + 18].copy_from_slice(&mtime.to_le_bytes());
                        DATA[at + 18..at + need].copy_from_slice(e.name());
                    }
                    at += need;
                    count += 1;
                    cur[..usize::from(e.len)].copy_from_slice(e.name());
                    cur_len = usize::from(e.len);
                }
            }
            unsafe { core::ptr::copy_nonoverlapping(DATA.as_ptr(), g.io as *mut u8, at) };
            let mut r = reply(S_OK, count);
            r.bytes[0] = u8::from(more);
            r
        }
        OP_OPEN => {
            let (object, typ) = if req.name_len == 0 {
                match vol.stat(g.object) {
                    Ok(st) => (g.object, st.typ),
                    Err(e) => return reply(status(e), 0),
                }
            } else {
                // Naming a child is listing: it reveals what exists.
                if !has(R_LIST) {
                    return reply(S_DENIED, 0);
                }
                let Some(n) = name() else {
                    return reply(S_INVAL, 0);
                };
                match vol.lookup(g.object, n) {
                    Ok(x) => x,
                    Err(e) => return reply(status(e), 0),
                }
            };
            // A lent record of this service places the new record in its
            // lineage (the broker granting an application: it walks names
            // in its own lineage and the grant is counted against, and
            // revoked with, the application's). Rights still come only
            // from the called record: attenuation only.
            let place = if landed == CAP_NONE {
                i
            } else {
                let b = unsafe { syscall2(SYS_ENDPOINT_BADGE, SLOT_EP, landed) };
                match (b > 0).then(|| record(b as u32)).flatten() {
                    Some(j) if j != 1 => j,
                    _ => return reply(S_DENIED, 0),
                }
            };
            match mint(i, place, object, req.rights & g.rights) {
                Ok((slot, minted)) => Reply {
                    status: S_OK,
                    value: u64::from(typ),
                    cap: slot,
                    minted,
                    bytes: [0; BYTES],
                },
                Err(s) => reply(s, 0),
            }
        }
        OP_CREATE | OP_MKDIR => {
            if !has(R_CREATE) {
                return reply(S_DENIED, 0);
            }
            let Some(n) = name() else {
                return reply(S_INVAL, 0);
            };
            let r = if req.op == OP_CREATE {
                vol.create(g.object, n, wall)
            } else {
                vol.mkdir(g.object, n, wall)
            };
            match r {
                Ok(_) => reply(S_OK, 0),
                Err(e) => reply(status(e), 0),
            }
        }
        wire::OP_READ => {
            if !has(R_READ) || req.len as usize > PAGE {
                return reply(S_DENIED, 0);
            }
            let len = req.len as usize;
            let buf = unsafe { &mut DATA[..len] };
            match vol.read(g.object, req.offset, buf) {
                Ok(n) => {
                    unsafe { core::ptr::copy_nonoverlapping(DATA.as_ptr(), g.io as *mut u8, n) };
                    reply(S_OK, n as u64)
                }
                Err(e) => reply(status(e), 0),
            }
        }
        wire::OP_WRITE => {
            if !has(R_WRITE) || req.len as usize > PAGE {
                return reply(S_DENIED, 0);
            }
            let len = req.len as usize;
            unsafe {
                for k in 0..len {
                    DATA[k] = core::ptr::read_volatile((g.io as *const u8).add(k));
                }
            }
            match vol.write(g.object, req.offset, unsafe { &DATA[..len] }, wall) {
                Ok(n) => reply(S_OK, n as u64),
                Err(e) => reply(status(e), 0),
            }
        }
        OP_TRUNCATE => {
            if !has(R_WRITE) {
                return reply(S_DENIED, 0);
            }
            match vol.truncate(g.object, req.offset, wall) {
                Ok(()) => reply(S_OK, 0),
                Err(e) => reply(status(e), 0),
            }
        }
        OP_UNLINK | OP_RMDIR => {
            if !has(R_DELETE) {
                return reply(S_DENIED, 0);
            }
            let Some(n) = name() else {
                return reply(S_INVAL, 0);
            };
            let r = if req.op == OP_UNLINK {
                vol.unlink(g.object, n, wall)
            } else {
                vol.rmdir(g.object, n, wall)
            };
            match r {
                Ok(()) => reply(S_OK, 0),
                Err(e) => reply(status(e), 0),
            }
        }
        OP_RENAME => {
            if !has(R_RENAME) {
                return reply(S_DENIED, 0);
            }
            let dst = if landed == CAP_NONE {
                g.object
            } else {
                let b = unsafe { syscall2(SYS_ENDPOINT_BADGE, SLOT_EP, landed) };
                let Some(j) = (b > 0).then(|| record(b as u32)).flatten() else {
                    return reply(S_DENIED, 0);
                };
                let d = unsafe { GRANT[j] };
                if d.rights & R_CREATE == 0 {
                    return reply(S_DENIED, 0);
                }
                d.object
            };
            let nl = usize::from(req.name_len);
            let (Some(from), Some(to)) = (take(g.io, 0, nl), take(g.io, nl, req.len as usize))
            else {
                return reply(S_INVAL, 0);
            };
            let mut a = [0u8; 255];
            a[..from.len()].copy_from_slice(from);
            let mut b = [0u8; 255];
            b[..to.len()].copy_from_slice(to);
            match vol.rename(g.object, &a[..from.len()], dst, &b[..to.len()], wall) {
                Ok(_) => reply(S_OK, 0),
                Err(e) => reply(status(e), 0),
            }
        }
        OP_RELEASE => {
            revoke(i);
            reply(S_OK, 0)
        }
        OP_REVOKE => {
            if landed == CAP_NONE {
                return reply(S_INVAL, 0);
            }
            let b = unsafe { syscall2(SYS_ENDPOINT_BADGE, SLOT_EP, landed) };
            match (b > 0).then(|| record(b as u32)).flatten() {
                Some(j) if j != 1 => {
                    revoke(j);
                    reply(S_OK, 0)
                }
                _ => reply(S_DENIED, 0),
            }
        }
        OP_STATFS => {
            let s = vol.statfs();
            let mut r = reply(S_OK, s.free);
            r.bytes[..8].copy_from_slice(&s.blocks.to_le_bytes());
            r.bytes[8..12].copy_from_slice(&s.objects.to_le_bytes());
            r.bytes[16..24].copy_from_slice(&s.seq.to_le_bytes());
            r
        }
        _ => reply(S_INVAL, 0),
    }
}

// ---- format and the one-shot AFS1 import ------------------------------------

const IMPORT_MARKER: &[u8] = b"afs1-import-complete";

fn fs_call(op: u64, w1: u64, cap: u64, msg: &mut [u8; 64]) -> Result<u64, ()> {
    let mut out = [0, 0, CAP_NONE];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_FSD,
            op,
            w1,
            cap,
            out.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    if out[2] != CAP_NONE {
        unsafe {
            syscall1(SYS_CAP_DESTROY, out[2]);
        }
        return Err(());
    }
    if rc != 0 || out[0] != FS_OK {
        return Err(());
    }
    Ok(out[1])
}

fn mkdirs(vol: &mut Volume<Blk>, wall: u64) -> afs::Result<(u64, u64, u64)> {
    let root = vol.root_id()?;
    let sys = vol.mkdir(root, b"System", wall)?;
    let imported = vol.mkdir(sys, b"imported-afs1", wall)?;
    let users = vol.mkdir(root, b"Users", wall)?;
    let user = vol.mkdir(users, b"user", wall)?;
    vol.mkdir(user, b"Desktop", wall)?;
    let docs = vol.mkdir(user, b"Documents", wall)?;
    vol.mkdir(user, b".Trash", wall)?;
    Ok((sys, imported, docs))
}

/// Format the region and import AFS1 (read only, through fsd). User files
/// (`user-*`) go to Documents, everything else to /System/imported-afs1.
fn format_and_import() -> afs::Result<(u64, u64)> {
    let vol = unsafe { &mut *(&raw mut VOL) };
    let wall = wall_us();
    vol.format(Blk, VOLUME_BLOCKS, 0x4146_5332_0000_0001 ^ mono_us(), wall)?;
    let (sys, imported, docs) = mkdirs(vol, wall)?;
    let (mut users, mut system) = (0u64, 0u64);
    let mut cursor = 0u32;
    for _ in 0..64 {
        let mut b = [0u8; 64];
        if fs_call(FS_OP_LS, u64::from(cursor), CAP_NONE, &mut b).is_err() {
            return Err(E::Io);
        }
        let next = u32::from_le_bytes(b[..4].try_into().map_err(|_| E::Io)?);
        let size = u64::from_le_bytes(b[4..12].try_into().map_err(|_| E::Io)?);
        let len = u32::from_le_bytes(b[12..16].try_into().map_err(|_| E::Io)?) as usize;
        if next == FS_CURSOR_END {
            break;
        }
        if len == 0 || len > 31 || next <= cursor {
            return Err(E::Io);
        }
        let mut name = [0u8; 32];
        name[..len].copy_from_slice(&b[16..16 + len]);
        cursor = next;
        let to = if name.starts_with(b"user-") { docs } else { imported };
        let file = vol.create(to, &name[..len], wall)?;
        let mut open = [0u8; 64];
        open[..32].copy_from_slice(&name);
        let fh = fs_call(FS_OP_OPEN, 0, CAP_NONE, &mut open).map_err(|_| E::Io)?;
        let mut off = 0u64;
        while off < size {
            let want = (size - off).min(FS_XFER_MAX);
            let mut m = [0u8; 64];
            m[..8].copy_from_slice(&want.to_le_bytes());
            let n = fs_call(FS_OP_READ, fs_rw_w1(fh, off), SLOT_FRAME_LENT, &mut m)
                .map_err(|_| E::Io)?;
            if n == 0 || n > want {
                return Err(E::Io);
            }
            unsafe { core::ptr::copy_nonoverlapping(FRAME as *const u8, DATA.as_mut_ptr(), n as usize) };
            vol.write(file, off, unsafe { &DATA[..n as usize] }, wall)?;
            off += n;
        }
        let mut close = [0u8; 64];
        let _ = fs_call(FS_OP_CLOSE, fh, CAP_NONE, &mut close);
        if to == docs {
            users += 1;
        } else {
            system += 1;
        }
    }
    // The import is complete only when this marker commits.
    vol.create(sys, IMPORT_MARKER, wall)?;
    Ok((users, system))
}

fn user_root(vol: &mut Volume<Blk>) -> afs::Result<u64> {
    let root = vol.root_id()?;
    let users = vol.lookup(root, b"Users")?.0;
    Ok(vol.lookup(users, b"user")?.0)
}

/// Bring the volume online; false = offline (fail closed or no region).
fn bring_up() -> bool {
    let vol = unsafe { &mut *(&raw mut VOL) };
    let mut probe = [0u8; BLOCK];
    if Blk.read(VOLUME_BLOCKS - 1, &mut probe).is_err() {
        log(b"filesd: no AFS2 region (disk smaller than 72 MiB); file service offline\n");
        return false;
    }
    // A region that never committed (blank, or a format interrupted
    // before its first commit) is formatted; anything that ever committed
    // mounts or fails closed (ADR-0076).
    let blank = match afs::never_committed(&mut Blk) {
        Ok(b) => b,
        Err(_) => {
            log(b"filesd: AFS2 superblock unreadable; file service offline\n");
            return false;
        }
    };
    let mounted = if blank {
        false
    } else {
        match vol.mount(Blk) {
            Ok(()) => true,
            Err(e) => {
                log(b"filesd: AFS2 volume refused to mount (");
                log(match e {
                    E::Corrupt => b"CORRUPT",
                    E::Unsupported => b"UNSUPPORTED",
                    _ => b"IO",
                });
                log(b") - fail closed, never repaired; file service offline\n");
                return false;
            }
        }
    };
    let complete = mounted
        && vol
            .root_id()
            .and_then(|r| vol.lookup(r, b"System"))
            .and_then(|(s, _)| vol.lookup(s, IMPORT_MARKER))
            .is_ok();
    if !complete {
        if mounted {
            log(b"filesd: AFS1 import was interrupted; formatting the AFS2 region again\n");
        }
        match format_and_import() {
            Ok((users, system)) => {
                log(b"filesd: AFS2 formatted; AFS1 import complete: ");
                log_num(users);
                log(b" user file(s) to /Users/user/Documents, ");
                log_num(system);
                log(b" system record(s) to /System/imported-afs1 (AFS1 read only)\n");
            }
            Err(_) => {
                log(b"filesd: format/import failed; file service offline\n");
                return false;
            }
        }
    }
    match user_root(vol) {
        Ok(object) => unsafe {
            GRANT[1] = Grant {
                live: true,
                generation: 1,
                object,
                rights: R_ALL,
                io: 0,
                lineage: 1,
            };
        },
        Err(_) => {
            log(b"filesd: /Users/user missing; file service offline\n");
            return false;
        }
    }
    let s = vol.statfs();
    log(b"filesd: AFS2 mounted seq ");
    log_num(s.seq);
    log(b", ");
    log_num(s.free);
    log(b"/");
    log_num(s.blocks);
    log(b" blocks free, ");
    log_num(u64::from(s.objects));
    log(b" objects; wall clock ");
    log(if wall_us() == 0 { b"unknown" } else { b"from RTC" });
    log(b"\n");
    true
}

extern "C" fn main() -> ! {
    log(b"filesd: AFS2 file service starting (ADR-0076/0077)\n");
    unsafe {
        let phys = syscall1(SYS_ALLOC_FRAME, SLOT_FRAME);
        if phys <= 0 || syscall3(SYS_CAP_COPY, SLOT_FRAME, SLOT_FRAME_LENT, RIGHTS_ALL) < 0 {
            log(b"filesd: transfer frame refused\n");
            exit(70);
        }
        let win = syscall2(SYS_MAP_MEMORY, SLOT_FRAME, 1);
        if win <= 0 {
            log(b"filesd: transfer frame map refused\n");
            exit(71);
        }
        FRAME = win as u64;
        let mut seconds = 0u64;
        if syscall2(SYS_RTC_READ, SLOT_RTC, (&raw mut seconds) as u64) == 0 && seconds != 0 {
            WALL_BASE_US = seconds * 1_000_000;
            MONO_BASE_US = mono_us();
        }
    }
    let online = bring_up();
    loop {
        let mut out = [0u64; 4];
        let mut msg = [0u8; BYTES];
        let rc = unsafe {
            syscall6(
                SYS_IPC_RECV_BADGED,
                SLOT_EP,
                out.as_mut_ptr() as u64,
                msg.as_mut_ptr() as u64,
                1,
                0,
                0,
            )
        };
        if rc != 0 {
            log(b"filesd: receive refused\n");
            exit(72);
        }
        let (landed, badge) = (out[2], out[3] as u32);
        let r = match Request::decode(&msg) {
            _ if !online => reply(S_OFFLINE, 0),
            Some(req) => handle(badge, req, landed),
            None => reply(S_INVAL, 0),
        };
        // A landed capability is only ever lent for this request (a
        // session region stays pinned by its mapping): drop it.
        if landed != CAP_NONE {
            unsafe {
                syscall1(SYS_CAP_DESTROY, landed);
            }
        }
        let rr = unsafe {
            syscall5(
                SYS_IPC_REPLY_CHECKED,
                SLOT_EP,
                r.status,
                r.value,
                r.cap,
                r.bytes.as_ptr() as u64,
            )
        };
        if rr != 0 && r.cap != CAP_NONE {
            // The caller is gone: the minted capability never left; retire it.
            unsafe {
                syscall1(SYS_CAP_DESTROY, r.cap);
            }
            release(r.minted);
        }
    }
}
