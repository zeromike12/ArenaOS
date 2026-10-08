//! ABI-v2 entry gate. The child verifies its real slot table before the
//! application closure can run; metadata never creates or names authority.

use arena_lib::abi::{
    CAP_SLOTS, SYS_CAP_DESCRIBE, SYS_CAP_DESTROY, SYS_CAP_OCCUPIED, SYS_SHARED_MAP,
    SYS_SHARED_PAGES, SYS_SHARED_UNMAP, SYS_THREAD_EXIT, syscall1, syscall2, syscall6, write_all,
};
use arena_startup_abi::startup as abi;
use arena_startup_abi::startup::StartupView;
use core::sync::atomic::{AtomicBool, Ordering};

pub const STARTUP_SLOT: u64 = 0;
const USER_ADDRESS_END: u64 = 0x0000_8000_0000_0000;
static ENTERED: AtomicBool = AtomicBool::new(false);

// A private BSS copy removes both the live shared mapping and its cap before
// application code runs. This is fixed process storage, not a heap allocation
// or a large temporary on the one-page initial stack.
static mut SNAPSHOT: [u8; abi::BLOCK_BYTES] = [0; abi::BLOCK_BYTES];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    AlreadyEntered,
    DescribeStartup(i64),
    StartupCapability(abi::Error),
    PageCount(i64),
    Map(i64),
    BadMapAddress(u64),
    Parse(abi::Error),
    DescribeCapability {
        slot: u16,
        status: i64,
    },
    CapabilityMismatch {
        slot: u16,
    },
    MissingCapability {
        slot: u16,
    },
    UnexpectedCapability {
        slot: u16,
    },
    OccupancyQuery {
        slot: u16,
        status: i64,
    },
    Cleanup {
        unmap_status: i64,
        destroy_status: i64,
    },
}

/// Compare the entire live child cap table with the startup record after slot
/// 0 has been consumed. Object IDs are descriptive and are ignored; every
/// occupied slot must be listed exactly once with matching kind/rights, and
/// every unlisted (including undescribable) slot must be empty.
pub fn verify_live_capabilities<F, O>(
    view: &StartupView<'_>,
    mut describe: F,
    mut occupied: O,
) -> Result<(), Error>
where
    F: FnMut(u64) -> Result<[u64; 3], i64>,
    O: FnMut(u64) -> Result<bool, i64>,
{
    for slot in 0..CAP_SLOTS {
        let listed_index = if slot > 0 && slot <= view.capability_count() {
            Some(slot - 1)
        } else {
            None
        };
        let is_occupied = occupied(slot as u64).map_err(|status| Error::OccupancyQuery {
            slot: slot as u16,
            status,
        })?;
        match (listed_index, is_occupied) {
            (Some(_), false) => {
                return Err(Error::MissingCapability { slot: slot as u16 });
            }
            (None, true) => {
                return Err(Error::UnexpectedCapability { slot: slot as u16 });
            }
            (Some(index), true) => {
                let expected = view
                    .capability(index)
                    .ok_or(Error::CapabilityMismatch { slot: slot as u16 })?;
                let observed =
                    describe(slot as u64).map_err(|status| Error::DescribeCapability {
                        slot: expected.slot,
                        status,
                    })?;
                if expected.slot != (index + 1) as u16 || !view.capability_matches(index, observed)
                {
                    return Err(Error::CapabilityMismatch {
                        slot: expected.slot,
                    });
                }
            }
            (None, false) => {}
        }
    }
    Ok(())
}

