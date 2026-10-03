//! Owned, one-outstanding, polled 2D transport. BAR and DMA backing are
//! kernel-minted capabilities; neither PCI numbers nor client bytes grant
//! physical access. This module alone writes GPU registers and queue memory.
use crate::abi::*;
use arena_gpu2d_wire::{self as wire, Command, Mode};
use arena_gpu2d_wire::{device::DeviceInfo, queue::PollQueue};
#[path = "../../virtio.rs"]
mod virtio;

const SLOT_POOL: u64 = 2;
const SLOT_DMA: u64 = 3;
const RESOURCE: u32 = 1;
const RESPONSE_OFFSET: usize = 512;

pub struct Gpu {
    pub mode: Mode,
    queue: PollQueue,
    command_va: *mut u8,
    command_phys: u64,
}

fn run() -> Result<(*mut u8, u64), &'static str> {
    let mut create = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, SLOT_POOL, 2, create.as_mut_ptr() as u64) } != 0
        || create[1] == 0
        || create[2] != 8192
    {
        return Err("shared run allocation refused");
    }
    let slot = create[0];
    let mut backing = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_PHYS, slot, SLOT_DMA, backing.as_mut_ptr() as u64) } != 0
        || backing[0] == 0
        || backing[1] != 2
        || backing[2] != create[1]
    {
        return Err("DMA bearer refused run physical backing");
    }
    let va = unsafe { syscall2(SYS_SHARED_MAP, slot, 1) };
    if va <= 0 {
        return Err("shared run self-map refused");
    }
    Ok((va as *mut u8, backing[0]))
}

