//! Minimal native x86-64 user TLS rooted at the per-thread FS.base MSR.
//!
//! This is an ArenaOS TCB contract, not ELF PT_TLS, GNU TLS, POSIX, or a
//! dynamic linker ABI. The kernel validates and saves/restores only the
//! caller's FS.base; GS remains reserved for the syscall `swapgs` path.

use core::ptr::NonNull;

use arena_lib::abi::{SYS_TLS_SET, syscall6};

/// Minimal ArenaOS thread-control-block header. FS:0 is `self_pointer`; the
/// first application word is at FS:8. Applications may extend this with a
/// separately versioned runtime-owned layout.
#[repr(C, align(16))]
pub struct ThreadControlBlock {
    self_pointer: usize,
    pub application_word: u64,
}

impl ThreadControlBlock {
    pub const fn new() -> Self {
        Self {
            self_pointer: 0,
            application_word: 0,
        }
    }
}

impl Default for ThreadControlBlock {
    fn default() -> Self {
        Self::new()
    }
}

/// Install `block` as this user thread's FS.base. The kernel accepts only a
/// 16-byte-aligned, writable, NX page in this thread's registered user map.
///
/// # Safety
/// The block must remain writable and mapped in this process for as long as
/// this thread can execute with the new FS.base. It must not be freed or
/// repurposed until TLS is cleared or the thread exits.
pub unsafe fn install(block: NonNull<ThreadControlBlock>) -> Result<(), i64> {
    let base = block.as_ptr() as usize;
    // SAFETY: the caller promises this is a live, uniquely owned TCB. The
    // kernel independently validates the mapping and permissions before the
    // MSR is changed.
    unsafe { core::ptr::addr_of_mut!((*block.as_ptr()).self_pointer).write(base) };
    // SYS_TLS_SET is a one-argument call with all reserved registers zero.
    // SAFETY: the kernel treats `base` as data and validates its exact user PTE.
    let status = unsafe { syscall6(SYS_TLS_SET, base as u64, 0, 0, 0, 0, 0) };
    if status == 0 { Ok(()) } else { Err(status) }
}

/// Read FS:0 as an untrusted, non-null pointer value. Call only after a
/// successful [`install`]: when FS.base is zero, an FS-relative load from
/// address zero can fault. The caller must uphold the same lifetime rule as
/// [`install`] before dereferencing the result.
pub fn current() -> Option<NonNull<ThreadControlBlock>> {
    let base: usize;
    // SAFETY: FS:0 is the documented user TCB self pointer. This reads only
    // user-selected FS-relative memory and does not modify kernel GS state.
    unsafe {
        core::arch::asm!(
            "mov {}, fs:[0]",
            out(reg) base,
            options(nostack, readonly, preserves_flags),
        );
    }
    NonNull::new(base as *mut ThreadControlBlock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn tcb_layout_is_frozen_for_the_arena_fs_base_contract() {
        assert_eq!(size_of::<ThreadControlBlock>(), 16);
        assert_eq!(align_of::<ThreadControlBlock>(), 16);
        assert_eq!(offset_of!(ThreadControlBlock, self_pointer), 0);
        assert_eq!(offset_of!(ThreadControlBlock, application_word), 8);
        let block = ThreadControlBlock::new();
        assert_eq!(block.self_pointer, 0);
        assert_eq!(block.application_word, 0);
    }
}
