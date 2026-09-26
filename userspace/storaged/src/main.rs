//! ArenaOS block service — `storaged`, the userspace virtio-blk driver
//! (M5.2, ADR-0022). Spawn-registry image 2, spawned at boot by the
//! kernel (and as a short-lived test instance by the m5 suite) with
//! kernel-literal grants:
//!
//! - slot 0: `Mmio` over the BAR carrying the virtio structures
//!   (READ|WRITE — the handshake writes registers),
//! - slot 1: `Endpoint` (READ — the serve side of the block service),
//! - slot 2: `Notification` (READ|WRITE — the interrupt relay target).
//!
//! Everything else is this program's own work, all from ring 3:
//! discover the resolved device record (`SYS_DEV_INFO`), self-map the
//! window, run the virtio 1.0 handshake, build one split virtqueue out
//! of three OWNED frames (allocated with `SYS_ALLOC_FRAME`, self-mapped,
//! consumed — teardown reclaims them exactly), arm the MSI-X relay
//! (`SYS_IRQ_RELAY`), then serve requests: the caller LENDS its buffer
//! frame with the call, the driver points a descriptor at the frame's
//! phys (`SYS_CAP_PHYS`) and the device DMAs the caller's own page —
//! zero copy in both directions. Completions arrive as device MSI-X
//! interrupts relayed into `SYS_WAIT`; nothing is ever polled.
//!
//! Exit codes (the diagnostic contract with the m5 suite): 42 clean
//! shutdown (the poison request was served), 60..68 stage failures
//! (see the constants), 97 console refused, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "abi.rs"]
mod abi;
use abi::*;

// ---- the grant layout (kernel-literal: entry.rs / m5.rs) --------------------

const SLOT_MMIO: u64 = 0;
const SLOT_EP: u64 = 1;
const SLOT_NOTIF: u64 = 2;
/// Scratch slots for the four owned ring frames — self-map CONSUMES the
/// cap, so these go empty again; a landed caller-buffer cap takes the
/// first free slot (3) and is destroyed after every request.
const SLOT_FRAME_BASE: u64 = 8;

/// The badge the IRQ relay delivers (only the relay notifies this nid).
const IRQ_BADGE: u64 = 0x9E71;

// ---- failure exits (the m5 block_service test maps each one) ----------------

const EXIT_DEV_INFO: u64 = 60;
const EXIT_MAP: u64 = 61;
const EXIT_HANDSHAKE: u64 = 62;
const EXIT_QUEUE: u64 = 63;
const EXIT_RELAY: u64 = 64;
const EXIT_RECV: u64 = 65;
const EXIT_PHYS: u64 = 66;
const EXIT_COMPLETE: u64 = 67;
const EXIT_REPLY: u64 = 68;

// ---- virtio constants (OASIS virtio 1.0 §4.1.4.3, §5.2) ---------------------

/// Device status bits (§2.1).
const ST_ACK: u8 = 1;
const ST_DRIVER: u8 = 2;
const ST_DRIVER_OK: u8 = 4;
const ST_FEATURES_OK: u8 = 8;

/// `VIRTIO_F_VERSION_1` — bit 32 of the feature word, i.e. bit 0 of
/// device_feature with select = 1 (§6.2).
const FEATURE_VERSION_1: u32 = 1 << 0;

/// `virtio_pci_common_cfg` register offsets (§4.1.4.3).
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

/// Split-ring descriptor flags (§2.7.5).
const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;

/// virtio-blk request types (§5.2.6.1).
const BLK_T_IN: u32 = 0;
const BLK_T_OUT: u32 = 1;

/// The request page layout: `virtio_blk_outhdr` (16 bytes) at +0, the
/// device-writable status byte at +64 (own cache line, clear of the
/// header). Data buffers are never in this page — they are the
/// CALLER's frames.
const REQ_HEADER_LEN: u32 = 16;
const REQ_STATUS_OFF: u64 = 64;
/// Status sentinel: proves the completion status came from the device,
/// not from stale memory (the device MUST write it, §5.2.6.4).
const STATUS_SENTINEL: u8 = 0xFF;

/// Queue depth cap: desc (16 B/entry), avail, and used each fit one
/// 4 KiB frame at 256 entries, and the driver owns exactly one frame
/// per ring. The device's advertised maximum is clamped to this.
const QUEUE_MAX: u16 = 256;

/// SYS_DEV_INFO's word count (the ADR-0022 layout).
const INFO_WORDS: usize = 12;

// ---- diagnostics -------------------------------------------------------------

