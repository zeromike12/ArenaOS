//! `netd` — the ArenaOS network service (M6.1, ADR-0024). Spawn-registry
//! image 6, spawned at boot when the virtio-net fixture is attached (and
//! short-lived inside the m6 suite's net_service test). The grant layout
//! mirrors storaged's exactly:
//!
//! - slot 0: `Mmio` over the BAR carrying the virtio structures (R|W —
//!   the handshake writes registers),
//! - slot 1: `Endpoint` (READ — the serve side netd parks on),
//! - slot 2: `Notification` (R|W — the target of BOTH MSI-X relay
//!   badges: RX arrivals and TX completions; badges are distinct BITS
//!   because the notification merges pending words by OR).
//!
//! netd is the virtio-net driver, LINK-LAYER ONLY: raw Ethernet frames
//! in, raw Ethernet frames out — no protocols (Phase 7's business). It
//! discovers the device through SYS_DEV_INFO (order-independent: it
//! probes indices upward and adopts the first the kernel answers — the
//! Mmio-cap gate guarantees that is ITS device — then asserts virtio-net
//! from the device_id), runs the virtio 1.0 handshake negotiating
//! VERSION_1 + MAC and nothing else, reads the MAC from the
//! device-config region (word [6]'s location — config space itself never
//! crosses the boundary), and sets up TWO split virtqueues: q0 receive,
//! q1 transmit.
//!
//! Ring memory (ADR-0024): CAP_SLOTS=16 bounds a driver to eight frames
//! (slots 8..15), so each queue's three ring areas are PACKED into one
//! frame at the modern-virtio alignments (desc 16B, avail 2B, used 4B —
//! virtio 1.0 §4.1.4.3 needs no page alignment), and the queue size is
//! capped at 64. Frame budget: q0 rings 1 + three RX buffers 3 + q1
//! rings (sharing the page with the static TX virtio header at +3072) 1
//! = five frames, slots 8..12.
//!
//! Data flow: TX is zero-copy like storaged's READ — the 12-byte
//! VERSION_1 virtio header lives in netd's own page and chains in front
//! of the CALLER's LENT frame (phys via SYS_CAP_PHYS); the device DMAs
//! the caller's page directly. RX v1 posts netd-owned buffers, harvests
//! used entries on the RX badge into a single-frame hold slot (the
//! FIRST undelivered frame is held; later arrivals are dropped — with
//! an honest log and an immediate re-post, so the ring never shrinks),
//! and delivers frames up to MSG_BYTES through the REPLY's inline
//! message (the fstest-LS pattern — lent caps cannot be mapped, so a
//! copy into the caller's frame is structurally impossible; full-frame
//! delivery is the Phase 7 buffer-handoff design).
//!
//! Exit codes (the m6 contract, driver side): 42 clean shutdown,
//! 70 SYS_DEV_INFO refused / not virtio-net, 71 a self-map refused,
//! 72 the handshake failed (FEATURES_OK / VERSION_1 / MAC / queues),
//! 73 a virtqueue setup failed, 74 SYS_IRQ_RELAY refused,
//! 75 SYS_IPC_RECV refused, 76 SYS_CAP_PHYS refused, 77 a completion
//! never arrived or was malformed (or SYS_WAIT refused),
//! 78 SYS_IPC_REPLY refused, 97 console refused, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

// ---- the grant layout (kernel-literal, entry.rs / m6.rs) ---------------------

const SLOT_MMIO: u64 = 0;
const SLOT_EP: u64 = 1;
const SLOT_NOTIF: u64 = 2;

/// Owned frame slots (SYS_ALLOC_FRAME lands the cap, SYS_MAP_MEMORY
/// consumes it). Five frames: 0 = q0 rings, 1..3 = RX buffers,
/// 4 = q1 rings + the static TX virtio header at +TX_HDR_OFF.
const SLOT_FRAME_BASE: u64 = 8;
const FRAMES_TOTAL: usize = 5;
const FRAME_Q0_RING: usize = 0;
const FRAME_RX_BASE: usize = 1;
const RX_BUFS: usize = 3;
const FRAME_Q1_RING: usize = 4;

/// The RX/TX interrupt badges (distinct bits — the notification merges
/// pending badges by OR, and netd decodes by bit).
const IRQ_BADGE_RX: u64 = 0x6E01;
const IRQ_BADGE_TX: u64 = 0x6E02;

// ---- the diagnostic exit contract (m6.rs maps every code) --------------------

const EXIT_DEV_INFO: u64 = 70;
const EXIT_MAP: u64 = 71;
const EXIT_HANDSHAKE: u64 = 72;
const EXIT_QUEUE: u64 = 73;
const EXIT_RELAY: u64 = 74;
const EXIT_RECV: u64 = 75;
const EXIT_PHYS: u64 = 76;
const EXIT_COMPLETE: u64 = 77;
const EXIT_REPLY: u64 = 78;

// ---- virtio 1.0 common-config register offsets (§4.1.4.3) --------------------

