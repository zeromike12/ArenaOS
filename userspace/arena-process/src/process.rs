//! Explicit native child-process ownership through held Process capabilities.
//!
//! A spawn result PID is retained only as descriptive diagnostics while the
//! wrapper scans for the exact Process capability the kernel granted to the
//! caller. Liveness and finish operations always use that cap's slot. Process
//! group membership is a generation-checked runtime handle, never a PID or app
//! name. Groups are single-owner; user-thread sharing needs synchronization.

use arena_lib::abi::{
    CAP_KIND_PROCESS, CAP_NONE, CAP_SLOTS, MAX_SPAWN_INHERIT, RIGHTS_ALL, RIGHTS_DESTROY,
    RIGHTS_READ, SYS_CAP_DESCRIBE, SYS_CAP_OCCUPIED, SYS_PROC_FINISH, SYS_PROC_LIVE,
    SYS_PROC_STATUS, SYS_SPAWN, SYS_WAIT, syscall1, syscall2, syscall5, syscall6,
};

use crate::handles::{Error as HandleError, Handle, HandleTable, InsertError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct InheritGrant {
    pub source_slot: u64,
    pub rights: u64,
}

impl InheritGrant {
    pub const fn new(source_slot: u8, rights: u32) -> Self {
        Self {
            source_slot: source_slot as u64,
            rights: rights as u64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitSignal {
    pub notification_slot: u8,
    pub badge: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum FinishMode {
    ReapExited = 0,
    StopAndReap = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnError {
    ImageSlotOutOfRange(u8),
    TooManyGrants(usize),
    InvalidGrant(usize),
    InvalidExitSignal,
    Kernel(i64),
    InvalidPidPayload(i64),
    CapabilityQuery(i64),
    ProcessCapabilityMissing(u64),
    ProcessCapabilityAmbiguous(u64),
    ProcessCapabilityRights(u64),
}

/// Owns one exact parent-held Process capability for a spawned child.
/// `diagnostic_pid` is descriptive only; all operations below use `cap_slot`.
/// This wrapper is deliberately not RAII: callers must finish/reap it through
/// `ProcessGroup` or call `finish` explicitly before dropping it.
#[derive(Debug, PartialEq, Eq)]
pub struct ChildProcess {
    diagnostic_pid: u64,
    cap_slot: u8,
    active: bool,
}

impl ChildProcess {
    /// Spawn from a caller-held readable Image/BootImage capability with only
    /// the listed, attenuation-checked grants. The kernel returns a PID as a
    /// payload; it is used solely to locate and verify the newly granted
    /// Process capability before this wrapper is constructed.
    pub fn spawn(
        image_slot: u8,
        grants: &[InheritGrant],
        exit_signal: Option<ExitSignal>,
    ) -> Result<Self, SpawnError> {
        if u64::from(image_slot) >= CAP_SLOTS as u64 {
            return Err(SpawnError::ImageSlotOutOfRange(image_slot));
        }
        if grants.len() > MAX_SPAWN_INHERIT {
            return Err(SpawnError::TooManyGrants(grants.len()));
        }
        for (index, grant) in grants.iter().enumerate() {
            if grant.source_slot >= CAP_SLOTS as u64
                || grant.rights == 0
                || grant.rights & !RIGHTS_ALL != 0
            {
                return Err(SpawnError::InvalidGrant(index));
            }
        }
        let (notification_slot, badge) = match exit_signal {
            Some(signal)
                if u64::from(signal.notification_slot) < CAP_SLOTS as u64 && signal.badge != 0 =>
            {
                (u64::from(signal.notification_slot), signal.badge)
            }
            Some(_) => return Err(SpawnError::InvalidExitSignal),
            None => (CAP_NONE, 0),
        };
        let spec_ptr = if grants.is_empty() {
            0
        } else {
            grants.as_ptr() as u64
        };
        let result = unsafe {
            syscall5(
                SYS_SPAWN,
                u64::from(image_slot),
                spec_ptr,
                grants.len() as u64,
                notification_slot,
                badge,
            )
        };
        if result < 0 {
            return Err(SpawnError::Kernel(result));
        }
        if result == 0 {
            return Err(SpawnError::InvalidPidPayload(result));
        }
        let pid = result as u64;
        let cap_slot = find_process_capability(pid)?;
        Ok(Self {
            diagnostic_pid: pid,
            cap_slot,
            active: true,
        })
    }

    /// Diagnostic metadata only. Never pass this value to authorize an
    /// operation; `is_live` and `finish` use the held Process-cap slot.
    pub const fn diagnostic_pid(&self) -> u64 {
        self.diagnostic_pid
    }

    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Check liveness through the held Process/READ capability.
    pub fn is_live(&self) -> Result<bool, i64> {
        if !self.active {
            return Ok(false);
        }
        match unsafe { syscall1(SYS_PROC_LIVE, u64::from(self.cap_slot)) } {
            0 => Ok(false),
            1 => Ok(true),
            status => Err(status),
        }
    }

    /// Query the process record's stable final status through this exact
    /// Process/READ cap. `None` means at least one thread remains live;
    /// `Some(code)` remains available until this wrapper reaps the child.
    pub fn exit_status(&self) -> Result<Option<u64>, i64> {
        if !self.active {
            return Err(-1);
        }
        let mut result = [0u64; 2];
        let status = unsafe {
            syscall6(
                SYS_PROC_STATUS,
                u64::from(self.cap_slot),
                result.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        };
        if status != 0 {
            return Err(status);
        }
        match result[0] {
            0 => Ok(None),
            1 => Ok(Some(result[1])),
            _ => Err(-1),
        }
    }

    /// Reap an exited child or explicitly stop a live child, as selected by
    /// `mode`. On syscall refusal the wrapper remains active and retryable.
    pub fn finish(&mut self, mode: FinishMode) -> Result<(), i64> {
        if !self.active {
            return Ok(());
        }
        let status = unsafe { syscall2(SYS_PROC_FINISH, u64::from(self.cap_slot), mode as u64) };
        if status == 0 {
            self.active = false;
            Ok(())
        } else {
            Err(status)
        }
    }
}

fn find_process_capability(pid: u64) -> Result<u8, SpawnError> {
    let required_rights = RIGHTS_READ | RIGHTS_DESTROY;
    let mut found = None;
    for slot in 0..CAP_SLOTS {
        match unsafe { syscall6(SYS_CAP_OCCUPIED, slot as u64, 0, 0, 0, 0, 0) } {
            0 => continue,
            1 => {}
            status => return Err(SpawnError::CapabilityQuery(status)),
        }
        let mut words = [0u64; 3];
        let status = unsafe { syscall2(SYS_CAP_DESCRIBE, slot as u64, words.as_mut_ptr() as u64) };
        // Other occupied kinds may intentionally be undescribable. The exact
        // Process capability minted by SYS_SPAWN must be describable as kind 4.
        if status != 0 || words[0] != u64::from(CAP_KIND_PROCESS) {
            continue;
        }
        if words[1] != pid {
            continue;
        }
        if words[2] != required_rights {
            return Err(SpawnError::ProcessCapabilityRights(pid));
        }
        if found.replace(slot as u8).is_some() {
            return Err(SpawnError::ProcessCapabilityAmbiguous(pid));
        }
    }
    found.ok_or(SpawnError::ProcessCapabilityMissing(pid))
}

/// Only a process-like runtime owner can be added to a group. The trait keeps
/// bounded group policy host-testable; the native implementation still uses
/// exact Process-cap syscalls.
pub trait ChildLifecycle {
    fn is_active(&self) -> bool;
    fn is_live(&self) -> Result<bool, i64>;
    fn exit_status(&self) -> Result<Option<u64>, i64> {
        Ok(None)
    }
    fn finish(&mut self, mode: FinishMode) -> Result<(), i64>;
}

impl ChildLifecycle for ChildProcess {
    fn is_active(&self) -> bool {
        ChildProcess::is_active(self)
    }

    fn is_live(&self) -> Result<bool, i64> {
        ChildProcess::is_live(self)
    }

    fn exit_status(&self) -> Result<Option<u64>, i64> {
        ChildProcess::exit_status(self)
    }

    fn finish(&mut self, mode: FinishMode) -> Result<(), i64> {
        ChildProcess::finish(self, mode)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum GroupSpawnError<E, T> {
    Full,
    Spawn(E),
    Record(InsertError<T>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupError {
    Handle(HandleError),
    Process(i64),
}

/// Fixed-capacity userspace process group. Members are inserted only from a
/// successful capability-authorized spawn, and are addressed by opaque runtime
/// handles rather than PIDs or application names.
pub struct ProcessGroup<T, const N: usize> {
    members: HandleTable<T, N>,
}

impl<T, const N: usize> ProcessGroup<T, N> {
    pub fn new() -> Self {
        Self {
            members: HandleTable::new(),
        }
    }

    pub const fn len(&self) -> usize {
        self.members.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn can_spawn(&self) -> bool {
        self.members.can_insert()
    }

    /// Preflight group capacity before running `spawn`. A post-spawn table
    /// refusal returns the still-owned child wrapper for explicit rollback.
    pub fn spawn<E>(
        &mut self,
        spawn: impl FnOnce() -> Result<T, E>,
    ) -> Result<Handle, GroupSpawnError<E, T>> {
        if !self.members.can_insert() {
            return Err(GroupSpawnError::Full);
        }
        let child = spawn().map_err(GroupSpawnError::Spawn)?;
        self.members.insert(child).map_err(GroupSpawnError::Record)
    }

    pub fn get(&self, handle: Handle) -> Result<&T, HandleError> {
        self.members.get(handle)
    }

    pub fn is_live(&self, handle: Handle) -> Result<bool, GroupError>
    where
        T: ChildLifecycle,
    {
        self.members
            .get(handle)
            .map_err(GroupError::Handle)?
            .is_live()
            .map_err(GroupError::Process)
    }

    pub fn exit_status(&self, handle: Handle) -> Result<Option<u64>, GroupError>
    where
        T: ChildLifecycle,
    {
        self.members
            .get(handle)
            .map_err(GroupError::Handle)?
            .exit_status()
            .map_err(GroupError::Process)
    }

    /// Wait until this member exits, using the caller's explicitly held
    /// Notification/READ slot. A shared notification may wake for another
    /// child or event; the held Process cap remains the authority, so status
    /// is rechecked after every wake.
    pub fn wait(&self, handle: Handle, notification_slot: u8) -> Result<u64, GroupError>
    where
        T: ChildLifecycle,
    {
        loop {
            if let Some(status) = self.exit_status(handle)? {
                return Ok(status);
            }
            let wake = unsafe { syscall1(SYS_WAIT, u64::from(notification_slot)) };
            if wake < 0 {
                return Err(GroupError::Process(wake));
            }
        }
    }

    pub fn reap_exited(&mut self, handle: Handle) -> Result<(), GroupError>
    where
        T: ChildLifecycle,
    {
        self.finish_and_remove(handle, FinishMode::ReapExited)
    }

    pub fn stop_and_reap(&mut self, handle: Handle) -> Result<(), GroupError>
    where
        T: ChildLifecycle,
    {
        self.finish_and_remove(handle, FinishMode::StopAndReap)
    }

    /// Explicit group teardown: live children are stopped; already-exited
    /// children are reaped. Any refusal preserves that member and all members
    /// not yet processed so the caller can report or retry cleanup.
    pub fn stop_all(&mut self) -> Result<(), GroupError>
    where
        T: ChildLifecycle,
    {
        let mut snapshot = [None; N];
        for (index, handle) in self.members.handles().enumerate() {
            snapshot[index] = Some(handle);
        }
        for handle in snapshot.into_iter().flatten() {
            let child = self.members.get_mut(handle).map_err(GroupError::Handle)?;
            if child.is_active() {
                let mode = if child.is_live().map_err(GroupError::Process)? {
                    FinishMode::StopAndReap
                } else {
                    FinishMode::ReapExited
                };
                child.finish(mode).map_err(GroupError::Process)?;
            }
            self.members.close(handle).map_err(GroupError::Handle)?;
        }
        Ok(())
    }

    fn finish_and_remove(&mut self, handle: Handle, mode: FinishMode) -> Result<(), GroupError>
    where
        T: ChildLifecycle,
    {
        let child = self.members.get_mut(handle).map_err(GroupError::Handle)?;
        child.finish(mode).map_err(GroupError::Process)?;
        self.members.close(handle).map_err(GroupError::Handle)?;
        Ok(())
    }
}

impl<T, const N: usize> Default for ProcessGroup<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use core::cell::Cell;
    use std::rc::Rc;

    #[derive(Debug)]
    struct FakeChild {
        active: bool,
        live: bool,
        exit_status: Option<u64>,
        last_finish: Rc<Cell<Option<FinishMode>>>,
        fail_finish: bool,
    }

    impl ChildLifecycle for FakeChild {
        fn is_active(&self) -> bool {
            self.active
        }

        fn is_live(&self) -> Result<bool, i64> {
            Ok(self.live)
        }

        fn exit_status(&self) -> Result<Option<u64>, i64> {
            Ok(self.exit_status)
        }

        fn finish(&mut self, mode: FinishMode) -> Result<(), i64> {
            if self.fail_finish {
                return Err(-3);
            }
            self.last_finish.set(Some(mode));
            self.active = false;
            Ok(())
        }
    }

    fn fake(live: bool) -> (FakeChild, Rc<Cell<Option<FinishMode>>>) {
        fake_with_status(live, (!live).then_some(0))
    }

    fn fake_with_status(
        live: bool,
        exit_status: Option<u64>,
    ) -> (FakeChild, Rc<Cell<Option<FinishMode>>>) {
        let last_finish = Rc::new(Cell::new(None));
        (
            FakeChild {
                active: true,
                live,
                exit_status,
                last_finish: last_finish.clone(),
                fail_finish: false,
            },
            last_finish,
        )
    }

    #[test]
    fn group_exit_status_is_member_scoped_and_preserves_full_u64() {
        let mut group = ProcessGroup::<FakeChild, 2>::new();
        let (running, _) = fake(true);
        let (exited, _) = fake_with_status(false, Some(u64::MAX));
        let running_handle = group.spawn(|| Ok::<_, ()>(running)).unwrap();
        let exited_handle = group.spawn(|| Ok::<_, ()>(exited)).unwrap();
        assert_eq!(group.exit_status(running_handle), Ok(None));
        assert_eq!(group.exit_status(exited_handle), Ok(Some(u64::MAX)));
        assert!(matches!(
            group.exit_status(Handle::from_raw(u32::MAX)),
            Err(GroupError::Handle(_))
        ));
    }

    #[test]
    fn group_refuses_capacity_before_spawning_and_stales_removed_members() {
        let mut group = ProcessGroup::<FakeChild, 1>::new();
        let (child, _) = fake(true);
        let handle = group.spawn(|| Ok::<_, ()>(child)).unwrap();
        let mut spawn_called = false;
        assert!(matches!(
            group.spawn(|| {
                spawn_called = true;
                Ok::<_, ()>(fake(false).0)
            }),
            Err(GroupSpawnError::Full)
        ));
        assert!(!spawn_called);
        assert_eq!(group.len(), 1);
        group.stop_and_reap(handle).unwrap();
        assert!(matches!(group.get(handle), Err(HandleError::Stale)));
        assert!(group.is_empty());
    }

    #[test]
    fn group_teardown_stops_live_and_reaps_exited_members() {
        let mut group = ProcessGroup::<FakeChild, 2>::new();
        let (live, live_mode) = fake(true);
        let (exited, exited_mode) = fake(false);
        group.spawn(|| Ok::<_, ()>(live)).unwrap();
        group.spawn(|| Ok::<_, ()>(exited)).unwrap();
        group.stop_all().unwrap();
        assert_eq!(live_mode.get(), Some(FinishMode::StopAndReap));
        assert_eq!(exited_mode.get(), Some(FinishMode::ReapExited));
        assert!(group.is_empty());
    }

    #[test]
    fn failed_member_cleanup_keeps_its_exact_handle_for_retry() {
        let mut group = ProcessGroup::<FakeChild, 1>::new();
        let (mut child, _) = fake(true);
        child.fail_finish = true;
        let handle = group.spawn(|| Ok::<_, ()>(child)).unwrap();
        assert_eq!(group.stop_all(), Err(GroupError::Process(-3)));
        assert_eq!(group.len(), 1);
        assert_eq!(group.is_live(handle), Ok(true));
    }
}
