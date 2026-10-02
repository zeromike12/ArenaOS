//! Bounded, allocation-free compositor *policy model*, not a running server.
//!
//! Input IDs must be derived by a future server from receiver-verified held
//! caps. Neither the numeric `owner` nor `backing` passed to this pure model
//! confers authority by itself. No client can call these methods across IPC;
//! do not connect untrusted requests before ADR-0057's cap/lifecycle bridge.
#![no_std]

pub mod render;
pub mod wire;

use arena_gfxkit::{Canvas, Rect};

pub const MAX_OWNERS: usize = 4;
pub const MAX_SURFACES: usize = 6;
pub const MAX_KEYS: usize = 16;
pub const MAX_SURFACE_WIDTH: u16 = 320;
pub const MAX_SURFACE_HEIGHT: u16 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Invalid,
    NoSpace,
    Stale,
    NotOwner,
    BadBacking,
    QueueFull,
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub ascii: u8,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub handle: u64,
    pub backing: u32,
    pub owner: u64,
    pub width: u16,
    pub height: u16,
    pub x: i32,
    pub y: i32,
    pub z: u64,
}

#[derive(Clone, Copy)]
struct Owner {
    id: u64,
    keys: [Key; MAX_KEYS],
    read: u8,
    len: u8,
}
const EMPTY_KEY: Key = Key {
    ascii: 0,
    pressed: false,
};
const EMPTY_OWNER: Owner = Owner {
    id: 0,
    keys: [EMPTY_KEY; MAX_KEYS],
    read: 0,
    len: 0,
};

/// `pixels` is an immutable, already-authorized view of one SharedRegion.
/// This type has no ability to map memory or nominate physical pages.
pub struct Source<'a> {
    pub backing: u32,
    pub pixels: &'a [u32],
    pub stride: usize,
}

/// Slots are not handles: monotonically advancing handles prevent stale
/// client requests from reviving a deleted surface after slot reuse.
pub struct State {
    owners: [Owner; MAX_OWNERS],
    surfaces: [Option<Surface>; MAX_SURFACES],
    next_handle: u64,
    next_z: u64,
    focused: Option<u64>,
    dropped_keys: u64,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub const fn new() -> Self {
        Self {
            owners: [EMPTY_OWNER; MAX_OWNERS],
            surfaces: [None; MAX_SURFACES],
            next_handle: 1,
            next_z: 1,
            focused: None,
            dropped_keys: 0,
        }
    }

    pub fn register_owner(&mut self, verified_owner: u64) -> Result<(), Refusal> {
        if verified_owner == 0 || self.owners.iter().any(|o| o.id == verified_owner) {
            return Err(Refusal::Invalid);
        }
        let owner = self
            .owners
            .iter_mut()
            .find(|o| o.id == 0)
            .ok_or(Refusal::NoSpace)?;
        *owner = Owner {
            id: verified_owner,
            ..EMPTY_OWNER
        };
        Ok(())
    }

    /// Only a verified lifecycle notification may call this: cap deletion
    /// alone is not revocation of copies. Forget queued input and focus
    /// atomically before the owner's slot is made reusable.
    pub fn retire_owner(&mut self, verified_owner: u64) -> Result<(), Refusal> {
        let owner = self
            .owners
            .iter_mut()
            .find(|o| o.id == verified_owner && o.id != 0)
            .ok_or(Refusal::NotOwner)?;
        *owner = EMPTY_OWNER;
        for surface in &mut self.surfaces {
            if surface.is_some_and(|s| s.owner == verified_owner) {
                if self.focused == surface.map(|s| s.handle) {
                    self.focused = None;
                }
                *surface = None;
            }
        }
        Ok(())
    }

    pub fn create(
        &mut self,
        owner: u64,
        backing: u32,
        width: u16,
        height: u16,
        x: i32,
        y: i32,
    ) -> Result<u64, Refusal> {
        if !self.owners.iter().any(|o| o.id != 0 && o.id == owner) {
            return Err(Refusal::NotOwner);
        }
        if backing == 0
            || width == 0
            || height == 0
            || width > MAX_SURFACE_WIDTH
            || height > MAX_SURFACE_HEIGHT
        {
            return Err(Refusal::Invalid);
        }
        if self.surfaces.iter().flatten().any(|s| s.backing == backing) {
            return Err(Refusal::BadBacking);
        }
        let slot = self
            .surfaces
            .iter()
            .position(Option::is_none)
            .ok_or(Refusal::NoSpace)?;
        let handle = self.next_handle;
        let z = self.next_z;
        let next_handle = handle.checked_add(1).ok_or(Refusal::Exhausted)?;
        let next_z = z.checked_add(1).ok_or(Refusal::Exhausted)?;
        self.surfaces[slot] = Some(Surface {
            handle,
            backing,
            owner,
            width,
            height,
            x,
            y,
            z,
        });
        self.next_handle = next_handle;
        self.next_z = next_z;
        Ok(handle)
    }

