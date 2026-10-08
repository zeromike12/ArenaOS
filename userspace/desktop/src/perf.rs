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
    pub bytes: [u8; 1024],
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
            bytes: [0; 1024],
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
    /// Emit through `log` in pieces the kernel's 256-byte debug write
    /// takes whole: each piece repeats the line's first token (its
    /// `[perf ...]` prefix) and breaks only between fields.
    pub fn emit(&self, mut log: impl FnMut(&[u8])) {
        let all = self.as_bytes();
        let body = all.strip_suffix(b"\n").unwrap_or(all);
        let cut = body.iter().position(|b| *b == b']').map_or(0, |i| i + 1);
        let (prefix, mut rest) = body.split_at(cut);
        while !rest.is_empty() {
            let room = 240 - prefix.len();
            let take = if rest.len() <= room {
                rest.len()
            } else {
                rest[..room]
                    .iter()
                    .rposition(|b| *b == b' ')
                    .filter(|i| *i > 0)
                    .unwrap_or(room)
            };
            let mut piece = [0u8; 256];
            piece[..prefix.len()].copy_from_slice(prefix);
            piece[prefix.len()..prefix.len() + take].copy_from_slice(&rest[..take]);
            piece[prefix.len() + take] = b'\n';
            log(&piece[..prefix.len() + take + 1]);
            rest = &rest[take..];
        }
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
        for _ in 0..300 {
            full.push(b"xxxxx");
        }
        assert_eq!(full.len, 1024);
    }
    #[test]
    fn long_lines_are_emitted_in_prefixed_pieces() {
        extern crate std;
        let mut l = Line::new();
        l.push(b"[perf desktop]");
        for _ in 0..30 {
            l.stat(b"render", Stat::ZERO);
        }
        l.push(b"\n");
        let mut pieces = std::vec::Vec::new();
        l.emit(|p| pieces.push(p.to_vec()));
        assert!(pieces.len() > 1);
        let mut fields = 0;
        for p in &pieces {
            assert!(p.len() <= 256 && p.starts_with(b"[perf desktop] ") && p.ends_with(b"\n"));
            fields += p.windows(7).filter(|w| w == b"render=").count();
        }
        assert_eq!(fields, 30);
    }
}
