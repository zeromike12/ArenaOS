//! One-outstanding-command, polled split virtqueue helper for GPU controlq.
//!
//! No IRQ relay, no notification grant, no kernel/device discovery and no
//! implicit DMA authority. Caller must own both the MMIO cap and physical
//! queue/buffer runs and enforce a wall-clock deadline outside this module.
use core::{
    ptr,
    sync::atomic::{Ordering, compiler_fence},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    Geometry,
    Address,
    Busy,
    Device,
    Corrupt,
}

const QUEUE_SELECT: usize = 0x16;
const QUEUE_SIZE: usize = 0x18;
const QUEUE_MSIX: usize = 0x1a;
const QUEUE_ENABLE: usize = 0x1c;
const QUEUE_NOTIFY_OFF: usize = 0x1e;
const QUEUE_DESC: usize = 0x20;
const QUEUE_DRIVER: usize = 0x28;
const QUEUE_DEVICE: usize = 0x30;
const CFG_REQUIRED: usize = 0x38;
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;

pub struct PollQueue {
    ring: *mut u8,
    doorbell: *mut u16,
    size: u16,
    avail_at: usize,
    used_at: usize,
    published: u16,
    completed: u16,
    outstanding: bool,
    reply_limit: u32,
}

fn offsets(size: u16) -> Result<(usize, usize), QueueError> {
    if !(2..=64).contains(&size) || !size.is_power_of_two() {
        return Err(QueueError::Geometry);
    }
    let avail = 16 * size as usize;
    let used = (avail + 6 + 2 * size as usize + 3) & !3;
    if used + 6 + 8 * size as usize > 4096 {
        return Err(QueueError::Geometry);
    }
    Ok((avail, used))
}

unsafe fn w16(base: *mut u8, off: usize, value: u16) {
    unsafe {
        ptr::write_volatile(base.add(off) as *mut u16, value.to_le());
    }
}
unsafe fn r16(base: *mut u8, off: usize) -> u16 {
    u16::from_le(unsafe { ptr::read_volatile(base.add(off) as *const u16) })
}
unsafe fn w32(base: *mut u8, off: usize, value: u32) {
    unsafe {
        ptr::write_volatile(base.add(off) as *mut u32, value.to_le());
    }
}
unsafe fn r32(base: *mut u8, off: usize) -> u32 {
    u32::from_le(unsafe { ptr::read_volatile(base.add(off) as *const u32) })
}
unsafe fn w64(base: *mut u8, off: usize, value: u64) {
    unsafe {
        ptr::write_volatile(base.add(off) as *mut u64, value.to_le());
    }
}

