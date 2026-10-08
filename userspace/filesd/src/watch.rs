//! Directory watches (Phase 11, ADR-0079): pure bookkeeping, no syscalls.
//!
//! A watch hangs off one capability record naming a directory. The client
//! lends one of its own notifications and names a badge bit; when the
//! directory changes, filesd ORs that bit into the notification and counts
//! the event on the watch. The badge is only a hint: the client confirms
//! through its own record (`OP_WATCHED`), which returns and clears the count.
//!
//! * Authority is the held record, never a path.
//! * A watch matches only the exact AFS2 object id (index and generation),
//!   so a directory removed and its index reused never signals the old
//!   watcher.
//! * Capacity is checked before anything is kept: per lineage and global.
//! * A watch dies with its record (release, revoke, lineage retirement,
//!   hence process death through the broker's revoke).

/// Watches filesd holds at once (each keeps one lent notification slot).
pub const WATCHES: usize = 24;
/// Watches one lineage may hold.
pub const LINEAGE_WATCHES: usize = 4;
pub const S_FULL: u64 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch {
    pub live: bool,
    /// The record (index) the watch belongs to and that record's lineage.
    pub record: u16,
    pub lineage: u8,
    /// The watched directory: the exact object id (generation included).
    pub object: u64,
    /// filesd's slot holding the lent notification, and the badge bit.
    pub slot: u64,
    pub bit: u8,
    /// Changes since the client last asked.
    pub events: u32,
    /// The directory no longer exists.
    pub gone: bool,
}
pub const NO_WATCH: Watch = Watch {
    live: false,
    record: 0,
    lineage: 0,
    object: 0,
    slot: 0,
    bit: 0,
    events: 0,
    gone: false,
};

/// Same AFS2 object: index and generation both (ids are `gen << 32 | index`).
pub fn same(a: u64, b: u64) -> bool {
    a == b
}

pub struct Table {
    pub w: [Watch; WATCHES],
}