const ST_ACK: u8 = 1;
const ST_DRIVER: u8 = 2;
const ST_DRIVER_OK: u8 = 4;
const ST_FEATURES_OK: u8 = 8;

/// The modern driver's one mandatory feature bit (word 1, bit 0).
const FEATURE_VERSION_1: u32 = 1 << 0;
/// VIRTIO_NET_F_MAC (word 0, bit 5) — without it the config-space MAC
/// is not guaranteed meaningful.
const FEATURE_NET_MAC: u32 = 1 << 5;

const CFG_DEV_FEATURE_SELECT: u64 = 0x00;
const CFG_DEV_FEATURE: u64 = 0x04;
const CFG_DRV_FEATURE_SELECT: u64 = 0x08;
const CFG_DRV_FEATURE: u64 = 0x0C;
const CFG_NUM_QUEUES: u64 = 0x12;
const CFG_STATUS: u64 = 0x14;
const CFG_QUEUE_SELECT: u64 = 0x16;
const CFG_QUEUE_SIZE: u64 = 0x18;
const CFG_QUEUE_MSIX_VECTOR: u64 = 0x1A;
const CFG_QUEUE_ENABLE: u64 = 0x1C;
const CFG_QUEUE_NOTIFY_OFF: u64 = 0x1E;
const CFG_QUEUE_DESC: u64 = 0x20;
const CFG_QUEUE_DRIVER: u64 = 0x28;
const CFG_QUEUE_DEVICE: u64 = 0x30;

const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;

/// The queue size netd picks (bounded so all three ring areas of a
/// queue pack into ONE frame — the CAP_SLOTS=16 budget, ADR-0024).
const QUEUE_MAX: u16 = 64;

/// virtio_net_hdr length with VERSION_1 negotiated (no MRG_RXBUF):
/// the 10-byte struct plus 2 bytes of padding (§5.1.6.1).
const VNET_HDR_LEN: u32 = 12;
/// The TX header's home inside the q1 ring frame — clear of the packed
/// used area (which ends at 1678 bytes for qsz=64).
const TX_HDR_OFF: u64 = 3072;
/// One posted RX buffer: header + the largest frame NET_SEND accepts.
const RX_BUF_LEN: u32 = VNET_HDR_LEN + NET_FRAME_MAX as u32;

/// virtio-net PCI device IDs: modern (0x1040 + type 1) and the
/// transitional net ID.
const DEV_ID_NET_MODERN: u64 = 0x1041;
const DEV_ID_NET_TRANSITIONAL: u64 = 0x1000;

/// SYS_DEV_INFO's word count (the ADR-0022 layout — netd adds no
/// words; word [6]'s device-config location carries the MAC).
const INFO_WORDS: usize = 12;

/// How many device-index probes the order-independent discovery makes
/// before giving up (the kernel's virtio table is small; the gate
/// refuses everything but THIS driver's device anyway).
const DEV_IDX_PROBES: u64 = 8;