/// Validate slot 0, map it read-only, snapshot the exact page into private
/// static storage, unmap and destroy the transport cap, parse the snapshot,
/// verify every listed live cap, and only then call application code.
///
/// The HRTB prevents the application closure from returning a borrowed slice
/// into runtime-owned startup storage. It may inspect or copy the bounded
/// argv/env bytes while it runs.
pub fn run<R, F>(application: F) -> Result<R, Error>
where
    F: for<'page> FnOnce(StartupView<'page>) -> R,
{
    if ENTERED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(Error::AlreadyEntered);
    }

    let observed = describe(STARTUP_SLOT).map_err(Error::DescribeStartup)?;
    if observed[0] != u64::from(abi::CAP_KIND_SHARED_REGION)
        || observed[2] != u64::from(abi::RIGHT_READ | abi::RIGHT_DESTROY)
    {
        return Err(reject_startup_cap(
            Error::StartupCapability(abi::Error::InvalidCapability),
            observed,
        ));
    }

    // SYS_SHARED_PAGES is a positive page count, or a negative typed status.
    let pages = unsafe { syscall6(SYS_SHARED_PAGES, STARTUP_SLOT, 0, 0, 0, 0, 0) };
    if pages < 0 {
        return Err(reject_startup_cap(Error::PageCount(pages), observed));
    }
    if let Err(error) = abi::validate_startup_cap(observed, pages as u64) {
        return Err(reject_startup_cap(
            Error::StartupCapability(error),
            observed,
        ));
    }

    let mapped = unsafe { syscall2(SYS_SHARED_MAP, STARTUP_SLOT, 0) };
    if mapped <= 0 {
        return match destroy_startup_cap() {
            Ok(()) => Err(Error::Map(mapped)),
            Err(cleanup) => Err(cleanup),
        };
    }
    let map_va = mapped as u64;
    let Some(map_end) = map_va.checked_add(abi::BLOCK_BYTES as u64) else {
        return match cleanup(Some(map_va)) {
            Ok(()) => Err(Error::BadMapAddress(map_va)),
            Err(error) => Err(error),
        };
    };
    if !map_va.is_multiple_of(abi::BLOCK_BYTES as u64) || map_end > USER_ADDRESS_END {
        return match cleanup(Some(map_va)) {
            Ok(()) => Err(Error::BadMapAddress(map_va)),
            Err(error) => Err(error),
        };
    }

    // SAFETY: the kernel returned an aligned user VA for exactly one page
    // after checking the held slot-0 SharedRegion and its READ right. SNAPSHOT
    // is process-private static BSS and ENTERED serializes its sole writer.
    unsafe {
        core::ptr::copy_nonoverlapping(
            map_va as *const u8,
            core::ptr::addr_of_mut!(SNAPSHOT).cast::<u8>(),
            abi::BLOCK_BYTES,
        );
    }
    cleanup(Some(map_va))?;

    // SAFETY: the snapshot is immutable after the one-time copy, and the
    // process-global ENTERED gate prevents another runtime invocation.
    let page = unsafe { &*core::ptr::addr_of!(SNAPSHOT) };
    let view = abi::parse(page).map_err(Error::Parse)?;
    verify_live_capabilities(&view, describe, occupied)?;
    Ok(application(view))
}

fn describe(slot: u64) -> Result<[u64; 3], i64> {
    let mut words = [0u64; 3];
    let status = unsafe { syscall2(SYS_CAP_DESCRIBE, slot, words.as_mut_ptr() as u64) };
    if status == 0 { Ok(words) } else { Err(status) }
}

fn occupied(slot: u64) -> Result<bool, i64> {
    match unsafe { syscall6(SYS_CAP_OCCUPIED, slot, 0, 0, 0, 0, 0) } {
        0 => Ok(false),
        1 => Ok(true),
        status => Err(status),
    }
}

fn can_discard_startup_cap(observed: [u64; 3]) -> bool {
    observed[0] == u64::from(abi::CAP_KIND_SHARED_REGION)
        && observed[2] & u64::from(abi::RIGHT_DESTROY) != 0
}

fn reject_startup_cap(error: Error, observed: [u64; 3]) -> Error {
    if can_discard_startup_cap(observed) {
        match destroy_startup_cap() {
            Ok(()) => error,
            Err(cleanup) => cleanup,
        }
    } else {
        error
    }
}

