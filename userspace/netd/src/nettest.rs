//! `nettest` — the network-service client (M6.1, ADR-0024). Spawn-
//! registry image 7, spawned ONLY by the m6 suite's net_service test
//! with one kernel-literal grant:
//!
//! - slot 0: `Endpoint` (WRITE — the call side of netd's service).
//!
//! The whole point of this program is the LINK PROOF: it is a different
//! process from the driver, it never sees a device register, and its
//! transmit frame is never copied by the kernel — it allocates one
//! frame (SYS_ALLOC_FRAME), copies its cap (SYS_CAP_COPY: the copy is
//! LENT, the original stays owning), self-maps the ORIGINAL, and hands
//! the COPY to netd with the call. The device DMAs this process's own
//! page onto the wire.
//!
//! The probe is one ARP round trip against QEMU's slirp gateway — the
//! smallest exchange that proves a NIC genuinely transmits AND
//! receives (ADR-0024):
//!
//!   1. NET_MAC: fetch the device's config-space MAC through the
//!      service (proves the device-config read path),
//!   2. hand-build the 42-byte Ethernet/ARP frame "who-has 10.0.2.2
//!      tell 10.0.2.15" byte-for-byte at wire offsets,
//!   3. NET_SEND: the frame leaves through the transmit queue (zero
//!      copy — chained behind netd's virtio header),
//!   4. NET_RECV: slirp's reply arrives on the receive queue (an MSI-X
//!      interrupt, never a poll) and returns in the reply's inline
//!      message,
//!   5. verify the reply's protocol fields at their exact offsets:
//!      ethertype ARP, opcode reply, sender IP 10.0.2.2, target MAC
//!      == our MAC, and the ARP sender MAC == the reply's Ethernet
//!      source,
//!   6. poison the driver (NET_SHUTDOWN) so it exits cleanly, exit 42.
//!
//! Exit codes (the m6 contract): 42 verified, 43 alloc/copy/map
//! refused, 44 the NET_MAC call refused, 45 the MAC reply invalid,
//! 46 the NET_SEND call refused, 47 SEND error status, 48 the
//! NET_RECV call refused, 49 RECV error status, 50 the ARP reply's
//! protocol fields MISMATCHED, 51 the poison call refused, 97 console
//! refused, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

/// The grant layout (m6.rs net_service): slot 0 = endpoint call side.
const SLOT_EP: u64 = 0;

/// How long to wait for slirp's ARP reply before giving up. Generous
/// by four orders of magnitude — the wire answers in microseconds.
const RECV_TIMEOUT_US: u64 = 2_000_000;
/// Slot for the owned TX frame cap (consumed by the self-map).
const SLOT_BUF: u64 = 1;
/// Slot for the LENT copy that travels with the SEND call.
const SLOT_BUF_LENT: u64 = 2;

const EXIT_SETUP: u64 = 43;
const EXIT_MAC_CALL: u64 = 44;
const EXIT_MAC_REPLY: u64 = 45;
const EXIT_SEND_CALL: u64 = 46;
const EXIT_SEND_STATUS: u64 = 47;
const EXIT_RECV_CALL: u64 = 48;
const EXIT_RECV_STATUS: u64 = 49;
const EXIT_MISMATCH: u64 = 50;
const EXIT_POISON: u64 = 51;

/// QEMU slirp's built-in addresses (tools/arena_env.py mirrors them):
/// the gateway answers ARP; the guest address is the conventional
/// slirp lease — as the ARP request's sender field it needs no stack.
const SLIRP_GATEWAY: [u8; 4] = [10, 0, 2, 2];
const SLIRP_GUEST: [u8; 4] = [10, 0, 2, 15];

/// The ARP request nettest hand-builds: 42 bytes on the wire.
const ARP_FRAME_LEN: u64 = 42;

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("nettest: FAIL: ");
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
    write_str("nettest: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

/// One synchronous request through the service boundary. `msg_out` is
/// the inline-message buffer (RECV's frame lands there) or 0. Returns
/// (reply status word, reply payload word); a refused CALL fails with
/// `call_fail`.
fn request(op: u64, w0: u64, send_slot: u64, msg_out: u64, call_fail: u64) -> (u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract; `reply` is on this thread's own
    // (registered) stack; the endpoint cap is the granted slot 0.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_EP,
            w0,
            op,
            send_slot,
            reply.as_mut_ptr() as u64,
            msg_out,
        )
    };
    if r < 0 {
        log_line(|o| {
            o.str("nettest: call(op ");
            o.u64(op);
            o.str(") returned ");
            o.i64(r);
        });
        fail(call_fail, "the service call was refused");
    }
    (reply[0], reply[1])
}

