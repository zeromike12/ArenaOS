//! Application side of filesd (ADR-0077).
//!
//! A capability names one AFS2 object. Requests carry only names relative
//! to it, through this process's registered I/O page. Paths are a
//! presentation convenience walked here, one component at a time, from a
//! directory capability: there is no global path and no ambient root, so
//! `..` is resolved lexically and can never climb above the capability
//! the walk starts from.
use crate::abi::*;
pub use crate::filesd_wire as wire;
use wire::{BYTES, PAGE, Request, StatReply};

/// Status words are `wire::S_*`; a failed transport reads as `S_IO`.
pub type Status = u64;

pub struct Reply {
    pub value: u64,
    pub cap: u64,
    pub bytes: [u8; BYTES],
}

/// One request through capability slot `cap`, lending `lend` (moved).
pub fn call(cap: u64, req: Request, lend: u64) -> Result<Reply, Status> {
    let mut bytes = req.encode();
    let mut out = [0, 0, CAP_NONE];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            cap,
            0,
            0,
            lend,
            out.as_mut_ptr() as u64,
            bytes.as_mut_ptr() as u64,
        )
    };
    if rc != 0 {
        return Err(wire::S_IO);
    }
    if out[0] != wire::S_OK {
        if out[2] != CAP_NONE {
            unsafe {
                syscall1(SYS_CAP_DESTROY, out[2]);
            }
        }
        return Err(out[0]);
    }
    Ok(Reply {
        value: out[1],
        cap: out[2],
        bytes,
    })
}

fn describe(slot: u64) -> Option<[u64; 3]> {
    let mut d = [0u64; 3];
    (unsafe { syscall2(SYS_CAP_DESCRIBE, slot, d.as_mut_ptr() as u64) } == 0).then_some(d)
}

/// A free capability slot at or above `from`.
pub fn free_slot(from: u64) -> Option<u64> {
    (from..64).find(|s| describe(*s).is_none())
}

/// A copy of `slot` to lend (lending moves the capability).
fn lendable(slot: u64) -> Result<u64, Status> {
    let tmp = free_slot(8).ok_or(wire::S_FULL)?;
    if unsafe { syscall3(SYS_CAP_COPY, slot, tmp, RIGHTS_WRITE | RIGHTS_COPY) } != 0 {
        return Err(wire::S_DENIED);
    }
    Ok(tmp)
}

pub const NAME_MAX: usize = 255;

/// One directory entry of a listing.
#[derive(Clone, Copy)]
pub struct Entry {
    pub typ: u8,
    pub len: u8,
    pub size: u64,
    /// Wall microseconds; 0 = unknown.
    pub mtime: u64,
    pub name: [u8; NAME_MAX],
}
impl Entry {
    pub const EMPTY: Entry = Entry {
        typ: 0,
        len: 0,
        size: 0,
        mtime: 0,
        name: [0; NAME_MAX],
    };
    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.len)]
    }
    pub fn is_dir(&self) -> bool {
        self.typ == 2
    }
}

/// A process's view of filesd: its I/O page (the last page of its own
/// region, registered once per lineage).
pub struct Files {
    io: *mut u8,
}

impl Files {
    /// Register page `page` of the region in slot `region` (mapped here
    /// at `va`) as the I/O page of the lineage of capability `cap`.
    pub fn session(cap: u64, region: u64, va: u64, page: u64) -> Result<Self, Status> {
        let lend = lendable_region(region)?;
        let mut req = Request::new(wire::OP_SESSION);
        req.offset = page;
        call(cap, req, lend)?;
        Ok(Files {
            io: (va + page * PAGE as u64) as *mut u8,
        })
    }

    /// Whether the service behind `cap` is online (no session needed).
    pub fn online(cap: u64) -> bool {
        call(cap, Request::new(wire::OP_STATFS), CAP_NONE).is_ok()
    }

