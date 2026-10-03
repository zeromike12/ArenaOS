//! Opt-in latency instrumentation. Compiled in only when the image is built
//! with `ARENA_PERF=1` in the environment; otherwise `ENABLED` is false and
//! every probe folds away. Probes measure with the monotonic microsecond
//! clock and report aggregated statistics on the debug log. They observe
//! timing only: no authority, policy or rendering depends on them.

pub const ENABLED: bool = match option_env!("ARENA_PERF") {
    Some(v) => !v.is_empty() && !matches!(v.as_bytes(), b"0"),
    None => false,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub count: u64,
    pub total_us: u64,
    pub max_us: u64,
}

impl Stat {
    pub const ZERO: Stat = Stat {
        count: 0,
        total_us: 0,
        max_us: 0,
    };
    pub fn add(&mut self, us: u64) {
        self.count += 1;
        self.total_us = self.total_us.saturating_add(us);
        self.max_us = self.max_us.max(us);
    }
    pub fn mean_us(&self) -> u64 {
        self.total_us.checked_div(self.count).unwrap_or(0)
    }
}

/// Bounded line formatter for `[perf]` records: `name=count/mean/max`.
pub struct Line {
    pub bytes: [u8; 240],
    pub len: usize,
}

impl Default for Line {
    fn default() -> Self {
        Self::new()
    }
}

impl Line {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 240],
            len: 0,
        }
    }
    pub fn push(&mut self, s: &[u8]) {
        let n = s.len().min(self.bytes.len() - self.len);
        self.bytes[self.len..self.len + n].copy_from_slice(&s[..n]);
        self.len += n;
    }
    pub fn number(&mut self, mut v: u64) {
        let mut d = [0u8; 20];
        let mut n = 0;
        loop {
            d[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for i in (0..n).rev() {
            self.push(core::slice::from_ref(&d[i]));
        }
    }
    pub fn stat(&mut self, name: &[u8], s: Stat) {
        self.push(b" ");
        self.push(name);
        self.push(b"=");
        self.number(s.count);
        self.push(b"/");
        self.number(s.mean_us());
        self.push(b"/");
        self.number(s.max_us);
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stat_and_line_format() {
        let mut s = Stat::ZERO;
        s.add(10);
        s.add(30);
        assert_eq!((s.count, s.mean_us(), s.max_us), (2, 20, 30));
        let mut l = Line::new();
        l.push(b"[perf]");
        l.stat(b"render", s);
        assert_eq!(l.as_bytes(), b"[perf] render=2/20/30");
        let mut full = Line::new();
        for _ in 0..100 {
            full.push(b"xxxxx");
        }
        assert_eq!(full.len, 240);
    }
}
