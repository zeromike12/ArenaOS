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
}