    fn put(&self, at: usize, bytes: &[u8]) -> Result<(), Status> {
        if at + bytes.len() > PAGE {
            return Err(wire::S_INVAL);
        }
        for (k, b) in bytes.iter().enumerate() {
            unsafe { core::ptr::write_volatile(self.io.add(at + k), *b) };
        }
        Ok(())
    }
    fn get(&self, at: usize, out: &mut [u8]) {
        for (k, b) in out.iter_mut().enumerate() {
            *b = unsafe { core::ptr::read_volatile(self.io.add(at + k)) };
        }
    }

    fn named(&self, cap: u64, op: u8, name: &[u8], lend: u64) -> Result<Reply, Status> {
        if name.is_empty() || name.len() > NAME_MAX {
            return Err(wire::S_INVAL);
        }
        self.put(0, name)?;
        let mut req = Request::new(op);
        req.name_len = name.len() as u16;
        call(cap, req, lend)
    }

    pub fn stat(&self, cap: u64, child: Option<&[u8]>) -> Result<StatReply, Status> {
        let r = match child {
            Some(n) => self.named(cap, wire::OP_STAT, n, CAP_NONE)?,
            None => call(cap, Request::new(wire::OP_STAT), CAP_NONE)?,
        };
        Ok(StatReply::decode(&r.bytes))
    }

    /// Entries after `after` (empty = from the start), in name order.
    /// Returns (count, more).
    pub fn list(&self, cap: u64, after: &[u8], out: &mut [Entry]) -> Result<(usize, bool), Status> {
        self.put(0, after)?;
        let mut req = Request::new(wire::OP_LIST);
        req.name_len = after.len() as u16;
        let r = call(cap, req, CAP_NONE)?;
        let count = r.value as usize;
        let mut at = 0usize;
        let mut n = 0usize;
        let mut head = [0u8; wire::LIST_HEAD];
        for _ in 0..count {
            if n == out.len() || at + wire::LIST_HEAD > PAGE {
                return Ok((n, true));
            }
            self.get(at, &mut head);
            let len = usize::from(head[1]);
            if len == 0 || at + wire::LIST_HEAD + len > PAGE {
                return Err(wire::S_CORRUPT);
            }
            let e = &mut out[n];
            e.typ = head[0];
            e.len = head[1];
            e.size = u64::from_le_bytes(head[2..10].try_into().unwrap_or([0; 8]));
            e.mtime = u64::from_le_bytes(head[10..18].try_into().unwrap_or([0; 8]));
            self.get(at + wire::LIST_HEAD, &mut e.name[..len]);
            at += wire::LIST_HEAD + len;
            n += 1;
        }
        Ok((n, r.bytes[0] == 1 || n < count))
    }

    /// A new capability for child `name` of directory `cap` (or `cap`
    /// itself), with at most `rights`. Returns (slot, type).
    pub fn open(&self, cap: u64, name: Option<&[u8]>, rights: u8) -> Result<(u64, u8), Status> {
        let r = match name {
            Some(n) => {
                self.put(0, n)?;
                let mut req = Request::new(wire::OP_OPEN);
                req.name_len = n.len() as u16;
                req.rights = rights;
                call(cap, req, CAP_NONE)?
            }
            None => {
                let mut req = Request::new(wire::OP_OPEN);
                req.rights = rights;
                call(cap, req, CAP_NONE)?
            }
        };
        if r.cap == CAP_NONE {
            return Err(wire::S_IO);
        }
        Ok((r.cap, r.value as u8))
    }

    /// `open` of `cap` itself with at most `rights`, the new record placed
    /// in the lineage of `place` (lent as a copy): the broker's grant.
    pub fn open_in(&self, cap: u64, rights: u8, place: u64) -> Result<(u64, u8), Status> {
        let mut req = Request::new(wire::OP_OPEN);
        req.rights = rights;
        let r = call(cap, req, lendable(place)?)?;
        if r.cap == CAP_NONE {
            return Err(wire::S_IO);
        }
        Ok((r.cap, r.value as u8))
    }

    /// Retire the lineage headed by `head` through `cap` (moves `head`).
    pub fn revoke(&self, cap: u64, head: u64) -> Result<(), Status> {
        call(cap, Request::new(wire::OP_REVOKE), head).map(|_| ())
    }

