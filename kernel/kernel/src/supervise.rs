//! The service supervisor (M6.5, ADR-0028): restarting a driver that
//! died, with the capabilities it needs to work again.
//!
//! ADR-0022 deferred this with a promise: drivers run in ring 3 so
//! that a broken one cannot take the kernel with it, but "the process
//! survives its driver" is only half a claim if the SERVICE does not
//! come back. This module is the other half, and it is deliberately
//! small.
//!
//! **What a restart is here.** A supervised service is a registry
//! image plus the exact grant list it was spawned with. Restarting it
//! means: reap the corpse's spawn record, mint the SAME capabilities
//! again, and spawn the image afresh. The capabilities are values —
//! an `Mmio` window is a physical base and a page count, an
//! `Endpoint` is an index — so "the same grants" is not a
//! reconstruction, it is the identical list, replayed.
//!
//! **Why clients survive it.** A client's capability names the
//! ENDPOINT, not the process. The endpoint is a kernel object that
//! outlives its server (proven by the `service_death` test), so a
//! restarted instance picks the serve side back up and the clients'
//! caps keep working untouched. A client that was mid-call when the
//! server died got `STATUS_SERVICE_GONE` and has only to call again.
//!
//! **What this is NOT, yet.** There is no policy beyond a restart
//! bound, no back-off, no dependency ordering, and no state recovery:
//! a restarted driver starts from nothing, and a client that was
//! halfway through a multi-request transaction must cope. Those are
//! real problems and they belong to a later milestone with its own
//! evidence; pretending to solve them here would be the kind of
//! half-built claim this project does not make.
//!
//! **Why the kernel and not a userspace init.** Minting an `Mmio`
//! capability is the one authority the kernel has never delegated
//! (ADR-0021), and a supervisor that cannot re-grant a device window
//! cannot restart a driver. Handing that authority to a ring-3
//! supervisor is a real design option — it is how a microkernel
//! usually ends up — but it is a new syscall and a new trust boundary,
//! and it should be decided on its own merits rather than smuggled in
//! as a detail of "restart a driver". Recorded in ADR-0028 as the
//! obvious next question.

use crate::cap::Cap;
use crate::log::{log_error as error, log_info as info};
use crate::spawn::MAX_INHERIT;
use crate::sync::{SyncCell, without_interrupts};

/// How many services may be supervised at once. One per resident
/// driver with room to spare; the bound exists so the table is a
/// plain array and every index is checked.
pub const MAX_SUPERVISED: usize = 8;

/// How many times a service may be restarted before the supervisor
/// gives up on it.
///
/// A bound, not a policy: a driver that dies three times in a row is
/// not going to be fixed by a fourth spawn, and an unbounded
/// supervisor turns a broken driver into a machine that does nothing
/// but restart it. Giving up is logged loudly and leaves the service
/// offline — which is the same honest state the machine is in when
/// the device is absent, and which every client already handles.
pub const MAX_RESTARTS: u32 = 3;

#[derive(Clone, Copy)]
struct Service {
    live: bool,
    /// Registry image id (`spawn::image_bytes`).
    img_id: u32,
    /// For logs — the name a human looks for.
    name: &'static str,
    /// The grant list, replayed verbatim on every restart.
    grants: [Cap; MAX_INHERIT],
    grant_count: usize,
    /// The currently running instance (0 = none: dead or given up).
    pid: u64,
    /// The instance that just died, still holding a spawn record.
    /// Reaped by [`poll`] before the replacement is spawned — the GC
    /// debt ADR-0025 named: records are a bounded table, and a
    /// service that restarts would otherwise exhaust it one corpse at
    /// a time.
    dead_pid: u64,
    restarts: u32,
    /// Set when `MAX_RESTARTS` is exhausted; the service stays off.
    abandoned: bool,
}

const EMPTY: Service = Service {
    live: false,
    img_id: 0,
    name: "",
    grants: [Cap::EMPTY; MAX_INHERIT],
    grant_count: 0,
    pid: 0,
    dead_pid: 0,
    restarts: 0,
    abandoned: false,
};

static SERVICES: SyncCell<[Service; MAX_SUPERVISED]> = SyncCell::new([EMPTY; MAX_SUPERVISED]);

/// Snapshot of one supervised service, for the suites and logs.
#[derive(Clone, Copy)]
pub struct ServiceStatus {
    pub name: &'static str,
    pub pid: u64,
    pub restarts: u32,
    pub abandoned: bool,
}

/// Register an ALREADY SPAWNED service for supervision.
///
/// The caller spawns first and registers second on purpose: spawning
/// is where the grants are decided and where failure is easiest to
/// report, and a supervisor that also owned first-start policy would
/// be two jobs in one module.
pub fn register(
    name: &'static str,
    img_id: u32,
    grants: &[Cap],
    pid: u64,
) -> Result<(), &'static str> {
    if grants.len() > MAX_INHERIT {
        return Err("supervise: grant list longer than MAX_INHERIT");
    }
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let table = &mut *SERVICES.get();
            let Some(slot) = table.iter_mut().find(|s| !s.live) else {
                return Err("supervise: table full (MAX_SUPERVISED)");
            };
            let mut g = [Cap::EMPTY; MAX_INHERIT];
            g[..grants.len()].copy_from_slice(grants);
            *slot = Service {
                live: true,
                img_id,
                name,
                grants: g,
                grant_count: grants.len(),
                pid,
                dead_pid: 0,
                restarts: 0,
                abandoned: false,
            };
            Ok(())
        }
    })
}

