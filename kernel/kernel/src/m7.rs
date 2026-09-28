//! Milestone 7 test suite — Phase 7 opens with the facility every
//! protocol in it will depend on (step 7.0, ADR-0029).
//!
//! Runs in `kmain` after the m6 RESULT line; suite discipline as
//! m3/m4/m5/m6 (self-contained, ring-3 claims proven by real images,
//! kernel claims proven by counted state):
//!
//! 1. `timer_facility` — REAL timers, measured. The kernel spawns
//!    `timertest` (registry image 16) with one notification, and the
//!    image arms timers against it and times them with the monotonic
//!    clock `SYS_CLOCK_NOW` returns: a 50 ms deadline must not deliver
//!    early (the one property a timeout must have — a timer that can
//!    fire early makes every retransmission rule built on it wrong),
//!    must not be much late, a CANCELLED timer must stay silent, a
//!    second cancel of the same timer must be refused, and two due
//!    timers on one notification must both deliver so a service can
//!    wait for "work OR timeout" in a single blocking call.
//!
//!    The kernel then proves its own side: the facility counted the
//!    arms, fires and cancels the client claims, and a timer the
//!    client deliberately left armed is SWEPT when the process is
//!    destroyed — a dead process must not be able to keep signalling.
//!    Frame-exact teardown, as always.
//!
//! This suite needs no fixture and no host actor: it is about the
//! kernel's own clock, so it runs and must pass on the barest machine.
//!
//! Markers: `m7:test:<name>`, `m7: RESULT`.

use crate::arch::x86_64::{paging, syscall};
use crate::cap::{Cap, CapObj};
use crate::drivers::intc;
use crate::frames;
use crate::ipc;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::proc;
use crate::relay;
use crate::sched;
use crate::{cap, timer};

type Res = Result<(), &'static str>;

/// The ARP test needs the slirp NIC. A machine without one is not a
/// failing machine (ADR-0024's rule, kept), so the suite reports the
/// distinction rather than flattening it — the marker says SKIP and
/// the count says so too.
const NO_NIC: &str = "SKIP:no virtio-net device — attach it with: -netdev user,id=net0 -device virtio-net-pci,netdev=net0";

/// timertest's verified-success exit.
const TIMERTEST_EXIT_OK: u64 = 42;
const TIMERTEST_EXIT_BADGE: u64 = 0x0070_0001;