    pub fn read(&self, cap: u64, offset: u64, out: &mut [u8]) -> Result<usize, Status> {
        let mut req = Request::new(wire::OP_READ);
        req.offset = offset;
        req.len = out.len().min(PAGE) as u32;
        let r = call(cap, req, CAP_NONE)?;
        let n = (r.value as usize).min(req.len as usize);
        self.get(0, &mut out[..n]);
        Ok(n)
    }

    /// The whole file into `out` (refuses a file larger than `out`).
    pub fn read_all<'a>(&self, cap: u64, out: &'a mut [u8]) -> Result<&'a [u8], Status> {
        let size = self.stat(cap, None)?.size;
        if size > out.len() as u64 {
            return Err(wire::S_FBIG);
        }
        let mut at = 0usize;
        while (at as u64) < size {
            let n = self.read(cap, at as u64, &mut out[at..size as usize])?;
            if n == 0 {
                return Err(wire::S_IO);
            }
            at += n;
        }
        Ok(&out[..at])
    }

    pub fn write(&self, cap: u64, offset: u64, data: &[u8]) -> Result<usize, Status> {
        let n = data.len().min(PAGE);
        self.put(0, &data[..n])?;
        let mut req = Request::new(wire::OP_WRITE);
        req.offset = offset;
        req.len = n as u32;
        Ok(call(cap, req, CAP_NONE)?.value as usize)
    }

    /// Replace the file's contents. Each chunk is its own transaction; the
    /// final truncate makes the size exact.
    pub fn write_all(&self, cap: u64, data: &[u8]) -> Result<(), Status> {
        let mut at = 0usize;
        while at < data.len() {
            let n = self.write(cap, at as u64, &data[at..])?;
            if n == 0 {
                return Err(wire::S_IO);
            }
            at += n;
        }
        self.truncate(cap, data.len() as u64)
    }

    pub fn truncate(&self, cap: u64, size: u64) -> Result<(), Status> {
        let mut req = Request::new(wire::OP_TRUNCATE);
        req.offset = size;
        call(cap, req, CAP_NONE).map(|_| ())
    }

    pub fn create(&self, dir: u64, name: &[u8]) -> Result<(), Status> {
        self.named(dir, wire::OP_CREATE, name, CAP_NONE).map(|_| ())
    }
    pub fn mkdir(&self, dir: u64, name: &[u8]) -> Result<(), Status> {
        self.named(dir, wire::OP_MKDIR, name, CAP_NONE).map(|_| ())
    }
    pub fn unlink(&self, dir: u64, name: &[u8]) -> Result<(), Status> {
        self.named(dir, wire::OP_UNLINK, name, CAP_NONE).map(|_| ())
    }
    pub fn rmdir(&self, dir: u64, name: &[u8]) -> Result<(), Status> {
        self.named(dir, wire::OP_RMDIR, name, CAP_NONE).map(|_| ())
    }

    /// Move `from` in directory `dir` to `to` in `dst` (another directory
    /// capability of this service, or `dir` itself when `None`).
    pub fn rename(&self, dir: u64, from: &[u8], dst: Option<u64>, to: &[u8]) -> Result<(), Status> {
        if from.is_empty() || to.is_empty() || from.len() > NAME_MAX || to.len() > NAME_MAX {
            return Err(wire::S_INVAL);
        }
        self.put(0, from)?;
        self.put(from.len(), to)?;
        let lend = match dst {
            Some(d) => lendable(d)?,
            None => CAP_NONE,
        };
        let mut req = Request::new(wire::OP_RENAME);
        req.name_len = from.len() as u16;
        req.len = to.len() as u32;
        call(dir, req, lend).map(|_| ())
    }

    /// Retire a capability's record and drop the slot.
    pub fn release(&self, cap: u64) {
        let _ = call(cap, Request::new(wire::OP_RELEASE), CAP_NONE);
        unsafe {
            syscall1(SYS_CAP_DESTROY, cap);
        }
    }

    /// Walk `path` (components separated by `/`; `.` skipped; `..`
    /// removes the previous component and never climbs above `root`)
    /// from directory capability `root`. Intermediate directories are
    /// opened with LIST only and released. Returns (slot, type) for the
    /// final component (or a copy of `root` for an empty path).
    pub fn walk(&self, root: u64, path: &[u8], rights: u8) -> Result<(u64, u8), Status> {
        let mut parts: [(usize, usize); 32] = [(0, 0); 32];
        let mut n = 0usize;
        for (start, part) in components(path) {
            match part {
                b"." => {}
                b".." => n = n.saturating_sub(1),
                _ => {
                    if n == parts.len() {
                        return Err(wire::S_LOOP);
                    }
                    parts[n] = (start, part.len());
                    n += 1;
                }
            }
        }
        if n == 0 {
            return self.open(root, None, rights);
        }
        let mut cur = root;
        for (k, (start, len)) in parts[..n].iter().enumerate() {
            let last = k + 1 == n;
            let want = if last { rights } else { wire::R_LIST };
            let next = self.open(cur, Some(&path[*start..start + len]), want);
            if cur != root {
                self.release(cur);
            }
            let (slot, typ) = next?;
            if !last && typ != 2 {
                self.release(slot);
                return Err(wire::S_NOTDIR);
            }
            cur = slot;
            if last {
                return Ok((slot, typ));
            }
        }
        Err(wire::S_INVAL)
    }
}

