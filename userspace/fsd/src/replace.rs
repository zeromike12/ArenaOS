//! Nonmutating reservation of complete AFS1 copy-on-write replacement.
//! Existing bitmap only: no reclaim or partially allocated refusal path.
pub const MAX_BYTES: u64 = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub data: [u32; 8],
    pub count: usize,
    pub extent: u32,
    pub objects: u32,
    pub bitmap: u32,
}
impl Plan {
    pub fn sectors(&self) -> impl Iterator<Item = u32> + '_ {
        self.data[..self.count]
            .iter()
            .copied()
            .chain((self.extent != 0).then_some(self.extent))
            .chain(
                (self.objects != 0)
                    .then_some(self.objects)
                    .into_iter()
                    .flat_map(|base| base..base + 4),
            )
            .chain(
                (self.bitmap != 0)
                    .then_some(self.bitmap)
                    .into_iter()
                    .flat_map(|base| base..base + 4),
            )
    }
}
pub fn reserve(bitmap: &[u8], total: u32, bytes: u64) -> Option<Plan> {
    if total < 3 || total as usize > bitmap.len() * 8 || bytes > MAX_BYTES {
        return None;
    }
    let mut p = Plan {
        data: [0; 8],
        count: 0,
        extent: 0,
        objects: 0,
        bitmap: 0,
    };
    let free = |s: u32, p: &Plan| {
        s >= 3 && bitmap[s as usize / 8] & (1 << (s % 8)) == 0 && !p.sectors().any(|x| x == s)
    };
    // Preserve contiguous metadata space before choosing individual data pages.
    p.objects = (3..=total.checked_sub(4)?).find(|s| (0..4).all(|k| free(s + k, &p)))?;
    p.bitmap = (3..=total.checked_sub(4)?).find(|s| (0..4).all(|k| free(s + k, &p)))?;
    if bytes != 0 {
        p.extent = (3..total).find(|s| free(*s, &p))?;
        for _ in 0..bytes.div_ceil(512) {
            let sector = (3..total).find(|s| free(*s, &p))?;
            p.data[p.count] = sector;
            p.count += 1;
        }
    }
    Some(p)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_complete_reservation_and_mutation_free_refusal() {
        let mut bm = [0xffu8; 8];
        for s in [
            8, 9, 10, 11, 20, 21, 22, 23, 30, 32, 34, 36, 38, 40, 42, 44, 46,
        ] {
            bm[s / 8] &= !(1 << (s % 8));
        }
        let before = bm;
        let p = reserve(&bm, 64, 4096).unwrap();
        assert_eq!(p.objects, 8);
        assert_eq!(p.bitmap, 20);
        assert_eq!(p.count, 8);
        let mut chosen = [false; 64];
        for s in p.sectors() {
            assert!(!chosen[s as usize]);
            chosen[s as usize] = true;
            assert_eq!(bm[s as usize / 8] & (1 << (s % 8)), 0);
        }
        assert_eq!(bm, before);
        bm[46 / 8] |= 1 << (46 % 8);
        let before = bm;
        assert_eq!(reserve(&bm, 64, 4096), None);
        assert_eq!(bm, before);
        assert!(reserve(&bm, 64, 3584).is_some());
    }
    #[test]
    fn empty_geometry_and_reserved_sectors() {
        let bm = [0u8; 4];
        let p = reserve(&bm, 32, 0).unwrap();
        assert_eq!(p.count, 0);
        assert_eq!(p.extent, 0);
        assert!(p.sectors().all(|s| s >= 3));
        assert_eq!(reserve(&bm, 33, 0), None);
        assert_eq!(reserve(&bm, 32, 4097), None);
        let full = [0xff; 4];
        assert_eq!(reserve(&full, 32, 0), None);
    }
}