impl Table {
    pub const fn new() -> Self {
        Table {
            w: [NO_WATCH; WATCHES],
        }
    }
    fn at(&self, record: u16) -> Option<usize> {
        self.w.iter().position(|w| w.live && w.record == record)
    }
    /// Where a watch for `record` would go, checked before anything is
    /// kept: its existing watch (replaced), else a free entry within the
    /// lineage and global bounds.
    pub fn reserve(&self, record: u16, lineage: u8) -> Result<usize, u64> {
        if let Some(i) = self.at(record) {
            return Ok(i);
        }
        let held = self
            .w
            .iter()
            .filter(|w| w.live && w.lineage == lineage)
            .count();
        if held >= LINEAGE_WATCHES {
            return Err(S_FULL);
        }
        self.w.iter().position(|w| !w.live).ok_or(S_FULL)
    }
    /// Install at a reserved entry; returns the slot of a replaced watch
    /// (the caller drops it).
    pub fn install(&mut self, at: usize, watch: Watch) -> Option<u64> {
        let old = self.w[at];
        self.w[at] = Watch {
            live: true,
            ..watch
        };
        (old.live && old.slot != watch.slot).then_some(old.slot)
    }
    /// The record's watch ends (unwatch, release, revoke): its slot.
    pub fn remove(&mut self, record: u16) -> Option<u64> {
        let i = self.at(record)?;
        let slot = self.w[i].slot;
        self.w[i] = NO_WATCH;
        Some(slot)
    }
    /// Directory `object` changed: every live watch on exactly it counts
    /// an event and is signalled. `signal(slot, bit)` false means the lent
    /// notification is gone: that watch is removed and `drop` gets its slot.
    pub fn changed(
        &mut self,
        object: u64,
        mut signal: impl FnMut(u64, u8) -> bool,
        mut drop: impl FnMut(u64),
    ) -> usize {
        let mut n = 0;
        // The exact object id is the only guard: a removed directory's id
        // never comes back (its index returns with a new generation).
        for w in self
            .w
            .iter_mut()
            .filter(|w| w.live && same(w.object, object))
        {
            w.events = w.events.saturating_add(1);
            n += 1;
            if !signal(w.slot, w.bit) {
                drop(w.slot);
                *w = NO_WATCH;
            }
        }
        n
    }
    /// Directory `object` was removed: watchers learn it once.
    pub fn removed(
        &mut self,
        object: u64,
        signal: impl FnMut(u64, u8) -> bool,
        drop: impl FnMut(u64),
    ) {
        self.changed(object, signal, drop);
        for w in self
            .w
            .iter_mut()
            .filter(|w| w.live && same(w.object, object))
        {
            w.gone = true;
        }
    }
    /// The client asks: (events since last asked, gone), cleared.
    pub fn take(&mut self, record: u16) -> Option<(u32, bool)> {
        let i = self.at(record)?;
        let w = &mut self.w[i];
        let r = (w.events, w.gone);
        w.events = 0;
        Some(r)
    }
    pub fn live(&self) -> usize {
        self.w.iter().filter(|w| w.live).count()
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// What a mutating operation changes, for watchers: the directory it was
/// performed in, plus the destination directory of a rename (moving an
/// entry changes both parents), or the parent of a file whose contents
/// changed (its size and time show in that listing).
pub fn affected(dir: u64, rename_to: Option<u64>) -> [Option<u64>; 2] {
    match rename_to {
        Some(dst) if !same(dst, dir) => [Some(dir), Some(dst)],
        _ => [Some(dir), None],
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    const fn oid(index: u32, generation: u32) -> u64 {
        (generation as u64) << 32 | index as u64
    }
    fn watch(record: u16, lineage: u8, object: u64, slot: u64) -> Watch {
        Watch {
            live: true,
            record,
            lineage,
            object,
            slot,
            bit: 1,
            events: 0,
            gone: false,
        }
    }
    fn fire(t: &mut Table, object: u64) -> Vec<u64> {
        let mut hit = Vec::new();
        t.changed(
            object,
            |slot, _| {
                hit.push(slot);
                true
            },
            |_| {},
        );
        hit
    }

    #[test]
    fn a_reused_directory_index_never_signals_the_old_watcher() {
        let mut t = Table::new();
        let a = oid(7, 1);
        let at = t.reserve(3, 3).unwrap();
        t.install(at, watch(3, 3, a, 40));
        t.removed(a, |_, _| true, |_| {});
        assert_eq!(t.take(3), Some((1, true)));
        // Index 7 comes back as a new directory (generation 2).
        let reused = oid(7, 2);
        assert!(
            fire(&mut t, reused).is_empty(),
            "stale watcher signalled for a reused index"
        );
        assert_eq!(t.take(3), Some((0, true)));
        // A watch on the new directory is its own.
        let at = t.reserve(4, 3).unwrap();
        t.install(at, watch(4, 3, reused, 41));
        assert_eq!(fire(&mut t, reused), [41]);
    }

    #[test]
    fn a_rename_changes_both_parents() {
        let (src, dst) = (oid(2, 1), oid(5, 1));
        assert_eq!(affected(src, Some(dst)), [Some(src), Some(dst)]);
        assert_eq!(affected(src, Some(src)), [Some(src), None]);
        assert_eq!(affected(src, None), [Some(src), None]);
        let mut t = Table::new();
        t.install(0, watch(1, 9, src, 30));
        t.install(1, watch(2, 9, dst, 31));
        let mut hit = Vec::new();
        for d in affected(src, Some(dst)).into_iter().flatten() {
            hit.extend(fire(&mut t, d));
        }
        assert_eq!(hit, [30, 31], "the destination parent was not signalled");
    }

    #[test]
    fn capacity_is_refused_before_anything_is_kept() {
        let mut t = Table::new();
        for r in 0..LINEAGE_WATCHES as u16 {
            let at = t.reserve(10 + r, 1).unwrap();
            t.install(at, watch(10 + r, 1, oid(r.into(), 1), 100 + u64::from(r)));
        }
        assert_eq!(t.reserve(99, 1), Err(S_FULL), "lineage quota");
        // Re-watching a record replaces its watch in place.
        let at = t.reserve(10, 1).unwrap();
        assert_eq!(t.install(at, watch(10, 1, oid(0, 1), 200)), Some(100));
        assert_eq!(t.live(), LINEAGE_WATCHES);
        // Fill the global table from other lineages.
        let mut lineage = 2u8;
        while t.live() < WATCHES {
            let at = t.reserve(1000 + t.live() as u16, lineage).unwrap();
            t.install(at, watch(1000 + t.live() as u16, lineage, oid(50, 1), 300));
            if t.w
                .iter()
                .filter(|w| w.live && w.lineage == lineage)
                .count()
                == LINEAGE_WATCHES
            {
                lineage += 1;
            }
        }
        assert_eq!(t.reserve(5000, 200), Err(S_FULL), "global bound");
    }

    #[test]
    fn removal_and_dead_notifications_end_watches() {
        let mut t = Table::new();
        let d = oid(3, 4);
        t.install(0, watch(8, 2, d, 50));
        t.install(1, watch(9, 2, d, 51));
        assert_eq!(t.remove(8), Some(50));
        assert_eq!(t.remove(8), None);
        let mut dropped = Vec::new();
        t.changed(d, |_, _| false, |s| dropped.push(s));
        assert_eq!(dropped, [51]);
        assert_eq!(t.live(), 0);
        assert_eq!(t.take(9), None);
    }
}