impl PollQueue {
    /// Configure queue zero, disable MSI-X (0xffff) and select a single
    /// physically owned 4-KiB ring. No DRIVER_OK is set here. Caller must
    /// have completed VERSION_1 negotiation first and must ensure that
    /// `cfg`, `notify`, and `ring` are valid, disjoint, mapped windows of
    /// the given lengths. A kernel possession-gated DMA query must have
    /// vouched for `ring_phys`; raw numbers cannot grant this call.
    ///
    /// # Safety
    /// Pointers and DMA ownership as above; only one thread touches the
    /// queue. Device may write its used ring asynchronously. The mapped
    /// register space must permit volatile access, and the ring must stay
    /// pinned until the device is reset and no DMA can race its release.
    pub unsafe fn setup(
        cfg: *mut u8,
        cfg_len: usize,
        notify: *mut u8,
        notify_len: usize,
        notify_multiplier: u64,
        ring: *mut u8,
        ring_phys: u64,
    ) -> Result<Self, QueueError> {
        if cfg.is_null()
            || notify.is_null()
            || ring.is_null()
            || cfg_len < CFG_REQUIRED
            || notify_len < 2
            || ring_phys == 0
            || !ring_phys.is_multiple_of(4096)
            || ring_phys.checked_add(4096).is_none()
        {
            return Err(QueueError::Address);
        }
        unsafe {
            w16(cfg, QUEUE_SELECT, 0);
            let offered = r16(cfg, QUEUE_SIZE);
            if offered < 2 {
                return Err(QueueError::Geometry);
            }
            // The packed ring is one frame; a non-power-of-two offer is
            // clamped to the largest safe power of two below it.
            let mut size = 64u16;
            while size > offered {
                size /= 2;
            }
            let (avail_at, used_at) = offsets(size)?;
            let qnoff = u64::from(r16(cfg, QUEUE_NOTIFY_OFF));
            let byte_offset = qnoff
                .checked_mul(notify_multiplier)
                .ok_or(QueueError::Address)?;
            if byte_offset > notify_len.saturating_sub(2) as u64 || !byte_offset.is_multiple_of(2) {
                return Err(QueueError::Address);
            }
            // Preflight all geometry BEFORE writing to the mapped ring or
            // enabling the device. The device ring fields are native LE;
            // no packed Rust struct with host-dependent alignment.
            ptr::write_bytes(ring, 0, 4096);
            w16(cfg, QUEUE_SIZE, size);
            w16(cfg, QUEUE_MSIX, 0xffff);
            w64(cfg, QUEUE_DESC, ring_phys);
            w64(cfg, QUEUE_DRIVER, ring_phys + avail_at as u64);
            w64(cfg, QUEUE_DEVICE, ring_phys + used_at as u64);
            compiler_fence(Ordering::Release);
            w16(cfg, QUEUE_ENABLE, 1);
            if r16(cfg, QUEUE_ENABLE) != 1 {
                return Err(QueueError::Device);
            }
            Ok(Self {
                ring,
                doorbell: notify.add(byte_offset as usize) as *mut u16,
                size,
                avail_at,
                used_at,
                published: 0,
                completed: 0,
                outstanding: false,
                reply_limit: 0,
            })
        }
    }

    /// Publish exactly one request and one device-writable reply descriptor.
    /// The guest has already filled and pinned both buffers; a response
    /// cannot be reused until `try_complete` has checked the used entry.
    ///
    /// # Safety
    /// Valid DMA-owned buffer runs, device may access them until completion
    /// or explicit reset. `req` and `reply` do not overlap the ring and
    /// `reply_max` bounds the actual mapped response allocation.
    pub unsafe fn submit(
        &mut self,
        req: u64,
        req_len: u32,
        reply: u64,
        reply_max: u32,
    ) -> Result<(), QueueError> {
        if self.outstanding {
            return Err(QueueError::Busy);
        }
        if req == 0
            || reply == 0
            || req_len < super::HEADER as u32
            || req_len > super::MAX_REQUEST as u32
            || reply_max < super::HEADER as u32
            || reply_max > super::DISPLAY_REPLY as u32
            || req.checked_add(u64::from(req_len)).is_none()
            || reply.checked_add(u64::from(reply_max)).is_none()
        {
            return Err(QueueError::Address);
        }
        unsafe {
            if r16(self.ring, self.used_at + 2) != self.completed {
                return Err(QueueError::Corrupt);
            }
            // Descriptor zero: request, NEXT=1. Descriptor one: device
            // writable reply. All ring and request stores precede avail.
            w64(self.ring, 0, req);
            w32(self.ring, 8, req_len);
            w16(self.ring, 12, DESC_NEXT);
            w16(self.ring, 14, 1);
            w64(self.ring, 16, reply);
            w32(self.ring, 24, reply_max);
            w16(self.ring, 28, DESC_WRITE);
            w16(self.ring, 30, 0);
            w16(
                self.ring,
                self.avail_at + 4 + 2 * (self.published as usize % self.size as usize),
                0,
            );
            compiler_fence(Ordering::Release);
            self.published = self.published.wrapping_add(1);
            w16(self.ring, self.avail_at + 2, self.published);
            compiler_fence(Ordering::Release);
            ptr::write_volatile(self.doorbell, 0u16.to_le());
            self.outstanding = true;
            self.reply_limit = reply_max;
        }
        Ok(())
    }