/// Forget a supervised service entirely (test teardown, or a service
/// being retired deliberately).
pub fn unregister(pid: u64) -> bool {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let table = &mut *SERVICES.get();
            match table.iter_mut().find(|s| s.live && s.pid == pid) {
                Some(slot) => {
                    *slot = EMPTY;
                    true
                }
                None => false,
            }
        }
    })
}

/// Is `pid` a supervised service? (`proc::destroy` asks before
/// announcing a death, so ordinary teardown stays silent.)
pub fn is_supervised(pid: u64) -> bool {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (*SERVICES.get()).iter().any(|s| s.live && s.pid == pid) }
    })
}

/// Whether this supervisor owns a live OR just-dead instance. The
/// Process-cap finish syscall must not steal a dead driver's record
/// before `poll` has reaped it and replayed its grants.
pub fn owns_pid(pid: u64) -> bool {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            (*SERVICES.get())
                .iter()
                .any(|s| s.live && (s.pid == pid || s.dead_pid == pid))
        }
    })
}

/// Status of the supervised service whose CURRENT pid is `pid`, or
/// whose name matches — the suites assert on both.
pub fn status_of(name: &str) -> Option<ServiceStatus> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            (*SERVICES.get())
                .iter()
                .find(|s| s.live && s.name == name)
                .map(|s| ServiceStatus {
                    name: s.name,
                    pid: s.pid,
                    restarts: s.restarts,
                    abandoned: s.abandoned,
                })
        }
    })
}

/// Note that `pid` has died. Called by `proc::destroy` (a kill) and
/// by the exit path (a crash or a clean exit).
///
/// This only MARKS the service; it never spawns. Deaths are announced
/// from contexts that must not allocate frames or map pages — inside
/// `proc::destroy`, and potentially from an exit path holding
/// scheduler state — so the work happens later in [`poll`], at plain
/// thread context. Separating "notice" from "act" is the same rule
/// the console mirror follows for its wake (ADR-0027).
pub fn note_death(pid: u64) {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            if let Some(s) = (*SERVICES.get())
                .iter_mut()
                .find(|s| s.live && s.pid == pid)
            {
                s.pid = 0;
                s.dead_pid = pid;
            }
        }
    });
}

/// Restart every supervised service whose instance has died.
///
/// Returns the number of services restarted. Call from thread context
/// only: this spawns, which allocates frames and maps pages.
pub fn poll() -> usize {
    let mut restarted = 0;
    loop {
        // Take one pending restart at a time, ending the borrow before
        // spawning — `spawn_init` touches the process table, the frame
        // allocator, and the scheduler, none of which may be entered
        // with this table borrowed.
        let pending = without_interrupts(|| {
            // SAFETY: single reader under IF=0.
            unsafe {
                (*SERVICES.get())
                    .iter()
                    .enumerate()
                    .find(|(_, s)| s.live && s.pid == 0 && !s.abandoned)
                    .map(|(i, s)| {
                        (
                            i,
                            s.img_id,
                            s.name,
                            s.grants,
                            s.grant_count,
                            s.restarts,
                            s.dead_pid,
                        )
                    })
            }
        });
        let Some((idx, img_id, name, grants, count, restarts, dead_pid)) = pending else {
            return restarted;
        };

        // Reap the corpse's spawn record first (ADR-0025's debt). The
        // record table is bounded, and a service that restarts would
        // otherwise consume one entry per death until spawning became
        // impossible. A failing `forget` is not fatal: it only means
        // somebody already reaped it.
        if dead_pid != 0 {
            let _ = crate::spawn::forget(dead_pid);
            without_interrupts(|| {
                // SAFETY: single writer under IF=0.
                unsafe { (*SERVICES.get())[idx].dead_pid = 0 };
            });
        }

        if restarts >= MAX_RESTARTS {
            without_interrupts(|| {
                // SAFETY: single writer under IF=0.
                unsafe { (*SERVICES.get())[idx].abandoned = true };
            });
            error!(
                "supervise",
                "{name}: died {restarts} time(s) — giving up. The service stays OFFLINE, which is the same state the machine is in without the device; it is not pretended to be running"
            );
            continue;
        }

        // Replay the identical grant list. An Mmio window is a
        // physical base and a page count; an Endpoint is an index —
        // the same values describe the same authorities, and the
        // endpoint the clients hold is untouched.
        match crate::spawn::spawn_init(img_id, &grants[..count], None) {
            Ok(pid) => {
                without_interrupts(|| {
                    // SAFETY: single writer under IF=0.
                    unsafe {
                        let s = &mut (*SERVICES.get())[idx];
                        s.pid = pid;
                        s.restarts += 1;
                    }
                });
                restarted += 1;
                info!(
                    "supervise",
                    "{name}: RESTARTED as pid {pid} with its {count} original capability/capabilities replayed (restart {}/{MAX_RESTARTS}) — clients keep the endpoint they already hold",
                    restarts + 1
                );
            }
            Err(e) => {
                without_interrupts(|| {
                    // SAFETY: single writer under IF=0.
                    unsafe {
                        let s = &mut (*SERVICES.get())[idx];
                        s.restarts += 1;
                        if s.restarts >= MAX_RESTARTS {
                            s.abandoned = true;
                        }
                    }
                });
                error!("supervise", "{name}: respawn failed: {e}");
            }
        }
    }
}
