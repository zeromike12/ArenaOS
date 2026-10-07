//! Bounded native user threads in one ArenaOS Process address space.
//!
//! Each created thread owns an exact, non-copyable VM reservation containing
//! its TLS block, an internal guard page, and a committed RW/NX stack. The
//! Process and its capability table remain shared; no capabilities are copied
//! to the new execution context.

use core::ptr::NonNull;

use arena_lib::abi::{
    SYS_THREAD_COUNT, SYS_THREAD_CREATE, SYS_THREAD_DETACH, SYS_THREAD_EXIT, SYS_THREAD_JOIN,
    SYS_THREAD_YIELD, syscall1, syscall6,
};

use crate::vm::{self, Protection, Region};

const STACK_REGION_PAGES: u32 = 16;
const FIRST_STACK_PAGE: u32 = 2;
const STACK_PAGE_COUNT: u32 = STACK_REGION_PAGES - FIRST_STACK_PAGE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Vm(vm::Error),
    Status(i64),
    InvalidReply,
}

impl From<vm::Error> for Error {
    fn from(value: vm::Error) -> Self {
        Self::Vm(value)
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct StartContext {
    entry: extern "C" fn(u64) -> u64,
    argument: u64,
}

/// Result collected from a joined thread. `application_word` is the TCB's
/// ArenaOS-defined FS:8 word at exit, useful for small runtime-owned thread
/// state without a process-global lookup table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub exit_status: u64,
    pub application_word: u64,
}

/// A descriptive same-Process thread ID plus the exact private stack region
/// retained until join. Dropping an unjoined handle detaches the thread; the
/// kernel then releases its exact VM cap after the thread is off its stack.
pub struct JoinHandle {
    id: u64,
    tls_base: u64,
    region: Option<Region>,
    tcb: NonNull<crate::tls::ThreadControlBlock>,
    joined: bool,
    detached: bool,
}

impl JoinHandle {
    pub const fn id(&self) -> u64 {
        self.id
    }

    pub const fn tls_base(&self) -> u64 {
        self.tls_base
    }

    /// Descriptive slot for targeted diagnostics. Possession of this number
    /// does not authorize operations; the kernel revalidates its live cap.
    pub fn stack_cap_slot(&self) -> Option<u8> {
        self.region.as_ref().map(Region::cap_slot)
    }

    /// Wait for exit, collect the stable status and TLS application word, and
    /// release the exact stack region. A refused join leaves this handle
    /// eligible for its Drop detach attempt.
    pub fn join(mut self) -> Result<Outcome, Error> {
        let mut exit_status = 0u64;
        let status = unsafe {
            syscall6(
                SYS_THREAD_JOIN,
                self.id,
                &mut exit_status as *mut u64 as u64,
                0,
                0,
                0,
                0,
            )
        };
        if status != 0 {
            return Err(Error::Status(status));
        }
        self.joined = true;
        let application_word =
            unsafe { core::ptr::addr_of!((*self.tcb.as_ptr()).application_word).read_volatile() };
        let region = self.region.take().ok_or(Error::InvalidReply)?;
        region.release()?;
        Ok(Outcome {
            exit_status,
            application_word,
        })
    }

