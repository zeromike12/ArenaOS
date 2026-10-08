//! Safe wrappers for ArenaOS's process-owned native VM operations.
//!
//! A `Region` stores the caller's exact cap slot as a descriptive handle. The
//! kernel revalidates that live capability on every operation; copying this
//! Rust value does not copy authority.

use arena_lib::abi::{
    SYS_VM_COMMIT, SYS_VM_PROTECT, SYS_VM_QUERY, SYS_VM_RELEASE, SYS_VM_RESERVE, VM_PROT_EXEC,
    VM_PROT_READ, VM_PROT_WRITE, syscall6,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protection(u64);

impl Protection {
    pub const READ_ONLY: Self = Self(VM_PROT_READ);
    pub const READ_WRITE: Self = Self(VM_PROT_READ | VM_PROT_WRITE);
    pub const READ_EXECUTE: Self = Self(VM_PROT_READ | VM_PROT_EXEC);

    pub const fn bits(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query {
    pub base: u64,
    pub capacity_pages: u32,
    pub committed_pages: u32,
    pub global_committed_pages: u32,
    pub guard_pages: u32,
    pub global_regions: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Status(i64),
    InvalidReply,
}

/// A process-owned VM region. Call `release` explicitly; dropping this
/// descriptive wrapper does not discard a kernel capability.
#[derive(Debug, PartialEq, Eq)]
pub struct Region {
    slot: u8,
    base: u64,
    pages: u32,
}

impl Region {
    pub fn reserve(pages: u32) -> Result<Self, Error> {
        let mut out = [0u64; 3];
        let status = unsafe {
            syscall6(
                SYS_VM_RESERVE,
                u64::from(pages),
                out.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        };
        if status != 0 {
            return Err(Error::Status(status));
        }
        let Ok(slot) = u8::try_from(out[0]) else {
            return Err(Error::InvalidReply);
        };
        let Ok(pages) = u32::try_from(out[2]) else {
            return Err(Error::InvalidReply);
        };
        if out[0] >= 128 || out[1] == 0 || pages == 0 || !out[1].is_multiple_of(4096) {
            return Err(Error::InvalidReply);
        }
        Ok(Self {
            slot,
            base: out[1],
            pages,
        })
    }

    pub const fn base(&self) -> u64 {
        self.base
    }

    pub const fn pages(&self) -> u32 {
        self.pages
    }

    pub const fn cap_slot(&self) -> u8 {
        self.slot
    }

    pub fn commit(
        &self,
        offset_pages: u32,
        pages: u32,
        protection: Protection,
    ) -> Result<(), Error> {
        let status = unsafe {
            syscall6(
                SYS_VM_COMMIT,
                u64::from(self.slot),
                u64::from(offset_pages),
                u64::from(pages),
                protection.bits(),
                0,
                0,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(Error::Status(status))
        }
    }

    pub fn protect(
        &self,
        offset_pages: u32,
        pages: u32,
        protection: Protection,
    ) -> Result<(), Error> {
        let status = unsafe {
            syscall6(
                SYS_VM_PROTECT,
                u64::from(self.slot),
                u64::from(offset_pages),
                u64::from(pages),
                protection.bits(),
                0,
                0,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(Error::Status(status))
        }
    }

    pub fn query(&self) -> Result<Query, Error> {
        let mut out = [0u64; 6];
        let status = unsafe {
            syscall6(
                SYS_VM_QUERY,
                u64::from(self.slot),
                out.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        };
        if status != 0 {
            return Err(Error::Status(status));
        }
        let (
            Ok(capacity_pages),
            Ok(committed_pages),
            Ok(global_committed_pages),
            Ok(guard_pages),
            Ok(global_regions),
        ) = (
            u32::try_from(out[1]),
            u32::try_from(out[2]),
            u32::try_from(out[3]),
            u32::try_from(out[4]),
            u32::try_from(out[5]),
        )
        else {
            return Err(Error::InvalidReply);
        };
        if out[0] != self.base || capacity_pages != self.pages || guard_pages != 1 {
            return Err(Error::InvalidReply);
        }
        Ok(Query {
            base: out[0],
            capacity_pages,
            committed_pages,
            global_committed_pages,
            guard_pages,
            global_regions,
        })
    }

    pub fn release(self) -> Result<(), Error> {
        let status = unsafe { syscall6(SYS_VM_RELEASE, u64::from(self.slot), 0, 0, 0, 0, 0) };
        if status == 0 {
            Ok(())
        } else {
            Err(Error::Status(status))
        }
    }
}