/// A copy of a region capability to lend for OP_SESSION.
fn lendable_region(region: u64) -> Result<u64, Status> {
    let tmp = free_slot(8).ok_or(wire::S_FULL)?;
    if unsafe { syscall3(SYS_CAP_COPY, region, tmp, RIGHTS_READ | RIGHTS_WRITE) } != 0 {
        return Err(wire::S_DENIED);
    }
    Ok(tmp)
}

/// Non-empty `/`-separated components with their offsets.
pub fn components(path: &[u8]) -> impl Iterator<Item = (usize, &[u8])> {
    let mut at = 0usize;
    path.split(|b| *b == b'/').filter_map(move |part| {
        let start = at;
        at += part.len() + 1;
        (!part.is_empty()).then_some((start, part))
    })
}

/// The parent path and final name of `path` (lexical).
pub fn split_last(path: &[u8]) -> (&[u8], &[u8]) {
    let trimmed = match path.iter().rposition(|b| *b != b'/') {
        Some(e) => &path[..=e],
        None => return (b"", b""),
    };
    match trimmed.iter().rposition(|b| *b == b'/') {
        Some(s) => (&trimmed[..s], &trimmed[s + 1..]),
        None => (b"", trimmed),
    }
}

/// A status as a short user-facing phrase.
pub fn describe_status(s: Status) -> &'static str {
    match s {
        wire::S_NOENT => "No such file or folder",
        wire::S_EXIST => "Already exists",
        wire::S_NOTDIR => "Not a folder",
        wire::S_ISDIR => "Is a folder",
        wire::S_NOTEMPTY => "Folder not empty",
        wire::S_INVAL => "Invalid name",
        wire::S_NOSPC => "Disk full",
        wire::S_FBIG => "File too large",
        wire::S_LOOP => "Cannot move a folder into itself",
        wire::S_CORRUPT => "Volume damaged",
        wire::S_DENIED => "Not permitted",
        wire::S_NO_SESSION => "No file session",
        wire::S_FULL => "Too many open items",
        wire::S_OFFLINE => "File service offline",
        _ => "I/O error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn components_and_split_are_lexical() {
        let want: [(usize, &[u8]); 3] = [(1, b"a"), (3, b"bc"), (7, b"d")];
        let mut got = components(b"/a/bc//d");
        for w in want {
            assert_eq!(got.next(), Some(w));
        }
        assert_eq!(got.next(), None);
        assert_eq!(components(b"//").count(), 0);
        assert_eq!(split_last(b"a/b/c"), (&b"a/b"[..], &b"c"[..]));
        assert_eq!(split_last(b"c/"), (&b""[..], &b"c"[..]));
        assert_eq!(split_last(b"/"), (&b""[..], &b""[..]));
    }
}
