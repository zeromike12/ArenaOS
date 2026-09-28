//! Milestone 6 test suite (step 6.1 — the virtio-net link proof;
//! ADR-0024).
//! Runs in `kmain` after the m5 RESULT line; suite discipline as m3/m4/m5
//! (self-contained, no imports from the other suites, ring-3 claims proven
//! by real images, kernel claims proven by counted relay/frame state):
//!
//! 1. `net_service` — the REAL network boundary. The kernel spawns `netd`
//!    (registry image 6, the userspace virtio-net driver) with the same
//!    grant shape storaged got — an Mmio cap over the structure BAR, an
//!    endpoint's serve side, and one interrupt notification carrying BOTH
//!    MSI-X relay badges (RX arrival, TX completion) — then `nettest`
//!    (image 7) with the endpoint's call side. The client fetches the
//!    device MAC through the service, hand-builds a 42-byte ARP request
//!    for the slirp gateway (10.0.2.2), sends it (the device DMAs the
//!    CLIENT's own frame, chained behind netd's virtio header — zero
//!    copy), and receives the reply into its second frame: ethertype,
//!    opcode, sender IP, and target MAC verified byte-for-byte at their
//!    protocol offsets. Both children then shut the service down by its
//!    own hand. The kernel proves the machine side: both exit badges,
//!    both exit codes (42 = the client verified the reply), exactly ONE
//!    relay delivery per vector (RX on the first, TX on the second — no
//!    polling anywhere), the dead driver's relays swept by
//!    `proc::destroy`, and frame-exact teardown.
//!    ABSENT fixture: bus 0 without a virtio-net function is an honest
//!    SKIP (pre-v0.6.0 QEMU invocations stay bootable-green) — never a
//!    FAIL, never a fake PASS.
//!
//! Markers: `m6:test:<name>`, `m6: RESULT`.

use crate::arch::x86_64::{paging, syscall};
use crate::cap::{self, Cap, CapObj};
use crate::drivers::{intc, pci};
use crate::frames;
use crate::ipc;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::proc;
use crate::relay;
use crate::sched;

