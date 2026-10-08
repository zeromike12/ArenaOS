//! Explicit operations on capabilities already held in the current process.
//!
//! `HeldCapability` is a checked slot reference, not a transferable integer
//! authority and not an RAII destructor. The kernel slot remains the authority;
//! callers keep this value in an owner such as `HandleTable` and explicitly
//! destroy the slot when the object-specific lifecycle permits it. Process
//! caps require the dedicated finish/reap operation, not generic cap discard.

use arena_lib::abi::{
    CAP_SLOTS, RIGHTS_ALL, RIGHTS_COPY, SYS_CAP_COPY, SYS_CAP_DESCRIBE, SYS_CAP_DESTROY,
    SYS_CAP_OCCUPIED, syscall1, syscall2, syscall3, syscall6,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    SlotOutOfRange(u8),
    Empty(u8),
    OccupancyQuery(i64),
    Description(i64),
    InvalidDescription,
    NoCopyRight,
    InvalidRights,
    RightsAmplification,
    DestinationOccupied(u8),
    Copy(i64),
}

/// A snapshot of a capability actually held in this process's slot table.
/// Kind, object ID and rights are descriptive; methods always pass `slot` to
/// the kernel, which revalidates the live capability before acting.
#[derive(Debug, PartialEq, Eq)]
pub struct HeldCapability {
    slot: u8,
    kind: u8,
    object: u64,
    rights: u32,
}

impl HeldCapability {
    /// Inspect a live, describable capability in the calling process.
    pub fn from_slot(slot: u8) -> Result<Self, Error> {
        if u64::from(slot) >= CAP_SLOTS as u64 {
            return Err(Error::SlotOutOfRange(slot));
        }
        match unsafe { syscall6(SYS_CAP_OCCUPIED, u64::from(slot), 0, 0, 0, 0, 0) } {
            0 => return Err(Error::Empty(slot)),
            1 => {}
            status => return Err(Error::OccupancyQuery(status)),
        }
        let mut words = [0u64; 3];
        let status =
            unsafe { syscall2(SYS_CAP_DESCRIBE, u64::from(slot), words.as_mut_ptr() as u64) };
        if status != 0 {
            return Err(Error::Description(status));
        }
        let Ok(kind) = u8::try_from(words[0]) else {
            return Err(Error::InvalidDescription);
        };
        let Ok(rights) = u32::try_from(words[2]) else {
            return Err(Error::InvalidDescription);
        };
        if kind == 0 || rights == 0 || u64::from(rights) & !RIGHTS_ALL != 0 {
            return Err(Error::InvalidDescription);
        }
        Ok(Self {
            slot,
            kind,
            object: words[1],
            rights,
        })
    }

    pub const fn slot(&self) -> u8 {
        self.slot
    }

    pub const fn kind(&self) -> u8 {
        self.kind
    }

    /// Descriptive object identity only. Never use this value to authorize a
    /// syscall; kernel operations use the held slot and re-check its cap.
    pub const fn object_id(&self) -> u64 {
        self.object
    }

    pub const fn rights(&self) -> u32 {
        self.rights
    }

    /// Copy this held cap into an explicitly selected empty slot with a strict
    /// rights subset. The kernel repeats every check atomically; the occupancy
    /// query is an early diagnostic, not a reservation.
    pub fn copy_to(&self, destination: u8, rights: u32) -> Result<Self, Error> {
        if u64::from(destination) >= CAP_SLOTS as u64 {
            return Err(Error::SlotOutOfRange(destination));
        }
        validate_copy_rights(self.rights, rights)?;
        match unsafe { syscall6(SYS_CAP_OCCUPIED, u64::from(destination), 0, 0, 0, 0, 0) } {
            0 => {}
            1 => return Err(Error::DestinationOccupied(destination)),
            status => return Err(Error::OccupancyQuery(status)),
        }
        let status = unsafe {
            syscall3(
                SYS_CAP_COPY,
                u64::from(self.slot),
                u64::from(destination),
                u64::from(rights),
            )
        };
        if status != 0 {
            return Err(Error::Copy(status));
        }
        Ok(Self {
            slot: destination,
            kind: self.kind,
            object: self.object,
            rights,
        })
    }

    /// Discard this cap reference. On failure the exact wrapper is returned so
    /// the caller can retry, report, or retain it rather than losing ownership.
    /// For a Process cap use `SYS_PROC_FINISH`; dropping that reference alone
    /// does not terminate or reap the child.
    pub fn destroy(self) -> Result<(), DestroyFailure> {
        let status = unsafe { syscall1(SYS_CAP_DESTROY, u64::from(self.slot)) };
        if status == 0 {
            Ok(())
        } else {
            Err(DestroyFailure {
                status,
                capability: self,
            })
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct DestroyFailure {
    pub status: i64,
    pub capability: HeldCapability,
}

fn validate_copy_rights(source: u32, requested: u32) -> Result<(), Error> {
    if requested == 0 || u64::from(requested) & !RIGHTS_ALL != 0 {
        return Err(Error::InvalidRights);
    }
    if source & RIGHTS_COPY as u32 == 0 {
        return Err(Error::NoCopyRight);
    }
    if requested & !source != 0 {
        return Err(Error::RightsAmplification);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: u32 = 1;
    const WRITE: u32 = 2;
    const COPY: u32 = 4;
    const DESTROY: u32 = 8;

    #[test]
    fn capability_copy_requires_copy_right_and_strictly_attenuates() {
        assert_eq!(
            validate_copy_rights(READ | WRITE, READ),
            Err(Error::NoCopyRight)
        );
        assert_eq!(
            validate_copy_rights(READ | COPY, READ | WRITE),
            Err(Error::RightsAmplification)
        );
        assert_eq!(
            validate_copy_rights(READ | COPY, 0),
            Err(Error::InvalidRights)
        );
        assert_eq!(
            validate_copy_rights(READ | COPY, READ | 0x10),
            Err(Error::InvalidRights)
        );
        assert_eq!(validate_copy_rights(READ | WRITE | COPY, READ), Ok(()));
    }

    #[test]
    fn destroyable_attenuation_preserves_only_requested_rights() {
        assert_eq!(
            validate_copy_rights(READ | WRITE | COPY | DESTROY, READ | DESTROY),
            Ok(())
        );
    }
}
