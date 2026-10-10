//! ArenaOS entropy service — `rngd`, the userspace virtio-rng driver
//! (M6.2, ADR-0025). Spawn-registry image 8, spawned at boot by the
//! kernel (and as a short-lived test instance by the m6 suite) with
//! kernel-literal grants:
//!
//! - slot 0: `Mmio` over the BAR carrying the virtio structures
//!   (READ|WRITE — the handshake writes registers),
//! - slot 1: `Endpoint` (READ — the serve side of the entropy
//!   service),
//! - slot 2: `Notification` (READ|WRITE — the interrupt relay target).
//! - optional production-only slot 3: a distinct Notification/WRITE
//!   for its own DRIVER_OK signal to the manager (ADR-0038). A test
//!   instance lacks it and ignores the refused optional notify.
//!
//! Everything else is this program's own work, all from ring 3 on top
//! of the SHARED virtio core (`userspace/virtio.rs` — the third-driver
//! extraction this crate was designed against): discover the resolved
//! device record, self-map the window, run the §3.1 handshake, build
//! ONE split virtqueue packed into a single owned frame (virtio-rng's
//! request queue — the device advertises just 8 entries, and the
//! shared clamp honors the device), arm the MSI-X relay, then serve:
//!
//! A GET LENDs the caller's frame with the call; rngd resolves the
//! frame's phys (`SYS_CAP_PHYS` — never a message word), points ONE
//! device-writable descriptor at it, and blocks on the relay badge.
//! The device DMAs entropy DIRECTLY into the caller's page — zero-copy
//! fill (storaged's READ direction, as a service). Completions are
//! interrupts, never polls; the used-ring length is the device's own
//! count of bytes written.
//!
//! Exit codes (the diagnostic contract with the m6 suite): 42 clean
//! shutdown (the poison request was served), 80..88 stage failures
//! (the same nine-stage block as storaged/netd), 97 console refused,
//! 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

#[path = "../../virtio.rs"]
mod virtio;
use virtio::*;

// ---- the grant layout (kernel-literal: entry.rs / m6.rs) ---------------------

const SLOT_MMIO: u64 = 0;
const SLOT_EP: u64 = 1;
const SLOT_DIAG: u64 = 4; // ADR-0047 boot-granted proof
const SLOT_NOTIF: u64 = 2;
/// Optional production-only boot readiness channel (ADR-0037).
const SLOT_MANAGER_READY: u64 = 3;
/// Production-only exact kernel CSPRNG seed authority (ADR-0110).
const SLOT_KERNEL_SEED: u64 = 5;

/// Owned frame slots: one request-queue frame plus one private seed frame.
/// The seed frame is used once after DRIVER_OK, then wiped by the kernel.
const SLOT_FRAME_BASE: u64 = 8;
const FRAMES_TOTAL: usize = 2;

/// The IRQ badge the relay delivers (only the relay notifies this nid).
const IRQ_BADGE_RNG: u64 = 0x7201;

// ---- the diagnostic exit contract (m6.rs maps every code) --------------------

const EXIT_DEV_INFO: u64 = 80;
const EXIT_MAP: u64 = 81;
const EXIT_HANDSHAKE: u64 = 82;
const EXIT_QUEUE: u64 = 83;
const EXIT_RELAY: u64 = 84;
const EXIT_RECV: u64 = 85;
const EXIT_PHYS: u64 = 86;
const EXIT_COMPLETE: u64 = 87;
const EXIT_REPLY: u64 = 88;
const EXIT_SEED: u64 = 89;

// ---- virtio-rng specifics (the shared 1.0 core lives in userspace/virtio.rs) --

/// virtio-rng PCI device IDs: modern (0x1040 + type 4 = 0x1044) and
/// the transitional entropy-source ID — 0x1005, NOT 0x1000+type: the
/// legacy IDs are a hand-assigned list (0x1001 block, 0x1000 net,
/// 0x1005 entropy), which is exactly what QEMU 11 presents.
const DEV_ID_RNG_MODERN: u64 = 0x1044;
const DEV_ID_RNG_TRANSITIONAL: u64 = 0x1005;

/// The queue-size cap rngd asks the shared core to clamp to (one
/// packed frame holds far more; QEMU's virtio-rng advertises 8 — the
/// device's own number wins, as always).
const QUEUE_MAX: u16 = 64;

/// Defensive bound on re-waits for one specific completion (a device
/// that raises the interrupt without publishing the used entry is
/// broken; the bound turns that into a typed failure instead of a
/// park the kernel's drain would have to catch).
const COMPLETION_REWAITS: u32 = 16;

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("rngd: FAIL: ");
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

