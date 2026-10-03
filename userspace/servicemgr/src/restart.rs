//! The bounded runtime transition used by the real ring-3 manager.
//! A badge is not a pid or authority: callers verify/reap through the
//! held Process cap BEFORE invoking `exited`. The state machine makes
//! budget and unexpected-exit policy testable without privileged calls.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    WrongChild,
    NoLiveChild,
    BudgetExhausted,
    InvalidPolicy,
}

pub struct Restart {
    live: Option<u64>,
    used: u8,
    limit: u8,
    backoff_us: u64,
    abandoned: bool,
}

impl Restart {
    pub fn new(pid: u64, limit: u8, backoff_us: u64) -> Result<Self, Refusal> {
        if pid == 0 || limit == 0 || limit > 3 || backoff_us == 0 || backoff_us > 1_000_000 {
            return Err(Refusal::InvalidPolicy);
        }
        Ok(Self {
            live: Some(pid),
            used: 0,
            limit,
            backoff_us,
            abandoned: false,
        })
    }
    /// Called only after `SYS_PROC_FINISH` has actually retired the
    /// unique held Process cap. The manifest controls the budget.
    pub fn exited(&mut self, pid: u64) -> Result<u64, Refusal> {
        if self.live != Some(pid) {
            return Err(Refusal::WrongChild);
        }
        self.live = None;
        if self.used == self.limit {
            self.abandoned = true;
            return Err(Refusal::BudgetExhausted);
        }
        self.used += 1;
        Ok(self.backoff_us)
    }
    pub fn ready(&mut self, pid: u64) -> Result<(), Refusal> {
        if self.abandoned {
            return Err(Refusal::BudgetExhausted);
        }
        if self.live.is_some() || pid == 0 {
            return Err(Refusal::WrongChild);
        }
        if self.used == 0 {
            return Err(Refusal::NoLiveChild);
        }
        self.live = Some(pid);
        Ok(())
    }
    pub fn live(&self) -> Option<u64> {
        self.live
    }
    pub fn restarts(&self) -> u8 {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_child_and_bounded_restart_budget() {
        let mut p = Restart::new(91, 3, 50_000).unwrap();
        assert_eq!(p.exited(90), Err(Refusal::WrongChild));
        assert_eq!(p.live(), Some(91));
        for n in 0..3 {
            let old = 91 + n;
            assert_eq!(p.exited(old), Ok(50_000));
            assert_eq!(p.exited(old), Err(Refusal::WrongChild));
            assert_eq!(p.ready(old + 1), Ok(()));
            assert_eq!(p.restarts(), (n + 1) as u8);
        }
        assert_eq!(p.exited(94), Err(Refusal::BudgetExhausted));
        assert_eq!(p.live(), None);
        assert_eq!(p.ready(95), Err(Refusal::BudgetExhausted));
    }
    #[test]
    fn invalid_policy_and_premature_ready_refused() {
        for (pid, limit, delay) in [
            (0, 1, 1),
            (1, 0, 1),
            (1, 4, 1),
            (1, 1, 0),
            (1, 1, 1_000_001),
        ] {
            assert!(matches!(
                Restart::new(pid, limit, delay),
                Err(Refusal::InvalidPolicy)
            ));
        }
        let mut p = Restart::new(5, 1, 100).unwrap();
        assert_eq!(p.ready(6), Err(Refusal::WrongChild));
        assert_eq!(p.live(), Some(5));
    }
}