fn destroy_startup_cap() -> Result<(), Error> {
    let status = unsafe { syscall1(SYS_CAP_DESTROY, STARTUP_SLOT) };
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Cleanup {
            unmap_status: 0,
            destroy_status: status,
        })
    }
}

fn cleanup(map_va: Option<u64>) -> Result<(), Error> {
    let unmap_status = map_va.map_or(0, |va| unsafe {
        syscall6(SYS_SHARED_UNMAP, va, 0, 0, 0, 0, 0)
    });
    let destroy_status = unsafe { syscall1(SYS_CAP_DESTROY, STARTUP_SLOT) };
    if unmap_status == 0 && destroy_status == 0 {
        Ok(())
    } else {
        Err(Error::Cleanup {
            unmap_status,
            destroy_status,
        })
    }
}

fn error_detail(error: Error) -> &'static [u8] {
    match error {
        Error::AlreadyEntered => b"arena-runtime: detail already-entered\n",
        Error::DescribeStartup(_) => b"arena-runtime: detail describe-slot0\n",
        Error::StartupCapability(_) => b"arena-runtime: detail slot0-capability\n",
        Error::PageCount(_) => b"arena-runtime: detail slot0-page-query\n",
        Error::Map(_) => b"arena-runtime: detail slot0-map\n",
        Error::BadMapAddress(_) => b"arena-runtime: detail slot0-map-address\n",
        Error::Parse(abi::Error::BadMagic) => b"arena-runtime: detail parse-magic\n",
        Error::Parse(abi::Error::UnsupportedVersion) => b"arena-runtime: detail parse-version\n",
        Error::Parse(_) => b"arena-runtime: detail parse-record\n",
        Error::DescribeCapability { .. } => b"arena-runtime: detail describe-listed-cap\n",
        Error::CapabilityMismatch { .. } => b"arena-runtime: detail listed-cap-mismatch\n",
        Error::MissingCapability { .. } => b"arena-runtime: detail missing-listed-cap\n",
        Error::UnexpectedCapability { .. } => b"arena-runtime: detail unexpected-capability\n",
        Error::OccupancyQuery { .. } => b"arena-runtime: detail cap-occupancy\n",
        Error::Cleanup { .. } => b"arena-runtime: detail slot0-cleanup\n",
    }
}

