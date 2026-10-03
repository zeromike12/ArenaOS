//! Small ordinary userspace graphical client adapter. No raw display/input.
use crate::abi::*;
use crate::{model::Event, wire::Frame};
pub const ENDPOINT: u64 = 0;
pub const BACKING: u64 = 1;
/// First page is reserved for explicit service I/O; pixels follow it.
pub const PIXEL_OFFSET: usize = 4096;
/// A caller validates geometry before using this mapping as a Canvas.
pub struct Client {
    pub handle: u64,
    pub pixels: *mut u32,
    pub io: *mut u8,
    pub appearance: core::cell::Cell<u8>,
    pub width: usize,
    pub height: usize,
}
fn exchange(frame: Frame) -> Result<([u64; 3], Frame), i64> {
    let mut bytes = frame.encode().map_err(|_| -2)?;
    let mut out = [0, 0, CAP_NONE];
    // SAFETY: stack-owned buffers span exactly the syscall ABI's declared sizes.
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT,
            0,
            0,
            BACKING,
            out.as_mut_ptr() as u64,
            bytes.as_mut_ptr() as u64,
        )
    };
    if out[2] != CAP_NONE {
        // SAFETY: drop only the newly landed reply cap, including malformed replies.
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
        return Err(-2);
    }
    if rc != 0 {
        return Err(rc);
    }
    if out[0] != 0 {
        return Err(-2);
    }
    Ok((out, Frame::decode(&bytes).map_err(|_| -2)?))
}
impl Client {
    pub fn cancel_close(&self) -> Result<(), i64> {
        let f = Frame::CancelClose {
            handle: self.handle,
        };
        if exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    pub fn connect(width: usize, height: usize, title: &str) -> Result<Self, i64> {
        if width < 80
            || height < 60
            || width > 448
            || height > 288
            || title.is_empty()
            || title.len() > 32
            || !title.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        {
            return Err(-2);
        }
        let mut bound = [0; 2];
        // SAFETY: caller-owned descriptor buffer; held region capability gates the query.
        let rc = unsafe {
            syscall6(
                SYS_SHARED_INFO,
                BACKING,
                bound.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        };
        if rc != 0 || bound[0] == 0 || bound[1] * 4096 < (PIXEL_OFFSET + width * height * 4) as u64
        {
            return Err(-2);
        }
        // SAFETY: mapping an already-held region; kernel validates writable authority.
        let va = unsafe { syscall2(SYS_SHARED_MAP, BACKING, 1) };
        if va <= 0 {
            return Err(va);
        }
        let request = Frame::Create {
            width: width as u16,
            height: height as u16,
        };
        let (reply, echo) = exchange(request)?;
        if echo != request || reply[1] == 0 {
            return Err(-2);
        }
        let mut text = [0; 32];
        text[..title.len()].copy_from_slice(title.as_bytes());
        let metadata = Frame::Title {
            handle: reply[1],
            text,
        };
        if exchange(metadata)?.1 != metadata {
            return Err(-2);
        }
        Ok(Self {
            handle: reply[1],
            pixels: (va as usize + PIXEL_OFFSET) as *mut u32,
            io: va as *mut u8,
            appearance: core::cell::Cell::new(2),
            width,
            height,
        })
    }
    pub fn damage(&self) -> Result<(), i64> {
        let f = Frame::Damage {
            handle: self.handle,
        };
        if exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    pub fn poll(&self) -> Result<Option<Event>, i64> {
        let f = Frame::Poll {
            handle: self.handle,
        };
        let (out, reply) = exchange(f)?;
        if out[1] > 3 {
            return Err(-2);
        }
        self.appearance.set(out[1] as u8);
        match reply {
            Frame::Poll { handle } if handle == self.handle => Ok(None),
            Frame::Event { handle, event } if handle == self.handle => Ok(Some(event)),
            _ => Err(-2),
        }
    }
}
/// Debug-log line (used only by opt-in `perf` probes).
pub fn log(bytes: &[u8]) {
    // SAFETY: the kernel copies at most `len` bytes from this live slice.
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, bytes.as_ptr() as u64, bytes.len() as u64) };
}
pub fn exit(code: u64) -> ! {
    // SAFETY: terminating only this application thread.
    let _ = unsafe { syscall1(SYS_THREAD_EXIT, code) };
    loop {
        core::hint::spin_loop();
    }
}
