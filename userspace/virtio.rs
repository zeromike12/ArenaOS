//! The shared virtio 1.0 core (M6.2, ADR-0025) — ADR-0024's
//! third-driver rule executed: with storaged (block), netd (net), and
//! rngd (entropy) all in the tree, the genuinely common code is the
//! discovery probe, the window map, the §3.1 handshake, the frame
//! budget loop, the split-queue setup, and the ring primitives. This
//! module is that code, extracted mechanically from the two existing
//! drivers under the full regression suite — the same bytes-on-the-wire
//! behavior, one copy.
//!
//! Inclusion contract (the abi.rs pattern): a driver's crate root does
//!
//! ```ignore
//! #[path = "../../virtio.rs"]
//! mod virtio;
//! ```
//!
//! and this module resolves its syscall/console surface through
//! `crate::abi` — so the includer must ALSO declare `mod abi` at its
//! crate root (every image already does).
//!
//! What stays driver-side by design: the grant layout, exit-code
//! contracts (each suite maps its own driver's codes), serve loops,
//! device-class specifics (feature bits beyond VERSION_1, MAC reads,
//! request headers), and every log line a harness asserts. Failure
//! paths return [`VErr`] with the stage and a reason; the driver maps
//! the stage to ITS exit code and fails with the reason — the typed
//! diagnostic contract is unchanged.

// Each driver image uses an overlapping subset of this core (storaged
// never reads Window::devcfg, netd never builds a RingMem::Split) —
// the same discipline abi.rs has always had.
#![allow(dead_code)]

use crate::abi::*;

// ---- the frozen SYS_DEV_INFO layout (ADR-0022; extended by appending only) ----

/// SYS_DEV_INFO's word count. FROZEN at 12 — storaged checks the
/// return equals this; extension must APPEND and update every driver
/// in lockstep (ADR-0022/0024).
pub const INFO_WORDS: usize = 12;

/// How many device-index probes the order-independent discovery makes
/// before giving up (the kernel's virtio table is small; the gate
/// refuses everything but the calling driver's device anyway — the
/// Mmio cap in the caller's slot names its structure BAR).
pub const DEV_IDX_PROBES: u64 = 8;

// ---- virtio 1.0 constants (OASIS §4.1.4.3, §2.6, §3.1) ------------------------

pub const ST_ACK: u8 = 1;
pub const ST_DRIVER: u8 = 2;
pub const ST_DRIVER_OK: u8 = 4;
pub const ST_FEATURES_OK: u8 = 8;

/// The modern driver's one mandatory feature bit (word 1, bit 0).
pub const FEATURE_VERSION_1: u32 = 1 << 0;

pub const CFG_DEV_FEATURE_SELECT: u64 = 0x00;
pub const CFG_DEV_FEATURE: u64 = 0x04;
pub const CFG_DRV_FEATURE_SELECT: u64 = 0x08;
pub const CFG_DRV_FEATURE: u64 = 0x0C;
pub const CFG_NUM_QUEUES: u64 = 0x12;
pub const CFG_STATUS: u64 = 0x14;
pub const CFG_QUEUE_SELECT: u64 = 0x16;
pub const CFG_QUEUE_SIZE: u64 = 0x18;
pub const CFG_QUEUE_MSIX_VECTOR: u64 = 0x1A;
pub const CFG_QUEUE_ENABLE: u64 = 0x1C;
pub const CFG_QUEUE_NOTIFY_OFF: u64 = 0x1E;
pub const CFG_QUEUE_DESC: u64 = 0x20;
pub const CFG_QUEUE_DRIVER: u64 = 0x28;
pub const CFG_QUEUE_DEVICE: u64 = 0x30;

pub const DESC_F_NEXT: u16 = 1;
pub const DESC_F_WRITE: u16 = 2;

// ---- the typed failure surface ------------------------------------------------

