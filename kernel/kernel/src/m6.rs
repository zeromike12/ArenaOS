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
    let checks: [(&str, fn() -> Outcome); 2] = [
        ("net_service", test_net_service),
        ("rng_service", test_rng_service),
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
                "m6: RESULT SKIP ({passed}/{total} passed, {skipped} skipped — fixture absent)"
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
