//! Read-only CMOS real-time clock (Phase 11.5, ADR-0076 "Time").
//!
//! Wall time is data, never authority: it only stamps file times. The
//! clock is read through `CapObj::Rtc` (held by the filesystem service).
//! An update in progress is waited out (bounded), the registers are read
//! twice until they agree, and any out-of-range field (a missing or
//! uninitialized RTC) is reported as unknown rather than guessed.
use crate::arch::x86_64::{inb, outb};

fn reg(r: u8) -> u8 {
    // SAFETY: CMOS index/data ports; NMI-disable bit kept set while
    // selecting, as on every PC; reads have no side effects.
    unsafe {
        outb(0x70, 0x80 | r);
        inb(0x71)
    }
}

fn snapshot() -> [u8; 7] {
    [reg(0x00), reg(0x02), reg(0x04), reg(0x07), reg(0x08), reg(0x09), reg(0x32)]
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since 1970 (UTC as the RTC holds it), or `None` = unknown.
pub fn read_unix_seconds() -> Option<u64> {
    let mut a = [0u8; 7];
    let mut agreed = false;
    for _ in 0..8 {
        let mut spins = 0;
        while reg(0x0A) & 0x80 != 0 {
            spins += 1;
            if spins > 100_000 {
                return None;
            }
        }
        a = snapshot();
        if a == snapshot() {
            agreed = true;
            break;
        }
    }
    if !agreed {
        return None;
    }
    let b = reg(0x0B);
    let bcd = b & 0x04 == 0;
    let conv = |v: u8| -> Option<u8> {
        if !bcd {
            return Some(v);
        }
        let (hi, lo) = (v >> 4, v & 0x0f);
        (hi <= 9 && lo <= 9).then_some(hi * 10 + lo)
    };
    let sec = conv(a[0])?;
    let min = conv(a[1])?;
    let pm = a[2] & 0x80 != 0;
    let mut hour = conv(a[2] & 0x7f)?;
    if b & 0x02 == 0 {
        // 12-hour mode.
        if !(1..=12).contains(&hour) {
            return None;
        }
        hour %= 12;
        if pm {
            hour += 12;
        }
    }
    let day = conv(a[3])?;
    let month = conv(a[4])?;
    let year = conv(a[5])?;
    let century = conv(a[6]).filter(|c| (19..=21).contains(c)).unwrap_or(20);
    let full = i64::from(century) * 100 + i64::from(year);
    if sec > 59 || min > 59 || hour > 23 || !(1..=31).contains(&day) || !(1..=12).contains(&month)
        || !(2000..=2199).contains(&full)
    {
        return None;
    }
    let d = days(full, i64::from(month), i64::from(day));
    Some((d * 86_400 + i64::from(hour) * 3600 + i64::from(min) * 60 + i64::from(sec)) as u64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_days() {
        assert_eq!(super::days(1970, 1, 1), 0);
        assert_eq!(super::days(2000, 3, 1), 11_017);
        assert_eq!(super::days(2026, 10, 5), 20_731);
    }
}
