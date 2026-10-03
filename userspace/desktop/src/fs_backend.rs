//! Trusted broker adapter. Raw fsd endpoint and LENT frame stay in this
//! service; application-scoped authorization precedes each call here.
use crate::abi::*;
pub struct Fs {
    pub va: u64,
    endpoint: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub next: u32,
    pub size: u64,
    pub name: [u8; 32],
}
fn call(endpoint: u64, op: u64, arg: u64, cap: u64, bytes: &mut [u8; 64]) -> Result<u64, i64> {
    let mut out = [0, 0, CAP_NONE];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            endpoint,
            op,
            arg,
            cap,
            out.as_mut_ptr() as u64,
            bytes.as_mut_ptr() as u64,
        )
    };
    if out[2] != CAP_NONE {
        unsafe {
            syscall1(SYS_CAP_DESTROY, out[2]);
        }
        return Err(-2);
    }
    if rc != 0 {
        return Err(rc);
    }
    if out[0] != FS_OK {
        return Err(-1000 + out[0] as i64);
    }
    Ok(out[1])
}
impl Fs {
    pub fn start(endpoint: u64) -> Result<Self, i64> {
        let phys = unsafe { syscall1(SYS_ALLOC_FRAME, 30) };
        if phys <= 0 {
            return Err(phys);
        }
        let rc = unsafe { syscall3(SYS_CAP_COPY, 30, 31, RIGHTS_ALL) };
        if rc != 0 {
            unsafe {
                syscall1(SYS_CAP_DESTROY, 30);
            }
            return Err(rc);
        }
        let va = unsafe { syscall2(SYS_MAP_MEMORY, 30, 1) };
        if va <= 0 {
            unsafe {
                syscall1(SYS_CAP_DESTROY, 30);
                syscall1(SYS_CAP_DESTROY, 31);
            }
            return Err(va);
        }
        Ok(Self {
            va: va as u64,
            endpoint,
        })
    }
    pub fn list(&mut self, cursor: u32) -> Result<Entry, i64> {
        let mut b = [0; 64];
        call(self.endpoint, FS_OP_LS, u64::from(cursor), CAP_NONE, &mut b)?;
        let next = u32::from_le_bytes(b[..4].try_into().map_err(|_| -2)?);
        let size = u64::from_le_bytes(b[4..12].try_into().map_err(|_| -2)?);
        let len = u32::from_le_bytes(b[12..16].try_into().map_err(|_| -2)?) as usize;
        if len > 31 || (next != FS_CURSOR_END && len == 0) {
            return Err(-2);
        }
        let mut name = [0; 32];
        name[..len].copy_from_slice(&b[16..16 + len]);
        Ok(Entry { next, size, name })
    }
    /// Caller supplies a broker-owned or exact session I/O destination.
    /// # Safety
    /// `target` spans `capacity` writable bytes; no alias of the LENT frame.
    pub unsafe fn read(
        &mut self,
        name: [u8; 32],
        target: *mut u8,
        capacity: usize,
    ) -> Result<usize, i64> {
        let mut cursor = 0;
        let mut size = None;
        for _ in 0..32 {
            let e = self.list(cursor)?;
            if e.next == FS_CURSOR_END {
                break;
            }
            if e.name == name {
                size = Some(e.size);
                break;
            }
            if e.next <= cursor {
                return Err(-2);
            }
            cursor = e.next;
        }
        let length = size.ok_or(-1000 + FS_ERR_NOT_FOUND as i64)? as usize;
        if length > capacity || length > 4096 {
            return Err(-1000 + FS_ERR_RANGE as i64);
        }
        let mut b = [0; 64];
        b[..32].copy_from_slice(&name);
        let handle = call(self.endpoint, FS_OP_OPEN, 0, CAP_NONE, &mut b)?;
        let result = (|| {
            let mut offset = 0;
            while offset < length {
                b.fill(0);
                let wanted = (length - offset).min(FS_XFER_MAX as usize);
                b[..8].copy_from_slice(&(wanted as u64).to_le_bytes());
                let n = call(
                    self.endpoint,
                    FS_OP_READ,
                    fs_rw_w1(handle, offset as u64),
                    31,
                    &mut b,
                )? as usize;
                if n == 0 || n > wanted {
                    return Err(-2);
                }
                unsafe {
                    core::ptr::copy_nonoverlapping(self.va as *const u8, target.add(offset), n);
                }
                offset += n;
            }
            Ok(offset)
        })();
        b.fill(0);
        let close = call(self.endpoint, FS_OP_CLOSE, handle, CAP_NONE, &mut b);
        if result.is_ok() {
            close?;
        }
        result
    }
    /// Snapshot application data before yielding to fsd. Tail bytes are zero.
    /// # Safety
    /// `source` spans `length` readable bytes and does not alias this frame.
    pub unsafe fn put(
        &mut self,
        name: [u8; 32],
        source: *const u8,
        length: usize,
    ) -> Result<(), i64> {
        if length > 4096 {
            return Err(-1000 + FS_ERR_RANGE as i64);
        }
        unsafe {
            core::ptr::write_bytes(self.va as *mut u8, 0, 4096);
            core::ptr::copy_nonoverlapping(source, self.va as *mut u8, length);
        }
        let mut b = [0; 64];
        b[..32].copy_from_slice(&name);
        let n = call(
            self.endpoint,
            FS_OP_PUT,
            length as u64,
            if length == 0 { CAP_NONE } else { 31 },
            &mut b,
        )?;
        if n != length as u64 {
            return Err(-2);
        }
        Ok(())
    }
    pub fn delete(&mut self, name: [u8; 32]) -> Result<(), i64> {
        let mut b = [0; 64];
        b[..32].copy_from_slice(&name);
        call(self.endpoint, FS_OP_UNLINK, 0, CAP_NONE, &mut b).map(|_| ())
    }
}