/// How long the suite waits for the client to finish its measurements
/// (it sleeps ~200 ms in total across five timers), in HPET ticks at
/// QEMU's 100 MHz: ~5 s, generous against a loaded host and still far
/// inside the boot timeout.
const CLIENT_DEADLINE_TICKS: u32 = 500_000_000;

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Res); 2] = [
        ("timer_facility", test_timer_facility),
        ("arp_service", test_arp_service),
    ];
    let mut passed = 0u32;
    let mut skipped = 0u32;
    for (name, test) in checks {
        match test() {
            Ok(()) => {
                passed += 1;
                write_marker(format_args!("m7:test:{name}: PASS"));
            }
            Err(reason) if reason.starts_with("SKIP:") => {
                skipped += 1;
                let why = &reason[5..];
                info!("m7", "test {name} skipped: {why}");
                write_marker(format_args!("m7:test:{name}: SKIP ({why})"));
            }
            Err(reason) => {
                error!("m7", "test {name} failed: {reason}");
                write_marker(format_args!("m7:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = checks.len() as u32;
    if passed + skipped == total {
        if skipped > 0 {
            write_marker(format_args!(
                "m7: RESULT SKIP ({passed}/{total} passed, {skipped} skipped — nothing to prove them against; see the skip reasons above)"
            ));
        } else {
            write_marker(format_args!("m7: RESULT PASS ({passed}/{total})"));
        }
        true
    } else {
        write_marker(format_args!("m7: RESULT FAIL ({passed}/{total})"));
        false
    }
}

/// netstackd's and arptest's exit badges.
const NETSTACKD_EXIT_BADGE: u64 = 0x0071_0001;
const ARPTEST_EXIT_BADGE: u64 = 0x0071_0002;
const ARPTEST_EXIT_OK: u64 = 42;

/// M7.1 (ADR-0030): the first protocol, and the boundary it sits on.
///
/// The kernel spawns `netd` (the L2 driver, unchanged) and
/// `netstackd` (image 17, which owns ARP and its cache), wires the
/// stack to the driver as a CLIENT, and lets `arptest` (image 18) ask
/// for a real address. What is being tested is as much the SPLIT as
/// the protocol: netd never learns what an ARP packet is, and the
/// stack never touches a virtqueue.
fn test_arp_service() -> Res {
    let baseline = frames::free_frames();

    let Some(v) = crate::drivers::pci::find_virtio(crate::drivers::pci::VIRTIO_TYPE_NET) else {
        return Err(NO_NIC);
    };
    let Some(f) = crate::drivers::pci::pci_function(v.pci_index) else {
        return Err("recorded function vanished from the table");
    };
    let bar = v.common.bar as usize;
    let bar_phys = f.bar_base[bar];
    let bar_pages = (f.bar_size[bar] / 4096) as u32;
    if bar_phys == 0 || bar_pages == 0 || bar_pages > 16 {
        return Err("the structure BAR is unusable for a window grant");
    }

    // Three parties, two endpoints: arptest → netstackd → netd.
    let ep_netd = ipc::create_endpoint().map_err(|_| "endpoint table full")?;
    let ep_stack = ipc::create_endpoint().map_err(|_| "endpoint table full")?;
    let nid_irq = ipc::create_notification().map_err(|_| "notification table full")?;
    let nid_stack = ipc::create_notification().map_err(|_| "notification table full")?;
    let nid_client = ipc::create_notification().map_err(|_| "notification table full")?;
    let nid_backoff = ipc::create_notification().map_err(|_| "notification table full")?;
    let nid_sync = ipc::create_notification().map_err(|_| "notification table full")?;
    let nid_go = ipc::create_notification().map_err(|_| "notification table full")?;

    let netd_grants = [
        Cap {
            obj: CapObj::Mmio {
                phys: bar_phys,
                pages: bar_pages,
            },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
        Cap {
            obj: CapObj::Endpoint { eid: ep_netd },
            rights: cap::RIGHTS_READ,
        },
        Cap {
            obj: CapObj::Notification { nid: nid_irq },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
    ];
    let netd_pid = crate::spawn::spawn_init(6, &netd_grants, None)
        .map_err(|_| "netd (image 6) spawn failed")?;
    // M7.1b: the driver runs under real supervision, so killing it is
    // survivable rather than terminal. The grant list is what gets
    // replayed — including the SAME endpoint, which is why the stack's
    // capability keeps working across the restart (ADR-0028).
    crate::supervise::register("netd", 6, &netd_grants, netd_pid)
        .map_err(|_| "the supervisor refused netd's registration")?;

    // netstackd: the driver's call side, its own serve side, and a
    // frame slot. NO device capability — it cannot touch the NIC even
    // if it wanted to, which is the boundary made structural.
    let stack_grants = [
        Cap {
            obj: CapObj::Endpoint { eid: ep_netd },
            rights: cap::RIGHTS_WRITE,
        },
        Cap {
            obj: CapObj::Endpoint { eid: ep_stack },
            rights: cap::RIGHTS_READ,
        },
        // A notification it arms backoff timers on while waiting for a
        // dead driver to be restarted (M7.1b). Still NO device
        // capability — the boundary is unchanged. (Padding the list
        // with empty caps to place this higher does not work: granting
        // a `CapObj::None` is refused, which is the right answer to a
        // meaningless grant.)
        Cap {
            obj: CapObj::Notification { nid: nid_backoff },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
    ];
    let stack_pid =
        crate::spawn::spawn_init(17, &stack_grants, Some((nid_stack, NETSTACKD_EXIT_BADGE)))
            .map_err(|_| "netstackd (image 17) spawn failed")?;

    let client_grants = [
        Cap {
            obj: CapObj::Endpoint { eid: ep_stack },
            rights: cap::RIGHTS_WRITE,
        },
        // The handshake: the client tells us when to kill the driver
        // and waits for us to say it is back. Deterministic by
        // agreement rather than by sleep (the faultd pattern, M6.5).
        //
        // TWO notifications with one-way rights. Sharing one let the
        // client's own wait swallow the badge it had just sent — the
        // same bug, in the same shape, as faultd's first version.
        Cap {
            obj: CapObj::Notification { nid: nid_sync },
            rights: cap::RIGHTS_WRITE,
        },
        Cap {
            obj: CapObj::Notification { nid: nid_go },
            rights: cap::RIGHTS_READ,
        },
    ];
    let client_pid =
        crate::spawn::spawn_init(18, &client_grants, Some((nid_client, ARPTEST_EXIT_BADGE)))
            .map_err(|_| "arptest (image 18) spawn failed")?;
    info!(
        "m7",
        "arp_service: netd pid {netd_pid} (L2 only), netstackd pid {stack_pid} (ARP + cache, NO device cap), arptest pid {client_pid} — resolving on the real wire"
    );

    // ---- the fault, at a moment both sides agreed on ----------------
    // The client signals when it has finished phase 1; no sleeps, no
    // guessing which instruction it is on.
    //
    // POLLED, not waited on. The boot thread must stay runnable while
    // its children are blocked on devices: `block_current` treats an
    // empty ready ring as a deadlock and halts, which is what the
    // first version of this handshake did. Other tests can call
    // `ipc::wait` only because they do it after a drain, when the
    // badge is already pending.
    if let Err(reason) = await_badge(nid_sync, 1 << 16) {
        return Err(reason);
    }

    // Count the timers armed so far. netd arms one per bounded RECV,
    // and it is about to die; after that the ONLY thing in the system
    // that arms a timer is netstackd's re-attach backoff. So a rise in
    // this counter is the kernel's own evidence that the stack met a
    // dead driver and started coping — observable without asking the
    // service anything.
    let timers_before = timer::stats().armed_total;
    proc::destroy(netd_pid).map_err(|_| "destroying netd failed")?;

    // Release the client IMMEDIATELY, while the driver is still dead.
    //
    // The first version restarted netd first and then let the client
    // go, which proved something real (a client's capability survives
    // a restart) but not the thing this milestone is about: netstackd
    // never met a dead driver, so its re-establish path never ran and
    // it reported zero re-attaches. Now the stack makes its next call
    // into a corpse, gets STATUS_SERVICE_GONE, and has to cope.
    ipc::notify(nid_go, 1 << 17).map_err(|_| "the go-ahead could not be sent")?;

    // Do NOT restart it yet. Wait until the stack has actually met the
    // corpse — proven by it arming a backoff timer — and only then let
    // the supervisor work.
    //
    // Without this the proof was hollow: the supervisor's first poll
    // ran before the client had even woken, so netstackd called into a
    // driver that was already back and reported zero re-attaches. The
    // milestone is about surviving a dead dependency, so the test has
    // to actually produce one.
    if let Err(reason) = await_timer_armed(timers_before) {
        return Err(reason);
    }
    info!(
        "m7",
        "arp_service: the stack has met the dead driver and is backing off — releasing the supervisor now"
    );

    // The supervisor's hands, on the boot thread. In production this
    // is the idle loop (M7.0) running continuously beside live
    // services.
    if let Err(reason) = drain_pid_supervised(client_pid) {
        error!(
            "m7",
            "arp_service: client threads={}, stack threads={}, netd threads={}",
            sched::proc_live_threads(client_pid),
            sched::proc_live_threads(stack_pid),
            sched::proc_live_threads(netd_pid)
        );
        return Err(reason);
    }

    let Some(st) = crate::supervise::status_of("netd") else {
        return Err("netd vanished from the supervisor's table");
    };
    if st.pid == 0 || st.pid == netd_pid {
        return Err("the restarted driver has no new pid");
    }
    if st.restarts != 1 {
        return Err("the supervisor's restart accounting is wrong");
    }
    let netd_pid2 = st.pid;
    info!(
        "m7",
        "arp_service: netd died as pid {netd_pid} and came back as pid {netd_pid2} with its capabilities replayed — the stack met the corpse, re-established, and carried on"
    );

    let bc = ipc::wait(nid_client).map_err(|_| "the client's exit badge never arrived")?;
    if bc != ARPTEST_EXIT_BADGE {
        return Err("the client's exit badge is not the granted word");
    }
    let recs = crate::spawn::records_snapshot();
    let Some(tid) = recs
        .iter()
        .flatten()
        .find(|&&(p, _)| p == client_pid)
        .map(|&(_, t)| t)
    else {
        return Err("the client has no spawn record");
    };
    match syscall::exit_status_of(tid) {
        Some(ARPTEST_EXIT_OK) => {}
        Some(60) => return Err("client: a call to the stack was refused (60)"),
        Some(61) => return Err("client: the gateway did not resolve (61)"),
        Some(62) => return Err("client: the stack returned an all-zero MAC (62)"),
        Some(63) => return Err("client: the cache returned a different MAC (63)"),
        Some(64) => return Err("client: a cached answer still put a request on the wire (64)"),
        Some(65) => {
            return Err("client: a silent address did not come back unreachable (65)");
        }
        Some(66) => return Err("client: the stack would not report its counters (66)"),
        Some(67) => {
            return Err(
                "client: the stack could not resolve after the driver was RESTARTED under it (67)",
            );
        }
        Some(68) => return Err("client: the gateway did not answer an ICMP echo (68)"),
        Some(69) => return Err("client: the reported round-trip time is not plausible (69)"),
        Some(70) => {
            return Err(
                "client: a ping to an unresolvable address did not fail at the ARP layer (70)",
            );
        }
        Some(71) => {
            return Err("client: the demultiplexer did not see both protocols (71)");
        }
        Some(99) => return Err("client: the panic handler ran (99)"),
        _ => return Err("the client exited with a code from nowhere in the contract"),
    }

    // The machine's own witness: frames really left and really
    // arrived. Without this the client's story could be a lookup
    // table with good manners.
    let tx = relay::delivery_count(49);
    let rx = relay::delivery_count(48);
    if tx == 0 || rx == 0 {
        error!("m7", "arp_service: relay deliveries rx={rx} tx={tx}");
        return Err("the ARP exchange did not go through real interrupts");
    }

    // Teardown. netd's FIRST incarnation was reaped by the supervisor
    // when it respawned (ADR-0025's GC debt, closed in ADR-0028), so
    // only the live one is destroyed here.
    crate::supervise::unregister(netd_pid2);
    proc::destroy(netd_pid2).map_err(|_| "destroying the restarted netd failed")?;
    proc::destroy(stack_pid).map_err(|_| "destroying netstackd failed")?;
    proc::destroy(client_pid).map_err(|_| "destroying arptest failed")?;
    for p in [netd_pid2, stack_pid, client_pid] {
        crate::spawn::forget(p).map_err(|_| "spawn record forget refused")?;
    }
    ipc::destroy_endpoint(ep_netd).map_err(|_| "endpoint teardown refused")?;
    ipc::destroy_endpoint(ep_stack).map_err(|_| "endpoint teardown refused")?;
    for n in [
        nid_irq,
        nid_stack,
        nid_client,
        nid_backoff,
        nid_sync,
        nid_go,
    ] {
        ipc::destroy_notification(n).map_err(|_| "notification teardown refused")?;
    }
    for _ in 0..4 {
        sched::yield_now();
    }
    let after = frames::free_frames();
    if after != baseline {
        error!(
            "m7",
            "arp_service teardown accounting: baseline {baseline}, after {after}"
        );
        return Err("arp-service teardown is not frame-exact");
    }
    info!(
        "m7",
        "arp_service: arptest drove netstackd (pid {stack_pid}) over netd (pid {netd_pid}) through the whole Phase-7-so-far story — ARP resolved on the wire ({tx} transmit and {rx} receive interrupt delivery/deliveries), a cached lookup that touched no wire, a silent address reported UNREACHABLE, the driver KILLED and re-established with, and an ICMP echo answered over IPv4 with both checksums verified and the demultiplexer sorting each frame to its protocol; the driver never parsed a protocol and the stack never touched a virtqueue; teardown frame-exact (frames {after})"
    );
    Ok(())
}

fn test_timer_facility() -> Res {
    let baseline = frames::free_frames();
    let before = timer::stats();
    if before.armed_now != 0 {
        return Err("a timer was already armed before the test (nothing else should be)");
    }

    let nid = ipc::create_notification().map_err(|_| "notification table full")?;
    let nid_exit = ipc::create_notification().map_err(|_| "notification table full")?;

    // One grant: the notification it arms timers against and waits on.
    // READ|WRITE because it is both the notify side (arming) and the
    // wait side — and the WRITE right is exactly what `SYS_TIMER_ARM`
    // checks, so a process can only aim a timer at something it was
    // already trusted to signal. No new capability kind was needed to
    // make timers safe (ADR-0029).
    let grants = [Cap {
        obj: CapObj::Notification { nid },
        rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
    }];
    let pid = crate::spawn::spawn_init(16, &grants, Some((nid_exit, TIMERTEST_EXIT_BADGE)))
        .map_err(|_| "timertest (image 16) spawn failed")?;
    info!(
        "m7",
        "timer_facility: timertest pid {pid} (notification {nid}) — measuring real deadlines against the monotonic clock"
    );

    // The client sleeps on real timers, so this drain is deliberately
    // patient. It is NOT a poll of the client: the boot thread yields,
    // the client is blocked in SYS_WAIT, and the tick is what wakes it.
    if let Err(reason) = drain_pid(pid) {
        error!(
            "m7",
            "timer_facility: client threads={}, timers armed={}",
            sched::proc_live_threads(pid),
            timer::stats().armed_now
        );
        return Err(reason);
    }

    let badge = ipc::wait(nid_exit).map_err(|_| "the client's exit badge never arrived")?;
    if badge != TIMERTEST_EXIT_BADGE {
        return Err("the client's exit badge is not the granted word");
    }
    let recs = crate::spawn::records_snapshot();
    let Some(tid) = recs
        .iter()
        .flatten()
        .find(|&&(p, _)| p == pid)
        .map(|&(_, t)| t)
    else {
        return Err("the client has no spawn record");
    };
    match syscall::exit_status_of(tid) {
        Some(TIMERTEST_EXIT_OK) => {}
        Some(60) => return Err("client: SYS_CLOCK_NOW read as zero or negative (60)"),
        Some(61) => return Err("client: arming a timer or waiting on it was refused (61)"),
        Some(62) => return Err("client: a deadline fired EARLY (62)"),
        Some(63) => return Err("client: a deadline fired far later than the granularity (63)"),
        Some(64) => return Err("client: a timer delivered the wrong badge (64)"),
        Some(65) => return Err("client: cancelling an armed timer was refused (65)"),
        Some(66) => {
            return Err("client: cancelling an already-cancelled timer reported success (66)");
        }
        Some(67) => return Err("client: a CANCELLED timer delivered its badge anyway (67)"),
        Some(68) => return Err("client: two due timers did not both deliver (68)"),
        Some(69) => {
            return Err(
                "client: a STALE timer id was accepted — it could have cancelled a stranger's timer (69)",
            );
        }
        Some(99) => return Err("client: the panic handler ran (99)"),
        _ => return Err("the client exited with a code from nowhere in the contract"),
    }

    // The kernel's own accounting of what the client just did.
    let after = timer::stats();
    if after.armed_total < before.armed_total + 8 {
        error!(
            "m7",
            "timer_facility: armed_total {} → {}, expected at least 8 more",
            before.armed_total,
            after.armed_total
        );
        return Err("the kernel counted fewer arms than the client made");
    }
    if after.fired < before.fired + 6 {
        return Err("the kernel counted fewer firings than the client observed");
    }
    if after.cancelled != before.cancelled + 1 {
        return Err("the kernel did not count exactly one cancellation");
    }
    // The client left one armed ON PURPOSE. It must still be armed —
    // and then swept, not left signalling a dead process's notification.
    if after.armed_now != 1 {
        error!(
            "m7",
            "timer_facility: {} timers armed at exit, expected the 1 the client left",
            after.armed_now
        );
        return Err("the client's deliberately-abandoned timer is not there to sweep");
    }

    proc::destroy(pid).map_err(|_| "destroying the client failed")?;
    let swept = timer::stats();
    if swept.armed_now != 0 || swept.swept != after.swept + 1 {
        error!(
            "m7",
            "timer_facility: after destroy {} armed, swept {} → {}",
            swept.armed_now,
            after.swept,
            swept.swept
        );
        return Err("destroying the owner did not sweep its armed timer");
    }

    crate::spawn::forget(pid).map_err(|_| "client spawn record forget refused")?;
    ipc::destroy_notification(nid).map_err(|_| "notification teardown refused")?;
    ipc::destroy_notification(nid_exit).map_err(|_| "exit notification teardown refused")?;
    for _ in 0..4 {
        sched::yield_now();
    }
    let frames_after = frames::free_frames();
    if frames_after != baseline {
        error!(
            "m7",
            "timer_facility teardown accounting: baseline {baseline}, after {frames_after}"
        );
        return Err("timer-facility teardown is not frame-exact");
    }
    info!(
        "m7",
        "timer_facility: timertest (pid {pid}) armed {} timers and measured them against SYS_CLOCK_NOW — no deadline fired early, none lagged beyond one tick plus slack, a cancelled timer stayed silent, a second cancel was refused, two due timers merged into one wake; the kernel counted {} firing(s) and {} cancellation(s), swept the timer the client abandoned, and teardown is frame-exact (frames {frames_after})",
        after.armed_total - before.armed_total,
        after.fired - before.fired,
        after.cancelled - before.cancelled
    );
    Ok(())
}

/// Yield until something arms a timer beyond `before`, bounded by the
/// HPET wall clock. With netd dead, the only such thing is the stack's
/// re-attach backoff.
fn await_timer_armed(before: u64) -> Res {
    let if_before = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let hpet_va = paging::mmio_alias_va(intc::HPET_PHYS);
    // SAFETY: ring 0; mapped HPET alias; 32-bit main-counter read.
    let t0 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    let mut out = Ok(());
    while timer::stats().armed_total <= before {
        // SAFETY: the same alias and register.
        let now = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
        if now.wrapping_sub(t0) >= CLIENT_DEADLINE_TICKS {
            out = Err("the stack never noticed the driver was gone");
            break;
        }
        sched::yield_now();
    }
    if !if_before {
        crate::arch::x86_64::cli();
    }
    out
}

/// Drain like `drain_pid`, but ALSO run the supervisor each pass.
///
/// This is the idle thread's job (M7.0) done by the suite: in
/// production `supervise::poll` runs continuously beside live
/// services, so a driver that dies is restarted WHILE its clients are
/// retrying. Sequencing the restart before releasing the client would
/// test a much easier world.
fn drain_pid_supervised(pid: u64) -> Res {
    let if_before = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let hpet_va = paging::mmio_alias_va(intc::HPET_PHYS);
    // SAFETY: ring 0; mapped HPET alias; 32-bit main-counter read.
    let t0 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    let mut out = Ok(());
    while sched::proc_live_threads(pid) > 0 {
        crate::supervise::poll();
        // SAFETY: the same alias and register.
        let now = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
        if now.wrapping_sub(t0) >= CLIENT_DEADLINE_TICKS {
            out = Err("the client is still live after the wall-clock deadline");
            break;
        }
        sched::yield_now();
    }
    sched::yield_now();
    if !if_before {
        crate::arch::x86_64::cli();
    }
    out
}

/// Yield until `nid` carries `badge`, bounded by the HPET wall clock.
///
/// See `ipc::poll_pending` for why this cannot simply block.
fn await_badge(nid: u32, badge: u64) -> Res {
    let if_before = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let hpet_va = paging::mmio_alias_va(intc::HPET_PHYS);
    // SAFETY: ring 0; mapped HPET alias; 32-bit main-counter read.
    let t0 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    let mut seen = 0u64;
    let mut out = Ok(());
    loop {
        seen |= ipc::poll_pending(nid);
        if seen & badge != 0 {
            break;
        }
        // SAFETY: the same alias and register.
        let now = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
        if now.wrapping_sub(t0) >= CLIENT_DEADLINE_TICKS {
            out = Err("the expected signal never arrived");
            break;
        }
        sched::yield_now();
    }
    if !if_before {
        crate::arch::x86_64::cli();
    }
    out
}

/// Yield until `pid` has no live threads, bounded by the HPET wall
/// clock. The client spends most of its life BLOCKED on timers, so
/// this is a patient drain, not a poll of anything.
fn drain_pid(pid: u64) -> Res {
    let if_before = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let hpet_va = paging::mmio_alias_va(intc::HPET_PHYS);
    // SAFETY: ring 0; mapped HPET alias; 32-bit main-counter read.
    let t0 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    let mut out = Ok(());
    while sched::proc_live_threads(pid) > 0 {
        // SAFETY: the same alias and register.
        let now = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
        if now.wrapping_sub(t0) >= CLIENT_DEADLINE_TICKS {
            out = Err("the timer client is still live after the wall-clock deadline");
            break;
        }
        sched::yield_now();
    }
    sched::yield_now();
    if !if_before {
        crate::arch::x86_64::cli();
    }
    out
}
