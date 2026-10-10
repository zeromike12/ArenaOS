//! Small ordinary userspace graphical client adapter. No raw display/input.
use crate::abi::*;
use crate::{
    model::{Event, PopupKind, SURFACE_MAX_HEIGHT, SURFACE_MAX_WIDTH, TRANSIENT_MAX_PIXELS},
    wire::Frame,
};
/// Legacy Phase-11 client layout (kept for direct/older images).
pub const ENDPOINT: u64 = 0;
pub const BACKING: u64 = 1;
/// ABI-v2 built-in application layout: startup page is consumed from slot 0,
/// then the badged service endpoint and surface region occupy slots 1 and 2.
pub const V2_ENDPOINT: u64 = 1;
pub const V2_BACKING: u64 = 2;
/// First page is reserved for explicit service I/O; pixels follow it.
pub const PIXEL_OFFSET: usize = 4096;
/// A caller validates geometry before using this mapping as a Canvas.
pub struct Client {
    pub handle: u64,
    pub pixels: *mut u32,
    pub io: *mut u8,
    pub appearance: core::cell::Cell<u8>,
    /// The last poll reported more queued events.
    pub more: core::cell::Cell<bool>,
    pub width: usize,
    pub height: usize,
    endpoint: u64,
    /// Capability used to authenticate every request from this process.
    /// Additional windows have a distinct surface cap but share this control
    /// backing with the primary ABI-v2 session window.
    control_backing: u64,
    backing: u64,
    control_io: *mut u8,
    control_pages: usize,
    /// Pages of the session reservation (ADR-0075): the I/O page, the main
    /// surface, `TRANSIENT_PAGES` of transient surface, then `FILE_PAGES`.
    pages: usize,
}
/// Pages of the bounded transient surface, after the main surface.
pub const TRANSIENT_PAGES: usize = TRANSIENT_MAX_PIXELS * 4 / 4096;
/// The last page of the reservation: this client's filesd I/O page
/// (ADR-0077). The broker never reads it as pixels.
pub const FILE_PAGES: usize = 1;
/// The broker rejected a surface commit because a newer Configure superseded
/// the size this client just adopted. The client should poll for that size and
/// repaint instead of treating the ordinary drag race as a dead session.
pub const STATUS_RESIZE_SUPERSEDED: i64 = 3;
/// A live transient surface (menu, tooltip, dialog) of this client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transient {
    pub handle: u64,
    pub pixels: *mut u32,
    pub width: usize,
    pub height: usize,
}
fn exchange_at_with_cap(
    endpoint: u64,
    backing: u64,
    frame: Frame,
    allow_cap: bool,
) -> Result<([u64; 3], Frame), i64> {
    for attempt in 0..=crate::app_client::IPC_BUSY_RETRIES {
        let mut bytes = frame.encode().map_err(|_| -2)?;
        let mut out = [0, 0, CAP_NONE];
        // SAFETY: stack-owned buffers span exactly the syscall ABI's declared sizes.
        let rc = unsafe {
            syscall6(
                SYS_IPC_CALL,
                endpoint,
                0,
                0,
                backing,
                out.as_mut_ptr() as u64,
                bytes.as_mut_ptr() as u64,
            )
        };
        if out[2] != CAP_NONE && !allow_cap {
            // SAFETY: drop only the newly landed reply cap, including malformed replies.
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
            return Err(-2);
        }
        if rc == STATUS_BUSY && attempt < crate::app_client::IPC_BUSY_RETRIES {
            crate::app_client::retry_after_busy(attempt)?;
            continue;
        }
        if rc != 0 {
            if out[2] != CAP_NONE {
                // SAFETY: a failed IPC call cannot retain an unexpected cap.
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
            }
            return Err(rc);
        }
        if out[0] != 0 {
            if out[2] != CAP_NONE {
                // SAFETY: malformed replies do not retain their transferred cap.
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
            }
            // Preserve the server's typed status. In particular, a resize
            // commit can lose a race with a newer Configure and must reach
            // the application as STATUS_RESIZE_SUPERSEDED so it can adopt
            // the newer size and repaint.
            return Err(out[0] as i64);
        }
        let decoded = match Frame::decode(&bytes) {
            Ok(frame) => frame,
            Err(_) => {
                if out[2] != CAP_NONE {
                    // SAFETY: reject and release a cap attached to malformed bytes.
                    let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
                }
                return Err(-2);
            }
        };
        return Ok((out, decoded));
    }
    Err(STATUS_BUSY)
}
fn exchange_at(endpoint: u64, backing: u64, frame: Frame) -> Result<([u64; 3], Frame), i64> {
    exchange_at_with_cap(endpoint, backing, frame, false)
}
impl Client {
    fn exchange(&self, frame: Frame) -> Result<([u64; 3], Frame), i64> {
        exchange_at(self.endpoint, self.control_backing, frame)
    }
    /// Exact capability slot used for this client's surface reservation.
    pub fn backing_slot(&self) -> u64 {
        self.backing
    }
    pub fn cancel_close(&self) -> Result<(), i64> {
        let f = Frame::CancelClose {
            handle: self.handle,
        };
        if self.exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    /// Connect through the unchanged Phase-11 slot layout.
    pub fn connect(width: usize, height: usize, title: &str) -> Result<Self, i64> {
        Self::connect_with_slots(width, height, title, ENDPOINT, BACKING)
    }
    /// Connect a Phase-12 ABI-v2 application using its badged endpoint and
    /// explicitly inherited surface slots.
    pub fn connect_v2(width: usize, height: usize, title: &str) -> Result<Self, i64> {
        Self::connect_with_slots(width, height, title, V2_ENDPOINT, V2_BACKING)
    }
    fn connect_with_slots(
        width: usize,
        height: usize,
        title: &str,
        endpoint: u64,
        backing: u64,
    ) -> Result<Self, i64> {
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
                backing,
                bound.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        };
        if rc != 0
            || bound[0] == 0
            || bound[1] * 4096
                < (PIXEL_OFFSET + width * height * 4 + (TRANSIENT_PAGES + FILE_PAGES) * 4096) as u64
        {
            return Err(-2);
        }
        // SAFETY: mapping an already-held region; kernel validates writable authority.
        let va = unsafe { syscall2(SYS_SHARED_MAP, backing, 1) };
        if va <= 0 {
            return Err(va);
        }
        let request = Frame::Create {
            width: width as u16,
            height: height as u16,
        };
        let (reply, echo) = exchange_at(endpoint, backing, request)?;
        if echo != request || reply[1] == 0 {
            return Err(-2);
        }
        let mut text = [0; 32];
        text[..title.len()].copy_from_slice(title.as_bytes());
        let metadata = Frame::Title {
            handle: reply[1],
            text,
        };
        if exchange_at(endpoint, backing, metadata)?.1 != metadata {
            return Err(-2);
        }
        Ok(Self {
            handle: reply[1],
            pixels: (va as usize + PIXEL_OFFSET) as *mut u32,
            io: va as *mut u8,
            appearance: core::cell::Cell::new(2),
            more: core::cell::Cell::new(false),
            width,
            height,
            endpoint,
            control_backing: backing,
            backing,
            control_io: va as *mut u8,
            control_pages: bound[1] as usize,
            pages: bound[1] as usize,
        })
    }
    /// Create another ordinary window owned by this application process. The
    /// broker returns a fresh per-window SharedRegion capability; this method
    /// never accepts a caller-selected handle or surface identity.
    pub fn create_window(&self, width: usize, height: usize, title: &str) -> Result<Self, i64> {
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
        let request = Frame::CreateAdditional {
            width: width as u16,
            height: height as u16,
        };
        let (reply, echo) =
            exchange_at_with_cap(self.endpoint, self.control_backing, request, true)?;
        if echo != request || reply[1] == 0 || reply[2] == CAP_NONE {
            if reply[2] != CAP_NONE {
                // SAFETY: dispose of a cap attached to an invalid create reply.
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
            }
            return Err(-2);
        }
        let backing = reply[2];
        let mut bound = [0; 2];
        // SAFETY: the returned exact cap gates its own size query.
        let rc = unsafe {
            syscall6(
                SYS_SHARED_INFO,
                backing,
                bound.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        };
        if rc != 0 || bound[0] == 0 || bound[1] * 4096 < (PIXEL_OFFSET + width * height * 4) as u64
        {
            // SAFETY: discard the exact returned window capability.
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, backing) };
            return Err(-2);
        }
        // SAFETY: map the newly received SharedRegion using its held authority.
        let va = unsafe { syscall2(SYS_SHARED_MAP, backing, 1) };
        if va <= 0 {
            // SAFETY: no mapping was installed on failure.
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, backing) };
            return Err(va);
        }
        let mut text = [0; 32];
        text[..title.len()].copy_from_slice(title.as_bytes());
        let metadata = Frame::Title {
            handle: reply[1],
            text,
        };
        let title_result = self.exchange(metadata);
        if !matches!(title_result, Ok((_, reply)) if reply == metadata) {
            // SAFETY: release only this newly created window's map/cap.
            let _ = unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) };
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, backing) };
            return Err(-2);
        }
        Ok(Self {
            handle: reply[1],
            pixels: (va as usize + PIXEL_OFFSET) as *mut u32,
            io: va as *mut u8,
            appearance: core::cell::Cell::new(2),
            more: core::cell::Cell::new(false),
            width,
            height,
            endpoint: self.endpoint,
            control_backing: self.control_backing,
            backing,
            control_io: self.control_io,
            control_pages: self.control_pages,
            pages: bound[1] as usize,
        })
    }
    /// Close this ordinary window while keeping the process and sibling
    /// windows alive. Extra surface mappings and their exact caps are dropped
    /// after the broker acknowledges retirement.
    pub fn close_window(&self) -> Result<(), i64> {
        let frame = Frame::DestroyWindow {
            handle: self.handle,
        };
        if self.exchange(frame)?.1 != frame {
            return Err(-2);
        }
        if self.backing != self.control_backing {
            // SAFETY: this client owns the map and received surface cap.
            let rc = unsafe { syscall6(SYS_SHARED_UNMAP, self.io as u64, 0, 0, 0, 0, 0) };
            if rc != 0 {
                return Err(rc);
            }
            // SAFETY: retire only the capability returned for this window.
            let rc = unsafe { syscall1(SYS_CAP_DESTROY, self.backing) };
            if rc != 0 {
                return Err(rc);
            }
        }
        Ok(())
    }
    /// Page index (within the region) and address of the filesd I/O page.
    pub fn file_page(&self) -> (u64, u64) {
        let page = self.control_pages - FILE_PAGES;
        (page as u64, self.control_io as u64)
    }
    /// Main-surface pixels the reservation holds (the largest surface the
    /// window policy can configure fits in it).
    pub fn capacity(&self) -> usize {
        (self.pages - 1 - TRANSIENT_PAGES - FILE_PAGES) * 1024
    }
    /// Declare that this client re-lays out to any size of at least this.
    pub fn set_resizable(&self, min_width: u16, min_height: u16) -> Result<(), i64> {
        let f = Frame::Resizable {
            handle: self.handle,
            min_width,
            min_height,
        };
        if self.exchange(f)?.1 != f {
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
        if self.exchange(f)?.1 != f {
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
        let (out, echo) = self.exchange(f)?;
        if echo != f || out[1] == 0 {
            return Err(-2);
        }
        let base = self.io as usize + (self.pages - TRANSIENT_PAGES - FILE_PAGES) * 4096;
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
        if self.exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    /// Close this client's own transient surface.
    pub fn close_transient(&self, t: &Transient) -> Result<(), i64> {
        let f = Frame::Dismiss { handle: t.handle };
        if self.exchange(f)?.1 != f {
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
        if self.exchange(f)?.1 != f {
            return Err(-2);
        }
        Ok(())
    }
    pub fn poll(&self) -> Result<Option<Event>, i64> {
        let f = Frame::Poll {
            handle: self.handle,
        };
        let (out, reply) = self.exchange(f)?;
        if out[1] > 7 {
            return Err(-2);
        }
        self.appearance.set((out[1] & 3) as u8);
        self.more.set(out[1] & 4 != 0);
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