/// Stable ABI-v2 entry helper. An invalid startup block/cap table prints a
/// bounded diagnostic and exits without calling the application closure.
/// Returning from the application exits with its code after startup mapping
/// teardown.
pub fn enter<F>(application: F) -> !
where
    F: for<'page> FnOnce(StartupView<'page>) -> u64,
{
    let status = match run(application) {
        Ok(code) => code,
        Err(error) => {
            write_all(error_detail(error));
            write_all(b"arena-runtime: startup refused\n");
            0xA12
        }
    };
    unsafe { syscall1(SYS_THREAD_EXIT, status) };
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &[u8; abi::BLOCK_BYTES] =
        include_bytes!("../../arena-platform/tests/data/startup-v2.bin");

    #[test]
    fn rejection_discards_only_a_destroyable_shared_region_slot() {
        assert!(can_discard_startup_cap([
            u64::from(abi::CAP_KIND_SHARED_REGION),
            7,
            u64::from(abi::RIGHT_READ | abi::RIGHT_DESTROY),
        ]));
        assert!(!can_discard_startup_cap([
            u64::from(abi::CAP_KIND_SHARED_REGION),
            7,
            u64::from(abi::RIGHT_READ),
        ]));
        assert!(!can_discard_startup_cap([
            u64::from(abi::CAP_KIND_NOTIFICATION),
            7,
            u64::from(abi::RIGHT_READ | abi::RIGHT_DESTROY),
        ]));
    }

    fn listed_slots(slot: u64) -> Result<bool, i64> {
        Ok(slot == 1 || slot == 2)
    }

    fn descriptions(slot: u64) -> Result<[u64; 3], i64> {
        Ok(if slot == 1 {
            [u64::from(abi::CAP_KIND_BADGED_ENDPOINT), 0xDEAD_BEEF, 2]
        } else {
            [u64::from(abi::CAP_KIND_IMAGE), 0xCAFE_BABE, 1]
        })
    }

    #[test]
    fn full_cap_table_matches_listed_kinds_and_rights_but_ignores_ids() {
        let view = abi::parse(PAGE).unwrap();
        let mut checked = [0u64; 2];
        verify_live_capabilities(
            &view,
            |slot| {
                checked[slot as usize - 1] += 1;
                descriptions(slot)
            },
            listed_slots,
        )
        .unwrap();
        assert_eq!(checked, [1, 1]);
    }

    #[test]
    fn kind_or_rights_mismatch_refuses_before_application_entry() {
        let view = abi::parse(PAGE).unwrap();
        assert_eq!(
            verify_live_capabilities(
                &view,
                |slot| {
                    Ok(if slot == 1 {
                        [u64::from(abi::CAP_KIND_BADGED_ENDPOINT), 7, 1]
                    } else {
                        [u64::from(abi::CAP_KIND_IMAGE), 9, 1]
                    })
                },
                listed_slots,
            ),
            Err(Error::CapabilityMismatch { slot: 1 })
        );
        assert_eq!(
            verify_live_capabilities(
                &view,
                |slot| {
                    Ok(if slot == 1 {
                        [u64::from(abi::CAP_KIND_BADGED_ENDPOINT), 7, 2]
                    } else {
                        [u64::from(abi::CAP_KIND_IMAGE), 9, 2]
                    })
                },
                listed_slots,
            ),
            Err(Error::CapabilityMismatch { slot: 2 })
        );
    }

    #[test]
    fn missing_listed_capability_refuses_before_application_entry() {
        let view = abi::parse(PAGE).unwrap();
        assert_eq!(
            verify_live_capabilities(&view, descriptions, |slot| Ok(slot == 1)),
            Err(Error::MissingCapability { slot: 2 })
        );
    }

    #[test]
    fn extra_unlisted_capability_refuses_even_when_its_kind_is_unavailable() {
        let view = abi::parse(PAGE).unwrap();
        let mut described = [0u64; 2];
        assert_eq!(
            verify_live_capabilities(
                &view,
                |slot| {
                    described[slot as usize - 1] += 1;
                    descriptions(slot)
                },
                |slot| Ok(slot == 1 || slot == 2 || slot == 37),
            ),
            Err(Error::UnexpectedCapability { slot: 37 })
        );
        assert_eq!(described, [1, 1]);
    }

    #[test]
    fn unexpected_startup_slot_zero_and_occupancy_query_errors_refuse() {
        let view = abi::parse(PAGE).unwrap();
        assert_eq!(
            verify_live_capabilities(&view, descriptions, |slot| Ok(slot == 0
                || slot == 1
                || slot == 2)),
            Err(Error::UnexpectedCapability { slot: 0 })
        );
        assert_eq!(
            verify_live_capabilities(&view, descriptions, |slot| {
                if slot == 9 {
                    Err(-3)
                } else {
                    Ok(slot == 1 || slot == 2)
                }
            }),
            Err(Error::OccupancyQuery {
                slot: 9,
                status: -3,
            })
        );
    }

    #[test]
    fn an_unavailable_listed_slot_description_is_fail_closed() {
        let view = abi::parse(PAGE).unwrap();
        assert_eq!(
            verify_live_capabilities(
                &view,
                |slot| {
                    if slot == 1 {
                        Ok([u64::from(abi::CAP_KIND_BADGED_ENDPOINT), 7, 2])
                    } else {
                        Err(-2)
                    }
                },
                listed_slots,
            ),
            Err(Error::DescribeCapability {
                slot: 2,
                status: -2,
            })
        );
    }
}
