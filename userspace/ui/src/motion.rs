//! Integer interpolation driven by a caller's monotonic microsecond clock.
//! The caller owns frame scheduling. Sampling never sleeps or spins.
pub const OPEN_US: u64 = 120_000;
pub const CLOSE_US: u64 = 90_000;
pub const FOCUS_US: u64 = 80_000;
pub const DOCK_US: u64 = 140_000;
pub const FRAME_US: u64 = 20_000;
const UNIT: u64 = 65_536;
/// Opaque channel interpolation; this does not promise compositor alpha.
pub fn color(from: u32, to: u32, amount: i32) -> u32 {
    let amount = i64::from(amount.clamp(0, UNIT as i32));
    let mut result = 0xff00_0000;
    for shift in [0, 8, 16] {
        let a = i64::from((from >> shift) & 255);
        let b = i64::from((to >> shift) & 255);
        result |= ((a + (b - a) * amount / UNIT as i64) as u32) << shift;
    }
    result
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    Smooth,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Motion {
    from: i32,
    target: i32,
    start: u64,
    duration: u64,
    easing: Easing,
}
impl Motion {
    pub fn target(self) -> i32 {
        self.target
    }
    pub const fn fixed(value: i32) -> Self {
        Self {
            from: value,
            target: value,
            start: 0,
            duration: 0,
            easing: Easing::Linear,
        }
    }
    pub fn sample(self, now: u64) -> i32 {
        if self.duration == 0 || now.saturating_sub(self.start) >= self.duration {
            return self.target;
        }
        let t = ((u128::from(now.saturating_sub(self.start)) * u128::from(UNIT))
            / u128::from(self.duration)) as u64;
        let t = match self.easing {
            Easing::Linear => t,
            Easing::Smooth => t * t * (3 * UNIT - 2 * t) / (UNIT * UNIT),
        };
        let delta = i64::from(self.target) - i64::from(self.from);
        (i64::from(self.from) + delta * (t as i64) / (UNIT as i64)) as i32
    }
    pub fn retarget(&mut self, target: i32, now: u64, duration: u64, easing: Easing) {
        *self = Self {
            from: self.sample(now),
            target,
            start: now,
            duration,
            easing,
        };
    }
    pub fn active(self, now: u64) -> bool {
        self.from != self.target && now.saturating_sub(self.start) < self.duration
    }
    pub fn next_frame(self, now: u64) -> Option<u64> {
        self.active(now).then(|| {
            now.saturating_add(FRAME_US)
                .min(self.start.saturating_add(self.duration))
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_retarget_and_disabled_animation() {
        let mut m = Motion::fixed(-20);
        m.retarget(100, 1000, OPEN_US, Easing::Smooth);
        assert_eq!(m.sample(999), -20);
        assert_eq!(m.sample(61_000), 40);
        assert_eq!(m.sample(121_000), 100);
        assert!(m.active(120_999));
        assert_eq!(m.next_frame(120_999), Some(121_000));
        m.retarget(-100, 61_000, 0, Easing::Smooth);
        assert_eq!(m.sample(61_000), -100);
        assert!(!m.active(61_000));
    }
    #[test]
    fn monotone_extreme_geometry_and_clock_end() {
        let mut m = Motion::fixed(i32::MIN);
        m.retarget(i32::MAX, 0, OPEN_US, Easing::Smooth);
        let mut prev = i32::MIN;
        for t in (0..=OPEN_US).step_by(1000) {
            let v = m.sample(t);
            assert!(v >= prev);
            prev = v;
        }
        m.retarget(0, u64::MAX - 5, 10, Easing::Linear);
        assert_eq!(m.next_frame(u64::MAX - 2), Some(u64::MAX));
    }
}