/// The shared virtio core's typed stage failure → rngd's exit-code
/// contract (the m6 suite maps every code): the five setup stages in
/// order, 80..84.
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
    write_str("rngd: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

/// The whole driver state (single-threaded image; the serve loop owns
/// it end to end).
struct Drv {
    q: Queue,
    completions: u64,
}

impl Drv {
    /// Fill the caller's frame with device entropy: ONE
    /// device-writable descriptor at the frame's phys, published to
    /// the request queue, then blocked on the relay badge until the
    /// used ring carries the completion. Returns the device's own
    /// written-byte count. One request in flight (synchronous IPC).
    ///
    /// # Safety
    /// `buf_phys` is a caller frame the LENT cap named (resolved via
    /// SYS_CAP_PHYS); the queue's VAs are rngd's own mapped frame.
    unsafe fn fill(&mut self, buf_phys: u64, len: u32) -> Result<u32, ()> {
        // SAFETY: method contract.
        unsafe {
            desc_write(self.q.desc_va, 0, buf_phys, len, DESC_F_WRITE, 0);
            self.q.publish(0);
        }
        // The completion is an interrupt, never a poll: the device's
        // MSI-X write lands on the relay vector, the kernel stub
        // notifies, and SYS_WAIT returns the badge (already pending
        // if the MSI beat us here — the merged-badge protocol closes
        // that race). A single badge, so no bit-decoding (unlike
        // netd's two-queue word).
        let mut rewaits = 0u32;
        loop {
            // SAFETY: wrapper contract.
            let b = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
            if b != IRQ_BADGE_RNG as i64 {
                log_line(|o| {
                    o.str("rngd: wait returned ");
                    o.i64(b);
                    o.str(", expected the relay badge");
                });
                return Err(());
            }
            // SAFETY: the queue's VAs are rngd's own mapped frame.
            if unsafe { self.q.used_idx() } != self.q.used_seen {
                break;
            }
            rewaits += 1;
            if rewaits > COMPLETION_REWAITS {
                log("rngd: completion interrupt without a used entry");
                return Err(());
            }
        }
        // SAFETY: method contract; volatile ring reads after the
        // device's completion interrupt.
        unsafe {
            let slot = self.q.used_seen % self.q.qsz;
            let (id, written) = self.q.used_entry(slot);
            if id != 0 {
                // One synchronous request at a time, so the head is
                // always descriptor 0 — anything else is a device
                // contract break.
                log_line(|o| {
                    o.str("rngd: used entry names descriptor ");
                    o.u64(u64::from(id));
                    o.str(", expected 0");
                });
                return Err(());
            }
            self.q.used_seen = self.q.used_seen.wrapping_add(1);
            self.completions += 1;
            Ok(written)
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
        log("rngd: starting — ArenaOS entropy service (M6.2, ADR-0025)");

        // 1. Order-independent discovery + the window map + the §3.1
        //    handshake — the shared virtio core (userspace/virtio.rs).
        //    rngd's demands: a virtio-RNG function (a mis-spawn must
        //    fail loudly, not drive the wrong device), a one-entry
        //    MSI-X table for the relay, VERSION_1 and nothing else
        //    (virtio-rng defines no class feature bits), one
        //    virtqueue minimum (the request queue).
        let info = virtio::discover(
            "rngd",
            "virtio-rng",
            DEV_ID_RNG_MODERN,
            DEV_ID_RNG_TRANSITIONAL,
            1,
        )
        .unwrap_or_else(|e| vfail(e));
        let w =
            virtio::map_window("rngd", "virtio-rng", &info, SLOT_MMIO).unwrap_or_else(|e| vfail(e));
        virtio::handshake("rngd", &w, 0, FEATURE_VERSION_1, 1).unwrap_or_else(|e| vfail(e));

        // 2. The ONE owned frame: the request queue's ring areas,
        //    packed by the core at the CLAMPED size's offsets (the
        //    device advertises 8 entries; the core honors the
        //    device's number over the cap). Allocation pays the phys
        //    (the queue registers need it); self-map consumes the cap
        //    (ownership moves to this address space — teardown
        //    exact); the core zeroes the mapping (rings must START
        //    zeroed).
        let mut phys = [0u64; FRAMES_TOTAL];
        let mut va = [0u64; FRAMES_TOTAL];
        virtio::alloc_frames("rngd", SLOT_FRAME_BASE, FRAMES_TOTAL, &mut phys, &mut va)
            .unwrap_or_else(|e| vfail(e));

        // 3. The queue, through the core: MSI-X entry 0 armed through
        //    the kernel relay BEFORE enable (ring 3 never touches the
        //    table), doorbell resolved, enable sticky-checked. Then
        //    DRIVER_OK — rngd has no further setup (no buffers to
        //    post: the descriptors point at CALLER frames, per GET).
        let (q, vec) = virtio::queue_setup(
            "rngd",
            &w,
            &info,
            0,
            QUEUE_MAX,
            RingMem::Packed {
                frame_phys: phys[0],
                frame_va: va[0],
            },
            IrqPlan {
                msix_entry: 0,
                slot_notif: SLOT_NOTIF,
                badge: IRQ_BADGE_RNG,
            },
        )
        .unwrap_or_else(|e| vfail(e));
        virtio::driver_ok(&w);
        // One line, well inside WRITE_MAX=256 (Out::push drops past
        // the limit silently — the line budget is an invariant,
        // ADR-0023's HELP lesson).
        log_line(|o| {
            o.str("rngd: virtio-rng ready — DRIVER_OK, queue 0 size ");
            o.u64(u64::from(q.qsz));
            o.str(", ring frame ");
            o.hex(phys[0]);
            o.str("; doorbell ");
            o.hex(q.doorbell);
            o.str(", msix entry 0 → relay vector ");
            o.i64(vec);
        });

        // Announce only after the queue is DRIVER_OK; the test instance
        // has no slot 3, so its refused optional notify is harmless.
        let mut drv = Drv { q, completions: 0 };
        let mut ready_descriptor = [0u64; 3];
        let ready_occupied = syscall6(SYS_CAP_OCCUPIED, SLOT_MANAGER_READY, 0, 0, 0, 0, 0);
        if ready_occupied < 0 {
            fail(EXIT_SEED, "rngd manager-ready capability lookup failed");
        }
        let production_instance = if ready_occupied == 1 {
            let ready_cap = syscall2(
                SYS_CAP_DESCRIBE,
                SLOT_MANAGER_READY,
                ready_descriptor.as_mut_ptr() as u64,
            );
            if ready_cap != 0
                || ready_descriptor[0] != 3
                || (ready_descriptor[2] != RIGHTS_WRITE && ready_descriptor[2] != RIGHTS_READ)
            {
                fail(EXIT_SEED, "rngd manager-ready capability layout is invalid");
            }
            ready_descriptor[2] == RIGHTS_WRITE
        } else {
            false
        };
        let seed_cap = syscall6(SYS_CAP_OCCUPIED, SLOT_KERNEL_SEED, 0, 0, 0, 0, 0);
        if production_instance && seed_cap != 1 {
            fail(EXIT_SEED, "production rngd lacks its exact kernel seed cap");
        }
        if !production_instance && seed_cap == 1 {
            fail(
                EXIT_SEED,
                "test rngd received an unexpected kernel seed cap",
            );
        }
        if production_instance {
            let written = drv.fill(phys[1], 32).unwrap_or_else(|_| {
                fail(
                    EXIT_SEED,
                    "the initial 32-byte seed request did not complete",
                )
            });
            if written != 32 {
                fail(EXIT_SEED, "virtio-rng returned a short kernel seed");
            }
            let status = syscall6(SYS_ENTROPY_SEED, SLOT_KERNEL_SEED, va[1], 0, 0, 0, 0);
            if status != 0 {
                log_line(|o| {
                    o.str("rngd: kernel entropy seed refused with status ");
                    o.i64(status);
                });
                fail(EXIT_SEED, "the kernel did not accept the virtio-rng seed");
            }
            core::ptr::write_bytes(va[1] as *mut u8, 0, 32);
            log("rngd: submitted a device-filled 256-bit seed to the kernel CSPRNG");
        } else if seed_cap < 0 {
            fail(
                EXIT_SEED,
                "checking the optional kernel seed capability failed",
            );
        }
        // Production manager readiness is announced only after both the
        // device DRIVER_OK state and the kernel seed boundary succeeded.
        let _ = syscall2(SYS_NOTIFY, SLOT_MANAGER_READY, MGR_BADGE_RNGD_READY);
        let mut fault_next_get = false;
        let mut stall_next_get = false;

        // 4. The service loop: recv → zero-copy fill → interrupt →
        //    reply. One request in flight at a time (synchronous IPC).
        let mut msg = [0u64; 3];
        loop {
            let r = syscall3(SYS_IPC_RECV, SLOT_EP, msg.as_mut_ptr() as u64, 0);
            if r < 0 {
                log_line(|o| {
                    o.str("rngd: recv returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "SYS_IPC_RECV refused");
            }
            let (w0, op, landed) = (msg[0], msg[1], msg[2]);
            match op {
                RNG_OP_SHUTDOWN => {
                    if !take_diagnostic(landed, SLOT_DIAG) {
                        reply_err(RNG_S_BAD_OP);
                        continue;
                    }
                    // The poison request: reply FIRST (the caller
                    // blocks), then exit — a process is never
                    // destroyed with parked threads.
                    log_line(|o| {
                        o.str("rngd: shutdown requested after ");
                        o.u64(drv.completions);
                        o.str(" interrupt-delivered completion(s) — replying and exiting");
                    });
                    let rr = syscall5(
                        SYS_IPC_REPLY,
                        SLOT_EP,
                        RNG_S_OK,
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
                RNG_OP_FAULT_NEXT_GET => {
                    if !take_diagnostic(landed, SLOT_DIAG) {
                        reply_err(RNG_S_BAD_OP);
                        continue;
                    }
                    fault_next_get = true;
                    reply_ok(0);
                    log("rngd: opt-in next real GET will fault in ring 3");
                }
                RNG_OP_STALL_NEXT_GET => {
                    if !take_diagnostic(landed, SLOT_DIAG) {
                        reply_err(RNG_S_BAD_OP);
                        continue;
                    }
                    stall_next_get = true;
                    reply_ok(0);
                    log("rngd: opt-in next real GET will stall without device completion");
                }
                RNG_OP_GET => {
                    if stall_next_get {
                        log(
                            "rngd: intentionally withholding GET completion in destructive test VM",
                        );
                        loop {
                            let _ = syscall1(SYS_WAIT, SLOT_NOTIF);
                        }
                    }
                    if fault_next_get {
                        log("rngd: injecting genuine #UD with probe GET in flight");
                        core::arch::asm!("ud2", options(noreturn));
                    }
                    if landed == CAP_NONE {
                        reply_err(RNG_S_NO_BUF);
                        continue;
                    }
                    if !(1..=RNG_DRAW_MAX).contains(&w0) {
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                        reply_err(RNG_S_BAD_LEN);
                        continue;
                    }
                    // The zero-copy seam: the frame's phys comes from
                    // the LENT cap — never from a message word.
                    let bp = syscall1(SYS_CAP_PHYS, landed);
                    if bp <= 0 {
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                        log_line(|o| {
                            o.str("rngd: cap_phys on landed slot ");
                            o.u64(landed);
                            o.str(" returned ");
                            o.i64(bp);
                        });
                        fail(EXIT_PHYS, "SYS_CAP_PHYS on the landed buffer refused");
                    }
                    match drv.fill(bp as u64, w0 as u32) {
                        Ok(written) => {
                            // The draw is done: discard the lent
                            // reference (frees nothing — the caller's
                            // owned cap and its mapping stay alive)
                            // and answer with the DEVICE's own count.
                            let dr = syscall1(SYS_CAP_DESTROY, landed);
                            if dr < 0 {
                                log_line(|o| {
                                    o.str("rngd: WARNING — landed cap destroy returned ");
                                    o.i64(dr);
                                });
                            }
                            reply_ok(u64::from(written));
                            log_line(|o| {
                                o.str("rngd: GET ");
                                o.u64(w0);
                                o.str(" bytes buf ");
                                o.hex(bp as u64);
                                o.str(" → device wrote ");
                                o.u64(u64::from(written));
                                o.str(" (completion #");
                                o.u64(drv.completions);
                                o.str(")");
                            });
                        }
                        Err(()) => {
                            let _ = syscall1(SYS_CAP_DESTROY, landed);
                            fail(
                                EXIT_COMPLETE,
                                "the fill completion was missing or malformed",
                            );
                        }
                    }
                }
                _ => {
                    if landed != CAP_NONE {
                        let _ = syscall1(SYS_CAP_DESTROY, landed);
                    }
                    reply_err(RNG_S_BAD_OP);
                }
            }
        }
    }
}

/// Reply word 0 = RNG_S_OK with a payload word.
fn reply_ok(w1: u64) {
    // SAFETY: ABI v1 wrapper; the endpoint cap is the granted slot.
    let rr = unsafe { syscall5(SYS_IPC_REPLY, SLOT_EP, RNG_S_OK, w1, CAP_NONE, 0) };
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
