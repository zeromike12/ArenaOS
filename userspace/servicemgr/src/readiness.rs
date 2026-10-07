//! Bounded driver-readiness badge contract for ADR-0037.
//! A notification is a signal from a boot-granted WRITE cap, not a
//! guessed sleep or a text log. A timeout wins even if badges merge
//! with it, so a late driver never becomes a false-ready dependency.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidMask,
    Deadline,
    UnexpectedBadge,
}

/// Remove a wake-only sideband while retaining it for its independent
/// consumer. The sideband is never presented to [`Gate`] as readiness.
pub fn separate_sideband(observed: u64, sideband: u64) -> (u64, bool) {
    (observed & !sideband, observed & sideband != 0)
}

#[derive(Clone, Copy, Debug)]
pub struct Gate {
    required: u64,
    deadline: u64,
    seen: u64,
}
impl Gate {
    pub fn new(required: u64, deadline: u64) -> Result<Self, Error> {
        if required == 0 || deadline == 0 || required & deadline != 0 {
            return Err(Error::InvalidMask);
        }
        Ok(Self {
            required,
            deadline,
            seen: 0,
        })
    }

    /// Return true only when *all* required distinct badges arrived
    /// before the deadline. Do not mutate state on a refused signal.
    pub fn observe(&mut self, badges: u64) -> Result<bool, Error> {
        if badges & self.deadline != 0 {
            return Err(Error::Deadline);
        }
        if badges == 0 || badges & !(self.deadline | self.required) != 0 {
            return Err(Error::UnexpectedBadge);
        }
        self.seen |= badges;
        Ok(self.seen & self.required == self.required)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const NET: u64 = 1;
    const RNG: u64 = 2;
    const TIMEOUT: u64 = 4;
    #[test]
    fn requires_both_independent_driver_signals() {
        let mut g = Gate::new(NET | RNG, TIMEOUT).unwrap();
        assert_eq!(g.observe(NET), Ok(false));
        assert_eq!(g.observe(NET), Ok(false)); // duplicate is not a second driver
        assert_eq!(g.observe(RNG), Ok(true));
    }
    #[test]
    fn late_coalesced_ready_is_still_too_late() {
        let mut g = Gate::new(NET | RNG, TIMEOUT).unwrap();
        assert_eq!(g.observe(NET | RNG | TIMEOUT), Err(Error::Deadline));
        assert_eq!(g.observe(NET), Ok(false)); // refusal did not mutate
    }
    #[test]
    fn timeout_unknown_badge_and_invalid_masks_fail_closed() {
        assert!(matches!(Gate::new(0, TIMEOUT), Err(Error::InvalidMask)));
        assert!(matches!(Gate::new(NET, NET), Err(Error::InvalidMask)));
        let mut g = Gate::new(NET | RNG, TIMEOUT).unwrap();
        assert_eq!(g.observe(8), Err(Error::UnexpectedBadge));
        assert_eq!(g.observe(0), Err(Error::UnexpectedBadge));
        assert_eq!(g.observe(TIMEOUT), Err(Error::Deadline));
    }

    #[test]
    fn coalesced_apb1_wake_is_preserved_but_cannot_complete_readiness() {
        const APB1: u64 = 1 << 22;
        let mut g = Gate::new(NET | RNG, TIMEOUT).unwrap();

        let (badges, apb1) = separate_sideband(NET | APB1, APB1);
        assert!(apb1);
        assert_eq!(badges, NET);
        assert_eq!(g.observe(badges), Ok(false));

        let (badges, apb1) = separate_sideband(APB1, APB1);
        assert!(apb1);
        assert_eq!(badges, 0); // production skips Gate::observe on a sideband-only wake
        assert_eq!(g.observe(RNG), Ok(true)); // only the missing real driver completes it
    }
}
