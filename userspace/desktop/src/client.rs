//! Small ordinary userspace graphical client adapter. No raw display/input.
use crate::abi::*;
use crate::{
    model::{Event, PopupKind, SURFACE_MAX_HEIGHT, SURFACE_MAX_WIDTH, TRANSIENT_MAX_PIXELS},
    wire::Frame,
};
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
    /// Pages of the session reservation (ADR-0075): the last
    /// `TRANSIENT_PAGES` hold the transient surface.
    pages: usize,
}
/// Pages of the bounded transient surface at the end of the reservation.
pub const TRANSIENT_PAGES: usize = TRANSIENT_MAX_PIXELS * 4 / 4096;
/// A live transient surface (menu, tooltip, dialog) of this client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transient {
    pub handle: u64,
    pub pixels: *mut u32,
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
            || width > usize::from(SURFACE_MAX_WIDTH)
            || height > usize::from(SURFACE_MAX_HEIGHT)
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
        if rc != 0
            || bound[0] == 0
            || bound[1] * 4096 < (PIXEL_OFFSET + width * height * 4 + TRANSIENT_PAGES * 4096) as u64
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
            pages: bound[1] as usize,
        })
    }
    /// Main-surface pixels the reservation holds (the largest surface the
    /// window policy can configure fits in it).
    pub fn capacity(&self) -> usize {
        (self.pages - 1 - TRANSIENT_PAGES) * 1024
    }
    /// Declare that this client re-lays out to any size of at least this.
    pub fn set_resizable(&self, min_width: u16, min_height: u16) -> Result<(), i64> {
        let f = Frame::Resizable {
            handle: self.handle,
            min_width,
            min_height,
        };
        if exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    /// Adopt a configured size: from now on the surface is `width` x
    /// `height` (stride = width). Paint it completely, then `commit`.
    pub fn adopt(&mut self, width: usize, height: usize) -> Result<(), i64> {
        if width * height > self.capacity()
            || width > usize::from(SURFACE_MAX_WIDTH)
            || height > usize::from(SURFACE_MAX_HEIGHT)
        {
            return Err(-2);
        }
        self.width = width;
        self.height = height;
        Ok(())
    }
    /// Publish the whole surface at its (newly adopted) size.
    pub fn commit(&self) -> Result<(), i64> {
        let f = Frame::Resize {
            handle: self.handle,
            width: self.width as u16,
            height: self.height as u16,
        };
        if exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    /// Open this window's transient surface at window-local (`x`, `y`).
    /// Its pixels are private staging until `publish_transient`.
    pub fn open_transient(
        &self,
        kind: PopupKind,
        x: i32,
        y: i32,
        width: u16,
        height: u16,
    ) -> Result<Transient, i64> {
        let f = Frame::Popup {
            handle: self.handle,
            kind,
            x,
            y,
            width,
            height,
        };
        let (out, echo) = exchange(f)?;
        if echo != f || out[1] == 0 {
            return Err(-2);
        }
        let base = self.io as usize + (self.pages - TRANSIENT_PAGES) * 4096;
        Ok(Transient {
            handle: out[1],
            pixels: base as *mut u32,
            width: usize::from(width),
            height: usize::from(height),
        })
    }
    /// Publish the whole transient surface.
    pub fn publish_transient(&self, t: &Transient) -> Result<(), i64> {
        let f = Frame::Damage {
            handle: t.handle,
            rects: crate::wire::DamageRects::FULL,
        };
        if exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    /// Close this client's own transient surface.
    pub fn close_transient(&self, t: &Transient) -> Result<(), i64> {
        let f = Frame::Dismiss { handle: t.handle };
        if exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    /// Publish the whole surface.
    pub fn damage(&self) -> Result<(), i64> {
        self.publish(crate::wire::DamageRects::FULL)
    }
    /// Publish only these surface rectangles (Phase 11.1): the broker copies
    /// exactly them into the visible snapshot; every other published pixel
    /// keeps its previous value, whatever the staging bytes now hold.
    pub fn damage_rects(&self, rects: &[arena_gfxkit::Rect]) -> Result<(), i64> {
        if rects.is_empty() {
            return Ok(());
        }
        let mut d = crate::wire::DamageRects::FULL;
        if rects.len() > crate::wire::DamageRects::MAX {
            return self.publish(d);
        }
        for (slot, r) in d.r.iter_mut().zip(rects) {
            *slot = [r.x as u16, r.y as u16, r.width as u16, r.height as u16];
        }
        d.n = rects.len() as u8;
        self.publish(d)
    }
    fn publish(&self, rects: crate::wire::DamageRects) -> Result<(), i64> {
        let f = Frame::Damage {
            handle: self.handle,
            rects,
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
    // An abnormal exit names its stage (42 is the ordinary close).
    if code != 42 {
        let digits = [b'0' + ((code / 10) % 10) as u8, b'0' + (code % 10) as u8];
        log(b"[app] abnormal exit stage ");
        log(&digits);
        log(b"\n");
    }
    // SAFETY: terminating only this application thread.
    let _ = unsafe { syscall1(SYS_THREAD_EXIT, code) };
    loop {
        core::hint::spin_loop();
    }
}
