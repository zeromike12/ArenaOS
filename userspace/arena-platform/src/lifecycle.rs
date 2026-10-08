//! Generation-safe userspace app/process/window ownership model.
//!
//! This is metadata/state management only; it does not invoke SYS_SPAWN,
//! retain a kernel capability, allocate a surface, or authorize an IPC call.
//! A real manager must bind each `manager_cap_slot` only after it has received
//! and retained the exact Process capability returned by an allowlisted spawn.
//! PIDs, app IDs, and window handles never substitute for that held authority.

use crate::manifest::{ID_BYTES, Manifest};
use crate::registry::AppDefinition;

/// Must match the descriptive startup-slot domain; window capacity is a
/// separate table and app/process identities are still userspace-owned.
pub const MAX_APP_INSTANCES: usize = crate::startup::INSTANCE_SLOTS;
pub const MAX_GROUP_PROCESSES: usize = 4;
pub const MAX_ORDINARY_WINDOWS: usize = 32;
pub const MAX_WINDOW_WIDTH: u16 = 1024;
pub const MAX_WINDOW_HEIGHT: u16 = 768;
pub const MIN_WINDOW_WIDTH: u16 = 80;
pub const MIN_WINDOW_HEIGHT: u16 = 60;
const MANAGER_CAP_SLOTS: u8 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Full,
    SingleInstanceAlreadyLive,
    NotFound,
    StaleInstance,
    StaleProcess,
    StaleWindow,
    BadState,
    InvalidManagerCapSlot,
    DuplicateManagerCapSlot,
    TooManyProcesses,
    TooManyWindows,
    InvalidDimensions,
    NotOwner,
    LiveProcesses,
    LiveWindows,
    GenerationExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppInstanceId {
    slot: u8,
    generation: u32,
}
impl AppInstanceId {
    pub fn slot(self) -> usize {
        self.slot as usize
    }
    pub fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessRef {
    instance: AppInstanceId,
    slot: u8,
    generation: u32,
}
impl ProcessRef {
    pub fn instance(self) -> AppInstanceId {
        self.instance
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowHandle {
    slot: u8,
    generation: u32,
}
impl WindowHandle {
    pub fn slot(self) -> usize {
        self.slot as usize
    }
    pub fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceState {
    Starting,
    Running,
    Stopping,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessRole {
    Primary,
    Helper,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessMember {
    reference: ProcessRef,
    manager_cap_slot: u8,
    role: ProcessRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessGroup {
    members: [Option<ProcessMember>; MAX_GROUP_PROCESSES],
    generations: [u32; MAX_GROUP_PROCESSES],
    retired: [bool; MAX_GROUP_PROCESSES],
    len: usize,
}
impl ProcessGroup {
    const fn new() -> Self {
        Self {
            members: [None; MAX_GROUP_PROCESSES],
            generations: [1; MAX_GROUP_PROCESSES],
            retired: [false; MAX_GROUP_PROCESSES],
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn contains(&self, process: ProcessRef) -> bool {
        self.members
            .iter()
            .flatten()
            .any(|member| member.reference == process)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppInstance {
    id: AppInstanceId,
    app_id: [u8; ID_BYTES],
    flags: u32,
    state: InstanceState,
    group: ProcessGroup,
}
impl AppInstance {
    pub fn id(&self) -> AppInstanceId {
        self.id
    }
    pub fn app_id(&self) -> &[u8; ID_BYTES] {
        &self.app_id
    }
    pub fn state(&self) -> InstanceState {
        self.state
    }
    pub fn process_group(&self) -> &ProcessGroup {
        &self.group
    }
    pub fn allows_background(&self) -> bool {
        self.flags & crate::manifest::FLAG_BACKGROUND != 0
    }
    pub fn allows_headless(&self) -> bool {
        self.flags & crate::manifest::FLAG_HEADLESS != 0
    }
}

/// Fixed-capacity application-instance and process-group metadata.
/// Place this long-lived table in manager-owned static/runtime memory, not on
/// the current initial stack. An instance slot is not a kernel process slot.
pub struct AppInstanceTable {
    entries: [Option<AppInstance>; MAX_APP_INSTANCES],
    generations: [u32; MAX_APP_INSTANCES],
    retired: [bool; MAX_APP_INSTANCES],
    len: usize,
}
impl AppInstanceTable {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_APP_INSTANCES],
            generations: [1; MAX_APP_INSTANCES],
            retired: [false; MAX_APP_INSTANCES],
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn get(&self, id: AppInstanceId) -> Result<&AppInstance, Error> {
        self.record(id)
    }
    pub fn state(&self, id: AppInstanceId) -> Result<InstanceState, Error> {
        Ok(self.record(id)?.state)
    }
    pub fn app_id(&self, id: AppInstanceId) -> Result<&[u8; ID_BYTES], Error> {
        Ok(&self.record(id)?.app_id)
    }
    pub fn allows_background(&self, id: AppInstanceId) -> Result<bool, Error> {
        Ok(self.record(id)?.flags & crate::manifest::FLAG_BACKGROUND != 0)
    }
    pub fn allows_headless(&self, id: AppInstanceId) -> Result<bool, Error> {
        Ok(self.record(id)?.flags & crate::manifest::FLAG_HEADLESS != 0)
    }
    pub fn process_count(&self, id: AppInstanceId) -> Result<usize, Error> {
        Ok(self.record(id)?.group.len)
    }
    pub fn contains_process(&self, id: AppInstanceId, process: ProcessRef) -> bool {
        process.instance == id
            && self.record(id).is_ok_and(|record| {
                record
                    .group
                    .members
                    .iter()
                    .flatten()
                    .any(|m| m.reference == process)
            })
    }

    /// Reserve an app-instance record after registry/signature/policy
    /// resolution. IDs/names do not grant authority. Single-instance policy
    /// is derived from the signed manifest; full/single-instance refusal
    /// leaves every slot and generation untouched.
    pub fn create(&mut self, app: &AppDefinition) -> Result<AppInstanceId, Error> {
        if !app.manifest.allows_multiple_instances()
            && self
                .entries
                .iter()
                .flatten()
                .any(|record| record.app_id == *app.application_id())
        {
            return Err(Error::SingleInstanceAlreadyLive);
        }
        let Some(slot) = (0..MAX_APP_INSTANCES)
            .find(|&slot| self.entries[slot].is_none() && !self.retired[slot])
        else {
            return Err(Error::Full);
        };
        let id = AppInstanceId {
            slot: slot as u8,
            generation: self.generations[slot],
        };
        self.entries[slot] = Some(AppInstance {
            id,
            app_id: *app.application_id(),
            flags: app.manifest.flags(),
            state: InstanceState::Starting,
            group: ProcessGroup::new(),
        });
        self.len += 1;
        Ok(id)
    }

    /// Bind the exact manager-held Process cap slot for the primary process.
    /// This is a trusted-manager bookkeeping operation, not a process request.
    pub fn bind_primary(
        &mut self,
        id: AppInstanceId,
        manager_cap_slot: u8,
    ) -> Result<ProcessRef, Error> {
        if manager_cap_slot >= MANAGER_CAP_SLOTS {
            return Err(Error::InvalidManagerCapSlot);
        }
        if self.cap_slot_in_use(manager_cap_slot) {
            return Err(Error::DuplicateManagerCapSlot);
        }
        let record = self.record_mut(id)?;
        if record.state != InstanceState::Starting || record.group.len != 0 {
            return Err(Error::BadState);
        }
        let process = record
            .group
            .bind(id, manager_cap_slot, ProcessRole::Primary, 0)?;
        record.state = InstanceState::Running;
        Ok(process)
    }

    /// Record a helper only after the app manager performs its separate
    /// allowlist check and explicit attenuated-cap spawn. No ambient cap list
    /// is stored or inherited by this model.
    pub fn bind_helper(
        &mut self,
        id: AppInstanceId,
        manager_cap_slot: u8,
    ) -> Result<ProcessRef, Error> {
        if manager_cap_slot >= MANAGER_CAP_SLOTS {
            return Err(Error::InvalidManagerCapSlot);
        }
        if self.cap_slot_in_use(manager_cap_slot) {
            return Err(Error::DuplicateManagerCapSlot);
        }
        let record = self.record_mut(id)?;
        if record.state != InstanceState::Running {
            return Err(Error::BadState);
        }
        let slot = (1..MAX_GROUP_PROCESSES)
            .find(|&slot| record.group.members[slot].is_none() && !record.group.retired[slot])
            .ok_or(Error::TooManyProcesses)?;
        record
            .group
            .bind(id, manager_cap_slot, ProcessRole::Helper, slot)
    }

    pub fn request_stop(&mut self, id: AppInstanceId) -> Result<(), Error> {
        let record = self.record_mut(id)?;
        match record.state {
            InstanceState::Starting | InstanceState::Running => {
                record.state = if record.group.len == 0 {
                    InstanceState::Exited
                } else {
                    InstanceState::Stopping
                };
                Ok(())
            }
            InstanceState::Stopping => Ok(()),
            InstanceState::Exited => Err(Error::BadState),
        }
    }

    /// Consume a process member only after the manager has performed the
    /// matching kernel Process-cap FINISH/REAP operation. A PID or app ID is
    /// not accepted by this interface.
    pub fn finish_process(&mut self, process: ProcessRef) -> Result<(), Error> {
        let record = self.record_mut(process.instance)?;
        if record.state == InstanceState::Exited {
            return Err(Error::BadState);
        }
        let slot = process.slot as usize;
        let member = record
            .group
            .members
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or(Error::StaleProcess)?;
        if member.reference != process {
            return Err(Error::StaleProcess);
        }
        let role = member.role;
        record.group.members[slot] = None;
        record.group.len -= 1;
        if record.group.generations[slot] == u32::MAX {
            record.group.retired[slot] = true;
        } else {
            record.group.generations[slot] += 1;
        }
        if role == ProcessRole::Primary && record.state == InstanceState::Running {
            record.state = InstanceState::Stopping;
        }
        if record.group.len == 0 {
            record.state = InstanceState::Exited;
        }
        Ok(())
    }

    pub fn retire(&mut self, id: AppInstanceId, windows: &WindowSet) -> Result<(), Error> {
        let record = self.record(id)?;
        if record.state != InstanceState::Exited {
            return Err(Error::BadState);
        }
        if record.group.len != 0 {
            return Err(Error::LiveProcesses);
        }
        if windows.count_for_instance(id) != 0 {
            return Err(Error::LiveWindows);
        }
        let slot = id.slot as usize;
        self.entries[slot] = None;
        self.len -= 1;
        if self.generations[slot] == u32::MAX {
            self.retired[slot] = true;
        } else {
            self.generations[slot] += 1;
        }
        Ok(())
    }

    fn cap_slot_in_use(&self, slot: u8) -> bool {
        self.entries.iter().flatten().any(|record| {
            record
                .group
                .members
                .iter()
                .flatten()
                .any(|member| member.manager_cap_slot == slot)
        })
    }
    fn record(&self, id: AppInstanceId) -> Result<&AppInstance, Error> {
        let record = self
            .entries
            .get(id.slot as usize)
            .and_then(Option::as_ref)
            .ok_or(Error::NotFound)?;
        (record.id == id)
            .then_some(record)
            .ok_or(Error::StaleInstance)
    }
    fn record_mut(&mut self, id: AppInstanceId) -> Result<&mut AppInstance, Error> {
        let record = self
            .entries
            .get_mut(id.slot as usize)
            .and_then(Option::as_mut)
            .ok_or(Error::NotFound)?;
        (record.id == id)
            .then_some(record)
            .ok_or(Error::StaleInstance)
    }
}
impl Default for AppInstanceTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessGroup {
    fn bind(
        &mut self,
        instance: AppInstanceId,
        manager_cap_slot: u8,
        role: ProcessRole,
        slot: usize,
    ) -> Result<ProcessRef, Error> {
        if slot >= MAX_GROUP_PROCESSES || self.retired[slot] || self.members[slot].is_some() {
            return Err(Error::TooManyProcesses);
        }
        let reference = ProcessRef {
            instance,
            slot: slot as u8,
            generation: self.generations[slot],
        };
        self.members[slot] = Some(ProcessMember {
            reference,
            manager_cap_slot,
            role,
        });
        self.len += 1;
        Ok(reference)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowRecord {
    handle: WindowHandle,
    instance: AppInstanceId,
    owner: ProcessRef,
    width: u16,
    height: u16,
}

/// Global userspace ownership table for up to 32 ordinary windows, independent
/// of the process-instance count. It stores no surfaces or kernel capabilities.
pub struct WindowSet {
    entries: [Option<WindowRecord>; MAX_ORDINARY_WINDOWS],
    generations: [u32; MAX_ORDINARY_WINDOWS],
    retired: [bool; MAX_ORDINARY_WINDOWS],
    len: usize,
}
impl WindowSet {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_ORDINARY_WINDOWS],
            generations: [1; MAX_ORDINARY_WINDOWS],
            retired: [false; MAX_ORDINARY_WINDOWS],
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn create(
        &mut self,
        instances: &AppInstanceTable,
        instance: AppInstanceId,
        owner: ProcessRef,
        width: u16,
        height: u16,
    ) -> Result<WindowHandle, Error> {
        if !(MIN_WINDOW_WIDTH..=MAX_WINDOW_WIDTH).contains(&width)
            || !(MIN_WINDOW_HEIGHT..=MAX_WINDOW_HEIGHT).contains(&height)
        {
            return Err(Error::InvalidDimensions);
        }
        if !instances.contains_process(instance, owner) {
            return Err(Error::NotOwner);
        }
        if instances.state(instance)? != InstanceState::Running {
            return Err(Error::BadState);
        }
        let Some(slot) = (0..MAX_ORDINARY_WINDOWS)
            .find(|&slot| self.entries[slot].is_none() && !self.retired[slot])
        else {
            return Err(Error::TooManyWindows);
        };
        let handle = WindowHandle {
            slot: slot as u8,
            generation: self.generations[slot],
        };
        self.entries[slot] = Some(WindowRecord {
            handle,
            instance,
            owner,
            width,
            height,
        });
        self.len += 1;
        Ok(handle)
    }

    pub fn get(
        &self,
        handle: WindowHandle,
    ) -> Result<(AppInstanceId, ProcessRef, u16, u16), Error> {
        let record = self
            .entries
            .get(handle.slot as usize)
            .and_then(Option::as_ref)
            .ok_or(Error::NotFound)?;
        if record.handle != handle {
            return Err(Error::StaleWindow);
        }
        Ok((record.instance, record.owner, record.width, record.height))
    }

    /// Only the exact process identity recorded as owner can close its window.
    /// The compositor/app manager may separately request administrative close.
    pub fn close_for_process(
        &mut self,
        instances: &AppInstanceTable,
        handle: WindowHandle,
        process: ProcessRef,
    ) -> Result<(), Error> {
        let record = self
            .entries
            .get(handle.slot as usize)
            .and_then(Option::as_ref)
            .ok_or(Error::NotFound)?;
        if record.handle != handle {
            return Err(Error::StaleWindow);
        }
        if record.owner != process {
            return Err(Error::NotOwner);
        }
        if !instances.contains_process(record.instance, process) {
            return Err(Error::StaleProcess);
        }
        self.release_slot(handle.slot as usize)
    }

    pub fn close_all_for_instance(&mut self, instance: AppInstanceId) -> usize {
        let mut removed = 0;
        for slot in 0..MAX_ORDINARY_WINDOWS {
            if self.entries[slot].is_some_and(|record| record.instance == instance) {
                let _ = self.release_slot(slot);
                removed += 1;
            }
        }
        removed
    }
    pub fn count_for_instance(&self, instance: AppInstanceId) -> usize {
        self.entries
            .iter()
            .flatten()
            .filter(|record| record.instance == instance)
            .count()
    }
    pub fn owned_by(&self, process: ProcessRef) -> usize {
        self.entries
            .iter()
            .flatten()
            .filter(|record| record.owner == process)
            .count()
    }

    fn release_slot(&mut self, slot: usize) -> Result<(), Error> {
        if self.entries[slot].take().is_none() {
            return Err(Error::NotFound);
        }
        self.len -= 1;
        if self.generations[slot] == u32::MAX {
            self.retired[slot] = true;
        } else {
            self.generations[slot] += 1;
        }
        Ok(())
    }
}
impl Default for WindowSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Small helper for host tests and manager-side manifest derivation. It returns
/// only booleans; no field is authority.
pub fn app_policy(manifest: &Manifest) -> (bool, bool, bool) {
    (
        manifest.allows_multiple_instances(),
        manifest.allows_background(),
        manifest.allows_headless(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{BundleSource, SourceError, Workspace};
    use std::string::ToString;

    const PUBLIC_KEY: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];
    const BUNDLE: &[u8] = include_bytes!("../tests/data/editor.apb1");

    struct Source<'a>(&'a [u8]);
    impl BundleSource for Source<'_> {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
            let start = usize::try_from(offset).map_err(|_| SourceError)?;
            let end = start.checked_add(out.len()).ok_or(SourceError)?;
            out.copy_from_slice(self.0.get(start..end).ok_or(SourceError)?);
            Ok(())
        }
    }
    fn verified_definition(flags: u32) -> AppDefinition {
        let mut source = Source(BUNDLE);
        let mut workspace = Workspace::new();
        let bundle = workspace.verify(&mut source, &PUBLIC_KEY).unwrap();
        let mut app = AppDefinition::from_verified_bundle(&bundle);
        let mut bytes = app.manifest.encode();
        bytes[112..116].copy_from_slice(&flags.to_le_bytes());
        app.manifest = Manifest::parse(&bytes).unwrap();
        app
    }
    fn allocate_instance(
        table: &mut AppInstanceTable,
        app: &AppDefinition,
        cap_slot: u8,
    ) -> (AppInstanceId, ProcessRef) {
        let id = table.create(app).unwrap();
        let process = table.bind_primary(id, cap_slot).unwrap();
        (id, process)
    }

    #[test]
    fn app_ids_do_not_authorize_instance_and_stale_generation_never_aliases() {
        let app = verified_definition(0);
        let mut table = AppInstanceTable::new();
        let first = table.create(&app).unwrap();
        assert_eq!(table.create(&app), Err(Error::SingleInstanceAlreadyLive));
        assert_eq!(table.len(), 1);
        let primary = table.bind_primary(first, 5).unwrap();
        assert_eq!(table.state(first), Ok(InstanceState::Running));
        assert_eq!(
            table.bind_helper(first, 5),
            Err(Error::DuplicateManagerCapSlot)
        );
        assert_eq!(
            table.bind_helper(first, 128),
            Err(Error::InvalidManagerCapSlot)
        );
        table.request_stop(first).unwrap();
        assert_eq!(table.state(first), Ok(InstanceState::Stopping));
        assert_eq!(table.finish_process(primary), Ok(()));
        assert_eq!(table.state(first), Ok(InstanceState::Exited));
        let windows = WindowSet::new();
        table.retire(first, &windows).unwrap();
        let second = table.create(&app).unwrap();
        assert_eq!(second.slot(), first.slot());
        assert_ne!(second.generation(), first.generation());
        assert_eq!(table.state(first), Err(Error::StaleInstance));
        assert_eq!(table.bind_primary(first, 5), Err(Error::StaleInstance));
    }

    #[test]
    fn exact_process_group_binding_multiwindow_owner_and_stale_window_refusal() {
        let app = verified_definition(1);
        let mut table = AppInstanceTable::new();
        let (instance, primary) = allocate_instance(&mut table, &app, 8);
        let helper = table.bind_helper(instance, 9).unwrap();
        assert!(table.contains_process(instance, primary));
        assert!(table.contains_process(instance, helper));
        assert_eq!(table.process_count(instance), Ok(2));
        assert!(!table.contains_process(
            instance,
            ProcessRef {
                instance,
                slot: helper.slot,
                generation: helper.generation.wrapping_add(1),
            }
        ));

        let mut windows = WindowSet::new();
        let first = windows.create(&table, instance, primary, 640, 480).unwrap();
        let second = windows.create(&table, instance, primary, 320, 200).unwrap();
        assert_ne!(first, second);
        assert_eq!(windows.count_for_instance(instance), 2);
        assert_eq!(windows.owned_by(primary), 2);
        assert_eq!(
            windows.close_for_process(&table, first, helper),
            Err(Error::NotOwner)
        );
        assert_eq!(windows.close_for_process(&table, first, primary), Ok(()));
        assert_eq!(windows.get(first), Err(Error::NotFound));
        let replacement = windows.create(&table, instance, primary, 800, 600).unwrap();
        assert_eq!(replacement.slot(), first.slot());
        assert_ne!(replacement.generation(), first.generation());
        assert_eq!(
            windows.close_for_process(&table, first, primary),
            Err(Error::StaleWindow)
        );
        assert_eq!(windows.get(second), Ok((instance, primary, 320, 200)));
        assert_eq!(windows.get(replacement), Ok((instance, primary, 800, 600)));
    }

    #[test]
    fn thirty_two_windows_and_thirty_two_instances_refuse_without_mutation() {
        let app = verified_definition(1);
        let mut instances = AppInstanceTable::new();
        let (instance, primary) = allocate_instance(&mut instances, &app, 0);
        let mut windows = WindowSet::new();
        let mut handles = [None; MAX_ORDINARY_WINDOWS];
        for item in &mut handles {
            *item = Some(
                windows
                    .create(&instances, instance, primary, 800, 600)
                    .unwrap(),
            );
        }
        assert_eq!(windows.len(), MAX_ORDINARY_WINDOWS);
        let sample = windows.get(handles[0].unwrap()).unwrap();
        assert_eq!(
            windows.create(&instances, instance, primary, 800, 600),
            Err(Error::TooManyWindows)
        );
        assert_eq!(windows.len(), MAX_ORDINARY_WINDOWS);
        assert_eq!(windows.get(handles[0].unwrap()), Ok(sample));
        assert_eq!(
            windows.create(&instances, instance, primary, 79, 480),
            Err(Error::InvalidDimensions)
        );
        assert_eq!(windows.len(), MAX_ORDINARY_WINDOWS);
        assert_eq!(
            windows.create(&instances, instance, helper_ref(instance), 800, 600),
            Err(Error::NotOwner)
        );
        assert_eq!(windows.len(), MAX_ORDINARY_WINDOWS);

        let mut full = AppInstanceTable::new();
        let mut ids = [None; MAX_APP_INSTANCES];
        for n in 0..MAX_APP_INSTANCES {
            let mut synthetic = app;
            let mut bytes = synthetic.manifest.encode();
            let id = serial_app_id(n);
            bytes[8..40].copy_from_slice(&id);
            synthetic.manifest = Manifest::parse(&bytes).unwrap();
            ids[n] = Some(full.create(&synthetic).unwrap());
        }
        assert_eq!(full.len(), MAX_APP_INSTANCES);
        let mut extra = app;
        let mut bytes = extra.manifest.encode();
        bytes[8..40].copy_from_slice(&serial_app_id(MAX_APP_INSTANCES));
        extra.manifest = Manifest::parse(&bytes).unwrap();
        assert_eq!(full.create(&extra), Err(Error::Full));
        assert_eq!(full.len(), MAX_APP_INSTANCES);
        for id in ids.into_iter().flatten() {
            assert_eq!(full.state(id), Ok(InstanceState::Starting));
        }
    }

    #[test]
    fn integrated_target_models_thirty_two_apps_windows_and_a_headless_helper() {
        let base = verified_definition(1);
        let mut instances = AppInstanceTable::new();
        let mut windows = WindowSet::new();
        let mut total_processes = 0;
        for app_index in 0..MAX_APP_INSTANCES {
            let mut app = base;
            let mut bytes = app.manifest.encode();
            bytes[8..40].copy_from_slice(&serial_app_id(app_index));
            let flags = if app_index + 1 == MAX_APP_INSTANCES {
                crate::manifest::FLAG_MULTI_INSTANCE
                    | crate::manifest::FLAG_BACKGROUND
                    | crate::manifest::FLAG_HEADLESS
            } else {
                crate::manifest::FLAG_MULTI_INSTANCE | crate::manifest::FLAG_BACKGROUND
            };
            bytes[112..116].copy_from_slice(&flags.to_le_bytes());
            app.manifest = Manifest::parse(&bytes).unwrap();
            let instance = instances.create(&app).unwrap();
            let primary = instances.bind_primary(instance, app_index as u8).unwrap();
            total_processes += 1;
            if app_index + 1 == MAX_APP_INSTANCES {
                assert!(instances.allows_headless(instance).unwrap());
                assert!(instances.allows_background(instance).unwrap());
                instances
                    .bind_helper(instance, MAX_APP_INSTANCES as u8)
                    .unwrap();
                total_processes += 1;
                assert_eq!(windows.count_for_instance(instance), 0);
            } else {
                assert!(instances.allows_background(instance).unwrap());
                // One instance owns two independent windows; the remaining
                // visible instances own one each, for exactly 32 windows.
                let window_count = if app_index == 0 { 2 } else { 1 };
                for n in 0..window_count {
                    let side = if n == 0 { 640 } else { 320 };
                    windows
                        .create(&instances, instance, primary, side, 240)
                        .unwrap();
                }
            }
        }
        assert_eq!(instances.len(), 32);
        assert_eq!(windows.len(), 32);
        assert_eq!(total_processes, 33);
        let before = windows.len();
        let running = instances
            .get(AppInstanceId {
                slot: 0,
                generation: 1,
            })
            .unwrap();
        let owner = running.process_group().contains(ProcessRef {
            instance: running.id(),
            slot: 0,
            generation: 1,
        });
        assert!(owner);
        assert_eq!(before, 32);
        assert_eq!(
            instances.create(&base),
            Err(Error::Full),
            "capacity refusal must not mutate a full 32-app catalog"
        );
        assert_eq!(instances.len(), 32);
        assert_eq!(windows.len(), 32);
    }

    #[test]
    fn instance_retirement_waits_for_exact_process_and_window_cleanup() {
        let app = verified_definition(1);
        let mut instances = AppInstanceTable::new();
        let (instance, primary) = allocate_instance(&mut instances, &app, 12);
        let mut windows = WindowSet::new();
        let window = windows
            .create(&instances, instance, primary, 500, 350)
            .unwrap();
        assert_eq!(instances.retire(instance, &windows), Err(Error::BadState));
        instances.request_stop(instance).unwrap();
        assert_eq!(instances.retire(instance, &windows), Err(Error::BadState));
        instances.finish_process(primary).unwrap();
        assert_eq!(
            windows.close_for_process(&instances, window, primary),
            Err(Error::StaleProcess)
        );
        assert_eq!(
            instances.retire(instance, &windows),
            Err(Error::LiveWindows)
        );
        assert_eq!(windows.close_all_for_instance(instance), 1);
        assert_eq!(windows.get(window), Err(Error::NotFound));
        instances.retire(instance, &windows).unwrap();
        assert_eq!(instances.len(), 0);
    }

    fn helper_ref(instance: AppInstanceId) -> ProcessRef {
        ProcessRef {
            instance,
            slot: 3,
            generation: 1,
        }
    }
    fn serial_app_id(n: usize) -> [u8; ID_BYTES] {
        let mut out = [0; ID_BYTES];
        let prefix = b"test.app.";
        let number = n.to_string();
        out[..prefix.len()].copy_from_slice(prefix);
        out[prefix.len()..prefix.len() + number.len()].copy_from_slice(number.as_bytes());
        out
    }
}
