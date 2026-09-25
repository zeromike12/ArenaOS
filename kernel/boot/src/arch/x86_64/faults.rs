//! Controlled fault injection — the test side of the M2.1 exception path.
//!
//! The M2 suite must *prove* exception delivery, diagnosis, and recovery
//! without dying, so tests can arm an expected vector, trigger a real
//! faulting instruction, and continue. The protocol (all state lives in
//! this module, consumed by `arena_exception_handler` in `idt.rs`):
//!
//! 1. Test calls [`arm(vector)`].
//! 2. Test runs a fault-site helper which first writes its own resume
//!    address into [`RESUME`] (inline asm, `sym` operand) and *then*
//!    executes the faulting instruction.
//! 3. The exception handler calls [`take_expected(vector)`]: on a match it
//!    disarms, [`record`]s what was actually observed (vector, error code,
//!    CR2), and rewrites the interrupted frame's RIP to the resume address;
//!    the stub's `iretq` returns execution to the test.
//! 4. Test asserts on [`observed()`] — the *measured* delivery, never the
//!    expectation (an unmatched or unarmed fault still takes the full
//!    M1 diagnostics-and-halt path, so this cannot mask real bugs).
//!
//! Single-CPU, IF=0 boot context: `SyncCell` access per its contract.

use super::SyncCell;

/// What the exception handler actually saw for the most recent armed fault.
#[derive(Clone, Copy)]
pub struct ObservedFault {
    pub valid: bool,
    pub vector: u64,
    pub error_code: u64,
    /// CR2 at fault time (meaningful for #PF; recorded as 0 otherwise).
    pub cr2: u64,
}

/// Armed expectation: expected vector + 1 (0 = unarmed).
static ARMED: SyncCell<u16> = SyncCell::new(0);

/// Resume address written by the fault site just before faulting. Public
/// symbol so fault-site inline asm can address it with `sym`.
pub static RESUME: SyncCell<u64> = SyncCell::new(0);

static OBSERVED: SyncCell<ObservedFault> = SyncCell::new(ObservedFault {
    valid: false,
    vector: 0,
    error_code: 0,
    cr2: 0,
});

/// Arm `vector` as the next expected fault (clears any previous record).
pub fn arm(vector: u8) {
    // SAFETY: single writer, sequential boot context (SyncCell contract).
    unsafe {
        OBSERVED.get().write(ObservedFault {
            valid: false,
            vector: 0,
            error_code: 0,
            cr2: 0,
        });
        RESUME.get().write(0);
        ARMED.get().write(vector as u16 + 1);
    }
}

/// Forget any armed expectation (test teardown; keeps a mis-armed state from
/// leaking into later code).
pub fn disarm() {
    // SAFETY: SyncCell contract as above.
    unsafe { ARMED.get().write(0) };
}

/// Exception-handler side: if `vector` matches the armed expectation,
/// disarm and return the fault site's resume RIP. `None` = unexpected fault
/// (handler proceeds to diagnostics + halt). A zero/absent resume address
/// is treated as unexpected rather than jumping to address 0.
pub fn take_expected(vector: u64) -> Option<u64> {
    // SAFETY: SyncCell contract; handler runs with IF=0 on the CPU that
    // armed the expectation.
    unsafe {
        if *ARMED.get() as u64 == vector + 1 && vector < 32 {
            *ARMED.get() = 0;
            let resume = *RESUME.get();
            if resume != 0 {
                return Some(resume);
            }
        }
        None
    }
}

/// Exception-handler side: store what was actually delivered.
pub fn record(vector: u64, error_code: u64, cr2: u64) {
    // SAFETY: SyncCell contract as above.
    unsafe {
        OBSERVED.get().write(ObservedFault {
            valid: true,
            vector,
            error_code,
            cr2,
        });
    }
}

/// Test side: the recorded delivery facts.
pub fn observed() -> ObservedFault {
    // SAFETY: copy-out of one small struct; SyncCell contract.
    unsafe { *OBSERVED.get() }
}