/// The honest failure path: say what broke, exit with the stage code.
fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("storaged: FATAL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m5 contract.
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
    // No panicking path exists by construction (checked arithmetic, no
    // allocation, no indexing of foreign data). If one is ever reached,
    // say so through the ABI itself.
    write_str("storaged: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

// ---- ring/register helpers ----------------------------------------------------

/// Write one split-ring descriptor (16 bytes: addr, len, flags, next).
///
/// # Safety
/// `desc_va` is the driver's own mapped desc-table frame; `i < qsz`.
unsafe fn desc_write(desc_va: u64, i: u16, addr: u64, len: u32, flags: u16, next: u16) {
    // SAFETY: caller contract; all four field writes are volatile and
    // inside the mapped frame (i*16 + 16 <= 4096 for i < 256).
    unsafe {
        let d = desc_va + u64::from(i) * 16;
        core::ptr::write_volatile(d as *mut u64, addr);
        w32(d + 8, len);
        w16(d + 12, flags);
        w16(d + 14, next);
    }
}

/// 64-bit register write as the spec's lo-then-hi pair (§4.1.4.3: the
/// low half is written first).
///
/// # Safety
/// `reg_va` is a mapped 8-byte common-cfg register.
unsafe fn w64(reg_va: u64, v: u64) {
    // SAFETY: caller contract; two volatile dword writes.
    unsafe {
        w32(reg_va, v as u32);
        w32(reg_va + 4, (v >> 32) as u32);
    }
}

// ---- the program ----------------------------------------------------------------

/// The service loop's per-request state.
struct Queue {
    qsz: u16,
    desc_va: u64,
    avail_va: u64,
    used_va: u64,
    req_va: u64,
    req_phys: u64,
    doorbell: u64,
    avail_idx: u16,
    used_seen: u16,
    completions: u64,
}

/// Serve one request: build the 3-descriptor chain (header / caller's
/// buffer / status), publish it, ring the doorbell, and block on the
/// relay until the device's MSI-X completion arrives. Returns the
/// device's status byte and its used-ring length.
fn serve(q: &mut Queue, op: u64, sector: u64, buf_phys: u64) -> Result<(u64, u32), ()> {
    // The request header lives in the driver's own req page; the DATA
    // descriptor points at the CALLER's frame — the device DMAs it
    // directly (zero copy, ADR-0022).
    let req_type = if op == OP_WRITE { BLK_T_OUT } else { BLK_T_IN };
    // SAFETY: q's VAs are the driver's own mapped frames; single-
    // threaded image; volatile ring writes; the sentinel proves the
    // completion status is the device's.
    unsafe {
        w32(q.req_va, req_type);
        w32(q.req_va + 4, 0); // ioprio (unused in v1)
        core::ptr::write_volatile((q.req_va + 8) as *mut u64, sector);
        w8(q.req_va + REQ_STATUS_OFF, STATUS_SENTINEL);
        // Chain: 0 header (device-readable) → 1 data (readable for a
        // write, WRITABLE for a read) → 2 status (device-writable).
        desc_write(q.desc_va, 0, q.req_phys, REQ_HEADER_LEN, DESC_F_NEXT, 1);
        let data_flags = if op == OP_READ {
            DESC_F_NEXT | DESC_F_WRITE
        } else {
            DESC_F_NEXT
        };
        desc_write(q.desc_va, 1, buf_phys, SECTOR_BYTES as u32, data_flags, 2);
        desc_write(
            q.desc_va,
            2,
            q.req_phys + REQ_STATUS_OFF,
            1,
            DESC_F_WRITE,
            0,
        );
        // Publish: ring slot → fence → avail index → fence → doorbell.
        w16(q.avail_va + 4 + 2 * u64::from(q.avail_idx % q.qsz), 0);
        store_fence();
        q.avail_idx = q.avail_idx.wrapping_add(1);
        w16(q.avail_va + 2, q.avail_idx);
        store_fence();
        w16(q.doorbell, 0); // queue 0
    }

    // The completion is an interrupt, never a poll: the device's MSI-X
    // write lands on the relay vector, the kernel stub notifies, and
    // SYS_WAIT returns the badge (already pending if the MSI beat us
    // here — the merged-badge protocol closes that race).
    // SAFETY: wrapper contract.
    let b = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
    if b != IRQ_BADGE as i64 {
        log_line(|o| {
            o.str("storaged: wait returned ");
            o.i64(b);
            o.str(", expected the relay badge");
        });
        return Err(());
    }
    // SAFETY: q's VAs are the driver's own mapped frames; volatile ring
    // reads after the device's completion interrupt.
    unsafe {
        let used_idx = r16(q.used_va + 2);
        if used_idx == q.used_seen {
            log("storaged: completion interrupt without a used entry");
            return Err(());
        }
        let slot = u64::from(used_idx.wrapping_sub(1) % q.qsz);
        let id = r32(q.used_va + 4 + 8 * slot);
        let len = r32(q.used_va + 4 + 8 * slot + 4);
        if id != 0 {
            // One synchronous request at a time, so the head is always
            // descriptor 0 — anything else is a device contract break.
            log_line(|o| {
                o.str("storaged: used entry names descriptor ");
                o.u64(u64::from(id));
                o.str(", expected 0");
            });
            return Err(());
        }
        q.used_seen = used_idx;
        let status = r8(q.req_va + REQ_STATUS_OFF);
        if status == STATUS_SENTINEL {
            log("storaged: device never wrote the status byte");
            return Err(());
        }
        q.completions += 1;
        Ok((u64::from(status), len))
    }
}

/// The driver entry: the spawn protocol's first thread lands here at
/// ring 3 with the grant slots filled. Nothing returns — the service
/// ends only through the poison request's clean exit or a `fail`.
///
/// # Safety
/// Entered exactly as the shell's `_start` is: ring 3, RSP at the
/// derived stack top, the address space the kernel loaded and
/// validated, grants in slots 0..2. The body is ABI v1 wrappers over
/// this image's own statics and windows — every device and ring touch
/// is volatile, single-threaded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and its granted/allocated windows, inside the
    // address space the kernel loaded and validated. Single-threaded;
    // every device and ring touch is volatile.
    unsafe {
        log("storaged: starting — ArenaOS block service (M5.2, ADR-0022)");

        // 1. The kernel's resolved record of our device (SYS_DEV_INFO,
        //    gated on the Mmio cap in slot 0 — config space itself is
        //    never exposed to ring 3).
        // The virtio-DEVICE index this service drives (v1: the boot
        // scan's first virtio-block function — the same index space
        // SYS_DEV_INFO and SYS_IRQ_RELAY take; word[0]'s pci_index is
        // a function-table index, informational only).
        let dev_idx = 0u64;
        let mut info = [0u64; INFO_WORDS];
        let r = syscall2(SYS_DEV_INFO, dev_idx, info.as_mut_ptr() as u64);
        if r != INFO_WORDS as i64 {
            log_line(|o| {
                o.str("storaged: dev_info returned ");
                o.i64(r);
            });
            fail(EXIT_DEV_INFO, "SYS_DEV_INFO refused");
        }
        let pci_index = info[0];
        let common_off = info[2] & 0xFFFF_FFFF;
        let notify_off = info[3] & 0xFFFF_FFFF;
        let notify_mult = info[4];
        let msix_present = info[7] & 0xFFFF_FFFF;
        let msix_size = (info[7] >> 32) as u16;
        let device_id = info[8] & 0xFFFF;
        let transitional = (info[8] >> 32) & 0xFFFF;
        if msix_present == 0 || msix_size == 0 {
            fail(
                EXIT_RELAY,
                "the device has no MSI-X table — the relay plan needs it",
            );
        }

        // 2. The device window: self-map the granted Mmio cap WRITABLE
        //    (the handshake writes registers; access mode decides the
        //    right, and the cap survives — Mmio is descriptive).
        let win = syscall2(SYS_MAP_MEMORY, SLOT_MMIO, 1);
        if win <= 0 {
            log_line(|o| {
                o.str("storaged: window map returned ");
                o.i64(win);
            });
            fail(EXIT_MAP, "the device-window self-map refused");
        }
        let win = win as u64;
        let cfg = win + common_off;
        log_line(|o| {
            o.str("storaged: virtio-blk device_id ");
            o.hex(device_id);
            if transitional != 0 {
                o.str(" (transitional)");
            }
            o.str(" pci_index ");
            o.u64(pci_index);
            o.str(" — window ");
            o.hex(win);
            o.str(", common +");
            o.hex(common_off);
            o.str(", notify +");
            o.hex(notify_off);
            o.str(" mult ");
            o.u64(notify_mult);
            o.str(", msix ");
            o.u64(u64::from(msix_size));
        });

        // 3. The virtio 1.0 handshake (§3.1), all through the window.
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
        // Features: accept VERSION_1 iff offered (the modern driver's
        // one mandatory bit); no optional blk features in v1.
        w32(cfg + CFG_DEV_FEATURE_SELECT, 0);
        let f0 = r32(cfg + CFG_DEV_FEATURE);
        w32(cfg + CFG_DEV_FEATURE_SELECT, 1);
        let f1 = r32(cfg + CFG_DEV_FEATURE);
        w32(cfg + CFG_DRV_FEATURE_SELECT, 0);
        w32(cfg + CFG_DRV_FEATURE, 0);
        w32(cfg + CFG_DRV_FEATURE_SELECT, 1);
        w32(cfg + CFG_DRV_FEATURE, f1 & FEATURE_VERSION_1);
        w8(cfg + CFG_STATUS, ST_ACK | ST_DRIVER | ST_FEATURES_OK);
        if r8(cfg + CFG_STATUS) & ST_FEATURES_OK == 0 {
            log_line(|o| {
                o.str("storaged: device features ");
                o.hex(u64::from(f1) << 32 | u64::from(f0));
                o.str(" — FEATURES_OK did not stick");
            });
            fail(EXIT_HANDSHAKE, "feature negotiation refused by the device");
        }
        if r16(cfg + CFG_NUM_QUEUES) < 1 {
            fail(EXIT_HANDSHAKE, "the device offers no virtqueues");
        }

        // 4. Queue 0: one split virtqueue over three OWNED frames, plus
        //    a request page (header + status). Allocation pays the phys
        //    (the queue registers need it); self-map consumes each cap
        //    (ownership moves to this address space — teardown exact).
        w16(cfg + CFG_QUEUE_SELECT, 0);
        let qmax = r16(cfg + CFG_QUEUE_SIZE);
        if qmax == 0 {
            fail(EXIT_QUEUE, "the device advertises queue size 0");
        }
        let qsz = if qmax > QUEUE_MAX { QUEUE_MAX } else { qmax };
        w16(cfg + CFG_QUEUE_SIZE, qsz);
        // Frame order: 0 desc, 1 avail (driver area), 2 used (device
        // area), 3 request page.
        let mut phys = [0u64; 4];
        let mut va = [0u64; 4];
        for i in 0..4usize {
            let slot = SLOT_FRAME_BASE + i as u64;
            let p = syscall1(SYS_ALLOC_FRAME, slot);
            if p <= 0 {
                log_line(|o| {
                    o.str("storaged: frame alloc ");
                    o.u64(i as u64);
                    o.str(" returned ");
                    o.i64(p);
                });
                fail(EXIT_QUEUE, "a ring-frame allocation refused");
            }
            phys[i] = p as u64;
            let m = syscall2(SYS_MAP_MEMORY, slot, 1);
            if m <= 0 {
                fail(EXIT_MAP, "a ring-frame self-map refused");
            }
            va[i] = m as u64;
            // The allocator does not zero: rings must START zeroed
            // (idx 0, no descriptors, no used entries).
            core::ptr::write_bytes(va[i] as *mut u8, 0, 4096);
        }
        // SAFETY: the queue_* registers are mapped common-cfg dwords;
        // lo half first, per §4.1.4.3.
        w64(cfg + CFG_QUEUE_DESC, phys[0]);
        w64(cfg + CFG_QUEUE_DRIVER, phys[1]);
        w64(cfg + CFG_QUEUE_DEVICE, phys[2]);

        // 5. The interrupt story: arm MSI-X entry 0 through the kernel
        //    (it programs the table — ring 3 never touches it — and
        //    enables MSI-X in config space), then point queue 0 at
        //    entry 0. The config-change vector stays NO_VECTOR.
        let vec = syscall4(SYS_IRQ_RELAY, dev_idx, 0, SLOT_NOTIF, IRQ_BADGE);
        if !(48..=63).contains(&vec) {
            log_line(|o| {
                o.str("storaged: irq_relay returned ");
                o.i64(vec);
            });
            fail(EXIT_RELAY, "SYS_IRQ_RELAY refused");
        }
        w16(cfg + CFG_QUEUE_MSIX_VECTOR, 0);
        let qnoff = u64::from(r16(cfg + CFG_QUEUE_NOTIFY_OFF));
        let doorbell = win + notify_off + qnoff * notify_mult;
        w16(cfg + CFG_QUEUE_ENABLE, 1);
        if r16(cfg + CFG_QUEUE_ENABLE) != 1 {
            fail(EXIT_QUEUE, "queue_enable did not stick");
        }
        w8(
            cfg + CFG_STATUS,
            ST_ACK | ST_DRIVER | ST_FEATURES_OK | ST_DRIVER_OK,
        );
        log_line(|o| {
            o.str("storaged: virtio-blk ready — DRIVER_OK, queue 0 size ");
            o.u64(u64::from(qsz));
            o.str(", desc ");
            o.hex(phys[0]);
            o.str(", avail ");
            o.hex(phys[1]);
            o.str(", used ");
            o.hex(phys[2]);
            o.str(", req ");
            o.hex(phys[3]);
            o.str("; doorbell ");
            o.hex(doorbell);
            o.str(", msix entry 0 → relay vector ");
            o.i64(vec);
        });

        // 6. The service loop: recv → zero-copy request → interrupt →
        //    reply. One request in flight at a time (synchronous IPC).
        let mut q = Queue {
            qsz,
            desc_va: va[0],
            avail_va: va[1],
            used_va: va[2],
            req_va: va[3],
            req_phys: phys[3],
            doorbell,
            avail_idx: 0,
            used_seen: 0,
            completions: 0,
        };
        let mut msg = [0u64; 3];
        loop {
            let r = syscall2(SYS_IPC_RECV, SLOT_EP, msg.as_mut_ptr() as u64);
            if r < 0 {
                log_line(|o| {
                    o.str("storaged: recv returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "SYS_IPC_RECV refused");
            }
            let (sector, op, landed) = (msg[0], msg[1], msg[2]);
            if op == OP_SHUTDOWN {
                // The poison request: reply FIRST (the caller blocks),
                // then exit — a process is never destroyed with parked
                // threads, so the service always dies by its own hand.
                log_line(|o| {
                    o.str("storaged: shutdown requested after ");
                    o.u64(q.completions);
                    o.str(" interrupt-delivered completion(s) — replying and exiting");
                });
                let rr = syscall4(
                    SYS_IPC_REPLY,
                    SLOT_EP,
                    VIRTIO_BLK_S_OK,
                    q.completions,
                    CAP_NONE,
                );
                if rr < 0 {
                    fail(EXIT_REPLY, "the shutdown reply refused");
                }
                // SAFETY: thread_exit diverges; 42 is the clean-exit code.
                syscall1(SYS_THREAD_EXIT, EXIT_OK);
            }
            if op > OP_WRITE {
                let _ = syscall1(SYS_CAP_DESTROY, landed);
                let rr = syscall4(SYS_IPC_REPLY, SLOT_EP, VIRTIO_BLK_S_UNSUPP, 0, CAP_NONE);
                if rr < 0 {
                    fail(EXIT_REPLY, "the unsupported-op reply refused");
                }
                continue;
            }
            if landed == CAP_NONE {
                let rr = syscall4(SYS_IPC_REPLY, SLOT_EP, VIRTIO_BLK_S_IOERR, 0, CAP_NONE);
                if rr < 0 {
                    fail(EXIT_REPLY, "the error reply refused");
                }
                continue;
            }
            // The zero-copy seam: the buffer's phys comes from the
            // LENT cap the caller attached — never from a message word.
            let bp = syscall1(SYS_CAP_PHYS, landed);
            if bp <= 0 {
                let _ = syscall1(SYS_CAP_DESTROY, landed);
                log_line(|o| {
                    o.str("storaged: cap_phys on landed slot ");
                    o.u64(landed);
                    o.str(" returned ");
                    o.i64(bp);
                });
                fail(EXIT_PHYS, "SYS_CAP_PHYS on the landed buffer refused");
            }
            match serve(&mut q, op, sector, bp as u64) {
                Ok((status, len)) => {
                    // The request is done: discard the lent reference
                    // (frees nothing — the caller's owned cap and its
                    // mapping stay alive) and answer.
                    let dr = syscall1(SYS_CAP_DESTROY, landed);
                    if dr < 0 {
                        log_line(|o| {
                            o.str("storaged: WARNING — landed cap destroy returned ");
                            o.i64(dr);
                        });
                    }
                    let rr = syscall4(SYS_IPC_REPLY, SLOT_EP, status, u64::from(len), CAP_NONE);
                    if rr < 0 {
                        fail(EXIT_REPLY, "SYS_IPC_REPLY refused");
                    }
                    log_line(|o| {
                        o.str("storaged: ");
                        o.str(if op == OP_WRITE { "WRITE" } else { "READ " });
                        o.str(" sector ");
                        o.u64(sector);
                        o.str(" buf ");
                        o.hex(bp as u64);
                        o.str(" → status ");
                        o.u64(status);
                        o.str(", device wrote ");
                        o.u64(u64::from(len));
                        o.str(" bytes (completion #");
                        o.u64(q.completions);
                        o.str(", used idx ");
                        o.u64(u64::from(q.used_seen));
                        o.str(")");
                    });
                }
                Err(()) => {
                    let _ = syscall1(SYS_CAP_DESTROY, landed);
                    fail(EXIT_COMPLETE, "the completion was missing or malformed");
                }
            }
        }
    }
}