impl Gpu {
    /// Device discovery only succeeds if the caller holds the *entire*
    /// verified BAR. All scratch allocations happen after VERSION_1 was
    /// accepted, so a handshake rejection can use the independent GOP.
    pub fn open(slot: u64) -> Result<Self, &'static str> {
        let mut found = None;
        for i in 0..virtio::DEV_IDX_PROBES {
            let mut words = [0u64; 12];
            if unsafe { syscall2(SYS_DEV_INFO, i, words.as_mut_ptr() as u64) } == 12
                && let Ok(info) = DeviceInfo::parse(&words)
            {
                found = Some(info);
                break;
            }
        }
        let info = found.ok_or("no covering modern GPU BAR cap")?;
        let mapped = unsafe { syscall2(SYS_MAP_MEMORY, slot, 1) };
        if mapped <= 0 {
            return Err("GPU MMIO self-map refused");
        }
        if unsafe { syscall6(SYS_SHARED_UNMAP, mapped as u64, 0, 0, 0, 0, 0) } != -2 {
            return Err("GPU BAR incorrectly accepted as a shared mapping");
        }
        let (cfg, notify) = info.window(mapped as u64).map_err(|_| "GPU BAR offsets")?;
        let win = virtio::Window {
            win: mapped as u64,
            cfg,
            devcfg: 0,
        };
        unsafe { virtio::handshake("displayd", &win, 0, virtio::FEATURE_VERSION_1, 1) }
            .map_err(|_| "GPU VERSION_1/queue handshake refused")?;
        let (ring, ring_phys) = run()?;
        // One two-page owned run: page 0 is exclusively the polling ring,
        // page 1 the command/response buffers. Both remain pinned as long
        // as this service can address the device; no overlap or second
        // registry object is needed for the queue.
        let command_va = unsafe { ring.add(4096) };
        let command_phys = ring_phys + 4096;
        // SAFETY: cfg/notify are within the held kernel-validated BAR;
        // the two-page shared run is owner-pinned and disjoint by page.
        let queue = unsafe {
            PollQueue::setup(
                cfg as *mut u8,
                info.common_length as usize,
                notify as *mut u8,
                info.notify_length as usize,
                u64::from(info.notify_multiplier),
                ring,
                ring_phys,
            )
        }
        .map_err(|_| "GPU polling controlq refused")?;
        unsafe { virtio::driver_ok(&win) };
        let mut gpu = Self {
            mode: Mode::new(0, 1, 1).map_err(|_| "internal default mode")?,
            queue,
            command_va,
            command_phys,
        };
        // Past DRIVER_OK the device may own descriptors. A failed or
        // malformed response must terminate this service, never discard
        // the ring and pretend a safe GOP fallback while DMA could race.
        let (len, bytes) = gpu.send(Command::DisplayInfo).unwrap_or_else(|reason| {
            crate::write_log(b"[displayd] GPU display-info transport failed: ");
            crate::write_log(reason.as_bytes());
            crate::write_log(b"\n");
            crate::exit(83)
        });
        log_line(|o| {
            o.str("displayd: GPU display-info used=");
            o.u64(len as u64);
            o.str(" hdr=");
            o.hex(u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as u64);
            o.str(" first mode(w,h,enabled)=");
            for i in [32, 36, 40] {
                o.hex(u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as u64);
                o.str(" ");
            }
        });
        gpu.mode = wire::response_display_info(bytes, len).unwrap_or_else(|_| {
            crate::write_log(b"[displayd] GPU display-info malformed or exceeds bounded scanout\n");
            crate::exit(83)
        });
        Ok(gpu)
    }

    fn send(&mut self, command: Command) -> Result<(usize, &[u8]), &'static str> {
        let mut request = [0u8; wire::MAX_REQUEST];
        let size = command
            .encode(&mut request)
            .map_err(|_| "GPU command rejected")?;
        // SAFETY: both slices are wholly within the pinned 4096-byte
        // command run. The response is cleared before each submission; only
        // `used_len` returned by the queue is parsed. No command may overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(request.as_ptr(), self.command_va, size);
            core::ptr::write_bytes(self.command_va.add(RESPONSE_OFFSET), 0, wire::DISPLAY_REPLY);
            self.queue.submit(
                self.command_phys,
                size as u32,
                self.command_phys + RESPONSE_OFFSET as u64,
                wire::DISPLAY_REPLY as u32,
            )
        }
        .map_err(|_| "GPU queue rejected request")?;
        let now = unsafe { syscall0(SYS_CLOCK_NOW) };
        if now < 0 {
            return Err("GPU monotonic clock refused");
        }
        let deadline = (now as u64).saturating_add(2_000_000);
        let mut spins = 0u32;
        let len = loop {
            match unsafe { self.queue.try_complete() } {
                Ok(Some(n)) => break n as usize,
                Ok(None) => (),
                Err(_) => return Err("GPU used ring corrupt"),
            }
            spins = spins.wrapping_add(1);
            if spins.is_multiple_of(256) {
                let tick = unsafe { syscall0(SYS_CLOCK_NOW) };
                if tick < 0 || tick as u64 >= deadline {
                    return Err("GPU completion deadline expired");
                }
            }
            core::hint::spin_loop();
        };
        // SAFETY: used_len is checked by PollQueue against DISPLAY_REPLY,
        // and RESPONSE_OFFSET + DISPLAY_REPLY < 4096. No outstanding DMA.
        let bytes = unsafe {
            core::slice::from_raw_parts(self.command_va.add(RESPONSE_OFFSET), wire::DISPLAY_REPLY)
        };
        Ok((len, bytes))
    }

    fn nodata(&mut self, cmd: Command) -> Result<(), &'static str> {
        let (len, bytes) = self.send(cmd)?;
        wire::response_no_data(bytes, len).map_err(|_| "GPU ACK type/size/header mismatch")
    }

    /// Only this service knows the physical scanout run, obtained with
    /// SYS_SHARED_PHYS on its held region and independent SharedDma bearer.
    pub fn refresh(&mut self, rect: wire::Rect) -> Result<(), &'static str> {
        let mode = self.mode;
        if !rect.within(mode.width, mode.height) {
            return Err("GPU PRESENT rectangle outside mode");
        }
        let offset = (u64::from(rect.y) * u64::from(mode.width) + u64::from(rect.x)) * 4;
        self.nodata(Command::Transfer {
            mode,
            rect,
            offset,
            resource: RESOURCE,
        })?;
        self.nodata(Command::Flush {
            mode,
            rect,
            resource: RESOURCE,
        })
    }

    pub fn paint(&mut self, phys: u64, pages: u64) -> Result<(), &'static str> {
        let mode = self.mode;
        let bytes = pages
            .checked_mul(4096)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or("GPU scanout pages overflow")?;
        self.nodata(Command::Create {
            mode,
            resource: RESOURCE,
        })?;
        self.nodata(Command::Attach {
            mode,
            resource: RESOURCE,
            physical: phys,
            bytes,
        })?;
        self.nodata(Command::SetScanout {
            mode,
            resource: RESOURCE,
        })?;
        self.nodata(Command::Transfer {
            mode,
            rect: mode.full_rect(),
            offset: 0,
            resource: RESOURCE,
        })?;
        self.nodata(Command::Flush {
            mode,
            rect: mode.full_rect(),
            resource: RESOURCE,
        })
    }
}