    /// Nonblocking used-ring poll. `None` requires the caller to check an
    /// external monotonic time deadline; do not spin forever on a dead GPU.
    /// Returned length is still passed to the strict response parser.
    ///
    /// # Safety
    /// As `submit`; the device owns used-ring writes until completion.
    pub unsafe fn try_complete(&mut self) -> Result<Option<u32>, QueueError> {
        if !self.outstanding {
            return Err(QueueError::Busy);
        }
        unsafe {
            let used = r16(self.ring, self.used_at + 2);
            if used == self.completed {
                return Ok(None);
            }
            if used.wrapping_sub(self.completed) != 1 {
                return Err(QueueError::Corrupt);
            }
            compiler_fence(Ordering::Acquire);
            let slot = self.completed as usize % self.size as usize;
            let id = r32(self.ring, self.used_at + 4 + slot * 8);
            let len = r32(self.ring, self.used_at + 8 + slot * 8);
            if id != 0 || len > self.reply_limit {
                return Err(QueueError::Corrupt);
            }
            self.completed = used;
            self.outstanding = false;
            Ok(Some(len))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mock_mmio_ring_setup_and_one_strict_completion() {
        let mut cfg = [0u64; 16];
        let mut notify = [0u16; 4];
        let mut ring = [0u64; 512];
        let c = cfg.as_mut_ptr() as *mut u8;
        let n = notify.as_mut_ptr() as *mut u8;
        let r = ring.as_mut_ptr() as *mut u8;
        unsafe {
            w16(c, QUEUE_SIZE, 128); // clamp to one-frame, power-of-two 64
            w16(c, QUEUE_NOTIFY_OFF, 1);
            let mut q = PollQueue::setup(c, 128, n, 8, 2, r, 0x120000).unwrap();
            assert_eq!(q.size, 64);
            assert_eq!(r16(c, QUEUE_MSIX), 0xffff);
            assert_eq!(r16(c, QUEUE_ENABLE), 1);
            assert_eq!(
                u64::from_le(ptr::read_volatile(c.add(QUEUE_DESC) as *const u64)),
                0x120000
            );
            q.submit(0x240000, 40, 0x250000, 408).unwrap();
            assert_eq!(q.submit(0x240000, 40, 0x250000, 408), Err(QueueError::Busy));
            assert_eq!(q.try_complete(), Ok(None));
            w32(r, q.used_at + 4, 0);
            w32(r, q.used_at + 8, 24);
            w16(r, q.used_at + 2, 1);
            assert_eq!(q.try_complete(), Ok(Some(24)));
            assert_eq!(q.try_complete(), Err(QueueError::Busy));
        }
    }
    #[test]
    fn invalid_offer_notify_offsets_and_used_entry_fail_closed() {
        let mut cfg = [0u64; 16];
        let mut notify = [0u16; 4];
        let mut ring = [0u64; 512];
        let (c, n, r) = (
            cfg.as_mut_ptr() as *mut u8,
            notify.as_mut_ptr() as *mut u8,
            ring.as_mut_ptr() as *mut u8,
        );
        unsafe {
            w16(c, QUEUE_SIZE, 1);
            assert!(matches!(
                PollQueue::setup(c, 128, n, 8, 2, r, 0x120000),
                Err(QueueError::Geometry)
            ));
            w16(c, QUEUE_SIZE, 64);
            w16(c, QUEUE_NOTIFY_OFF, 3);
            assert!(matches!(
                PollQueue::setup(c, 128, n, 2, 2, r, 0x120000),
                Err(QueueError::Address)
            ));
            assert!(matches!(
                PollQueue::setup(c, 128, n, 8, 2, r, 1),
                Err(QueueError::Address)
            ));
            let mut q = PollQueue::setup(c, 128, n, 8, 2, r, 0x120000).unwrap();
            q.submit(0x240000, 24, 0x250000, 24).unwrap();
            w32(r, q.used_at + 4, 7);
            w16(r, q.used_at + 2, 1);
            assert_eq!(q.try_complete(), Err(QueueError::Corrupt));
        }
    }
}
