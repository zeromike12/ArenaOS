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

#[path = "../../abi.rs"]
mod abi;
use abi::*;

#[path = "../../virtio.rs"]
mod virtio;
use virtio::*;

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

// ---- virtio-blk specifics (the shared 1.0 core lives in userspace/virtio.rs) --

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

/// virtio-blk PCI device IDs: modern (0x1040 + type 2) and the
/// transitional block ID — the discovery assert (ADR-0024).
const DEV_ID_BLK_MODERN: u64 = 0x1042;
const DEV_ID_BLK_TRANSITIONAL: u64 = 0x1001;

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

/// The shared virtio core's typed stage failure → storaged's
/// exit-code contract (the m5 suite maps every code): the five setup
/// stages in order, 60..64.
fn vfail(e: VErr) -> ! {
    match e {
        VErr::DevInfo(r) => fail(EXIT_DEV_INFO, r),
        VErr::Map(r) => fail(EXIT_MAP, r),
        VErr::Handshake(r) => fail(EXIT_HANDSHAKE, r),
        VErr::Queue(r) => fail(EXIT_QUEUE, r),
        VErr::Relay(r) => fail(EXIT_RELAY, r),
    }
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

// ---- the program ----------------------------------------------------------------

/// The service loop's per-request state: the shared core's virtqueue
/// (userspace/virtio.rs) plus the block request page and the
/// completion count.
struct Queue {
    vq: virtio::Queue,
    req_va: u64,
    req_phys: u64,
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
        desc_write(q.vq.desc_va, 0, q.req_phys, REQ_HEADER_LEN, DESC_F_NEXT, 1);
        let data_flags = if op == OP_READ {
            DESC_F_NEXT | DESC_F_WRITE
        } else {
            DESC_F_NEXT
        };
        desc_write(
            q.vq.desc_va,
            1,
            buf_phys,
            SECTOR_BYTES as u32,
            data_flags,
            2,
        );
        desc_write(
            q.vq.desc_va,
            2,
            q.req_phys + REQ_STATUS_OFF,
            1,
            DESC_F_WRITE,
            0,
        );
        // Publish (the shared core's fence discipline: ring slot →
        // fence → avail index → fence → doorbell) — head 0, the only
        // chain in flight.
        q.vq.publish(0);
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
        let used_idx = q.vq.used_idx();
        if used_idx == q.vq.used_seen {
            log("storaged: completion interrupt without a used entry");
            return Err(());
        }
        let slot = used_idx.wrapping_sub(1) % q.vq.qsz;
        let (id, len) = q.vq.used_entry(slot);
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
        q.vq.used_seen = used_idx;
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

        // 1. Order-independent discovery (the shared virtio core —
        //    userspace/virtio.rs, ADR-0025): probe device indices
        //    upward and adopt the first one SYS_DEV_INFO answers — the
        //    Mmio-cap gate inside the kernel guarantees the answer is
        //    OUR device (the cap names its structure BAR), so fixture
        //    order on the QEMU command line is not load-bearing. The
        //    core asserts virtio-BLOCK from the device_id: a mis-spawn
        //    must fail loudly, not drive the wrong device.
        let info = virtio::discover(
            "storaged",
            "virtio-blk",
            DEV_ID_BLK_MODERN,
            DEV_ID_BLK_TRANSITIONAL,
            1,
        )
        .unwrap_or_else(|e| vfail(e));

        // 2. The device window: self-map the granted Mmio cap WRITABLE
        //    (the handshake writes registers; access mode decides the
        //    right, and the cap survives — Mmio is descriptive). The
        //    shared core logs the resolved geometry: identity first,
        //    geometry second.
        let w = virtio::map_window("storaged", "virtio-blk", &info, SLOT_MMIO)
            .unwrap_or_else(|e| vfail(e));

        // 3. The virtio 1.0 handshake (§3.1), shared core: accept
        //    VERSION_1 (the modern driver's one mandatory bit) and
        //    NOTHING else — no optional blk features in v1; one
        //    virtqueue minimum; FEATURES_OK must stick.
        virtio::handshake("storaged", &w, 0, FEATURE_VERSION_1, 1).unwrap_or_else(|e| vfail(e));

        // 4. Queue 0: one split virtqueue over three OWNED frames, plus
        //    a request page (header + status). Frame order: 0 desc,
        //    1 avail (driver area), 2 used (device area), 3 request
        //    page. Allocation pays the phys (the queue registers need
        //    it); self-map consumes each cap (ownership moves to this
        //    address space — teardown exact). The shared core zeroes
        //    every mapping: rings must START zeroed (idx 0, no
        //    descriptors, no used entries).
        let mut phys = [0u64; 4];
        let mut va = [0u64; 4];
        virtio::alloc_frames("storaged", SLOT_FRAME_BASE, 4, &mut phys, &mut va)
            .unwrap_or_else(|e| vfail(e));

        // 5. The interrupt story, shared core: arm MSI-X entry 0
        //    through the kernel (it programs the table — ring 3 never
        //    touches it — and enables MSI-X in config space), point
        //    queue 0 at entry 0, resolve the doorbell, enable
        //    (sticky-checked). The config-change vector stays
        //    NO_VECTOR. Then DRIVER_OK.
        let (vq, vec) = virtio::queue_setup(
            "storaged",
            &w,
            &info,
            0,
            QUEUE_MAX,
            RingMem::Split {
                phys: (phys[0], phys[1], phys[2]),
                va: (va[0], va[1], va[2]),
            },
            IrqPlan {
                msix_entry: 0,
                slot_notif: SLOT_NOTIF,
                badge: IRQ_BADGE,
            },
        )
        .unwrap_or_else(|e| vfail(e));
        virtio::driver_ok(&w);
        let (qsz, doorbell) = (vq.qsz, vq.doorbell);
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
            vq,
            req_va: va[3],
            req_phys: phys[3],
            completions: 0,
        };
        let mut msg = [0u64; 3];
        loop {
            let r = syscall3(SYS_IPC_RECV, SLOT_EP, msg.as_mut_ptr() as u64, 0);
            if r < 0 {
                log_line(|o| {
                    o.str("storaged: recv returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "SYS_IPC_RECV refused");
            }
            let (sector, w1, landed) = (msg[0], msg[1], msg[2]);
            // v1.1: w1 = op | (buf_offset << 8). The offset is validated
            // before it can aim the device anywhere: a request whose
            // sector would run past the caller's 4 KiB frame is refused
            // with a typed status (ADR-0022/0023 security seam).
            let (op, buf_off) = (w1 & 0xFF, w1 >> 8);
            if op == OP_SHUTDOWN {
                // The poison request: reply FIRST (the caller blocks),
                // then exit — a process is never destroyed with parked
                // threads, so the service always dies by its own hand.
                log_line(|o| {
                    o.str("storaged: shutdown requested after ");
                    o.u64(q.completions);
                    o.str(" interrupt-delivered completion(s) — replying and exiting");
                });
                let rr = syscall5(
                    SYS_IPC_REPLY,
                    SLOT_EP,
                    VIRTIO_BLK_S_OK,
                    q.completions,
                    CAP_NONE,
                    0,
                );
                if rr < 0 {
                    fail(EXIT_REPLY, "the shutdown reply refused");
                }
                // SAFETY: thread_exit diverges; 42 is the clean-exit code.
                syscall1(SYS_THREAD_EXIT, EXIT_OK);
            }
            if op > OP_WRITE {
                let _ = syscall1(SYS_CAP_DESTROY, landed);
                let rr = syscall5(SYS_IPC_REPLY, SLOT_EP, VIRTIO_BLK_S_UNSUPP, 0, CAP_NONE, 0);
                if rr < 0 {
                    fail(EXIT_REPLY, "the unsupported-op reply refused");
                }
                continue;
            }
            if landed == CAP_NONE || buf_off + SECTOR_BYTES as u64 > BLOCK_FRAME_BYTES {
                let _ = syscall1(SYS_CAP_DESTROY, landed);
                let st = if landed == CAP_NONE {
                    VIRTIO_BLK_S_IOERR
                } else {
                    VIRTIO_BLK_S_UNSUPP // out-of-frame buffer offset
                };
                let rr = syscall5(SYS_IPC_REPLY, SLOT_EP, st, 0, CAP_NONE, 0);
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
            match serve(&mut q, op, sector, bp as u64 + buf_off) {
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
                    let rr = syscall5(SYS_IPC_REPLY, SLOT_EP, status, u64::from(len), CAP_NONE, 0);
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
                        o.u64(u64::from(q.vq.used_seen));
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
