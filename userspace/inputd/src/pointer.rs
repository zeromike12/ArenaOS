//! EV_ABS / button decoding, independent of MMIO and compositor authority.
//! Virtio 1.2 section 5.8.6 uses Linux input_event type/code/value semantics.
pub const ABS_MAX: u32 = 32767;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub x: u16,
    pub y: u16,
    pub buttons: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pointer {
    current: Sample,
    published: Sample,
    dirty: bool,
}
impl Default for Pointer {
    fn default() -> Self {
        Self::new()
    }
}
impl Pointer {
    pub const fn new() -> Self {
        Self {
            current: Sample {
                x: 0,
                y: 0,
                buttons: 0,
            },
            published: Sample {
                x: 0,
                y: 0,
                buttons: 0,
            },
            dirty: false,
        }
    }
    /// Publish one coherent event batch at SYN_REPORT. Partial coordinate
    /// updates never cause separate hit tests before their matching batch.
    pub fn event(&mut self, kind: u16, code: u16, value: u32) -> Option<Sample> {
        match (kind, code) {
            (3, 0) => {
                self.current.x = value.min(ABS_MAX) as u16;
                self.dirty = true;
            }
            (3, 1) => {
                self.current.y = value.min(ABS_MAX) as u16;
                self.dirty = true;
            }
            (1, 272..=274) if value <= 1 => {
                let bit = 1 << (code - 272);
                if value == 1 {
                    self.current.buttons |= bit;
                } else {
                    self.current.buttons &= !bit;
                }
                self.dirty = true;
            }
            (0, 0) if self.dirty => {
                self.dirty = false;
                if self.current != self.published {
                    self.published = self.current;
                    return Some(self.current);
                }
            }
            _ => {}
        }
        None
    }
}
impl Sample {
    pub fn screen(self, width: u16, height: u16) -> Option<(i32, i32, u8)> {
        if width == 0 || height == 0 {
            return None;
        }
        Some((
            (u32::from(self.x) * u32::from(width - 1) / ABS_MAX) as i32,
            (u32::from(self.y) * u32::from(height - 1) / ABS_MAX) as i32,
            self.buttons,
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coherent_batch_buttons_and_exact_edges() {
        let mut p = Pointer::new();
        assert_eq!(p.event(3, 0, 32767), None);
        assert_eq!(p.event(3, 1, 32767), None);
        assert_eq!(p.event(1, 272, 1), None);
        let a = p.event(0, 0, 0).unwrap();
        assert_eq!(a.screen(800, 600), Some((799, 599, 1)));
        assert_eq!(p.event(0, 0, 0), None);
        p.event(1, 272, 0);
        assert_eq!(p.event(0, 0, 0).unwrap().buttons, 0);
        p.event(3, 0, u32::MAX);
        p.event(3, 1, 0);
        assert_eq!(
            p.event(0, 0, 0).unwrap().screen(800, 600),
            Some((799, 0, 0))
        );
    }
    #[test]
    fn unknown_events_and_button_repeat_never_fabricate_motion() {
        let mut p = Pointer::new();
        for e in [(2, 0, 200), (1, 272, 2), (3, 2, 123), (0, 1, 0)] {
            assert_eq!(p.event(e.0, e.1, e.2), None);
        }
        assert_eq!(p.event(0, 0, 0), None);
        assert_eq!(
            Sample {
                x: 0,
                y: 0,
                buttons: 0
            }
            .screen(0, 600),
            None
        );
    }
}