/// Defensive bound on re-waits for one specific completion (a device
/// that raises the interrupt without publishing the used entry is
/// broken; the bound turns that into a typed failure instead of a
/// park the kernel's drain would have to catch).
const COMPLETION_REWAITS: u32 = 16;

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("netd: FAIL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m6 contract.
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

fn log(s: &str) {
    log_line(|o| o.str(s));
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write_str("netd: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

// ---- device/ring access primitives (volatile, single-threaded image) ---------

/// # Safety
/// `reg_va` is inside netd's own self-mapped windows (the device BAR or
/// an owned frame); the access is volatile and non-speculative.
unsafe fn r8(reg_va: u64) -> u8 {
    // SAFETY: caller contract.
    unsafe { core::ptr::read_volatile(reg_va as *const u8) }
}
/// # Safety
/// As `r8`.
unsafe fn r16(reg_va: u64) -> u16 {
    // SAFETY: caller contract; the virtio registers and ring fields are
    // naturally aligned by construction.
    unsafe { core::ptr::read_volatile(reg_va as *const u16) }
}
/// # Safety
/// As `r8`.
unsafe fn r32(reg_va: u64) -> u32 {
    // SAFETY: caller contract.
    unsafe { core::ptr::read_volatile(reg_va as *const u32) }
}
/// # Safety
/// As `r8`.
unsafe fn w8(reg_va: u64, v: u8) {
    // SAFETY: caller contract.
    unsafe { core::ptr::write_volatile(reg_va as *mut u8, v) }
}
/// # Safety
/// As `r8`.
unsafe fn w16(reg_va: u64, v: u16) {
    // SAFETY: caller contract.
    unsafe { core::ptr::write_volatile(reg_va as *mut u16, v) }
}
/// # Safety
/// As `r8`.
unsafe fn w32(reg_va: u64, v: u32) {
    // SAFETY: caller contract.
    unsafe { core::ptr::write_volatile(reg_va as *mut u32, v) }
}
/// # Safety
/// As `r8`; two volatile dword writes (lo first, §4.1.4.3).
unsafe fn w64(reg_va: u64, v: u64) {
    // SAFETY: caller contract.
    unsafe {
        w32(reg_va, v as u32);
        w32(reg_va + 4, (v >> 32) as u32);
    }
}

/// Write one split-ring descriptor (16 bytes, little-endian fields).
///
/// # Safety
/// `desc_va` is netd's own mapped ring frame.
unsafe fn desc_write(desc_va: u64, i: u16, addr: u64, len: u32, flags: u16, next: u16) {
    // SAFETY: caller contract; 16-byte-aligned entry inside our frame.
    unsafe {
        let base = desc_va + 16 * u64::from(i);
        core::ptr::write_volatile(base as *mut u64, addr);
        w32(base + 8, len);
        w16(base + 12, flags);
        w16(base + 14, next);
    }
}

// ---- the packed ring geometry (ADR-0024's CAP_SLOTS budget) -------------------

/// The three ring-area offsets inside ONE frame for a queue of `qsz`
/// entries: desc 16B-aligned at 0, avail 2B-aligned right after, used
/// 4B-aligned after that. For qsz=64: 0 / 1024 / 1160 — used ends at
/// 1678, far below the q1 frame's TX header at 3072.
fn ring_offsets(qsz: u16) -> (u64, u64, u64) {
    let desc = 0u64;
    let avail = 16 * u64::from(qsz);
    let used_raw = avail + 6 + 2 * u64::from(qsz);
    let used = (used_raw + 3) & !3;
    (desc, avail, used)
}

/// One split virtqueue's driver-side state.
struct Queue {
    qsz: u16,
    desc_va: u64,
    avail_va: u64,
    used_va: u64,
    doorbell: u64,
    avail_idx: u16,
    used_seen: u16,
}

impl Queue {
    /// Publish descriptor head `id` in the next avail slot and ring the
    /// doorbell (fence discipline as storaged's: slot → fence → index →
    /// fence → doorbell).
    ///
    /// # Safety
    /// The queue's VAs are netd's own mapped frames.
    unsafe fn publish(&mut self, id: u16) {
        // SAFETY: method contract.
        unsafe {
            w16(
                self.avail_va + 4 + 2 * u64::from(self.avail_idx % self.qsz),
                id,
            );
            store_fence();
            self.avail_idx = self.avail_idx.wrapping_add(1);
            w16(self.avail_va + 2, self.avail_idx);
            store_fence();
            w16(self.doorbell, 0);
        }
    }

    /// The used ring's current index (volatile read).
    ///
    /// # Safety
    /// The queue's VAs are netd's own mapped frames.
    unsafe fn used_idx(&self) -> u16 {
        // SAFETY: method contract.
        unsafe { r16(self.used_va + 2) }
    }

    /// Read used entry `slot` → (descriptor id, device-written length).
    ///
    /// # Safety
    /// The queue's VAs are netd's own mapped frames.
    unsafe fn used_entry(&self, slot: u16) -> (u32, u32) {
        // SAFETY: method contract.
        unsafe {
            let base = self.used_va + 4 + 8 * u64::from(slot);
            (r32(base), r32(base + 4))
        }
    }
}

/// The whole driver state (single-threaded image; the serve loop owns
/// it end to end).
struct Drv {
    q0: Queue,
    q1: Queue,
    /// The RX buffers' phys (descriptor addresses — the device never
    /// sees VAs) and netd-side VAs (the harvest copy reads through
    /// them).
    rx_phys: [u64; RX_BUFS],
    rx_va: [u64; RX_BUFS],
    mac: [u8; 6],
    /// Badge bits seen by SYS_WAIT but not yet consumed (the
    /// notification merges by OR; a TX wait may swallow an RX arrival,
    /// so bits accumulate here and are decoded by bit).
    pending: u64,
    /// The FIRST undelivered received frame: (RX buffer id, frame
    /// length with the virtio header stripped). Later arrivals while
    /// the hold is busy are dropped (logged, counted, buffer re-posted
    /// — the ring never shrinks).
    hold: Option<(u16, u32)>,
    completions: u64,
    rx_dropped: u64,
}

impl Drv {
    /// Post (or re-post) RX buffer `id`: one device-writable descriptor
    /// covering header + frame, published to the avail ring (the
    /// publish rings the doorbell).
    ///
    /// # Safety
    /// All VAs/phys are netd's own frames; single-threaded.
    unsafe fn post_rx(&mut self, id: u16, phys: u64) {
        // SAFETY: method contract.
        unsafe {
            desc_write(self.q0.desc_va, id, phys, RX_BUF_LEN, DESC_F_WRITE, 0);
            self.q0.publish(id);
        }
    }

    /// Drain the RX used ring into the hold slot (non-blocking). The
    /// hold keeps the FIRST undelivered frame; further arrivals are
    /// dropped with an honest log and their buffers re-posted at once,
    /// so the ring keeps its full depth.
    ///
    /// # Safety
    /// Method contract as `post_rx`.
    unsafe fn harvest_rx(&mut self) {
        // SAFETY: method contract.
        unsafe {
            while self.q0.used_seen != self.q0.used_idx() {
                let slot = self.q0.used_seen % self.q0.qsz;
                let (id, len) = self.q0.used_entry(slot);
                self.q0.used_seen = self.q0.used_seen.wrapping_add(1);
                self.completions += 1;
                if id >= RX_BUFS as u32 {
                    log_line(|o| {
                        o.str("netd: RX used entry names descriptor ");
                        o.u64(u64::from(id));
                        o.str(" — outside the posted buffers");
                    });
                    fail(EXIT_COMPLETE, "the RX completion was malformed");
                }
                let flen = len.saturating_sub(VNET_HDR_LEN);
                if self.hold.is_some() {
                    self.rx_dropped += 1;
                    log_line(|o| {
                        o.str("netd: RX hold busy — dropping a frame of ");
                        o.u64(u64::from(flen));
                        o.str(" bytes (v1 holds one undelivered frame)");
                    });
                    let p = self.rx_phys[id as usize];
                    self.post_rx(id as u16, p);
                } else {
                    self.hold = Some((id as u16, flen));
                }
            }
        }
    }

    /// Wait until one of the WANTED badge bits is pending, harvesting
    /// RX arrivals on the way (an RX bit consumed here fills the hold
    /// slot; unknown bits are ignored forever after). SYS_WAIT takes
    /// the notification's merged pending word, so every bit is
    /// accumulated locally before decoding.
    ///
    /// # Safety
    /// Method contract as `post_rx`.
    unsafe fn wait_bits(&mut self, want: u64) {
        loop {
            if self.pending & IRQ_BADGE_RX != 0 {
                self.pending &= !IRQ_BADGE_RX;
                // SAFETY: method contract.
                unsafe { self.harvest_rx() };
            }
            if self.pending & want != 0 {
                self.pending &= !want;
                return;
            }
            // SAFETY: ABI v1 wrapper; the notification cap is the
            // granted slot.
            let b = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
            if b < 0 {
                log_line(|o| {
                    o.str("netd: wait returned ");
                    o.i64(b);
                });
                fail(
                    EXIT_COMPLETE,
                    "SYS_WAIT on the interrupt notification refused",
                );
            }
            self.pending |= b as u64;
        }
    }

    /// Transmit one frame zero-copy: the static 12-byte virtio header
    /// (netd's own page) chains in front of the CALLER's frame (the
    /// device DMAs the caller's page), then the TX completion is
    /// interrupt-delivered — never polled.
    ///
    /// # Safety
    /// Method contract as `post_rx`; `hdr_phys`/`data_phys` name netd's
    /// TX header and a caller frame the device may read.
    unsafe fn tx_send(&mut self, hdr_phys: u64, data_phys: u64, len: u32) -> Result<(), ()> {
        // SAFETY: method contract.
        unsafe {
            desc_write(self.q1.desc_va, 0, hdr_phys, VNET_HDR_LEN, DESC_F_NEXT, 1);
            desc_write(self.q1.desc_va, 1, data_phys, len, 0, 0);
            self.q1.publish(0);
            self.wait_bits(IRQ_BADGE_TX);
            let mut rewaits = 0u32;
            while self.q1.used_idx() == self.q1.used_seen {
                rewaits += 1;
                if rewaits > COMPLETION_REWAITS {
                    log("netd: TX completion interrupt without a used entry");
                    return Err(());
                }
                self.wait_bits(IRQ_BADGE_TX);
            }
            let slot = self.q1.used_seen % self.q1.qsz;
            let (id, _written) = self.q1.used_entry(slot);
            if id != 0 {
                // One synchronous send at a time, so the head is always
                // descriptor 0 — anything else is a device contract
                // break.
                log_line(|o| {
                    o.str("netd: TX used entry names descriptor ");
                    o.u64(u64::from(id));
                    o.str(", expected 0");
                });
                return Err(());
            }
            self.q1.used_seen = self.q1.used_seen.wrapping_add(1);
            self.completions += 1;
            Ok(())
        }
    }
}

/// The driver entry: the spawn protocol's first thread lands here at
/// ring 3 with the grant slots filled. Nothing returns — the service
/// ends only through the poison request's clean exit or a `fail`.
///
/// # Safety
/// Entered exactly as storaged's `_start`: ring 3, RSP at the derived
/// stack top, the address space the kernel loaded and validated,
/// grants in slots 0..2. The body is ABI v1 wrappers over this image's
/// own statics and windows — every device and ring touch is volatile,
/// single-threaded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and its granted/allocated windows, inside
    // the address space the kernel loaded and validated.
    // Single-threaded; every device and ring touch is volatile.
    unsafe {
        log("netd: starting — ArenaOS network service, link layer only (M6.1, ADR-0024)");

        // 1. Order-independent discovery: probe device indices upward
        //    and adopt the first one SYS_DEV_INFO answers — the
        //    Mmio-cap gate inside the kernel guarantees the answer is
        //    OUR device (the cap names its structure BAR), so fixture
        //    order on the QEMU command line is not load-bearing. Then
        //    assert virtio-NET from the device_id (a mis-spawn must
        //    fail loudly, not drive the wrong device).
        let mut dev_idx = u64::MAX;
        let mut info = [0u64; INFO_WORDS];
        for idx in 0..DEV_IDX_PROBES {
            let r = syscall2(SYS_DEV_INFO, idx, info.as_mut_ptr() as u64);
            if r == INFO_WORDS as i64 {
                dev_idx = idx;
                break;
            }
        }
        if dev_idx == u64::MAX {
            log("netd: SYS_DEV_INFO refused every device index probe");
            fail(EXIT_DEV_INFO, "SYS_DEV_INFO refused (no granted device)");
        }
        let pci_index = info[0];
        let common_off = info[2] & 0xFFFF_FFFF;
        let notify_off = info[3] & 0xFFFF_FFFF;
        let notify_mult = info[4];
        let devcfg_off = info[6] & 0xFFFF_FFFF;
        let msix_present = info[7] & 0xFFFF_FFFF;
        let msix_size = (info[7] >> 32) as u16;
        let device_id = info[8] & 0xFFFF;
        let transitional = (info[8] >> 32) & 0xFFFF;
        if device_id != DEV_ID_NET_MODERN && device_id != DEV_ID_NET_TRANSITIONAL {
            log_line(|o| {
                o.str("netd: granted device_id ");
                o.hex(device_id);
                o.str(" is neither virtio-net modern 0x1041 nor transitional 0x1000");
            });
            fail(
                EXIT_DEV_INFO,
                "the granted device is not a virtio-net function",
            );
        }
        if msix_present == 0 || msix_size < 2 {
            fail(
                EXIT_RELAY,
                "the device has no MSI-X table with two entries — the relay plan needs RX + TX vectors",
            );
        }

        // 2. The device window: self-map the granted Mmio cap WRITABLE
        //    (the handshake writes registers; the cap survives — Mmio
        //    is descriptive).
        let win = syscall2(SYS_MAP_MEMORY, SLOT_MMIO, 1);
        if win <= 0 {
            log_line(|o| {
                o.str("netd: window map returned ");
                o.i64(win);
            });
            fail(EXIT_MAP, "the device-window self-map refused");
        }
        let win = win as u64;
        let cfg = win + common_off;
        log_line(|o| {
            o.str("netd: virtio-net device_id ");
            o.hex(device_id);
            if transitional != 0 {
                o.str(" (transitional)");
            }
            o.str(" pci_index ");
            o.u64(pci_index);
            o.str(" dev_idx ");
            o.u64(dev_idx);
            o.str(" — window ");
            o.hex(win);
            o.str(", common +");
            o.hex(common_off);
            o.str(", notify +");
            o.hex(notify_off);
            o.str(" mult ");
            o.u64(notify_mult);
            o.str(", devcfg +");
            o.hex(devcfg_off);
            o.str(", msix ");
            o.u64(u64::from(msix_size));
        });

        // 3. The virtio 1.0 handshake (§3.1): reset, ACK|DRIVER,
        //    features VERSION_1 + MAC and NOTHING else (no csum/GSO
        //    offload, no mergeable buffers, no control queue), then
        //    FEATURES_OK must stick. Both bits are mandatory for this
        //    driver: without VERSION_1 the 12-byte header and modern
        //    ring rules do not hold; without MAC the config-space
        //    address is not guaranteed meaningful.
        w8(cfg + CFG_STATUS, 0); // reset
        let mut spins = 0u32;
        while r8(cfg + CFG_STATUS) != 0 && spins < 100_000 {
            spins += 1;
            core::hint::spin_loop();
        }
        if r8(cfg + CFG_STATUS) != 0 {
            fail(
                EXIT_HANDSHAKE,
                "device status did not read back 0 after reset",
            );
        }
        w8(cfg + CFG_STATUS, ST_ACK);
        w8(cfg + CFG_STATUS, ST_ACK | ST_DRIVER);
        w32(cfg + CFG_DEV_FEATURE_SELECT, 0);
        let f0 = r32(cfg + CFG_DEV_FEATURE);
        w32(cfg + CFG_DEV_FEATURE_SELECT, 1);
        let f1 = r32(cfg + CFG_DEV_FEATURE);
        if f0 & FEATURE_NET_MAC == 0 {
            fail(EXIT_HANDSHAKE, "the device does not offer VIRTIO_NET_F_MAC");
        }
        if f1 & FEATURE_VERSION_1 == 0 {
            fail(
                EXIT_HANDSHAKE,
                "legacy-only virtio-net (no VERSION_1) — unsupported in v1",
            );
        }
        w32(cfg + CFG_DRV_FEATURE_SELECT, 0);
        w32(cfg + CFG_DRV_FEATURE, f0 & FEATURE_NET_MAC);
        w32(cfg + CFG_DRV_FEATURE_SELECT, 1);
        w32(cfg + CFG_DRV_FEATURE, f1 & FEATURE_VERSION_1);
        w8(cfg + CFG_STATUS, ST_ACK | ST_DRIVER | ST_FEATURES_OK);
        if r8(cfg + CFG_STATUS) & ST_FEATURES_OK == 0 {
            log_line(|o| {
                o.str("netd: device features ");
                o.hex(u64::from(f1) << 32 | u64::from(f0));
                o.str(" — FEATURES_OK did not stick");
            });
            fail(EXIT_HANDSHAKE, "feature negotiation refused by the device");
        }
        if r16(cfg + CFG_NUM_QUEUES) < 2 {
            fail(
                EXIT_HANDSHAKE,
                "the device offers fewer than two virtqueues (need receiveq + transmitq)",
            );
        }

        // 4. The five owned frames (ADR-0024's budget): q0 rings, three
        //    RX buffers, q1 rings + TX header. Allocation pays the phys
        //    (the queue registers need it); self-map consumes each cap
        //    (ownership moves to this address space — teardown exact).
        let mut phys = [0u64; FRAMES_TOTAL];
        let mut va = [0u64; FRAMES_TOTAL];
        for i in 0..FRAMES_TOTAL {
            let slot = SLOT_FRAME_BASE + i as u64;
            let p = syscall1(SYS_ALLOC_FRAME, slot);
            if p <= 0 {
                log_line(|o| {
                    o.str("netd: frame alloc ");
                    o.u64(i as u64);
                    o.str(" returned ");
                    o.i64(p);
                });
                fail(EXIT_QUEUE, "a ring/buffer frame allocation refused");
            }
            phys[i] = p as u64;
            let m = syscall2(SYS_MAP_MEMORY, slot, 1);
            if m <= 0 {
                fail(EXIT_MAP, "a frame self-map refused");
            }
            va[i] = m as u64;
            // The allocator does not zero: rings must START zeroed
            // (idx 0, no descriptors, no used entries).
            core::ptr::write_bytes(va[i] as *mut u8, 0, 4096);
        }

        // 5. Both queues: packed ring areas in ONE frame each (modern
        //    virtio needs no page alignment — desc 16B / avail 2B /
        //    used 4B), size capped at QUEUE_MAX, one MSI-X entry per
        //    queue armed through the kernel relay BEFORE enable.
        let mut qvecs = [0i64; 2];
        let mut qs: [Option<Queue>; 2] = [None, None];
        for (q, frame, entry, badge) in [
            (0u16, FRAME_Q0_RING, 0u16, IRQ_BADGE_RX),
            (1u16, FRAME_Q1_RING, 1u16, IRQ_BADGE_TX),
        ] {
            w16(cfg + CFG_QUEUE_SELECT, q);
            let qmax = r16(cfg + CFG_QUEUE_SIZE);
            if qmax == 0 {
                fail(EXIT_QUEUE, "the device advertises queue size 0");
            }
            let qsz = if qmax > QUEUE_MAX { QUEUE_MAX } else { qmax };
            w16(cfg + CFG_QUEUE_SIZE, qsz);
            let (desc_off, avail_off, used_off) = ring_offsets(qsz);
            let ring = phys[frame];
            w64(cfg + CFG_QUEUE_DESC, ring + desc_off);
            w64(cfg + CFG_QUEUE_DRIVER, ring + avail_off);
            w64(cfg + CFG_QUEUE_DEVICE, ring + used_off);
            let vec = syscall4(SYS_IRQ_RELAY, dev_idx, u64::from(entry), SLOT_NOTIF, badge);
            if !(48..=63).contains(&vec) {
                log_line(|o| {
                    o.str("netd: irq_relay(entry ");
                    o.u64(u64::from(entry));
                    o.str(") returned ");
                    o.i64(vec);
                });
                fail(EXIT_RELAY, "SYS_IRQ_RELAY refused");
            }
            qvecs[q as usize] = vec;
            w16(cfg + CFG_QUEUE_MSIX_VECTOR, entry);
            let qnoff = u64::from(r16(cfg + CFG_QUEUE_NOTIFY_OFF));
            let doorbell = win + notify_off + qnoff * notify_mult;
            w16(cfg + CFG_QUEUE_ENABLE, 1);
            if r16(cfg + CFG_QUEUE_ENABLE) != 1 {
                fail(EXIT_QUEUE, "queue_enable did not stick");
            }
            qs[q as usize] = Some(Queue {
                qsz,
                desc_va: va[frame] + desc_off,
                avail_va: va[frame] + avail_off,
                used_va: va[frame] + used_off,
                doorbell,
                avail_idx: 0,
                used_seen: 0,
            });
        }
        let (Some(q0), Some(q1)) = (qs[0].take(), qs[1].take()) else {
            fail(EXIT_QUEUE, "a queue was not constructed (internal)");
        };
        if qvecs[0] == qvecs[1] {
            fail(EXIT_RELAY, "both MSI-X entries landed on one relay vector");
        }

        // 6. The MAC from the device-config region (word [6]'s offset —
        //    through the self-mapped window, never through config
        //    space). An all-zero MAC means the config region is not
        //    what the scan promised — fail instead of sending frames
        //    nobody would answer.
        let devcfg = win + devcfg_off;
        let mut mac = [0u8; 6];
        for (i, m) in mac.iter_mut().enumerate() {
            *m = r8(devcfg + i as u64);
        }
        if mac.iter().all(|&b| b == 0) {
            fail(EXIT_HANDSHAKE, "the device-config MAC is all zeros");
        }

        // 7. The static TX header (12 zero bytes — no csum, no GSO) in
        //    the q1 frame, clear of the packed used area; then
        //    DRIVER_OK, the first RX posting, and the ready line.
        core::ptr::write_bytes(
            (va[FRAME_Q1_RING] + TX_HDR_OFF) as *mut u8,
            0,
            VNET_HDR_LEN as usize,
        );
        w8(
            cfg + CFG_STATUS,
            ST_ACK | ST_DRIVER | ST_FEATURES_OK | ST_DRIVER_OK,
        );
        // Two lines, both well inside WRITE_MAX=256 (Out::push drops
        // past the limit silently — the line budget is an invariant,
        // ADR-0023's HELP lesson): identity first, geometry second.
        log_line(|o| {
            o.str("netd: virtio-net ready — DRIVER_OK, mac ");
            for (i, b) in mac.iter().enumerate() {
                if i > 0 {
                    o.str(":");
                }
                o.hex2(*b);
            }
            o.str(", queue 0/1 size ");
            o.u64(u64::from(q0.qsz));
            o.str("/");
            o.u64(u64::from(q1.qsz));
            o.str(", ");
            o.u64(RX_BUFS as u64);
            o.str(" RX buffers of ");
            o.u64(u64::from(RX_BUF_LEN));
            o.str(" bytes");
        });
        log_line(|o| {
            o.str("netd: doorbells ");
            o.hex(q0.doorbell);
            o.str("/");
            o.hex(q1.doorbell);
            o.str(", msix entries 0/1 → relay vectors ");
            o.i64(qvecs[0]);
            o.str("/");
            o.i64(qvecs[1]);
        });

        let mut drv = Drv {
            q0,
            q1,
            rx_phys: [
                phys[FRAME_RX_BASE],
                phys[FRAME_RX_BASE + 1],
                phys[FRAME_RX_BASE + 2],
            ],
            rx_va: [
                va[FRAME_RX_BASE],
                va[FRAME_RX_BASE + 1],
                va[FRAME_RX_BASE + 2],
            ],
            mac,
            pending: 0,
            hold: None,
            completions: 0,
            rx_dropped: 0,
        };
        // First RX posting: every buffer is one device-writable
        // descriptor (header + frame in the same netd-owned page);
        // each publish rings the doorbell.
        for i in 0..RX_BUFS {
            let p = phys[FRAME_RX_BASE + i];
            drv.post_rx(i as u16, p);
        }

        // 8. The service loop: recv → op → interrupt-driven completion
        //    → reply. One request in flight at a time (synchronous
        //    IPC); both queues' badges decode from one notification.
        let tx_hdr_phys = phys[FRAME_Q1_RING] + TX_HDR_OFF;
        let mut msg = [0u64; 3];
        loop {
            let r = syscall3(SYS_IPC_RECV, SLOT_EP, msg.as_mut_ptr() as u64, 0);
            if r < 0 {
                log_line(|o| {
                    o.str("netd: recv returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "SYS_IPC_RECV refused");
            }
            let (w0, op, landed) = (msg[0], msg[1], msg[2]);
            match op {
                NET_OP_SHUTDOWN => {
                    // The poison request: reply FIRST (the caller
                    // blocks), then exit — a process is never destroyed
                    // with parked threads.
                    if landed != CAP_NONE {
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                    }
                    log_line(|o| {
                        o.str("netd: shutdown requested after ");
                        o.u64(drv.completions);
                        o.str(" interrupt-delivered completion(s), ");
                        o.u64(drv.rx_dropped);
                        o.str(" RX drop(s) — replying and exiting");
                    });
                    let rr = syscall5(
                        SYS_IPC_REPLY,
                        SLOT_EP,
                        NET_S_OK,
                        drv.completions,
                        CAP_NONE,
                        0,
                    );
                    if rr < 0 {
                        fail(EXIT_REPLY, "the shutdown reply refused");
                    }
                    // SAFETY: thread_exit diverges; 42 is the clean-exit code.
                    syscall1(SYS_THREAD_EXIT, EXIT_OK);
                }
                NET_OP_MAC => {
                    if landed != CAP_NONE {
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                        reply_err(NET_S_BAD_OP);
                        continue;
                    }
                    let mut packed = 0u64;
                    for (i, b) in drv.mac.iter().enumerate() {
                        packed |= u64::from(*b) << (8 * i);
                    }
                    reply_ok(packed);
                }
                NET_OP_SEND => {
                    if landed == CAP_NONE {
                        reply_err(NET_S_NO_BUF);
                        continue;
                    }
                    if !(NET_FRAME_MIN..=NET_FRAME_MAX).contains(&w0) {
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                        reply_err(NET_S_BAD_LEN);
                        continue;
                    }
                    // The zero-copy seam: the frame's phys comes from
                    // the LENT cap — never from a message word.
                    let bp = syscall1(SYS_CAP_PHYS, landed);
                    if bp <= 0 {
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                        log_line(|o| {
                            o.str("netd: cap_phys on landed slot ");
                            o.u64(landed);
                            o.str(" returned ");
                            o.i64(bp);
                        });
                        fail(EXIT_PHYS, "SYS_CAP_PHYS on the landed buffer refused");
                    }
                    match drv.tx_send(tx_hdr_phys, bp as u64, w0 as u32) {
                        Ok(()) => {
                            // The request is done: discard the lent
                            // reference (frees nothing — the caller's
                            // owned cap and its mapping stay alive).
                            let dr = syscall1(SYS_CAP_DESTROY, landed);
                            if dr < 0 {
                                log_line(|o| {
                                    o.str("netd: WARNING — landed cap destroy returned ");
                                    o.i64(dr);
                                });
                            }
                            reply_ok(w0);
                            log_line(|o| {
                                o.str("netd: SEND ");
                                o.u64(w0);
                                o.str(" bytes buf ");
                                o.hex(bp as u64);
                                o.str(" → completion #");
                                o.u64(drv.completions);
                            });
                        }
                        Err(()) => {
                            let _ = syscall1(SYS_CAP_DESTROY, landed);
                            fail(EXIT_COMPLETE, "the TX completion was missing or malformed");
                        }
                    }
                }
                NET_OP_RECV => {
                    if landed != CAP_NONE {
                        // v1 RECV delivers through the reply's inline
                        // message — a landed cap is a wire violation;
                        // destroy it (never leak) and refuse.
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                        reply_err(NET_S_BAD_OP);
                        continue;
                    }
                    // Block until the hold slot carries a frame: the
                    // wait consumes RX badges and harvests on the way
                    // (a TX wait may have swallowed the arrival bit).
                    while drv.hold.is_none() {
                        drv.wait_bits(IRQ_BADGE_RX);
                    }
                    let Some((id, flen)) = drv.hold.take() else {
                        fail(EXIT_COMPLETE, "the hold slot emptied itself (internal)");
                    };
                    if flen > MSG_BYTES as u32 {
                        // The documented v1 limit: inline delivery caps
                        // at MSG_BYTES. Drop honestly (the buffer goes
                        // straight back to the ring) and refuse typed.
                        log_line(|o| {
                            o.str("netd: received frame of ");
                            o.u64(u64::from(flen));
                            o.str(" bytes exceeds the v1 inline limit (");
                            o.u64(MSG_BYTES as u64);
                            o.str(") — dropped");
                        });
                        let p = drv.rx_phys[id as usize];
                        drv.post_rx(id, p);
                        reply_err(NET_S_BAD_LEN);
                        continue;
                    }
                    // Copy the frame out of the RX buffer into the
                    // reply's inline message, then re-post the buffer.
                    let mut out = [0u64; MSG_BYTES / 8];
                    core::ptr::copy_nonoverlapping(
                        (drv.rx_va[id as usize] + VNET_HDR_LEN as u64) as *const u8,
                        out.as_mut_ptr() as *mut u8,
                        flen as usize,
                    );
                    let p = drv.rx_phys[id as usize];
                    drv.post_rx(id, p);
                    let rr = syscall5(
                        SYS_IPC_REPLY,
                        SLOT_EP,
                        NET_S_OK,
                        u64::from(flen),
                        CAP_NONE,
                        out.as_ptr() as u64,
                    );
                    if rr < 0 {
                        fail(EXIT_REPLY, "SYS_IPC_REPLY refused");
                    }
                    log_line(|o| {
                        o.str("netd: RECV → ");
                        o.u64(u64::from(flen));
                        o.str(" bytes delivered inline (completion #");
                        o.u64(drv.completions);
                        o.str(")");
                    });
                }
                _ => {
                    if landed != CAP_NONE {
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                    }
                    reply_err(NET_S_BAD_OP);
                }
            }
        }
    }
}

/// Reply word 0 = NET_S_OK with a payload word.
fn reply_ok(w1: u64) {
    // SAFETY: ABI v1 wrapper; the endpoint cap is the granted slot.
    let rr = unsafe { syscall5(SYS_IPC_REPLY, SLOT_EP, NET_S_OK, w1, CAP_NONE, 0) };
    if rr < 0 {
        fail(EXIT_REPLY, "SYS_IPC_REPLY refused");
    }
}

/// Reply with a typed error status.
fn reply_err(status: u64) {
    // SAFETY: as reply_ok.
    let rr = unsafe { syscall5(SYS_IPC_REPLY, SLOT_EP, status, 0, CAP_NONE, 0) };
    if rr < 0 {
        fail(EXIT_REPLY, "the error reply refused");
    }
}