/// Which setup stage failed. Every driver's exit-code contract has
/// these five stages in this order (storaged 60..64, netd 70..74,
/// rngd 80..84) — the driver maps stage → code and fails with the
/// carried reason.
pub enum VErr {
    DevInfo(&'static str),
    Map(&'static str),
    Handshake(&'static str),
    Queue(&'static str),
    Relay(&'static str),
}

/// The kernel's resolved record of the calling driver's device —
/// SYS_DEV_INFO's 12 words, parsed once (ADR-0022 layout: [0]
/// pci_index, [1] BAR phys, [2..6] offset|length<<32 for common /
/// notify / (mult@4) / isr / device-config, [7] msix present |
/// table_size<<32, [8] device_id | transitional<<32, [9..11]
/// reserved).
pub struct DevInfo {
    pub dev_idx: u64,
    pub pci_index: u64,
    pub device_id: u64,
    pub transitional: u64,
    pub common_off: u64,
    pub notify_off: u64,
    pub notify_mult: u64,
    pub devcfg_off: u64,
    pub msix_size: u16,
}

/// The self-mapped device window and its derived region bases.
pub struct Window {
    pub win: u64,
    /// Common-config region base (window + common_off).
    pub cfg: u64,
    /// Device-config region base (window + devcfg_off) — netd reads
    /// the MAC here; other drivers ignore it.
    pub devcfg: u64,
}

/// Order-independent discovery (ADR-0024): probe device indices
/// upward and adopt the first one SYS_DEV_INFO answers — the
/// Mmio-cap gate inside the kernel guarantees the answer is the
/// CALLER's device (the cap names its structure BAR), so fixture
/// order on the QEMU command line is not load-bearing. Then assert
/// the driver's device class from the device_id: a mis-spawn must
/// fail loudly, not drive the wrong device.
///
/// `prefix` is the driver's log name ("netd"), `kind` the class name
/// for diagnostics ("virtio-net"); `min_msix` the MSI-X table entries
/// the driver's relay plan needs (net: 2 — one per queue; block/rng:
/// 1).
///
/// # Safety
/// Ring 3, single-threaded image; `info` buffer is this image's own
/// static/stack memory (SYS_DEV_INFO writes INFO_WORDS u64s).
pub unsafe fn discover(
    prefix: &str,
    kind: &str,
    id_modern: u64,
    id_transitional: u64,
    min_msix: u16,
) -> Result<DevInfo, VErr> {
    let mut dev_idx = u64::MAX;
    let mut info = [0u64; INFO_WORDS];
    for idx in 0..DEV_IDX_PROBES {
        // SAFETY: wrapper contract; `info` is this image's own buffer.
        let r = unsafe { syscall2(SYS_DEV_INFO, idx, info.as_mut_ptr() as u64) };
        if r == INFO_WORDS as i64 {
            dev_idx = idx;
            break;
        }
    }
    if dev_idx == u64::MAX {
        log_line(|o| {
            o.str(prefix);
            o.str(": SYS_DEV_INFO refused every device index probe");
        });
        return Err(VErr::DevInfo("SYS_DEV_INFO refused (no granted device)"));
    }
    let device_id = info[8] & 0xFFFF;
    let transitional = (info[8] >> 32) & 0xFFFF;
    if device_id != id_modern && device_id != id_transitional {
        log_line(|o| {
            o.str(prefix);
            o.str(": granted device_id ");
            o.hex(device_id);
            o.str(" is neither ");
            o.str(kind);
            o.str(" modern ");
            o.hex(id_modern);
            o.str(" nor transitional ");
            o.hex(id_transitional);
        });
        return Err(VErr::DevInfo(
            "the granted device is not this driver's virtio function",
        ));
    }
    let msix_present = info[7] & 0xFFFF_FFFF;
    let msix_size = (info[7] >> 32) as u16;
    if msix_present == 0 || msix_size < min_msix {
        return Err(VErr::Relay(
            "the device has no MSI-X table with enough entries for the relay plan",
        ));
    }
    Ok(DevInfo {
        dev_idx,
        pci_index: info[0],
        device_id,
        transitional,
        common_off: info[2] & 0xFFFF_FFFF,
        notify_off: info[3] & 0xFFFF_FFFF,
        notify_mult: info[4],
        devcfg_off: info[6] & 0xFFFF_FFFF,
        msix_size,
    })
}

/// Self-map the granted Mmio cap WRITABLE (the handshake writes
/// registers; the access mode decides the right, and the cap survives
/// — Mmio is descriptive) and log the resolved geometry: identity
/// first, geometry second (ADR-0023's HELP lesson).
///
/// # Safety
/// `slot_mmio` holds the granted Mmio cap; single-threaded image.
pub unsafe fn map_window(
    prefix: &str,
    kind: &str,
    info: &DevInfo,
    slot_mmio: u64,
) -> Result<Window, VErr> {
    // SAFETY: wrapper contract.
    let win = unsafe { syscall2(SYS_MAP_MEMORY, slot_mmio, 1) };
    if win <= 0 {
        log_line(|o| {
            o.str(prefix);
            o.str(": window map returned ");
            o.i64(win);
        });
        return Err(VErr::Map("the device-window self-map refused"));
    }
    let win = win as u64;
    log_line(|o| {
        o.str(prefix);
        o.str(": ");
        o.str(kind);
        o.str(" device_id ");
        o.hex(info.device_id);
        if info.transitional != 0 {
            o.str(" (transitional)");
        }
        o.str(" pci_index ");
        o.u64(info.pci_index);
        o.str(" dev_idx ");
        o.u64(info.dev_idx);
        o.str(" — window ");
        o.hex(win);
        o.str(", common +");
        o.hex(info.common_off);
        o.str(", notify +");
        o.hex(info.notify_off);
        o.str(" mult ");
        o.u64(info.notify_mult);
        o.str(", devcfg +");
        o.hex(info.devcfg_off);
        o.str(", msix ");
        o.u64(u64::from(info.msix_size));
    });
    Ok(Window {
        win,
        cfg: win + info.common_off,
        devcfg: win + info.devcfg_off,
    })
}

/// The virtio 1.0 handshake (§3.1): reset, ACK|DRIVER, read both
/// feature words, REQUIRE the caller's mandatory bits (`f0_req` /
/// `f1_req` — every driver needs at least VERSION_1 in word 1),
/// accept exactly those and nothing else (no offloads, no optional
/// queues in v1), then FEATURES_OK must stick. Returns the offered
/// (f0, f1) for diagnostics. `min_queues` is the virtqueue count the
/// driver's ring plan needs.
///
/// # Safety
/// `w` is a mapped device window; single-threaded image.
pub unsafe fn handshake(
    prefix: &str,
    w: &Window,
    f0_req: u32,
    f1_req: u32,
    min_queues: u16,
) -> Result<(u32, u32), VErr> {
    // SAFETY: the common-config registers are mapped window dwords;
    // every access is volatile (abi's accessors).
    unsafe {
        w8(w.cfg + CFG_STATUS, 0); // reset
        let mut spins = 0u32;
        while r8(w.cfg + CFG_STATUS) != 0 && spins < 100_000 {
            spins += 1;
            core::hint::spin_loop();
        }
        if r8(w.cfg + CFG_STATUS) != 0 {
            return Err(VErr::Handshake(
                "device status did not read back 0 after reset",
            ));
        }
        w8(w.cfg + CFG_STATUS, ST_ACK);
        w8(w.cfg + CFG_STATUS, ST_ACK | ST_DRIVER);
        w32(w.cfg + CFG_DEV_FEATURE_SELECT, 0);
        let f0 = r32(w.cfg + CFG_DEV_FEATURE);
        w32(w.cfg + CFG_DEV_FEATURE_SELECT, 1);
        let f1 = r32(w.cfg + CFG_DEV_FEATURE);
        if f1 & f1_req & FEATURE_VERSION_1 == 0 {
            return Err(VErr::Handshake(
                "legacy-only device (no VERSION_1) — unsupported in v1",
            ));
        }
        if f0 & f0_req != f0_req || f1 & f1_req != f1_req {
            return Err(VErr::Handshake("a required feature bit was not offered"));
        }
        w32(w.cfg + CFG_DRV_FEATURE_SELECT, 0);
        w32(w.cfg + CFG_DRV_FEATURE, f0 & f0_req);
        w32(w.cfg + CFG_DRV_FEATURE_SELECT, 1);
        w32(w.cfg + CFG_DRV_FEATURE, f1 & f1_req);
        w8(w.cfg + CFG_STATUS, ST_ACK | ST_DRIVER | ST_FEATURES_OK);
        if r8(w.cfg + CFG_STATUS) & ST_FEATURES_OK == 0 {
            log_line(|o| {
                o.str(prefix);
                o.str(": device features ");
                o.hex(u64::from(f1) << 32 | u64::from(f0));
                o.str(" — FEATURES_OK did not stick");
            });
            return Err(VErr::Handshake("feature negotiation refused by the device"));
        }
        if r16(w.cfg + CFG_NUM_QUEUES) < min_queues {
            return Err(VErr::Handshake(
                "the device offers too few virtqueues for this driver",
            ));
        }
        Ok((f0, f1))
    }
}

/// Allocate `n` owned frames (SYS_ALLOC_FRAME lands each cap at
/// `slot_base + i`, SYS_MAP_MEMORY consumes it — ownership moves to
/// this address space, teardown exact) and zero each mapping: the
/// allocator does not zero, and rings must START zeroed (idx 0, no
/// descriptors, no used entries).
///
/// # Safety
/// The cap slots are free in this image's slot table; `phys`/`va`
/// have room for `n` entries; single-threaded image.
pub unsafe fn alloc_frames(
    prefix: &str,
    slot_base: u64,
    n: usize,
    phys: &mut [u64],
    va: &mut [u64],
) -> Result<(), VErr> {
    for i in 0..n {
        let slot = slot_base + i as u64;
        // SAFETY: wrapper contract.
        let p = unsafe { syscall1(SYS_ALLOC_FRAME, slot) };
        if p <= 0 {
            log_line(|o| {
                o.str(prefix);
                o.str(": frame alloc ");
                o.u64(i as u64);
                o.str(" returned ");
                o.i64(p);
            });
            return Err(VErr::Queue("a ring/buffer frame allocation refused"));
        }
        phys[i] = p as u64;
        // SAFETY: wrapper contract.
        let m = unsafe { syscall2(SYS_MAP_MEMORY, slot, 1) };
        if m <= 0 {
            return Err(VErr::Map("a frame self-map refused"));
        }
        va[i] = m as u64;
        // SAFETY: a freshly mapped, owned, page-aligned 4 KiB frame.
        unsafe { core::ptr::write_bytes(va[i] as *mut u8, 0, 4096) };
    }
    Ok(())
}

/// The three ring-area offsets inside ONE frame for a queue of `qsz`
/// entries: desc 16B-aligned at 0, avail 2B-aligned right after, used
/// 4B-aligned after that (modern virtio needs no page alignment). For
/// qsz=64: 0 / 1024 / 1160 — the used area ends at 1678.
pub fn ring_offsets(qsz: u16) -> (u64, u64, u64) {
    let desc = 0u64;
    let avail = 16 * u64::from(qsz);
    let used_raw = avail + 6 + 2 * u64::from(qsz);
    let used = (used_raw + 3) & !3;
    (desc, avail, used)
}

/// One split virtqueue's driver-side state. Ring VAs point into the
/// driver's own mapped frame(s); `doorbell` is the notify address for
/// THIS queue (window + notify_off + queue_notify_off * multiplier).
pub struct Queue {
    pub qsz: u16,
    pub desc_va: u64,
    pub avail_va: u64,
    pub used_va: u64,
    pub doorbell: u64,
    pub avail_idx: u16,
    pub used_seen: u16,
}

impl Queue {
    /// Publish descriptor head `id` in the next avail slot and ring
    /// the doorbell (the fence discipline every driver has always
    /// used: slot → fence → index → fence → doorbell).
    ///
    /// # Safety
    /// The queue's VAs are the driver's own mapped frames;
    /// single-threaded image.
    pub unsafe fn publish(&mut self, id: u16) {
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
    /// As `publish`.
    pub unsafe fn used_idx(&self) -> u16 {
        // SAFETY: method contract.
        unsafe { r16(self.used_va + 2) }
    }

    /// Read used entry `slot` → (descriptor id, device-written length).
    ///
    /// # Safety
    /// As `publish`.
    pub unsafe fn used_entry(&self, slot: u16) -> (u32, u32) {
        // SAFETY: method contract.
        unsafe {
            let base = self.used_va + 4 + 8 * u64::from(slot);
            (r32(base), r32(base + 4))
        }
    }
}

/// Where one queue's three ring areas live (the frame budget is the
/// driver's decision, ADR-0024/0025): packed into ONE owned frame at
/// `ring_offsets(qsz)` (netd, rngd — CAP_SLOTS pressure), or one
/// frame each (storaged's 256-entry rings).
pub enum RingMem {
    Packed {
        frame_phys: u64,
        frame_va: u64,
    },
    Split {
        phys: (u64, u64, u64),
        va: (u64, u64, u64),
    },
}

/// The driver's interrupt plan for one queue: which MSI-X table entry
/// to arm, the cap slot holding the notification the relay delivers
/// to, and the badge word that names THIS queue in the merged
/// notification (netd decodes two by bit; single-queue drivers just
/// compare).
pub struct IrqPlan {
    pub msix_entry: u16,
    pub slot_notif: u64,
    pub badge: u64,
}

/// Set up one split virtqueue end to end: select it, clamp its size
/// to `qmax_cap` (the driver's frame budget — ring areas must fit the
/// frames it owns), write the three ring addresses (PHYS — the device
/// never sees VAs), arm the driver's MSI-X `entry` through the kernel
/// relay (ring 3 never touches the MSI-X table; SYS_IRQ_RELAY programs
/// it and returns the relay vector), point the queue at that entry,
/// resolve the doorbell, and enable (sticky-checked). Returns the
/// queue state and the relay vector. DRIVER_OK is NOT written here —
/// the driver finishes its own setup (buffer posting, static headers)
/// first, then calls [`driver_ok`].
///
/// # Safety
/// `w` a mapped device window; the ring frames are owned mappings of
/// this image; `slot_notif` holds the interrupt notification cap;
/// single-threaded image.
pub unsafe fn queue_setup(
    prefix: &str,
    w: &Window,
    info: &DevInfo,
    qidx: u16,
    qmax_cap: u16,
    rings: RingMem,
    irq: IrqPlan,
) -> Result<(Queue, i64), VErr> {
    // SAFETY: mapped common-config registers; volatile accessors.
    unsafe {
        w16(w.cfg + CFG_QUEUE_SELECT, qidx);
        let qmax = r16(w.cfg + CFG_QUEUE_SIZE);
        if qmax == 0 {
            return Err(VErr::Queue("the device advertises queue size 0"));
        }
        let qsz = if qmax > qmax_cap { qmax_cap } else { qmax };
        w16(w.cfg + CFG_QUEUE_SIZE, qsz);
        let (ring_phys, ring_va) = match rings {
            RingMem::Packed {
                frame_phys,
                frame_va,
            } => {
                let (d, a, u) = ring_offsets(qsz);
                (
                    (frame_phys + d, frame_phys + a, frame_phys + u),
                    (frame_va + d, frame_va + a, frame_va + u),
                )
            }
            RingMem::Split { phys, va } => (phys, va),
        };
        w64(w.cfg + CFG_QUEUE_DESC, ring_phys.0);
        w64(w.cfg + CFG_QUEUE_DRIVER, ring_phys.1);
        w64(w.cfg + CFG_QUEUE_DEVICE, ring_phys.2);
        let vec = syscall4(
            SYS_IRQ_RELAY,
            info.dev_idx,
            u64::from(irq.msix_entry),
            irq.slot_notif,
            irq.badge,
        );
        if !(48..=63).contains(&vec) {
            log_line(|o| {
                o.str(prefix);
                o.str(": irq_relay(entry ");
                o.u64(u64::from(irq.msix_entry));
                o.str(") returned ");
                o.i64(vec);
            });
            return Err(VErr::Relay("SYS_IRQ_RELAY refused"));
        }
        w16(w.cfg + CFG_QUEUE_MSIX_VECTOR, irq.msix_entry);
        let qnoff = u64::from(r16(w.cfg + CFG_QUEUE_NOTIFY_OFF));
        let doorbell = w.win + info.notify_off + qnoff * info.notify_mult;
        w16(w.cfg + CFG_QUEUE_ENABLE, 1);
        if r16(w.cfg + CFG_QUEUE_ENABLE) != 1 {
            return Err(VErr::Queue("queue_enable did not stick"));
        }
        Ok((
            Queue {
                qsz,
                desc_va: ring_va.0,
                avail_va: ring_va.1,
                used_va: ring_va.2,
                doorbell,
                avail_idx: 0,
                used_seen: 0,
            },
            vec,
        ))
    }
}

/// The final handshake step: DRIVER_OK (the device may now process
/// the virtqueues). Written as the full status byte — every prior bit
/// is re-asserted, exactly as both original drivers did.
///
/// # Safety
/// `w` a mapped device window.
pub unsafe fn driver_ok(w: &Window) {
    // SAFETY: mapped status register; volatile write.
    unsafe {
        w8(
            w.cfg + CFG_STATUS,
            ST_ACK | ST_DRIVER | ST_FEATURES_OK | ST_DRIVER_OK,
        )
    };
}

/// 64-bit register write as the spec's lo-then-hi pair (§4.1.4.3: the
/// low half is written first).
///
/// # Safety
/// `reg_va` is a mapped 8-byte register/ring location the driver owns.
pub unsafe fn w64(reg_va: u64, v: u64) {
    // SAFETY: caller contract; two volatile dword writes.
    unsafe {
        w32(reg_va, v as u32);
        w32(reg_va + 4, (v >> 32) as u32);
    }
}

/// Write one split-ring descriptor (16 bytes: addr, len, flags, next;
/// little-endian fields).
///
/// # Safety
/// `desc_va` is the driver's own mapped desc-table area; `i < qsz`.
pub unsafe fn desc_write(desc_va: u64, i: u16, addr: u64, len: u32, flags: u16, next: u16) {
    // SAFETY: caller contract; all four field writes are volatile and
    // inside the mapped frame.
    unsafe {
        let d = desc_va + u64::from(i) * 16;
        core::ptr::write_volatile(d as *mut u64, addr);
        w32(d + 8, len);
        w16(d + 12, flags);
        w16(d + 14, next);
    }
}