/// A test outcome: the net fixture is OPTIONAL until Phase 7 (ADR-0024),
/// so the suite distinguishes "ran and proved it" from "nothing to prove
/// it against" — the SKIP is printed loudly and never counted as PASS.
enum Outcome {
    Pass,
    Skip(&'static str),
    Fail(&'static str),
}

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Outcome); 4] = [
        ("net_service", test_net_service),
        ("rng_service", test_rng_service),
        ("input_service", test_input_service),
        ("console_service", test_console_service),
    ];
    let mut passed = 0u32;
    let mut skipped = 0u32;
    for (name, test) in checks {
        match test() {
            Outcome::Pass => {
                passed += 1;
                write_marker(format_args!("m6:test:{name}: PASS"));
            }
            Outcome::Skip(reason) => {
                skipped += 1;
                info!("m6", "test {name} skipped: {reason}");
                write_marker(format_args!("m6:test:{name}: SKIP ({reason})"));
            }
            Outcome::Fail(reason) => {
                error!("m6", "test {name} failed: {reason}");
                write_marker(format_args!("m6:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = checks.len() as u32;
    if passed + skipped == total {
        if skipped > 0 {
            write_marker(format_args!(
                "m6: RESULT SKIP ({passed}/{total} passed, {skipped} skipped — nothing to prove them against; see the skip reasons above)"
            ));
        } else {
            write_marker(format_args!("m6: RESULT PASS ({passed}/{total})"));
        }
        true
    } else {
        write_marker(format_args!("m6: RESULT FAIL ({passed}/{total})"));
        false
    }
}

// ---- the yield-drain (self-contained; suites do not share helpers) ----------

/// The wall-clock bound for this suite's device-bound drain, in HPET
/// main-counter ticks — the same discipline (and the same 1-in-100
/// host-load lesson) as the m5 suite's `DRAIN_DEADLINE_TICKS`,
/// re-derived here because the suites are deliberately independent:
/// ~21 s at QEMU's ~14.4 MHz HPET, orders of magnitude beyond a
/// healthy ARP round trip and inside every harness boot timeout. A
/// yield COUNT is the wrong unit when the awaited event (an MSI, a
/// slirp reply) is host-coupled. The counter is provably running: the
/// m5 suite's `test_mmio_user` asserted it earlier in this same boot.
const DRAIN_DEADLINE_TICKS: u32 = 300_000_000;

/// Drain while staying RUNNABLE and interruptible: netd and nettest
/// park in blocking IPC/`SYS_WAIT`, and the device MSIs are only TAKEN
/// while some thread runs with IF=1. Bounded by wall clock — see
/// `DRAIN_DEADLINE_TICKS`.
fn drain_interruptible() -> Result<(), &'static str> {
    let if_before = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let r = drain_to_deadline();
    if !if_before {
        crate::arch::x86_64::cli();
    }
    r
}

/// Yield until every spawned child has exited (only the boot thread is
/// live), then one more pass so the last zombie is reaped — bounded by
/// the HPET deadline, not a yield count. Caller holds IF=1.
fn drain_to_deadline() -> Result<(), &'static str> {
    let hpet_va = paging::mmio_alias_va(intc::HPET_PHYS);
    // SAFETY: ring 0; `hpet_va` is the mapped MMIO alias of the HPET
    // page; 32-bit register reads of the main counter's low half.
    let t0 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    while sched::live_threads() > 1 {
        // SAFETY: the same alias, the same register.
        let now = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
        if now.wrapping_sub(t0) >= DRAIN_DEADLINE_TICKS {
            return Err(
                "threads still live after the wall-clock deadline (a device completion or the scheduler is stuck)",
            );
        }
        sched::yield_now();
    }
    sched::yield_now();
    Ok(())
}

// ---- test 1: net_service (M6.1, ADR-0024) -----------------------------------

/// The net_service exit badges (distinct from every other suite's — a
/// merged or crossed badge must fail the exactness check, not pass it).
const NETTEST_EXIT_BADGE: u64 = 0x6E7C;
const NETD_EXIT_BADGE: u64 = 0x6ED0;

/// netd arms TWO MSI-X entries through two SYS_IRQ_RELAY calls — entry 0
/// (receiveq) first, entry 1 (transmitq) second — and the relay pool
/// hands out the first free vectors in order. The m5 suite released and
/// swept everything it armed, so this suite's driver deterministically
/// lands on 48 (RX) and 49 (TX): one counted hardware delivery each.
const NET_RELAY_VEC_RX: u64 = 48;
const NET_RELAY_VEC_TX: u64 = 49;

/// nettest's verified-success exit (the house number, abi's 42).
const NET_EXIT_OK: u64 = 42;

fn test_net_service() -> Outcome {
    match net_service_inner() {
        Ok(()) => Outcome::Pass,
        Err(SkipOrFail::Skip(reason)) => Outcome::Skip(reason),
        Err(SkipOrFail::Fail(reason)) => Outcome::Fail(reason),
    }
}

enum SkipOrFail {
    Skip(&'static str),
    Fail(&'static str),
}

/// So the teardown helpers' `Result<_, &'static str>` flow through `?`
/// as failures (a skip is only ever the absent-fixture decision).
impl From<&'static str> for SkipOrFail {
    fn from(reason: &'static str) -> Self {
        SkipOrFail::Fail(reason)
    }
}

type NetResult = Result<(), SkipOrFail>;

fn fail(reason: &'static str) -> SkipOrFail {
    SkipOrFail::Fail(reason)
}

fn net_service_inner() -> NetResult {
    let baseline = frames::free_frames();

    // The fixture is OPTIONAL (ADR-0024): no virtio-net function on the
    // bus → an honest SKIP. Pre-v0.6.0 QEMU invocations boot green.
    let Some(v) = pci::find_virtio(pci::VIRTIO_TYPE_NET) else {
        return Err(SkipOrFail::Skip(
            "no virtio-net device — attach it with: -netdev user,id=net0 -device virtio-net-pci,netdev=net0",
        ));
    };
    let Some(f) = pci::pci_function(v.pci_index) else {
        return Err(fail("recorded function vanished from the table"));
    };
    let bar = v.common.bar as usize;
    let bar_phys = f.bar_base[bar];
    let bar_pages = (f.bar_size[bar] / 4096) as u32;
    if bar_phys == 0 || bar_pages == 0 || bar_pages > 16 {
        return Err(fail("the structure BAR is unusable for a window grant"));
    }

    // The service objects: one endpoint (driver serves, client calls)
    // and three notifications — the driver's interrupt relay target
    // (BOTH badges land there) and one exit-badge channel per child.
    let eid = ipc::create_endpoint().map_err(|_| fail("endpoint table full"))?;
    let nid_irq = ipc::create_notification().map_err(|_| fail("notification table full"))?;
    let nid_client = ipc::create_notification().map_err(|_| fail("notification table full"))?;
    let nid_netd = ipc::create_notification().map_err(|_| fail("notification table full"))?;

    // netd (registry image 6): slot 0 = the device window (Mmio,
    // READ|WRITE), slot 1 = the serve side of the endpoint, slot 2 =
    // the interrupt notification. Same shape as storaged — the driver
    // discovers everything else through SYS_DEV_INFO (word [6]'s
    // device-config location carries the MAC; config space itself never
    // crosses the boundary).
    let netd_grants = [
        Cap {
            obj: CapObj::Mmio {
                phys: bar_phys,
                pages: bar_pages,
            },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: cap::RIGHTS_READ,
        },
        Cap {
            obj: CapObj::Notification { nid: nid_irq },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
    ];
    let s_pid = crate::spawn::spawn_init(6, &netd_grants, Some((nid_netd, NETD_EXIT_BADGE)))
        .map_err(|_| fail("netd (image 6) spawn failed"))?;
    // nettest (image 7): slot 0 = the call side of the same endpoint.
    let client_grants = [Cap {
        obj: CapObj::Endpoint { eid },
        rights: cap::RIGHTS_WRITE,
    }];
    let c_pid = crate::spawn::spawn_init(7, &client_grants, Some((nid_client, NETTEST_EXIT_BADGE)))
        .map_err(|_| fail("nettest (image 7) spawn failed"))?;
    info!(
        "m6",
        "net_service: netd pid {s_pid} (window bar{} phys {bar_phys:#x} {bar_pages} pages, endpoint {eid} serve side, irq notification {nid_irq}), nettest pid {c_pid} (endpoint {eid} call side) — running the ARP link probe against the slirp gateway",
        bar
    );

    // Run the two children to completion (client verifies the reply and
    // poisons; driver replies to the poison and exits). The bound is
    // wall-clock: one ARP round trip is a handful of context switches
    // plus two MSIs, and the reply itself walks slirp on the HOST —
    // the most host-coupled wait in the whole boot.
    if let Err(reason) = drain_interruptible() {
        error!(
            "m6",
            "net_service drain failed: netd threads={}, client threads={}, relay deliveries RX(vector {NET_RELAY_VEC_RX})={} TX(vector {NET_RELAY_VEC_TX})={}",
            sched::proc_live_threads(s_pid),
            sched::proc_live_threads(c_pid),
            relay::delivery_count(NET_RELAY_VEC_RX),
            relay::delivery_count(NET_RELAY_VEC_TX),
        );
        return Err(fail(reason));
    }

    // Both exit badges arrived, exact and unmerged.
    let bc = ipc::wait(nid_client).map_err(|_| fail("the client's exit badge never arrived"))?;
    if bc != NETTEST_EXIT_BADGE {
        return Err(fail("the client's exit badge is not the granted word"));
    }
    let bs = ipc::wait(nid_netd).map_err(|_| fail("the driver's exit badge never arrived"))?;
    if bs != NETD_EXIT_BADGE {
        return Err(fail("the driver's exit badge is not the granted word"));
    }

    // Both images exited with THEIR success code — the client only
    // exits 42 after the ARP reply's protocol fields verified, the
    // driver only after replying to the poison request.
    let recs = crate::spawn::records_snapshot();
    let tid_of = |pid: u64| -> Option<u64> {
        recs.iter()
            .flatten()
            .find(|&&(p, _)| p == pid)
            .map(|&(_, t)| t)
    };
    let (Some(c_tid), Some(s_tid)) = (tid_of(c_pid), tid_of(s_pid)) else {
        return Err(fail(
            "a spawned child has no record (spawn registry lost it)",
        ));
    };
    let client_status = syscall::exit_status_of(c_tid);
    let driver_status = syscall::exit_status_of(s_tid);
    if client_status != Some(NET_EXIT_OK) {
        return Err(fail(match client_status {
            Some(43) => "client: frame alloc/copy/map refused (43)",
            Some(44) => "client: the NET_MAC call was refused (44)",
            Some(45) => "client: the MAC reply was invalid (45)",
            Some(46) => "client: the NET_SEND call was refused (46)",
            Some(47) => "client: the driver reported a SEND error status (47)",
            Some(48) => "client: the NET_RECV call was refused (48)",
            Some(49) => "client: the driver reported a RECV error status (49)",
            Some(50) => "client: the ARP reply's protocol fields MISMATCHED (50)",
            Some(51) => "client: the poison shutdown call was refused (51)",
            Some(other) if (70..=78).contains(&other) => {
                "driver-side failure code surfaced on the client (70..78)"
            }
            _ => "the client exited with a code from nowhere in the contract",
        }));
    }
    if driver_status != Some(NET_EXIT_OK) {
        return Err(fail(match driver_status {
            Some(70) => "driver: SYS_DEV_INFO refused or short (70)",
            Some(71) => "driver: a self-map (device window or ring frame) refused (71)",
            Some(72) => {
                "driver: the virtio handshake failed — FEATURES_OK did not stick or VERSION_1/MAC missing (72)"
            }
            Some(73) => "driver: a virtqueue setup failed (73)",
            Some(74) => "driver: SYS_IRQ_RELAY refused (74)",
            Some(75) => "driver: SYS_IPC_RECV refused (75)",
            Some(76) => "driver: SYS_CAP_PHYS on the landed buffer refused (76)",
            Some(77) => "driver: a completion never arrived or was malformed (77)",
            Some(78) => "driver: SYS_IPC_REPLY refused (78)",
            Some(97) => "driver: the console refused an output write (97)",
            Some(99) => "driver: the panic handler ran (99)",
            _ => "the driver exited with a code from nowhere in the contract",
        }));
    }

    // The interrupt story, counted on the machine side: netd armed
    // vector 48 for MSI-X entry 0 (receiveq) and 49 for entry 1
    // (transmitq), and exactly ONE hardware delivery walked each
    // stub→relay→notify chain — the ARP reply's arrival and the send's
    // completion. No polling anywhere.
    let rx_deliveries = relay::delivery_count(NET_RELAY_VEC_RX);
    let tx_deliveries = relay::delivery_count(NET_RELAY_VEC_TX);
    if rx_deliveries != 1 {
        return Err(fail(
            "the RX relay vector did not deliver exactly one device interrupt (the ARP reply's arrival)",
        ));
    }
    if tx_deliveries != 1 {
        return Err(fail(
            "the TX relay vector did not deliver exactly one device interrupt (the send completion)",
        ));
    }
    if !relay::registered(NET_RELAY_VEC_RX) || !relay::registered(NET_RELAY_VEC_TX) {
        return Err(fail(
            "a driver relay registration vanished while the process lived",
        ));
    }

    // Teardown: destroying the dead driver must SWEEP both owned relay
    // vectors, then both address spaces come back frame-exact — the
    // driver's five frames (two packed ring pages, three RX buffers),
    // the client's one TX frame, every page table, and nothing else
    // (the device window and the LENT landed caps free nothing, by
    // construction).
    proc::destroy(s_pid)?;
    if relay::registered(NET_RELAY_VEC_RX) || relay::registered(NET_RELAY_VEC_TX) {
        return Err(fail(
            "proc::destroy did not sweep the dead driver's relay vectors",
        ));
    }
    proc::destroy(c_pid)?;
    crate::spawn::forget(s_pid).map_err(|_| fail("driver spawn record forget refused"))?;
    crate::spawn::forget(c_pid).map_err(|_| fail("client spawn record forget refused"))?;
    ipc::destroy_endpoint(eid).map_err(|_| fail("endpoint teardown refused"))?;
    ipc::destroy_notification(nid_irq).map_err(|_| fail("irq notification teardown refused"))?;
    ipc::destroy_notification(nid_client)
        .map_err(|_| fail("client notification teardown refused"))?;
    ipc::destroy_notification(nid_netd)
        .map_err(|_| fail("driver notification teardown refused"))?;

    let after = frames::free_frames();
    if after != baseline {
        error!(
            "m6",
            "net_service teardown accounting: baseline {baseline}, after {after}"
        );
        return Err(fail("net-service teardown is not frame-exact"));
    }
    info!(
        "m6",
        "net_service: nettest (pid {c_pid}) sent a hand-built ARP request for 10.0.2.2 and verified the reply byte-for-byte through netd's (pid {s_pid}) endpoint — zero-copy TX (the frame was LENT through IPC, the device DMA'd the caller's own page behind netd's virtio header), {rx_deliveries} RX + {tx_deliveries} TX interrupt deliveries on relay vectors {NET_RELAY_VEC_RX}/{NET_RELAY_VEC_TX} (no polling), both children exited {NET_EXIT_OK}, both exit badges exact, the dead driver's relays were swept by proc::destroy, teardown frame-exact (frames {after})"
    );
    Ok(())
}

// ---- test 2: rng_service (M6.2, ADR-0025) -----------------------------------

/// The rng_service exit badges (distinct from every other suite's — a
/// merged or crossed badge must fail the exactness check, not pass it).
const RNGTEST_EXIT_BADGE: u64 = 0x727C;
const RNGD_EXIT_BADGE: u64 = 0x72D0;

/// rngd arms ONE MSI-X entry (entry 0, the request queue). The
/// net_service test above released and swept both vectors it armed, so
/// the relay pool's first free vector is 48 again — deterministic, and
/// asserted: exactly two counted hardware deliveries (one per draw).
const RNG_RELAY_VEC: u64 = 48;

/// rngtest's verified-success exit (the house number, abi's 42).
const RNG_EXIT_OK: u64 = 42;

fn test_rng_service() -> Outcome {
    match rng_service_inner() {
        Ok(()) => Outcome::Pass,
        Err(SkipOrFail::Skip(reason)) => Outcome::Skip(reason),
        Err(SkipOrFail::Fail(reason)) => Outcome::Fail(reason),
    }
}

fn rng_service_inner() -> NetResult {
    let baseline = frames::free_frames();

    // The fixture is OPTIONAL (the ADR-0024 discipline, kept): no
    // virtio-rng function on the bus → an honest SKIP, never a fake
    // pass. Boots without the device stay green.
    let Some(v) = pci::find_virtio(pci::VIRTIO_TYPE_ENTROPY) else {
        return Err(SkipOrFail::Skip(
            "no virtio-rng device — attach it with: -device virtio-rng-pci",
        ));
    };
    let Some(f) = pci::pci_function(v.pci_index) else {
        return Err(fail("recorded function vanished from the table"));
    };
    let bar = v.common.bar as usize;
    let bar_phys = f.bar_base[bar];
    let bar_pages = (f.bar_size[bar] / 4096) as u32;
    if bar_phys == 0 || bar_pages == 0 || bar_pages > 16 {
        return Err(fail("the structure BAR is unusable for a window grant"));
    }

    // The service objects: one endpoint (driver serves, client calls)
    // and three notifications — the driver's interrupt relay target and
    // one exit-badge channel per child.
    let eid = ipc::create_endpoint().map_err(|_| fail("endpoint table full"))?;
    let nid_irq = ipc::create_notification().map_err(|_| fail("notification table full"))?;
    let nid_client = ipc::create_notification().map_err(|_| fail("notification table full"))?;
    let nid_rngd = ipc::create_notification().map_err(|_| fail("notification table full"))?;

    // rngd (registry image 8): the same grant shape as storaged and
    // netd — slot 0 = the device window (Mmio, READ|WRITE), slot 1 =
    // the serve side of the endpoint, slot 2 = the interrupt
    // notification. Everything else the driver discovers through
    // SYS_DEV_INFO.
    let rngd_grants = [
        Cap {
            obj: CapObj::Mmio {
                phys: bar_phys,
                pages: bar_pages,
            },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: cap::RIGHTS_READ,
        },
        Cap {
            obj: CapObj::Notification { nid: nid_irq },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
    ];
    let s_pid = crate::spawn::spawn_init(8, &rngd_grants, Some((nid_rngd, RNGD_EXIT_BADGE)))
        .map_err(|_| fail("rngd (image 8) spawn failed"))?;
    // rngtest (image 9): slot 0 = the call side of the same endpoint.
    let client_grants = [Cap {
        obj: CapObj::Endpoint { eid },
        rights: cap::RIGHTS_WRITE,
    }];
    let c_pid = crate::spawn::spawn_init(9, &client_grants, Some((nid_client, RNGTEST_EXIT_BADGE)))
        .map_err(|_| fail("rngtest (image 9) spawn failed"))?;
    info!(
        "m6",
        "rng_service: rngd pid {s_pid} (window bar{} phys {bar_phys:#x} {bar_pages} pages, endpoint {eid} serve side, irq notification {nid_irq}), rngtest pid {c_pid} (endpoint {eid} call side) — running two device-filled draws with variance checks",
        bar
    );

    // Run both children to completion (client draws twice, checks
    // variance, poisons; driver replies to the poison and exits). The
    // bound is wall-clock: each draw is a handful of context switches
    // plus one MSI from the host's entropy backend.
    if let Err(reason) = drain_interruptible() {
        error!(
            "m6",
            "rng_service drain failed: rngd threads={}, client threads={}, relay deliveries (vector {RNG_RELAY_VEC})={}",
            sched::proc_live_threads(s_pid),
            sched::proc_live_threads(c_pid),
            relay::delivery_count(RNG_RELAY_VEC),
        );
        return Err(fail(reason));
    }

    // Both exit badges arrived, exact and unmerged.
    let bc = ipc::wait(nid_client).map_err(|_| fail("the client's exit badge never arrived"))?;
    if bc != RNGTEST_EXIT_BADGE {
        return Err(fail("the client's exit badge is not the granted word"));
    }
    let bs = ipc::wait(nid_rngd).map_err(|_| fail("the driver's exit badge never arrived"))?;
    if bs != RNGD_EXIT_BADGE {
        return Err(fail("the driver's exit badge is not the granted word"));
    }

    // Both images exited with THEIR success code — the client only
    // exits 42 after both draws verified (full length, non-zero,
    // non-constant, mutually different) and the driver's completion
    // count came back exactly two; the driver only after replying to
    // the poison request.
    let recs = crate::spawn::records_snapshot();
    let tid_of = |pid: u64| -> Option<u64> {
        recs.iter()
            .flatten()
            .find(|&&(p, _)| p == pid)
            .map(|&(_, t)| t)
    };
    let (Some(c_tid), Some(s_tid)) = (tid_of(c_pid), tid_of(s_pid)) else {
        return Err(fail(
            "a spawned child has no record (spawn registry lost it)",
        ));
    };
    let client_status = syscall::exit_status_of(c_tid);
    let driver_status = syscall::exit_status_of(s_tid);
    if client_status != Some(RNG_EXIT_OK) {
        return Err(fail(match client_status {
            Some(52) => "client: frame alloc/copy/map refused (52)",
            Some(53) => "client: an RNG_GET call was refused (53)",
            Some(54) => "client: the driver reported a GET error status (54)",
            Some(55) => "client: the device wrote fewer bytes than requested (55)",
            Some(56) => {
                "client: a draw FAILED the variance checks — all-zero, constant, or two identical draws (56)"
            }
            Some(57) => {
                "client: the poison shutdown was refused or its completion count was not two (57)"
            }
            Some(other) if (80..=88).contains(&other) => {
                "driver-side failure code surfaced on the client (80..88)"
            }
            _ => "the client exited with a code from nowhere in the contract",
        }));
    }
    if driver_status != Some(RNG_EXIT_OK) {
        return Err(fail(match driver_status {
            Some(80) => "driver: SYS_DEV_INFO refused or short (80)",
            Some(81) => "driver: a self-map (device window or ring frame) refused (81)",
            Some(82) => {
                "driver: the virtio handshake failed — FEATURES_OK did not stick or VERSION_1 missing (82)"
            }
            Some(83) => "driver: the virtqueue setup failed (83)",
            Some(84) => "driver: SYS_IRQ_RELAY refused (84)",
            Some(85) => "driver: SYS_IPC_RECV refused (85)",
            Some(86) => "driver: SYS_CAP_PHYS on the landed buffer refused (86)",
            Some(87) => "driver: a completion never arrived or was malformed (87)",
            Some(88) => "driver: SYS_IPC_REPLY refused (88)",
            Some(97) => "driver: the console refused an output write (97)",
            Some(99) => "driver: the panic handler ran (99)",
            _ => "the driver exited with a code from nowhere in the contract",
        }));
    }

    // The interrupt story, counted on the machine side: rngd armed one
    // MSI-X entry, and EXACTLY TWO hardware deliveries walked the
    // stub→relay→notify chain — one per draw. No polling anywhere.
    let deliveries = relay::delivery_count(RNG_RELAY_VEC);
    if deliveries != 2 {
        error!(
            "m6",
            "rng_service: relay vector {RNG_RELAY_VEC} delivered {deliveries} interrupts, expected 2 (one per draw)"
        );
        return Err(fail(
            "the relay vector did not deliver exactly two device interrupts (one per draw)",
        ));
    }
    if !relay::registered(RNG_RELAY_VEC) {
        return Err(fail(
            "the driver's relay registration vanished while the process lived",
        ));
    }

    // Teardown: destroying the dead driver must SWEEP its relay vector,
    // then both address spaces come back frame-exact — the driver's ONE
    // ring frame, the client's TWO draw frames, every page table, and
    // nothing else (the device window and the LENT landed caps free
    // nothing, by construction).
    proc::destroy(s_pid)?;
    if relay::registered(RNG_RELAY_VEC) {
        return Err(fail(
            "proc::destroy did not sweep the dead driver's relay vector",
        ));
    }
    proc::destroy(c_pid)?;
    crate::spawn::forget(s_pid).map_err(|_| fail("driver spawn record forget refused"))?;
    crate::spawn::forget(c_pid).map_err(|_| fail("client spawn record forget refused"))?;
    ipc::destroy_endpoint(eid).map_err(|_| fail("endpoint teardown refused"))?;
    ipc::destroy_notification(nid_irq).map_err(|_| fail("irq notification teardown refused"))?;
    ipc::destroy_notification(nid_client)
        .map_err(|_| fail("client notification teardown refused"))?;
    ipc::destroy_notification(nid_rngd)
        .map_err(|_| fail("driver notification teardown refused"))?;

    let after = frames::free_frames();
    if after != baseline {
        error!(
            "m6",
            "rng_service teardown accounting: baseline {baseline}, after {after}"
        );
        return Err(fail("rng-service teardown is not frame-exact"));
    }
    info!(
        "m6",
        "rng_service: rngtest (pid {c_pid}) drew two 4 KiB frames of device entropy through rngd's (pid {s_pid}) endpoint — the device DMA'd directly into the client's OWN pages (each frame LENT through IPC, zero copy), both draws non-zero, non-constant, and mutually different, {deliveries} interrupt deliveries on relay vector {RNG_RELAY_VEC} (one per draw, no polling), both children exited {RNG_EXIT_OK}, both exit badges exact, the dead driver's relay was swept by proc::destroy, teardown frame-exact (frames {after})"
    );
    Ok(())
}

// ---- 6.3: the input proof (ADR-0026) ---------------------------------------

/// The input_service exit badges (distinct from every other suite's — a
/// merged or crossed badge must fail the exactness check, not pass it).
const INPUTTEST_EXIT_BADGE: u64 = 0x6970;
const INPUTD_EXIT_BADGE: u64 = 0x69D0;

/// inputd arms ONE MSI-X entry (entry 0, the event queue). The two
/// tests above released and swept every vector they armed, so the
/// relay pool's first free vector is 48 again — deterministic.
const INPUT_RELAY_VEC: u64 = 48;

/// inputtest's verified-success exit (the house number, abi's 42).
const INPUT_EXIT_OK: u64 = 42;

/// inputtest's "nobody typed" exit — an honest SKIP, not a failure.
const INPUT_EXIT_NO_KEYS: u64 = 68;

/// The badge this suite sends on inputd's relay notification to call
/// off a pending READ. MUST match `INPUT_BADGE_GIVE_UP` in
/// `userspace/abi.rs` — the one word of that notification that is not
/// the device's.
const INPUT_GIVE_UP_BADGE: u64 = 1 << 19;

/// How long the suite waits for a host-side actor before calling the
/// driver's wait off, in HPET main-counter ticks: ~1 s (QEMU's HPET
/// runs at 100 MHz, not the 14.318 MHz of the classic PIT). Generous
/// for an automated harness — which acts within milliseconds of the
/// driver's ready marker — and short enough that a person booting with
/// a keyboard or a console port nobody is using barely notices. THIS
/// is why the give-up exists: a driver cannot distinguish "nobody is
/// there" from "nobody is there YET", so only the supervisor can
/// decide to stop waiting (ADR-0026, extended in ADR-0027).
const ACTOR_WINDOW_TICKS: u32 = 100_000_000;

/// consoled's two relay vectors, in setup order (receive first). The
/// suite runs its tests in sequence and each releases its vectors, so
/// the pair is deterministic — and the RECEIVE one is what the
/// host-actor window watches: bytes leaving the guest need no listener
/// (QEMU consumes them either way), bytes arriving do.
const CONSOLE_RELAY_VEC_RX: u64 = 48;
const CONSOLE_RELAY_VEC_TX: u64 = 49;

/// contest's verified-success exit, and its honest "nobody was
/// attached to the port" exit.
const CONSOLE_EXIT_OK: u64 = 42;
const CONSOLE_EXIT_NO_DATA: u64 = 68;

/// Exit badges for the console test's two children.
const CONSOLED_EXIT_BADGE: u64 = 0x00C0_0001;
const CONTEST_EXIT_BADGE: u64 = 0x00C0_0002;

/// The give-up word on consoled's notification. MUST match
/// `CONSOLE_BADGE_GIVE_UP` in `userspace/abi.rs`.
const CONSOLE_GIVE_UP_BADGE: u64 = 1 << 19;

fn test_console_service() -> Outcome {
    match console_service_inner() {
        Ok(()) => Outcome::Pass,
        Err(SkipOrFail::Skip(reason)) => Outcome::Skip(reason),
        Err(SkipOrFail::Fail(reason)) => Outcome::Fail(reason),
    }
}

fn console_service_inner() -> NetResult {
    let baseline = frames::free_frames();

    // Optional fixture, as every M6 fixture is: no virtio-console
    // function on the bus → an honest SKIP, and the machine stays
    // fully usable on the serial console.
    let Some(v) = pci::find_virtio(pci::VIRTIO_TYPE_CONSOLE) else {
        return Err(SkipOrFail::Skip(
            "no virtio-console device — attach one with: -device virtio-serial-pci,max_ports=1 -device virtconsole,chardev=<id>",
        ));
    };
    let Some(f) = pci::pci_function(v.pci_index) else {
        return Err(fail("recorded function vanished from the table"));
    };
    let bar = v.common.bar as usize;
    let bar_phys = f.bar_base[bar];
    let bar_pages = (f.bar_size[bar] / 4096) as u32;
    if bar_phys == 0 || bar_pages == 0 || bar_pages > 16 {
        return Err(fail("the structure BAR is unusable for a window grant"));
    }

    let eid = ipc::create_endpoint().map_err(|_| fail("endpoint table full"))?;
    let nid_irq = ipc::create_notification().map_err(|_| fail("notification table full"))?;
    let nid_client = ipc::create_notification().map_err(|_| fail("notification table full"))?;
    let nid_consoled = ipc::create_notification().map_err(|_| fail("notification table full"))?;

    // consoled (registry image 12) gets the three-cap driver shape and
    // NEITHER console capability: the production instance holds both
    // (ConsoleInput to feed the line discipline, ConsoleOutput to
    // mirror what the machine prints), and this one holds neither, so
    // the same image serves the port over IPC instead. The mode switch
    // is under test as much as the device is (ADR-0027).
    let consoled_grants = [
        Cap {
            obj: CapObj::Mmio {
                phys: bar_phys,
                pages: bar_pages,
            },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: cap::RIGHTS_READ,
        },
        Cap {
            obj: CapObj::Notification { nid: nid_irq },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
    ];
    let s_pid = crate::spawn::spawn_init(
        12,
        &consoled_grants,
        Some((nid_consoled, CONSOLED_EXIT_BADGE)),
    )
    .map_err(|_| fail("consoled (image 12) spawn failed"))?;
    let client_grants = [Cap {
        obj: CapObj::Endpoint { eid },
        rights: cap::RIGHTS_WRITE,
    }];
    let c_pid =
        crate::spawn::spawn_init(13, &client_grants, Some((nid_client, CONTEST_EXIT_BADGE)))
            .map_err(|_| fail("contest (image 13) spawn failed"))?;
    info!(
        "m6",
        "console_service: consoled pid {s_pid} (window bar{} phys {bar_phys:#x} {bar_pages} pages, endpoint {eid} serve side, irq notification {nid_irq}, NO console authority — service mode), contest pid {c_pid} (endpoint {eid} call side) — the port's far end is the harness's socket",
        bar
    );

    // The host-actor window, watching the RECEIVE vector: the guest→host
    // half needs nobody (QEMU takes the bytes whether or not anyone is
    // listening), the host→guest half needs a listener that answers.
    if let Err(reason) = wait_for_host_actor(
        "console_service",
        nid_irq,
        CONSOLE_RELAY_VEC_RX,
        CONSOLE_GIVE_UP_BADGE,
        // Start the clock when the guest's fixture has actually gone
        // out (a TRANSMIT completion), not when the driver armed its
        // relay: the harness cannot answer a question it has not
        // been asked.
        Some(CONSOLE_RELAY_VEC_TX),
        // A LONGER window than the keyboard's, because this exchange
        // crosses the host twice: the guest asks, the host reads,
        // answers, and the guest reads back. (The 6-in-100 SKIPs that
        // first prompted this were NOT a timing problem — they were
        // consoled's badge words aliasing under `&`, fixed in abi.rs.
        // The wider budget stays as honest headroom for a two-hop
        // exchange, and is paid only by a boot where nobody is
        // attached, which then waits a few seconds before skipping.)
        ACTOR_WINDOW_TICKS * 4,
    ) {
        return Err(fail(reason));
    }

    if let Err(reason) = drain_interruptible() {
        error!(
            "m6",
            "console_service drain failed: consoled threads={}, client threads={}, relay deliveries rx/tx={}/{}",
            sched::proc_live_threads(s_pid),
            sched::proc_live_threads(c_pid),
            relay::delivery_count(CONSOLE_RELAY_VEC_RX),
            relay::delivery_count(CONSOLE_RELAY_VEC_TX),
        );
        return Err(fail(reason));
    }

    let bc = ipc::wait(nid_client).map_err(|_| fail("the client's exit badge never arrived"))?;
    if bc != CONTEST_EXIT_BADGE {
        return Err(fail("the client's exit badge is not the granted word"));
    }
    let bs = ipc::wait(nid_consoled).map_err(|_| fail("the driver's exit badge never arrived"))?;
    if bs != CONSOLED_EXIT_BADGE {
        return Err(fail("the driver's exit badge is not the granted word"));
    }

    let recs = crate::spawn::records_snapshot();
    let tid_of = |pid: u64| -> Option<u64> {
        recs.iter()
            .flatten()
            .find(|&&(p, _)| p == pid)
            .map(|&(_, t)| t)
    };
    let (Some(c_tid), Some(s_tid)) = (tid_of(c_pid), tid_of(s_pid)) else {
        return Err(fail(
            "a spawned child has no record (spawn registry lost it)",
        ));
    };
    let client_status = syscall::exit_status_of(c_tid);
    let driver_status = syscall::exit_status_of(s_tid);

    // Nobody was attached to the port's far end: tear down exactly as a
    // passing run does and report the honest SKIP.
    if client_status == Some(CONSOLE_EXIT_NO_DATA) {
        console_teardown(s_pid, c_pid, eid, nid_irq, nid_client, nid_consoled)?;
        if frames::free_frames() != baseline {
            return Err(fail("console-service teardown is not frame-exact"));
        }
        return Err(SkipOrFail::Skip(
            "nothing arrived on the port — nobody was attached to its far end (connect to the chardev socket, e.g. nc -U)",
        ));
    }
    if client_status != Some(CONSOLE_EXIT_OK) {
        return Err(fail(match client_status {
            Some(60) => "client: a console call was refused (60)",
            Some(61) => "client: the service refused the port WRITE (61)",
            Some(63) => "client: the driver reported a READ error status (63)",
            Some(64) => "client: the service returned an impossible byte count (64)",
            Some(65) => "client: the bytes read back are NOT what the harness sent (65)",
            Some(66) => "client: the service under-counted the bytes it sent (66)",
            Some(67) => "client: the poison shutdown was refused (67)",
            Some(other) if (80..=89).contains(&other) => {
                "driver-side failure code surfaced on the client (80..89)"
            }
            _ => "the client exited with a code from nowhere in the contract",
        }));
    }
    if driver_status != Some(CONSOLE_EXIT_OK) {
        return Err(fail(match driver_status {
            Some(80) => "driver: SYS_DEV_INFO refused or short (80)",
            Some(81) => "driver: a self-map (device window or ring frame) refused (81)",
            Some(82) => {
                "driver: the virtio handshake failed — FEATURES_OK did not stick or VERSION_1 missing (82)"
            }
            Some(83) => {
                "driver: a virtqueue setup failed, or the port buffers did not fit their ring frame (83)"
            }
            Some(84) => "driver: SYS_IRQ_RELAY refused (84)",
            Some(85) => "driver: SYS_IPC_RECV refused (85)",
            Some(86) => "driver: the notification wait failed (86)",
            Some(87) => "driver: SYS_CONSOLE_PULL refused in console mode (87)",
            Some(88) => "driver: SYS_IPC_REPLY refused (88)",
            Some(89) => {
                "driver: the console grants were incomplete (89) — the suite's instance should hold NEITHER"
            }
            Some(99) => "driver: the panic handler ran (99)",
            _ => "the driver exited with a code from nowhere in the contract",
        }));
    }

    // The interrupt story, counted on the machine side: BOTH directions
    // must have been interrupt-completed. Lower bounds again — the
    // device coalesces at its own discretion.
    let rx = relay::delivery_count(CONSOLE_RELAY_VEC_RX);
    let tx = relay::delivery_count(CONSOLE_RELAY_VEC_TX);
    if tx == 0 {
        return Err(fail(
            "the transmit vector delivered no interrupts (the bytes cannot have left by interrupt)",
        ));
    }
    if rx == 0 {
        return Err(fail(
            "the receive vector delivered no interrupts (the bytes cannot have arrived by interrupt)",
        ));
    }

    console_teardown(s_pid, c_pid, eid, nid_irq, nid_client, nid_consoled)?;

    let after = frames::free_frames();
    if after != baseline {
        error!(
            "m6",
            "console_service teardown accounting: baseline {baseline}, after {after}"
        );
        return Err(fail("console-service teardown is not frame-exact"));
    }
    info!(
        "m6",
        "console_service: contest (pid {c_pid}) drove a port ROUND TRIP through consoled's (pid {s_pid}) endpoint — bytes out the transmit queue that the harness read off the host socket, and the harness's answer back in on the receive queue, verified byte-for-byte; {tx} transmit and {rx} receive interrupt delivery/deliveries on relay vectors {CONSOLE_RELAY_VEC_TX}/{CONSOLE_RELAY_VEC_RX} (no polling), the driver in SERVICE mode because the suite withheld BOTH console capabilities, both children exited {CONSOLE_EXIT_OK}, both exit badges exact, both relays swept by proc::destroy, teardown frame-exact (frames {after})"
    );
    Ok(())
}

/// The console test's teardown, shared by the pass and honest-SKIP
/// paths: destroying the driver must sweep BOTH its relay vectors.
fn console_teardown(
    s_pid: u64,
    c_pid: u64,
    eid: u32,
    nid_irq: u32,
    nid_client: u32,
    nid_consoled: u32,
) -> NetResult {
    proc::destroy(s_pid)?;
    if relay::registered(CONSOLE_RELAY_VEC_RX) || relay::registered(CONSOLE_RELAY_VEC_TX) {
        return Err(fail(
            "proc::destroy did not sweep both of the dead driver's relay vectors",
        ));
    }
    proc::destroy(c_pid)?;
    crate::spawn::forget(s_pid).map_err(|_| fail("driver spawn record forget refused"))?;
    crate::spawn::forget(c_pid).map_err(|_| fail("client spawn record forget refused"))?;
    ipc::destroy_endpoint(eid).map_err(|_| fail("endpoint teardown refused"))?;
    ipc::destroy_notification(nid_irq).map_err(|_| fail("irq notification teardown refused"))?;
    ipc::destroy_notification(nid_client)
        .map_err(|_| fail("client notification teardown refused"))?;
    ipc::destroy_notification(nid_consoled)
        .map_err(|_| fail("driver notification teardown refused"))?;
    Ok(())
}

fn test_input_service() -> Outcome {
    match input_service_inner() {
        Ok(()) => Outcome::Pass,
        Err(SkipOrFail::Skip(reason)) => Outcome::Skip(reason),
        Err(SkipOrFail::Fail(reason)) => Outcome::Fail(reason),
    }
}

fn input_service_inner() -> NetResult {
    let baseline = frames::free_frames();

    // The fixture is OPTIONAL (the ADR-0024 discipline, kept): no
    // virtio-input function on the bus → an honest SKIP. A machine
    // with no keyboard must stay fully usable on the serial console.
    let Some(v) = pci::find_virtio(pci::VIRTIO_TYPE_INPUT) else {
        return Err(SkipOrFail::Skip(
            "no virtio-input device — attach it with: -device virtio-keyboard-pci",
        ));
    };
    let Some(f) = pci::pci_function(v.pci_index) else {
        return Err(fail("recorded function vanished from the table"));
    };
    let bar = v.common.bar as usize;
    let bar_phys = f.bar_base[bar];
    let bar_pages = (f.bar_size[bar] / 4096) as u32;
    if bar_phys == 0 || bar_pages == 0 || bar_pages > 16 {
        return Err(fail("the structure BAR is unusable for a window grant"));
    }

    // The service objects: one endpoint (driver serves, client calls)
    // and three notifications — the driver's interrupt relay target and
    // one exit-badge channel per child.
    let eid = ipc::create_endpoint().map_err(|_| fail("endpoint table full"))?;
    let nid_irq = ipc::create_notification().map_err(|_| fail("notification table full"))?;
    let nid_client = ipc::create_notification().map_err(|_| fail("notification table full"))?;
    let nid_inputd = ipc::create_notification().map_err(|_| fail("notification table full"))?;

    // inputd (registry image 10) gets the THREE-cap driver shape and
    // deliberately NOT the ConsoleInput cap the production instance
    // holds: the same image, probing for that authority, runs as an
    // IPC service here instead of feeding the console. That is the
    // mode switch under test as much as the device is (ADR-0026).
    let inputd_grants = [
        Cap {
            obj: CapObj::Mmio {
                phys: bar_phys,
                pages: bar_pages,
            },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: cap::RIGHTS_READ,
        },
        Cap {
            obj: CapObj::Notification { nid: nid_irq },
            rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
        },
    ];
    let s_pid = crate::spawn::spawn_init(10, &inputd_grants, Some((nid_inputd, INPUTD_EXIT_BADGE)))
        .map_err(|_| fail("inputd (image 10) spawn failed"))?;
    // inputtest (image 11): slot 0 = the call side of the same endpoint.
    let client_grants = [Cap {
        obj: CapObj::Endpoint { eid },
        rights: cap::RIGHTS_WRITE,
    }];
    let c_pid =
        crate::spawn::spawn_init(11, &client_grants, Some((nid_client, INPUTTEST_EXIT_BADGE)))
            .map_err(|_| fail("inputtest (image 11) spawn failed"))?;
    info!(
        "m6",
        "input_service: inputd pid {s_pid} (window bar{} phys {bar_phys:#x} {bar_pages} pages, endpoint {eid} serve side, irq notification {nid_irq}, NO console authority — service mode), inputtest pid {c_pid} (endpoint {eid} call side) — awaiting the keystrokes the harness types on the virtual keyboard",
        bar
    );

    // The keystroke window: run the children while watching for the
    // FIRST device interrupt. Unlike every other fixture in this
    // suite, this one needs a host-side ACTOR — slirp answers ARP by
    // itself and the entropy backend fills buffers by itself, but a
    // keyboard produces nothing unless someone types. So the suite
    // waits a bounded time and then calls the wait off; the children
    // wind themselves up cleanly and the test reports an honest SKIP.
    // A boot with a keyboard attached and nobody at it must stay
    // green — attaching a device must never make a machine unusable.
    if let Err(reason) = wait_for_host_actor(
        "input_service",
        nid_irq,
        INPUT_RELAY_VEC,
        INPUT_GIVE_UP_BADGE,
        None, // a keyboard needs no prompting; the harness types first
        ACTOR_WINDOW_TICKS,
    ) {
        return Err(fail(reason));
    }

    // Run both children to completion. Wall-clock bounded,
    // interruptible — the only correct unit when the awaited event is
    // host-coupled.
    if let Err(reason) = drain_interruptible() {
        error!(
            "m6",
            "input_service drain failed: inputd threads={}, client threads={}, relay deliveries (vector {INPUT_RELAY_VEC})={}",
            sched::proc_live_threads(s_pid),
            sched::proc_live_threads(c_pid),
            relay::delivery_count(INPUT_RELAY_VEC),
        );
        return Err(fail(reason));
    }

    // Both exit badges arrived, exact and unmerged.
    let bc = ipc::wait(nid_client).map_err(|_| fail("the client's exit badge never arrived"))?;
    if bc != INPUTTEST_EXIT_BADGE {
        return Err(fail("the client's exit badge is not the granted word"));
    }
    let bs = ipc::wait(nid_inputd).map_err(|_| fail("the driver's exit badge never arrived"))?;
    if bs != INPUTD_EXIT_BADGE {
        return Err(fail("the driver's exit badge is not the granted word"));
    }

    // Both images exited with THEIR success code — the client only
    // exits 42 after the decoded bytes matched the injected sequence
    // byte-for-byte AND the driver accounted for at least one
    // interrupt-delivered batch.
    let recs = crate::spawn::records_snapshot();
    let tid_of = |pid: u64| -> Option<u64> {
        recs.iter()
            .flatten()
            .find(|&&(p, _)| p == pid)
            .map(|&(_, t)| t)
    };
    let (Some(c_tid), Some(s_tid)) = (tid_of(c_pid), tid_of(s_pid)) else {
        return Err(fail(
            "a spawned child has no record (spawn registry lost it)",
        ));
    };
    let client_status = syscall::exit_status_of(c_tid);
    let driver_status = syscall::exit_status_of(s_tid);

    // Nobody typed: tear down exactly as a passing run does (the
    // teardown is part of what this test proves either way) and report
    // the honest SKIP.
    if client_status == Some(INPUT_EXIT_NO_KEYS) {
        input_teardown(s_pid, c_pid, eid, nid_irq, nid_client, nid_inputd)?;
        if frames::free_frames() != baseline {
            return Err(fail("input-service teardown is not frame-exact"));
        }
        return Err(SkipOrFail::Skip(
            "no keystrokes arrived — nobody typed during the suite (type in the QEMU window, or see tools/qmp.py)",
        ));
    }
    if client_status != Some(INPUT_EXIT_OK) {
        return Err(fail(match client_status {
            Some(62) => "client: an INPUT_READ call was refused (62)",
            Some(63) => "client: the driver reported a READ error status (63)",
            Some(64) => "client: the service returned an impossible key count (64)",
            Some(65) => "client: the decoded keys are NOT the sequence the harness typed (65)",
            Some(66) => "client: the service never completed the sequence (66)",
            Some(67) => {
                "client: the poison shutdown was refused or reported no interrupt batches (67)"
            }
            Some(other) if (80..=89).contains(&other) => {
                "driver-side failure code surfaced on the client (80..89)"
            }
            _ => "the client exited with a code from nowhere in the contract",
        }));
    }
    if driver_status != Some(INPUT_EXIT_OK) {
        return Err(fail(match driver_status {
            Some(80) => "driver: SYS_DEV_INFO refused or short (80)",
            Some(81) => "driver: a self-map (device window or ring frame) refused (81)",
            Some(82) => {
                "driver: the virtio handshake failed — FEATURES_OK did not stick or VERSION_1 missing (82)"
            }
            Some(83) => {
                "driver: the virtqueue setup failed, or the event buffers did not fit the ring frame (83)"
            }
            Some(84) => "driver: SYS_IRQ_RELAY refused (84)",
            Some(85) => "driver: SYS_IPC_RECV refused (85)",
            Some(86) => "driver: the relay wait or the event harvest failed (86)",
            Some(88) => "driver: SYS_IPC_REPLY refused (88)",
            Some(89) => {
                "driver: SYS_CONSOLE_PUSH was refused in console mode (89) — the suite's instance should never have entered it"
            }
            Some(97) => "driver: the console refused an output write (97)",
            Some(99) => "driver: the panic handler ran (99)",
            _ => "the driver exited with a code from nowhere in the contract",
        }));
    }

    // The interrupt story, counted on the machine side. Unlike the net
    // and rng tests this asserts a LOWER BOUND, and deliberately: the
    // device flushes one batch per SYN_REPORT and raises one interrupt
    // per batch, so an exact count would be asserting QEMU's coalescing
    // policy rather than our driver's behavior. What is genuinely ours
    // — every injected key arrived, in order, delivered by interrupt —
    // the client already verified byte-for-byte.
    let deliveries = relay::delivery_count(INPUT_RELAY_VEC);
    if deliveries == 0 {
        error!(
            "m6",
            "input_service: relay vector {INPUT_RELAY_VEC} delivered no interrupts at all"
        );
        return Err(fail(
            "the relay vector delivered no device interrupts (the keys cannot have arrived by interrupt)",
        ));
    }
    if !relay::registered(INPUT_RELAY_VEC) {
        return Err(fail(
            "the driver's relay registration vanished while the process lived",
        ));
    }

    // Teardown: destroying the dead driver must SWEEP its relay vector,
    // then both address spaces come back frame-exact — the driver's ONE
    // ring frame (rings AND event buffers in the same page), every page
    // table, and nothing else.
    input_teardown(s_pid, c_pid, eid, nid_irq, nid_client, nid_inputd)?;

    let after = frames::free_frames();
    if after != baseline {
        error!(
            "m6",
            "input_service teardown accounting: baseline {baseline}, after {after}"
        );
        return Err(fail("input-service teardown is not frame-exact"));
    }
    info!(
        "m6",
        "input_service: inputtest (pid {c_pid}) read the keystrokes the harness typed on the virtual keyboard back through inputd's (pid {s_pid}) endpoint — evdev keycodes harvested from a device-writable virtqueue on {deliveries} interrupt delivery/deliveries on relay vector {INPUT_RELAY_VEC} (no polling), decoded to ASCII, and verified byte-for-byte; the driver ran in SERVICE mode because the suite withheld the ConsoleInput capability, both children exited {INPUT_EXIT_OK}, both exit badges exact, the dead driver's relay was swept by proc::destroy, teardown frame-exact (frames {after})"
    );
    Ok(())
}

/// Run the children while watching `vec` for the first device
/// interrupt, and send `give_up` on their notification if none arrives
/// inside [`ACTOR_WINDOW_TICKS`]. Shared by the two tests whose
/// fixtures need a host-side ACTOR — a keyboard nobody types on and a
/// port nobody is attached to are the same shape of nothing, and
/// neither driver can tell "never" from "not yet" (ADR-0026/0027).
/// Returns once either a keystroke landed or the give-up was sent —
/// the ordinary drain follows in both cases. Caller holds IF as the
/// suite found it; this enables interrupts exactly like the drain
/// (device MSIs are only TAKEN while some thread runs with IF=1).
fn wait_for_host_actor(
    test: &str,
    nid_irq: u32,
    vec: u64,
    give_up: u64,
    after_vec: Option<u64>,
    window_ticks: u32,
) -> Result<(), &'static str> {
    let if_before = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let hpet_va = paging::mmio_alias_va(intc::HPET_PHYS);
    let gave_up = actor_window(hpet_va, vec, after_vec, window_ticks);
    let notified = if gave_up {
        info!(
            "m6",
            "{test}: nothing arrived inside the window — calling the driver's wait off (badge {give_up:#x}); this boot simply had nobody at the other end"
        );
        ipc::notify(nid_irq, give_up).is_ok()
    } else {
        true
    };
    if !if_before {
        crate::arch::x86_64::cli();
    }
    if !notified {
        return Err("the give-up notification was refused");
    }
    Ok(())
}

/// The window itself: returns `true` if it expired without a
/// keystroke. Two phases, because the relay vector is RECYCLED across
/// this suite's tests — the rng test left its own delivery count on
/// vector 48, so "someone typed" cannot be read from the raw counter
/// until THIS driver has armed the vector (which resets it). Phase 1
/// waits for that arming; phase 2 waits for a delivery beyond the
/// baseline taken right after it. Both are bounded by the same window,
/// and a driver that never arms at all falls through to the ordinary
/// drain's diagnostics rather than being misreported as "nobody
/// typed". Caller holds IF=1.
fn actor_window(hpet_va: u64, vec: u64, after_vec: Option<u64>, window_ticks: u32) -> bool {
    // SAFETY: ring 0; `hpet_va` is the mapped MMIO alias of the HPET
    // page; 32-bit reads of the main counter's low half.
    let t0 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    let expired = |t: u32| -> bool {
        // SAFETY: the same alias, the same register.
        let now = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
        now.wrapping_sub(t) >= window_ticks
    };
    while !relay::registered(vec) {
        if sched::live_threads() <= 1 || expired(t0) {
            return false; // no driver to call off; let the drain speak
        }
        sched::yield_now();
    }
    let base = relay::delivery_count(vec);

    // Phase 1.5, when the host's action is a REPLY rather than an
    // opening move: do not start counting until the guest's own
    // message has actually left. The console test asks the harness to
    // answer something contest sends, so the clock that matters starts
    // at the TRANSMIT completion — starting it at relay-arming time
    // measured the guest's own setup instead, and under load that
    // expired before the question had even been asked (3 boots in 90).
    // Bounded generously: if the guest never manages to transmit, the
    // ordinary drain's deadline is the backstop and its diagnostics
    // are the right ones.
    if let Some(pre) = after_vec {
        let pre_base = relay::delivery_count(pre);
        // SAFETY: as above.
        let t_pre = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
        while relay::delivery_count(pre) <= pre_base {
            if sched::live_threads() <= 1 {
                return false;
            }
            // SAFETY: as above.
            let now = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
            if now.wrapping_sub(t_pre) >= window_ticks * 4 {
                break; // let the drain speak; do not blame the host
            }
            sched::yield_now();
        }
    }

    // SAFETY: as above.
    let t1 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    while sched::live_threads() > 1 {
        if relay::delivery_count(vec) > base {
            return false; // the far end is live — the drain takes over
        }
        if expired(t1) {
            return true;
        }
        sched::yield_now();
    }
    false
}

/// The input test's teardown, shared by the pass and the honest-SKIP
/// paths: both must leave the machine exactly as they found it, and
/// the sweep of the dead driver's relay vector is itself an assertion.
fn input_teardown(
    s_pid: u64,
    c_pid: u64,
    eid: u32,
    nid_irq: u32,
    nid_client: u32,
    nid_inputd: u32,
) -> NetResult {
    proc::destroy(s_pid)?;
    if relay::registered(INPUT_RELAY_VEC) {
        return Err(fail(
            "proc::destroy did not sweep the dead driver's relay vector",
        ));
    }
    proc::destroy(c_pid)?;
    crate::spawn::forget(s_pid).map_err(|_| fail("driver spawn record forget refused"))?;
    crate::spawn::forget(c_pid).map_err(|_| fail("client spawn record forget refused"))?;
    ipc::destroy_endpoint(eid).map_err(|_| fail("endpoint teardown refused"))?;
    ipc::destroy_notification(nid_irq).map_err(|_| fail("irq notification teardown refused"))?;
    ipc::destroy_notification(nid_client)
        .map_err(|_| fail("client notification teardown refused"))?;
    ipc::destroy_notification(nid_inputd)
        .map_err(|_| fail("driver notification teardown refused"))?;
    Ok(())
}