    fn owned_slot(&self, owner: u64, backing: u32, handle: u64) -> Result<usize, Refusal> {
        let (i, s) = self
            .surfaces
            .iter()
            .enumerate()
            .find_map(|(i, s)| s.filter(|s| s.handle == handle).map(|s| (i, s)))
            .ok_or(Refusal::Stale)?;
        if s.owner != owner || s.backing != backing {
            return Err(Refusal::NotOwner);
        }
        Ok(i)
    }

    pub fn move_to(
        &mut self,
        owner: u64,
        backing: u32,
        handle: u64,
        x: i32,
        y: i32,
    ) -> Result<(), Refusal> {
        let i = self.owned_slot(owner, backing, handle)?;
        let s = self.surfaces[i].as_mut().unwrap();
        s.x = x;
        s.y = y;
        Ok(())
    }

    pub fn focus(&mut self, owner: u64, backing: u32, handle: u64) -> Result<(), Refusal> {
        self.owned_slot(owner, backing, handle)?;
        self.focused = Some(handle);
        Ok(())
    }

    pub fn raise(&mut self, owner: u64, backing: u32, handle: u64) -> Result<(), Refusal> {
        let i = self.owned_slot(owner, backing, handle)?;
        let z = self.next_z;
        self.next_z = z.checked_add(1).ok_or(Refusal::Exhausted)?;
        self.surfaces[i].as_mut().unwrap().z = z;
        Ok(())
    }

    pub fn destroy(&mut self, owner: u64, backing: u32, handle: u64) -> Result<(), Refusal> {
        let i = self.owned_slot(owner, backing, handle)?;
        self.surfaces[i] = None;
        if self.focused == Some(handle) {
            self.focused = None;
        }
        Ok(())
    }

    pub fn focused(&self) -> Option<u64> {
        self.focused
    }
    pub fn dropped_keys(&self) -> u64 {
        self.dropped_keys
    }
    pub fn surface_count(&self) -> usize {
        self.surfaces.iter().flatten().count()
    }

    /// The caller must have checked the input producer cap *before* this
    /// method. There is no inferred authority from a key value or a PID.
    pub fn route_verified_key(&mut self, key: Key) -> Result<(), Refusal> {
        if !key.ascii.is_ascii_graphic() && key.ascii != b' ' {
            return Err(Refusal::Invalid);
        }
        let Some(handle) = self.focused else {
            self.dropped_keys = self.dropped_keys.saturating_add(1);
            return Err(Refusal::Stale);
        };
        let owner_id = self
            .surfaces
            .iter()
            .flatten()
            .find(|s| s.handle == handle)
            .map(|s| s.owner)
            .ok_or(Refusal::Stale)?;
        let owner = self
            .owners
            .iter_mut()
            .find(|o| o.id != 0 && o.id == owner_id)
            .ok_or(Refusal::NotOwner)?;
        if owner.len as usize == MAX_KEYS {
            self.dropped_keys = self.dropped_keys.saturating_add(1);
            return Err(Refusal::QueueFull);
        }
        let index = (owner.read as usize + owner.len as usize) % MAX_KEYS;
        owner.keys[index] = key;
        owner.len += 1;
        Ok(())
    }

    pub fn pop_key(&mut self, owner_id: u64) -> Result<Option<Key>, Refusal> {
        let owner = self
            .owners
            .iter_mut()
            .find(|o| o.id != 0 && o.id == owner_id)
            .ok_or(Refusal::NotOwner)?;
        if owner.len == 0 {
            return Ok(None);
        }
        let key = owner.keys[owner.read as usize];
        owner.read = ((owner.read as usize + 1) % MAX_KEYS) as u8;
        owner.len -= 1;
        Ok(Some(key))
    }