    /// Revoke the host side's join claim. The kernel retains and releases the
    /// target's stack reservation when its execution context becomes safe to
    /// reap.
    pub fn detach(mut self) -> Result<(), Error> {
        let status = unsafe { syscall6(SYS_THREAD_DETACH, self.id, 0, 0, 0, 0, 0) };
        if status != 0 {
            return Err(Error::Status(status));
        }
        self.detached = true;
        self.region.take();
        Ok(())
    }
}

impl Drop for JoinHandle {
    fn drop(&mut self) {
        if self.joined || self.detached {
            return;
        }
        let status = unsafe { syscall6(SYS_THREAD_DETACH, self.id, 0, 0, 0, 0, 0) };
        if status == 0 {
            self.detached = true;
            self.region.take();
        }
    }
}

/// Create a user thread in the calling Process. `entry` starts with `arg` in
/// RDI and returns a stable `u64` exit status. The wrapper provisions a lazy
/// 16-page VM region: one TLS/metadata page, one uncommitted guard page, and a
/// 14-page RW/NX stack. Only the exact VmRegion capability authorizes it.
pub fn spawn(entry: extern "C" fn(u64) -> u64, argument: u64) -> Result<JoinHandle, Error> {
    let region = Region::reserve(STACK_REGION_PAGES)?;
    if let Err(error) = region.commit(0, 1, Protection::READ_WRITE) {
        let _ = region.release();
        return Err(Error::Vm(error));
    }
    if let Err(error) = region.commit(FIRST_STACK_PAGE, STACK_PAGE_COUNT, Protection::READ_WRITE) {
        let _ = region.release();
        return Err(Error::Vm(error));
    }

    let base = region.base();
    let tls = base as *mut crate::tls::ThreadControlBlock;
    let context =
        (base + core::mem::size_of::<crate::tls::ThreadControlBlock>() as u64) as *mut StartContext;
    let stack_top = base + u64::from(STACK_REGION_PAGES) * 4096;
    let stack_low = base + u64::from(FIRST_STACK_PAGE) * 4096;
    let user_rsp = stack_top - 8;
    // SAFETY: the committed first and stack pages belong to this exact VM
    // reservation in the caller's process; no other thread has these pages.
    unsafe {
        tls.write(crate::tls::ThreadControlBlock::new());
        (*tls).initialize();
        context.write(StartContext { entry, argument });
        (user_rsp as *mut u64).write(0);
    }
    let fs_base = tls as u64;
    let status = unsafe {
        syscall6(
            SYS_THREAD_CREATE,
            thread_entry as *const () as u64,
            context as u64,
            u64::from(region.cap_slot()),
            stack_low,
            stack_top,
            fs_base,
        )
    };
    if status <= 0 {
        let _ = region.release();
        return Err(Error::Status(status));
    }
    let Ok(id) = u64::try_from(status) else {
        let _ = region.release();
        return Err(Error::InvalidReply);
    };
    Ok(JoinHandle {
        id,
        tls_base: fs_base,
        region: Some(region),
        // SAFETY: region.base() is page aligned and holds the initialized TCB.
        tcb: unsafe { NonNull::new_unchecked(tls) },
        joined: false,
        detached: false,
    })
}

extern "C" fn thread_entry(context: u64) -> ! {
    let context = unsafe { (context as *const StartContext).read_volatile() };
    let status = (context.entry)(context.argument);
    let _ = unsafe { syscall1(SYS_THREAD_EXIT, status) };
    loop {
        core::hint::spin_loop();
    }
}

/// Yield to another Ready thread in this Process or kernel service set.
pub fn yield_now() -> Result<(), Error> {
    let status = unsafe { syscall6(SYS_THREAD_YIELD, 0, 0, 0, 0, 0, 0) };
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Status(status))
    }
}

/// Number of live scheduler threads belonging to the calling Process,
/// including its original image thread and user-created threads.
pub fn count() -> Result<u64, Error> {
    let status = unsafe { syscall6(SYS_THREAD_COUNT, 0, 0, 0, 0, 0, 0) };
    u64::try_from(status).map_err(|_| Error::Status(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_layout_leaves_metadata_and_guard_pages_outside_stack() {
        assert_eq!(STACK_REGION_PAGES, 16);
        assert_eq!(FIRST_STACK_PAGE, 2);
        assert_eq!(STACK_PAGE_COUNT, 14);
        assert_eq!(FIRST_STACK_PAGE + STACK_PAGE_COUNT, STACK_REGION_PAGES);
        assert_eq!(core::mem::size_of::<StartContext>(), 16);
    }
}