/// The client entry: the spawn protocol's first thread lands here at
/// ring 3 with slot 0 holding the endpoint's call side.
///
/// # Safety
/// As the driver's `_start`: ring 3, derived stack top, kernel-loaded
/// address space, one grant in slot 0. The body is ABI v1 wrappers over
/// this image's own statics, stack, and allocated window.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics, stack, and granted/allocated windows.
    // Single-threaded, no aliases.
    unsafe {
        log("nettest: network-service client starting (ARP link probe against the slirp gateway)");

        // 1. The TX frame: one owned frame; the cap is COPIED before
        //    the map consumes the original, so the copy can travel
        //    with the call while this process keeps its window
        //    (ADR-0022's lend-keep pattern).
        let phys = syscall1(SYS_ALLOC_FRAME, SLOT_BUF);
        if phys <= 0 {
            fail(EXIT_SETUP, "TX frame allocation refused");
        }
        if syscall3(SYS_CAP_COPY, SLOT_BUF, SLOT_BUF_LENT, RIGHTS_ALL) < 0 {
            fail(EXIT_SETUP, "the TX cap copy refused");
        }
        let win = syscall2(SYS_MAP_MEMORY, SLOT_BUF, 1);
        if win <= 0 {
            fail(EXIT_SETUP, "the TX self-map refused");
        }
        let tx = win as u64;

        // 2. The MAC through the service — the device-config read path
        //    proven across the boundary (no cap rides this call).
        let (st, packed) = request(NET_OP_MAC, 0, CAP_NONE, 0, EXIT_MAC_CALL);
        if st != NET_S_OK {
            log_line(|o| {
                o.str("nettest: NET_MAC status ");
                o.i64(st as i64);
            });
            fail(EXIT_MAC_REPLY, "the MAC reply carried an error status");
        }
        let mut mac = [0u8; 6];
        for (i, m) in mac.iter_mut().enumerate() {
            *m = ((packed >> (8 * i)) & 0xFF) as u8;
        }
        if mac.iter().all(|&b| b == 0) {
            fail(EXIT_MAC_REPLY, "the service returned an all-zero MAC");
        }
        log_line(|o| {
            o.str("nettest: device MAC ");
            for (i, b) in mac.iter().enumerate() {
                if i > 0 {
                    o.str(":");
                }
                o.hex2(*b);
            }
            o.str(" (from the device-config region, through the service)");
        });

        // 3. Hand-build the ARP request at exact wire offsets —
        //    Ethernet big-endian, ARP over IPv4, "who-has 10.0.2.2
        //    tell 10.0.2.15".
        let put = |off: u64, b: u8| {
            *((tx + off) as *mut u8) = b;
        };
        for i in 0..6u64 {
            put(i, 0xFF); // broadcast destination
            put(32 + i, 0); // target MAC: unknown, that's the question
        }
        for (i, b) in mac.iter().enumerate() {
            put(6 + i as u64, *b); // our MAC as the source
        }
        put(12, 0x08);
        put(13, 0x06); // ethertype: ARP
        put(14, 0x00);
        put(15, 0x01); // htype: Ethernet
        put(16, 0x08);
        put(17, 0x00); // ptype: IPv4
        put(18, 6); // hardware address length
        put(19, 4); // protocol address length
        put(20, 0x00);
        put(21, 0x01); // op: request
        for (i, b) in mac.iter().enumerate() {
            put(22 + i as u64, *b); // sender MAC: us
        }
        for i in 0..4usize {
            put(28 + i as u64, SLIRP_GUEST[i]); // sender IP
            put(38 + i as u64, SLIRP_GATEWAY[i]); // target IP: the gateway
        }

        // 4. SEND: the frame leaves through the transmit queue — the
        //    device DMAs THIS process's page, chained behind netd's
        //    virtio header (zero copy).
        let (st, sent) = request(NET_OP_SEND, ARP_FRAME_LEN, SLOT_BUF_LENT, 0, EXIT_SEND_CALL);
        if st != NET_S_OK {
            log_line(|o| {
                o.str("nettest: NET_SEND status ");
                o.i64(st as i64);
            });
            fail(EXIT_SEND_STATUS, "the driver reported a SEND error");
        }
        log_line(|o| {
            o.str("nettest: SEND ");
            o.u64(sent);
            o.str(" bytes — ARP who-has 10.0.2.2 tell 10.0.2.15 is on the wire");
        });

        // 5. RECV: slirp's reply arrives on the receive queue (an
        //    interrupt, never a poll) and lands in the reply's inline
        //    message.
        let mut frame = [0u64; MSG_BYTES / 8];
        // A DEADLINE, not "forever" (M7.1, ADR-0030). slirp answers an
        // ARP request in microseconds, so two seconds is enormous
        // slack; the point is that a lost reply now ends this test
        // with a typed timeout instead of parking the boot.
        let (st, flen) = request(
            NET_OP_RECV,
            RECV_TIMEOUT_US,
            CAP_NONE,
            frame.as_mut_ptr() as u64,
            EXIT_RECV_CALL,
        );
        if st != NET_S_OK {
            log_line(|o| {
                o.str("nettest: NET_RECV status ");
                o.i64(st as i64);
            });
            fail(EXIT_RECV_STATUS, "the driver reported a RECV error");
        }
        let f = frame.as_ptr() as *const u8;
        // SAFETY: `f` is this thread's own stack buffer (the enclosing
        // unsafe block covers the closure body); the offsets are
        // bounded by the checks around every use.
        let get = |off: u64| -> u8 { *f.add(off as usize) };
        if flen < ARP_FRAME_LEN || flen > MSG_BYTES as u64 {
            log_line(|o| {
                o.str("nettest: reply frame length ");
                o.u64(flen);
                o.str(" is outside 42..64");
            });
            fail(EXIT_MISMATCH, "the reply frame has an impossible length");
        }

        // 6. Verify the protocol fields at their exact offsets — every
        //    one a byte-for-byte claim about what came back off the
        //    wire (slirp may pad past 42; offsets are what matter).
        let mut mismatch: Option<&'static str> = None;
        if get(12) != 0x08 || get(13) != 0x06 {
            mismatch = Some("ethertype is not ARP (0x0806)");
        } else if get(20) != 0x00 || get(21) != 0x02 {
            mismatch = Some("ARP opcode is not reply (2)");
        } else if (0..4usize).any(|i| get(28 + i as u64) != SLIRP_GATEWAY[i]) {
            mismatch = Some("the ARP sender IP is not the slirp gateway 10.0.2.2");
        } else if (0..6usize).any(|i| get(32 + i as u64) != mac[i]) {
            mismatch = Some("the ARP target MAC is not ours — the reply is not for this probe");
        } else if (0..6u64).any(|i| get(22 + i) != get(6 + i)) {
            mismatch = Some("the ARP sender MAC disagrees with the Ethernet source");
        }
        if let Some(what) = mismatch {
            // Compact hex2 dump — 42 bytes stay inside WRITE_MAX=256
            // (the full-width hex form would silently truncate).
            log_line(|o| {
                o.str("nettest: reply frame (");
                o.u64(flen);
                o.str(" bytes): ");
                for i in 0..ARP_FRAME_LEN {
                    o.hex2(get(i));
                    o.str(" ");
                }
            });
            fail(EXIT_MISMATCH, what);
        }
        log_line(|o| {
            o.str("nettest: RECV ");
            o.u64(flen);
            o.str(" bytes — ARP reply verified: ethertype 0x0806, opcode 2, sender 10.0.2.2, target MAC ours, sender MAC == Ethernet source");
        });

        // 7. Poison: the driver replies, then exits cleanly by its own
        //    hand (a service is never destroyed with parked threads).
        let (st, completions) = request(NET_OP_SHUTDOWN, 0, CAP_NONE, 0, EXIT_POISON);
        if st != NET_S_OK {
            fail(EXIT_POISON, "the shutdown reply carried an error status");
        }
        log_line(|o| {
            o.str("nettest: PASS — the ARP round trip completed (netd counted ");
            o.u64(completions);
            o.str(" interrupt-delivered completion(s)) and the service shut down cleanly");
        });
        // SAFETY: thread_exit diverges; 42 is the verified-success code.
        syscall1(SYS_THREAD_EXIT, EXIT_OK);
        #[allow(unreachable_code)]
        loop {
            core::hint::spin_loop();
        }
    }
}