    /// Preflight *all* visible source geometries before clearing the target:
    /// malformed backing or duplicate description must not publish a
    /// partial frame. The service must independently validate held caps.
    pub fn compose(&self, target: &mut Canvas<'_>, sources: &[Source<'_>]) -> Result<(), Refusal> {
        for surface in self.surfaces.iter().flatten() {
            let mut matches = sources.iter().filter(|s| s.backing == surface.backing);
            let src = matches.next().ok_or(Refusal::BadBacking)?;
            if matches.next().is_some()
                || src.stride < surface.width as usize
                || src.stride > arena_gfxkit::MAX_WIDTH
                || src
                    .stride
                    .checked_mul(surface.height as usize)
                    .is_none_or(|n| n > src.pixels.len())
            {
                return Err(Refusal::BadBacking);
            }
        }
        target.clear(0x00_18_20_35);
        let (w, h) = target.size();
        target.fill_rect(
            Rect {
                x: 0,
                y: 0,
                width: w as u32,
                height: 12.min(h) as u32,
            },
            0x00_22_33_55,
        );
        let mut previous_z = 0;
        for _ in 0..self.surface_count() {
            let surface = self
                .surfaces
                .iter()
                .flatten()
                .filter(|s| s.z > previous_z)
                .min_by_key(|s| s.z)
                .ok_or(Refusal::Invalid)?;
            let src = sources
                .iter()
                .find(|s| s.backing == surface.backing)
                .ok_or(Refusal::BadBacking)?;
            target
                .blit(
                    src.pixels,
                    surface.width as usize,
                    surface.height as usize,
                    src.stride,
                    surface.x,
                    surface.y,
                )
                .map_err(|_| Refusal::BadBacking)?;
            previous_z = surface.z;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_overlap_z_order_clipping_and_stale_reuse() {
        let mut s = State::new();
        s.register_owner(41).unwrap();
        s.register_owner(42).unwrap();
        let a = s.create(41, 1001, 4, 3, -1, 1).unwrap();
        let b = s.create(42, 1002, 4, 3, 1, 2).unwrap();
        let one = [0x00e33542; 12];
        let two = [0x002ec771; 12];
        let sources = [
            Source {
                backing: 1001,
                pixels: &one,
                stride: 4,
            },
            Source {
                backing: 1002,
                pixels: &two,
                stride: 4,
            },
        ];
        let mut pixels = [0u32; 7 * 6];
        {
            let mut target = Canvas::new(&mut pixels, 7, 6, 7).unwrap();
            s.compose(&mut target, &sources).unwrap();
        }
        assert_eq!(pixels[7], 0xe33542); // negative-x clipping
        assert_eq!(pixels[2 * 7 + 1], 0x2ec771); // later z wins overlap
        assert_eq!(s.move_to(42, 1001, b, 0, 0), Err(Refusal::NotOwner));
        s.raise(41, 1001, a).unwrap();
        {
            let mut target = Canvas::new(&mut pixels, 7, 6, 7).unwrap();
            s.compose(&mut target, &sources).unwrap();
        }
        assert_eq!(pixels[2 * 7 + 1], 0xe33542); // raised owner now on top
        s.destroy(41, 1001, a).unwrap();
        let c = s.create(41, 1003, 1, 1, 2, 2).unwrap();
        assert_ne!(c, a);
        assert_eq!(s.move_to(41, 1001, a, 0, 0), Err(Refusal::Stale));
        assert_eq!(s.move_to(41, 1003, c, i32::MAX, i32::MAX), Ok(()));
    }

    #[test]
    fn authority_preflight_and_capacity_never_mutate_on_refusal() {
        let mut s = State::new();
        for id in 1..=4 {
            s.register_owner(id).unwrap();
        }
        assert_eq!(s.register_owner(5), Err(Refusal::NoSpace));
        assert_eq!(s.create(999, 9, 1, 1, 0, 0), Err(Refusal::NotOwner));
        assert_eq!(s.create(1, 9, 321, 1, 0, 0), Err(Refusal::Invalid));
        let first = s.create(1, 9, 2, 2, 0, 0).unwrap();
        assert_eq!(s.create(2, 9, 2, 2, 0, 0), Err(Refusal::BadBacking));
        for id in 10..15 {
            s.create(1, id, 1, 1, 0, 0).unwrap();
        }
        assert_eq!(s.surface_count(), 6);
        assert_eq!(s.create(1, 15, 1, 1, 0, 0), Err(Refusal::NoSpace));
        assert_eq!(s.focus(2, 9, first), Err(Refusal::NotOwner));
        let mut pixels = [0x123u32; 8 * 8];
        let mut target = Canvas::new(&mut pixels, 8, 8, 8).unwrap();
        assert_eq!(s.compose(&mut target, &[]), Err(Refusal::BadBacking));
        assert_eq!(pixels, [0x123; 64]); // no partial frame
    }

    #[test]
    fn repeated_owner_retirement_clears_all_state_before_reuse() {
        let mut s = State::new();
        let mut last = 0;
        for i in 1..=1024u64 {
            s.register_owner(i).unwrap();
            let h = s.create(i, i as u32, 3, 3, -1, 2).unwrap();
            assert!(h > last);
            last = h;
            s.focus(i, i as u32, h).unwrap();
            s.route_verified_key(Key {
                ascii: b'X',
                pressed: true,
            })
            .unwrap();
            s.retire_owner(i).unwrap();
            assert_eq!(s.surface_count(), 0);
            assert_eq!(s.focused(), None);
            assert_eq!(s.pop_key(i), Err(Refusal::NotOwner));
            assert_eq!(s.move_to(i, i as u32, h, 0, 0), Err(Refusal::Stale));
        }
        assert_eq!(s.next_handle, 1025);
    }

    #[test]
    fn exhausted_monotonic_ids_refuse_without_mutation() {
        let mut s = State::new();
        s.register_owner(1).unwrap();
        s.next_z = u64::MAX;
        assert_eq!(s.create(1, 1, 1, 1, 0, 0), Err(Refusal::Exhausted));
        assert_eq!(s.surface_count(), 0);
        assert_eq!(s.next_handle, 1);
        s.next_z = 1;
        s.next_handle = u64::MAX;
        assert_eq!(s.create(1, 1, 1, 1, 0, 0), Err(Refusal::Exhausted));
        assert_eq!(s.surface_count(), 0);
        assert_eq!(s.next_z, 1);
        s.next_handle = 1;
        let handle = s.create(1, 1, 1, 1, 0, 0).unwrap();
        s.next_z = u64::MAX;
        assert_eq!(s.raise(1, 1, handle), Err(Refusal::Exhausted));
        assert_eq!(s.surfaces[0].unwrap().z, 1);
    }

    #[test]
    fn focus_input_ring_overflow_and_verified_death() {
        let mut s = State::new();
        s.register_owner(7).unwrap();
        s.register_owner(8).unwrap();
        let a = s.create(7, 100, 2, 2, 0, 0).unwrap();
        let b = s.create(8, 200, 2, 2, 1, 1).unwrap();
        assert_eq!(
            s.route_verified_key(Key {
                ascii: b'Q',
                pressed: true
            }),
            Err(Refusal::Stale)
        );
        s.focus(7, 100, a).unwrap();
        for _ in 0..MAX_KEYS {
            s.route_verified_key(Key {
                ascii: b'A',
                pressed: true,
            })
            .unwrap();
        }
        assert_eq!(
            s.route_verified_key(Key {
                ascii: b'B',
                pressed: true
            }),
            Err(Refusal::QueueFull)
        );
        assert_eq!(s.pop_key(8), Ok(None));
        assert_eq!(
            s.pop_key(7),
            Ok(Some(Key {
                ascii: b'A',
                pressed: true
            }))
        );
        s.retire_owner(7).unwrap();
        assert_eq!(s.focused(), None);
        assert_eq!(s.pop_key(7), Err(Refusal::NotOwner));
        assert_eq!(s.move_to(7, 100, a, 0, 0), Err(Refusal::Stale));
        assert_eq!(
            s.route_verified_key(Key {
                ascii: b'Z',
                pressed: true
            }),
            Err(Refusal::Stale)
        );
        s.focus(8, 200, b).unwrap();
        s.route_verified_key(Key {
            ascii: b'C',
            pressed: true,
        })
        .unwrap();
        assert_eq!(
            s.pop_key(8),
            Ok(Some(Key {
                ascii: b'C',
                pressed: true
            }))
        );
        assert_eq!(s.dropped_keys(), 3);
    }
}
