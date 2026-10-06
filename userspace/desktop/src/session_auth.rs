//! Pure badge-to-session selector used by Desktop's authenticated IPC path.
//! A zero badge is reserved for legacy endpoints; a nonzero badge must match
//! exactly one record and its held Process must still be live.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    DuplicateBadge,
}

pub fn select_live_badge<const N: usize>(
    badges: &[u32; N],
    badge: u32,
    mut is_live: impl FnMut(usize) -> bool,
) -> Result<Option<usize>, Error> {
    if badge == 0 {
        return Ok(None);
    }
    let mut selected = None;
    for (index, candidate) in badges.iter().enumerate() {
        if *candidate == badge {
            if selected.replace(index).is_some() {
                return Err(Error::DuplicateBadge);
            }
        }
    }
    Ok(selected.filter(|&index| is_live(index)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_unknown_and_dead_badges_never_select_a_session() {
        let badges = [11, 12, 13, 0];
        let mut checked = 0;
        assert_eq!(
            select_live_badge(&badges, 0, |_| {
                checked += 1;
                true
            }),
            Ok(None)
        );
        assert_eq!(checked, 0, "plain requests stay on the legacy path");
        assert_eq!(select_live_badge(&badges, 99, |_| true), Ok(None));
        assert_eq!(select_live_badge(&badges, 12, |_| false), Ok(None));
    }

    #[test]
    fn only_the_unique_live_badge_owner_is_selected() {
        let badges = [11, 12, 13, 0];
        assert_eq!(
            select_live_badge(&badges, 12, |index| index == 1),
            Ok(Some(1))
        );
        assert_eq!(select_live_badge(&badges, 12, |index| index != 1), Ok(None));
    }

    #[test]
    fn duplicate_badges_are_ambiguous_even_if_only_one_owner_is_live() {
        let badges = [21, 21, 0];
        assert_eq!(
            select_live_badge(&badges, 21, |index| index == 0),
            Err(Error::DuplicateBadge)
        );
    }
}
