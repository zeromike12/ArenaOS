//! Process-local integer handles over objects already held by the runtime.
//!
//! A `Handle` is only a generation-checked index into this table. The table
//! does not create or duplicate kernel authority: callers must insert an
//! object backed by a capability they already hold, and any duplicate
//! operation must call the object's explicit attenuation/copy path. The
//! table is intentionally single-owner (`&mut self` serializes mutation); a
//! future multi-user-thread runtime must protect it with a native lock.

/// Opaque process-local integer reference. Its bits are an index and reuse
/// generation, not a PID, capability slot, or transferable authority token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Handle(u32);

impl Handle {
    /// Numeric representation for storing in an application's private data.
    /// Passing this value to another process does not transfer the object.
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    /// Reconstruct a process-local reference from private serialized state.
    /// Table lookup still checks slot occupancy and the current generation.
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    fn parts(self) -> (usize, u16) {
        ((self.0 & 0xffff) as usize, (self.0 >> 16) as u16)
    }

    fn new(slot: usize, generation: u16) -> Self {
        Self(((generation as u32) << 16) | slot as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The value does not name a slot in this table.
    Invalid,
    /// The slot is empty or has since been reused for another object.
    Stale,
    /// Every reusable slot is occupied or permanently retired.
    Full,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InsertError<T> {
    /// The table is full; ownership of the rejected object is returned intact.
    Full(T),
}

#[derive(Debug, PartialEq, Eq)]
pub enum DuplicateError<E> {
    Table(Error),
    Object(E),
}

struct Entry<T> {
    value: T,
}

/// Fixed-capacity owner for runtime objects. `N` is bounded by the 16-bit
/// index encoded in [`Handle`]. Empty slots advance their generation on close;
/// a slot is retired instead of wrapping when its generation is exhausted.
pub struct HandleTable<T, const N: usize> {
    entries: [Option<Entry<T>>; N],
    generations: [u16; N],
    retired: [bool; N],
    len: usize,
}

impl<T, const N: usize> HandleTable<T, N> {
    pub fn new() -> Self {
        assert!(N <= u16::MAX as usize + 1, "handle table exceeds u16 index");
        Self {
            entries: core::array::from_fn(|_| None),
            generations: [1; N],
            retired: [false; N],
            len: 0,
        }
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether a new owner can be recorded without mutating the table.
    pub fn can_insert(&self) -> bool {
        self.free_slot().is_some()
    }

    /// Snapshot the current process-local handles for bounded group teardown.
    pub fn handles(&self) -> impl Iterator<Item = Handle> + '_ {
        self.entries.iter().enumerate().filter_map(|(slot, entry)| {
            entry
                .as_ref()
                .map(|_| Handle::new(slot, self.generations[slot]))
        })
    }

    /// Insert an object already owned by this process. On capacity refusal,
    /// the exact object is returned to the caller and the table is unchanged.
    pub fn insert(&mut self, value: T) -> Result<Handle, InsertError<T>> {
        let Some(slot) = self.free_slot() else {
            return Err(InsertError::Full(value));
        };
        let handle = Handle::new(slot, self.generations[slot]);
        self.entries[slot] = Some(Entry { value });
        self.len += 1;
        Ok(handle)
    }

    pub fn get(&self, handle: Handle) -> Result<&T, Error> {
        let slot = self.checked_slot(handle)?;
        Ok(&self.entries[slot]
            .as_ref()
            .expect("checked occupied slot")
            .value)
    }

    pub fn get_mut(&mut self, handle: Handle) -> Result<&mut T, Error> {
        let slot = self.checked_slot(handle)?;
        Ok(&mut self.entries[slot]
            .as_mut()
            .expect("checked occupied slot")
            .value)
    }

    /// Close a handle and return the owned object so its caller can perform
    /// the object's explicit cleanup (for example, destroy a held cap).
    pub fn close(&mut self, handle: Handle) -> Result<T, Error> {
        let slot = self.checked_slot(handle)?;
        let entry = self.entries[slot].take().expect("checked occupied slot");
        self.len -= 1;
        if self.generations[slot] == u16::MAX {
            self.retired[slot] = true;
        } else {
            self.generations[slot] += 1;
        }
        Ok(entry.value)
    }

    /// Create another local handle by delegating the actual object copy to
    /// `duplicate`. Capability-backed objects should implement that closure
    /// with the kernel's attenuation-only cap-copy syscall and return a new
    /// held-cap wrapper. A free target slot is reserved *before* invoking the
    /// closure, so a full table cannot create an untracked capability.
    pub fn duplicate_with<E>(
        &mut self,
        source: Handle,
        duplicate: impl FnOnce(&T) -> Result<T, E>,
    ) -> Result<Handle, DuplicateError<E>> {
        let source_slot = self.checked_slot(source).map_err(DuplicateError::Table)?;
        let Some(target_slot) = self.free_slot() else {
            return Err(DuplicateError::Table(Error::Full));
        };
        let value = duplicate(
            &self.entries[source_slot]
                .as_ref()
                .expect("checked occupied slot")
                .value,
        )
        .map_err(DuplicateError::Object)?;
        let handle = Handle::new(target_slot, self.generations[target_slot]);
        self.entries[target_slot] = Some(Entry { value });
        self.len += 1;
        Ok(handle)
    }

    fn free_slot(&self) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .find_map(|(slot, entry)| (entry.is_none() && !self.retired[slot]).then_some(slot))
    }

    fn checked_slot(&self, handle: Handle) -> Result<usize, Error> {
        let (slot, generation) = handle.parts();
        if slot >= N {
            return Err(Error::Invalid);
        }
        if generation == 0 || self.entries[slot].is_none() || self.generations[slot] != generation {
            return Err(Error::Stale);
        }
        Ok(slot)
    }
}

impl<T, const N: usize> Default for HandleTable<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: u8 = 1;
    const WRITE: u8 = 2;

    #[derive(Debug, PartialEq, Eq)]
    struct HeldCap {
        slot: u8,
        rights: u8,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CopyError {
        RightsEscalation,
    }

    impl HeldCap {
        fn attenuated_copy(&self, new_slot: u8, rights: u8) -> Result<Self, CopyError> {
            if rights & !self.rights != 0 {
                return Err(CopyError::RightsEscalation);
            }
            Ok(Self {
                slot: new_slot,
                rights,
            })
        }
    }

    #[test]
    fn close_and_reuse_advance_generation_without_stale_aliasing() {
        let mut table = HandleTable::<HeldCap, 2>::new();
        let first = table
            .insert(HeldCap {
                slot: 7,
                rights: READ | WRITE,
            })
            .unwrap();
        assert_eq!(table.close(first).unwrap().slot, 7);
        let replacement = table
            .insert(HeldCap {
                slot: 8,
                rights: READ,
            })
            .unwrap();
        assert_eq!(first.parts().0, replacement.parts().0);
        assert_ne!(first.parts().1, replacement.parts().1);
        assert_eq!(table.get(first), Err(Error::Stale));
        assert_eq!(table.get(replacement).unwrap().slot, 8);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn duplication_uses_attenuated_object_copy_and_is_failure_atomic() {
        let mut table = HandleTable::<HeldCap, 2>::new();
        let source = table
            .insert(HeldCap {
                slot: 3,
                rights: READ | WRITE,
            })
            .unwrap();
        let copied = table
            .duplicate_with(source, |held| held.attenuated_copy(9, READ))
            .unwrap();
        assert_eq!(table.get(copied).unwrap().rights, READ);
        assert_eq!(table.get(source).unwrap().rights, READ | WRITE);
        assert_eq!(table.len(), 2);

        let full_result = table.duplicate_with(source, |_| -> Result<HeldCap, CopyError> {
            panic!("full table must be rejected before copying a cap")
        });
        assert_eq!(full_result, Err(DuplicateError::Table(Error::Full)));
        assert_eq!(table.len(), 2);
        let rejected = HeldCap {
            slot: 13,
            rights: WRITE,
        };
        assert_eq!(
            table.insert(rejected),
            Err(InsertError::Full(HeldCap {
                slot: 13,
                rights: WRITE,
            }))
        );
        assert_eq!(table.len(), 2);

        let mut one_slot = HandleTable::<HeldCap, 2>::new();
        let source = one_slot
            .insert(HeldCap {
                slot: 11,
                rights: READ,
            })
            .unwrap();
        let before = one_slot.len();
        let denied = one_slot.duplicate_with(source, |held| held.attenuated_copy(12, READ | WRITE));
        assert_eq!(
            denied,
            Err(DuplicateError::Object(CopyError::RightsEscalation))
        );
        assert_eq!(one_slot.len(), before);
        assert_eq!(one_slot.get(source).unwrap().rights, READ);
    }

    #[test]
    fn invalid_raw_handle_and_empty_slot_are_not_resolvable() {
        let mut table = HandleTable::<u8, 1>::new();
        assert_eq!(table.get(Handle::from_raw(u32::MAX)), Err(Error::Invalid));
        let vacant = Handle::new(0, 1);
        assert_eq!(table.get(vacant), Err(Error::Stale));
        assert_eq!(table.insert(42), Ok(vacant));
        assert_eq!(table.get(vacant), Ok(&42));
    }

    #[test]
    fn exhausted_generation_retires_slot_instead_of_wrapping() {
        let mut table = HandleTable::<u8, 1>::new();
        table.generations[0] = u16::MAX;
        let last = table.insert(99).unwrap();
        assert_eq!(last.parts(), (0, u16::MAX));
        assert_eq!(table.close(last), Ok(99));
        assert!(table.retired[0]);
        assert_eq!(table.insert(100), Err(InsertError::Full(100)));
        assert_eq!(table.len(), 0);
    }
}
