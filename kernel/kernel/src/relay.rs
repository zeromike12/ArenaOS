//! The IRQ relay table (M5.1, ADR-0021): interrupt → notification bridge.
//!
//! IDT vectors 48..63 are *relay vectors*: their stubs (`idt.rs`) EOI both
//! interrupt controllers and call [`handle`], which looks up the registered
//! `(notification, badge)` pair and delivers through `ipc::notify` — the
//! same merged-badge primitive userspace already waits on with `SYS_WAIT`
//! (ADR-0018). A userspace driver's entire interrupt story therefore reads:
//! arm the device (M5.2's `SYS_IRQ_RELAY` programs the MSI-X entry), then
//! `SYS_WAIT` on a notification cap; no ring-3 code ever runs in interrupt
//! context and the kernel's ISR footprint stays one table lookup.
//!
//! Registration/release are kernel-side APIs (the M5 suites and, from M5.2,
//! the `SYS_IRQ_RELAY` handler). The table is deliberately tiny and static —
//! 16 slots, one per relay vector — like every other bounded kernel object.

use crate::arch::x86_64::idt;
use crate::ipc;
use crate::log::{log_info, log_warn};
use crate::sync::{SyncCell, without_interrupts};

/// One relay registration. `deliveries` counts every stub entry for the
/// vector (registered or spurious) — the suites assert on it.
#[derive(Clone, Copy)]
struct RelaySlot {
    live: bool,
    nid: u32,
    badge: u64,
    deliveries: u64,
}

const EMPTY: RelaySlot = RelaySlot {
    live: false,
    nid: 0,
    badge: 0,
    deliveries: 0,
};

/// One slot per relay vector (48..63).
static RELAYS: SyncCell<[RelaySlot; idt::RELAY_VECTOR_COUNT]> =
    SyncCell::new([EMPTY; idt::RELAY_VECTOR_COUNT]);

/// Relay-vector → slot index. The vector range itself is owned by `idt.rs`
/// (it installs and audits the stubs); this module never widens it.
fn idx(vector: u64) -> Result<usize, &'static str> {
    let base = idt::RELAY_VECTOR_BASE as u64;
    if !idt::is_relay_vector(vector as usize) {
        return Err("relay: vector outside the relay range 48..63");
    }
    Ok((vector - base) as usize)
}

/// Register: every delivery of `vector` becomes `ipc::notify(nid, badge)`.
/// One registration per vector — re-registering a live relay is refused
/// (release it first; a silent overwrite would strand the old waiter).
/// `badge` must be nonzero (zero is the empty word in the merged-flag
/// protocol, ADR-0018). Call with IF=0 or from a syscall (the dispatcher
/// runs IF=0).
pub fn register(vector: u64, nid: u32, badge: u64) -> Result<(), &'static str> {
    let i = idx(vector)?;
    if badge == 0 {
        return Err("relay: badge must be nonzero");
    }
    without_interrupts(|| {
        // SAFETY: single writer under IF=0 (SyncCell contract).
        unsafe {
            let s = &mut (*RELAYS.get())[i];
            if s.live {
                return Err("relay: vector already registered");
            }
            s.live = true;
            s.nid = nid;
            s.badge = badge;
            s.deliveries = 0;
        }
        log_info!(
            "relay",
            "vector {vector} → notification {nid} badge {badge:#x}"
        );
        Ok(())
    })
}

/// Release a relay registration. Releasing an unregistered vector is
/// refused — the caller's view of the table should match the table.
pub fn release(vector: u64) -> Result<(), &'static str> {
    let i = idx(vector)?;
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let s = &mut (*RELAYS.get())[i];
            if !s.live {
                return Err("relay: vector not registered");
            }
            *s = RelaySlot {
                live: false,
                nid: 0,
                badge: 0,
                deliveries: s.deliveries, // the count is history, not state
            };
        }
        Ok(())
    })
}

/// The relay stub's Rust half calls this AFTER the dual EOI, in interrupt
/// context (IF=0): count the delivery, then notify the registered pair.
///
/// Never fatal and never blocking beyond `ipc::notify`'s own wake path —
/// interrupt context must stay forward-progressing. A spurious delivery
/// (vector not registered: e.g. an MSI-X entry fires before its relay is
/// armed, or after release) is counted and warned; the EOI already
/// happened in the stub, so the controller state is clean regardless.
pub fn handle(vector: u64) {
    // SAFETY: interrupt context, IF=0; single accessor per slot (the
    // relay range is fixed-delivery, one vector in service at a time on
    // this CPU; cross-CPU registration races are excluded by IF=0 on
    // the registering side plus the boot-order contract).
    let (nid, badge) = unsafe {
        let Ok(i) = idx(vector) else {
            log_warn!("relay", "handle: vector {vector} outside the relay range");
            return;
        };
        let s = &mut (*RELAYS.get())[i];
        s.deliveries += 1;
        if !s.live {
            log_warn!("relay", "spurious delivery on unregistered vector {vector}");
            return;
        }
        (s.nid, s.badge)
    };
    if let Err(e) = ipc::notify(nid, badge) {
        log_warn!("relay", "notify failed for vector {vector}: {e}");
    }
}

/// Stub-entry count for `vector` since registration (spurious deliveries
/// on an unregistered vector count too; `release` preserves the count).
/// The m5 suite asserts the live-interrupt path through this.
pub fn delivery_count(vector: u64) -> u64 {
    let Ok(i) = idx(vector) else {
        return 0;
    };
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (*RELAYS.get())[i].deliveries }
    })
}

/// Whether `vector` currently has a live registration.
pub fn registered(vector: u64) -> bool {
    let Ok(i) = idx(vector) else {
        return false;
    };
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (*RELAYS.get())[i].live }
    })
}
