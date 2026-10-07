//! Native startup ABI v2 (ADR-0083): bounded descriptive data in a
//! caller-owned one-page SharedRegion, never implicit process authority.

use crate::manifest::{
    FLAG_BACKGROUND, FLAG_HEADLESS, FLAG_MULTI_INSTANCE, FLAG_NATIVE_SYNC, FLAG_STANDARD_STREAMS,
    ID_BYTES,
};

pub const BLOCK_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 128;
/// Bounded userspace app-instance slots. This matches the 32-session
/// Phase-12 manager envelope; session/window records remain separate.
pub const INSTANCE_SLOTS: usize = 32;
pub const ARGUMENT_MAX: usize = 32;
pub const ENVIRONMENT_MAX: usize = 32;
pub const CAPABILITY_MAX: usize = 7;
pub const STRING_BYTES_MAX: usize = 3072;
pub const CAPABILITY_DESCRIPTOR_BYTES: usize = 16;
pub const STRING_DESCRIPTOR_BYTES: usize = 8;
pub const VERSION: u16 = 2;
pub const NONE: u16 = u16::MAX;

pub const CAP_KIND_IMAGE: u8 = 1;
pub const CAP_KIND_ENDPOINT: u8 = 2;
pub const CAP_KIND_NOTIFICATION: u8 = 3;
pub const CAP_KIND_PROCESS: u8 = 4;
pub const CAP_KIND_IMAGE_REGISTRAR: u8 = 5;
pub const CAP_KIND_BOOT_IMAGE: u8 = 6;
pub const CAP_KIND_SHARED_REGION: u8 = 7;
pub const CAP_KIND_MEMORY_POOL: u8 = 8;
/// ADR-0095: process-owned native VM reservation capability.
pub const CAP_KIND_VM_REGION: u8 = 14;
pub const CAP_KIND_SHARED_DMA: u8 = 9;
pub const CAP_KIND_PROOF_TOKEN: u8 = 10;
pub const CAP_KIND_UNTYPED: u8 = 11;
pub const CAP_KIND_BADGED_ENDPOINT: u8 = 12;
pub const CAP_KIND_RTC: u8 = 13;
/// ADR-0107: explicit native synchronization wait authority.
pub const CAP_KIND_SYNC_DOMAIN: u8 = 16;

pub const RIGHT_READ: u32 = 1 << 0;
pub const RIGHT_WRITE: u32 = 1 << 1;
pub const RIGHT_COPY: u32 = 1 << 2;
pub const RIGHT_DESTROY: u32 = 1 << 3;
pub const RIGHTS_MASK: u32 = RIGHT_READ | RIGHT_WRITE | RIGHT_COPY | RIGHT_DESTROY;
const KNOWN_FLAGS: u32 = FLAG_MULTI_INSTANCE
    | FLAG_BACKGROUND
    | FLAG_HEADLESS
    | FLAG_STANDARD_STREAMS
    | FLAG_NATIVE_SYNC;
const USER_ADDRESS_END: u64 = 0x0000_8000_0000_0000;
const MAGIC: &[u8; 4] = b"ARST";

const OFF_TOTAL: usize = 8;
const OFF_FLAGS: usize = 12;
const OFF_APP_ID: usize = 16;
const OFF_INSTANCE_SLOT: usize = 48;
const OFF_GENERATION: usize = 52;
const OFF_ARGC: usize = 60;
const OFF_ENVC: usize = 62;
const OFF_CAPC: usize = 64;
const OFF_ARGS: usize = 68;
const OFF_ENV: usize = 72;
const OFF_CAPS: usize = 76;
const OFF_STRINGS: usize = 80;
const OFF_STRINGS_LEN: usize = 84;
const OFF_CWD: usize = 88;
const OFF_STDIN: usize = 90;
const OFF_STDOUT: usize = 92;
const OFF_STDERR: usize = 94;
const OFF_PAGE_SIZE: usize = 96;
const OFF_ENTRY: usize = 104;
const OFF_LOAD_BASE: usize = 112;
const OFF_CLOCK: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BadMagic,
    UnsupportedVersion,
    BadHeader,
    NonCanonical,
    Bounds,
    BadIdentity,
    BadFlags,
    BadEntry,
    InvalidString,
    InvalidCapability,
    InvalidRoleReference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CapabilityRole {
    CurrentDirectory = 1,
    StandardInput = 2,
    StandardOutput = 3,
    StandardError = 4,
    Other = 5,
    StandardStreamSet = 6,
    StreamWake = 7,
    SyncDomain = 8,
}
impl CapabilityRole {
    fn from_byte(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::CurrentDirectory),
            2 => Some(Self::StandardInput),
            3 => Some(Self::StandardOutput),
            4 => Some(Self::StandardError),
            5 => Some(Self::Other),
            6 => Some(Self::StandardStreamSet),
            7 => Some(Self::StreamWake),
            8 => Some(Self::SyncDomain),
            _ => None,
        }
    }
}

/// A descriptive reference to one cap that the trusted launcher explicitly
/// inherited. The cap in `slot` is the authority; this record is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub slot: u16,
    pub role: CapabilityRole,
    /// Stable `SYS_CAP_DESCRIBE` kind tag, not an object ID.
    pub kind: u8,
    /// Exact current native kernel rights mask to compare with the held cap.
    pub rights: u32,
}

pub struct StartupSpec<'a> {
    pub application_id: &'a [u8; ID_BYTES],
    pub instance_slot: u16,
    pub instance_generation: u64,
    pub flags: u32,
    pub arguments: &'a [&'a [u8]],
    pub environment: &'a [&'a [u8]],
    pub capabilities: &'a [CapabilityDescriptor],
    /// Zero-based descriptor indices, not cap slots. `None` means absent.
    pub cwd: Option<u16>,
    pub stdin: Option<u16>,
    pub stdout: Option<u16>,
    pub stderr: Option<u16>,
    pub entry: u64,
    pub load_base: u64,
    pub clock_us: u64,
}

/// Borrowed, fully bounds-checked startup record. Argument and environment
/// slices point into the read-only startup SharedRegion page.
#[derive(Clone, Copy)]
struct StringDescriptor {
    offset: u32,
    length: u32,
}

struct StringTables {
    args_offset: usize,
    argc: usize,
    env_offset: usize,
    envc: usize,
    strings_offset: usize,
    strings_len: usize,
}

impl StringDescriptor {
    const EMPTY: Self = Self {
        offset: 0,
        length: 0,
    };
}

const EMPTY_CAPABILITY: CapabilityDescriptor = CapabilityDescriptor {
    slot: 0,
    role: CapabilityRole::Other,
    kind: 0,
    rights: 0,
};

pub struct StartupView<'a> {
    page: &'a [u8; BLOCK_BYTES],
    total_bytes: usize,
    flags: u32,
    application_id: [u8; ID_BYTES],
    instance_slot: u16,
    instance_generation: u64,
    argc: usize,
    envc: usize,
    cap_count: usize,
    arguments: [StringDescriptor; ARGUMENT_MAX],
    environment: [StringDescriptor; ENVIRONMENT_MAX],
    capabilities: [CapabilityDescriptor; CAPABILITY_MAX],
    strings_offset: usize,
    cwd: Option<u16>,
    stdin: Option<u16>,
    stdout: Option<u16>,
    stderr: Option<u16>,
    stream_set: Option<u16>,
    stream_wake: Option<u16>,
    entry: u64,
    load_base: u64,
    clock_us: u64,
}

impl StartupView<'_> {
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }
    pub fn flags(&self) -> u32 {
        self.flags
    }
    pub fn application_id(&self) -> &[u8; ID_BYTES] {
        &self.application_id
    }
    pub fn instance_slot(&self) -> u16 {
        self.instance_slot
    }
    pub fn instance_generation(&self) -> u64 {
        self.instance_generation
    }
    pub fn argument_count(&self) -> usize {
        self.argc
    }
    pub fn environment_count(&self) -> usize {
        self.envc
    }
    pub fn capability_count(&self) -> usize {
        self.cap_count
    }
    pub fn cwd_descriptor(&self) -> Option<u16> {
        self.cwd
    }
    pub fn stdin_descriptor(&self) -> Option<u16> {
        self.stdin
    }
    pub fn stdout_descriptor(&self) -> Option<u16> {
        self.stdout
    }
    pub fn stderr_descriptor(&self) -> Option<u16> {
        self.stderr
    }
    /// Descriptor index of the held shared ring set, if the signed
    /// application requested standard streams.
    pub fn stream_set_descriptor(&self) -> Option<u16> {
        self.stream_set
    }
    /// Descriptor index of the exact write-only wake capability, if present.
    pub fn stream_wake_descriptor(&self) -> Option<u16> {
        self.stream_wake
    }
    pub fn page_size(&self) -> u32 {
        BLOCK_BYTES as u32
    }
    pub fn entry(&self) -> u64 {
        self.entry
    }
    pub fn load_base(&self) -> u64 {
        self.load_base
    }
    pub fn clock_us(&self) -> u64 {
        self.clock_us
    }
    pub fn argument(&self, index: usize) -> Option<&[u8]> {
        (index < self.argc).then(|| self.string_at(self.arguments[index]))
    }
    pub fn environment(&self, index: usize) -> Option<&[u8]> {
        (index < self.envc).then(|| self.string_at(self.environment[index]))
    }
    pub fn capability(&self, index: usize) -> Option<CapabilityDescriptor> {
        (index < self.cap_count).then(|| self.capabilities[index])
    }
    pub fn capability_for_role(&self, role: CapabilityRole) -> Option<CapabilityDescriptor> {
        self.capabilities[..self.cap_count]
            .iter()
            .copied()
            .find(|descriptor| descriptor.role == role)
    }
    pub fn capability_matches(&self, index: usize, observed: [u64; 3]) -> bool {
        let Some(expected) = self.capability(index) else {
            return false;
        };
        observed[0] == u64::from(expected.kind)
            && observed[2] == u64::from(expected.rights)
            && observed[2] & !u64::from(RIGHTS_MASK) == 0
    }
    fn string_at(&self, descriptor: StringDescriptor) -> &'_ [u8] {
        let start = self.strings_offset + descriptor.offset as usize;
        let end = start + descriptor.length as usize;
        &self.page[start..end]
    }
}

/// Parse and validate an exact one-page ABI-v2 block. It validates only
/// descriptive bytes; the runtime must separately compare every descriptor
/// with the actual cap in the current process table.
pub fn parse(page: &[u8; BLOCK_BYTES]) -> Result<StartupView<'_>, Error> {
    if &page[..4] != MAGIC {
        return Err(Error::BadMagic);
    }
    if get_u16(page, 4) != VERSION || get_u16(page, 6) as usize != HEADER_BYTES {
        return Err(Error::UnsupportedVersion);
    }
    if page[50..52].iter().any(|byte| *byte != 0)
        || page[66..68].iter().any(|byte| *byte != 0)
        || page[100..104].iter().any(|byte| *byte != 0)
    {
        return Err(Error::NonCanonical);
    }

    let total_bytes = get_u32(page, OFF_TOTAL) as usize;
    let argc = get_u16(page, OFF_ARGC) as usize;
    let envc = get_u16(page, OFF_ENVC) as usize;
    let cap_count = get_u16(page, OFF_CAPC) as usize;
    let strings_len = get_u32(page, OFF_STRINGS_LEN) as usize;
    let instance_slot = get_u16(page, OFF_INSTANCE_SLOT);
    let instance_generation = get_u64(page, OFF_GENERATION);
    let flags = get_u32(page, OFF_FLAGS);
    if !(HEADER_BYTES..=BLOCK_BYTES).contains(&total_bytes)
        || argc == 0
        || argc > ARGUMENT_MAX
        || envc > ENVIRONMENT_MAX
        || cap_count > CAPABILITY_MAX
        || strings_len > STRING_BYTES_MAX
        || usize::from(instance_slot) >= INSTANCE_SLOTS
        || instance_generation == 0
    {
        return Err(Error::Bounds);
    }
    if flags & !KNOWN_FLAGS != 0 {
        return Err(Error::BadFlags);
    }
    let application_id: [u8; ID_BYTES] = page[OFF_APP_ID..OFF_APP_ID + ID_BYTES]
        .try_into()
        .map_err(|_| Error::BadIdentity)?;
    if !valid_application_id(&application_id) {
        return Err(Error::BadIdentity);
    }

    let caps_offset = get_u32(page, OFF_CAPS) as usize;
    let args_offset = get_u32(page, OFF_ARGS) as usize;
    let env_offset = get_u32(page, OFF_ENV) as usize;
    let strings_offset = get_u32(page, OFF_STRINGS) as usize;
    let expected_caps = HEADER_BYTES;
    let expected_args = expected_caps
        .checked_add(
            cap_count
                .checked_mul(CAPABILITY_DESCRIPTOR_BYTES)
                .ok_or(Error::Bounds)?,
        )
        .ok_or(Error::Bounds)?;
    let expected_env = expected_args
        .checked_add(
            argc.checked_mul(STRING_DESCRIPTOR_BYTES)
                .ok_or(Error::Bounds)?,
        )
        .ok_or(Error::Bounds)?;
    let expected_strings = expected_env
        .checked_add(
            envc.checked_mul(STRING_DESCRIPTOR_BYTES)
                .ok_or(Error::Bounds)?,
        )
        .ok_or(Error::Bounds)?;
    if caps_offset != expected_caps
        || args_offset != expected_args
        || env_offset != expected_env
        || strings_offset != expected_strings
        || strings_offset.checked_add(strings_len) != Some(total_bytes)
    {
        return Err(Error::NonCanonical);
    }
    if total_bytes > BLOCK_BYTES || page[total_bytes..].iter().any(|byte| *byte != 0) {
        return Err(Error::NonCanonical);
    }

    let entry = get_u64(page, OFF_ENTRY);
    let load_base = get_u64(page, OFF_LOAD_BASE);
    if get_u32(page, OFF_PAGE_SIZE) as usize != BLOCK_BYTES
        || entry == 0
        || load_base == 0
        || !load_base.is_multiple_of(BLOCK_BYTES as u64)
        || entry < load_base
        || entry >= USER_ADDRESS_END
        || load_base >= USER_ADDRESS_END
    {
        return Err(Error::BadEntry);
    }

    let cwd = optional_index(page, OFF_CWD, cap_count)?;
    let stdin = optional_index(page, OFF_STDIN, cap_count)?;
    let stdout = optional_index(page, OFF_STDOUT, cap_count)?;
    let stderr = optional_index(page, OFF_STDERR, cap_count)?;
    let capabilities =
        validate_capabilities(page, caps_offset, cap_count, [cwd, stdin, stdout, stderr])?;
    let stream_set = role_descriptor(&capabilities, cap_count, CapabilityRole::StandardStreamSet);
    let stream_wake = role_descriptor(&capabilities, cap_count, CapabilityRole::StreamWake);
    let sync_domain = role_descriptor(&capabilities, cap_count, CapabilityRole::SyncDomain);
    let streams_requested = flags & FLAG_STANDARD_STREAMS != 0;
    if streams_requested != stream_set.is_some() || streams_requested != stream_wake.is_some() {
        return Err(Error::InvalidRoleReference);
    }
    if (flags & FLAG_NATIVE_SYNC != 0) != sync_domain.is_some() {
        return Err(Error::InvalidRoleReference);
    }
    if sync_domain.is_some_and(|index| {
        let descriptor = capabilities[usize::from(index)];
        descriptor.kind != CAP_KIND_SYNC_DOMAIN || descriptor.rights != (RIGHT_READ | RIGHT_WRITE)
    }) {
        return Err(Error::InvalidCapability);
    }
    let mut arguments = [StringDescriptor::EMPTY; ARGUMENT_MAX];
    let mut environment = [StringDescriptor::EMPTY; ENVIRONMENT_MAX];
    validate_strings(
        page,
        StringTables {
            args_offset,
            argc,
            env_offset,
            envc,
            strings_offset,
            strings_len,
        },
        &mut arguments,
        &mut environment,
    )?;
    Ok(StartupView {
        page,
        total_bytes,
        flags,
        application_id,
        instance_slot,
        instance_generation,
        argc,
        envc,
        cap_count,
        arguments,
        environment,
        capabilities,
        strings_offset,
        cwd,
        stdin,
        stdout,
        stderr,
        stream_set,
        stream_wake,
        entry,
        load_base,
        clock_us: get_u64(page, OFF_CLOCK),
    })
}

/// Validate the special child slot-0 cap and SharedRegion geometry before
/// treating this page as startup input. The object ID is not authority.
pub fn validate_startup_cap(observed: [u64; 3], pages: u64) -> Result<(), Error> {
    if observed[0] != u64::from(CAP_KIND_SHARED_REGION)
        || observed[2] != u64::from(RIGHT_READ | RIGHT_DESTROY)
        || pages != 1
    {
        return Err(Error::InvalidCapability);
    }
    Ok(())
}

/// Encode into a zeroed one-page destination without allocation. On success,
/// the returned byte count is exactly the canonical `total_bytes` field.
pub fn encode(spec: &StartupSpec<'_>, page: &mut [u8; BLOCK_BYTES]) -> Result<usize, Error> {
    if spec.arguments.is_empty()
        || spec.arguments.len() > ARGUMENT_MAX
        || spec.environment.len() > ENVIRONMENT_MAX
        || spec.capabilities.len() > CAPABILITY_MAX
        || usize::from(spec.instance_slot) >= INSTANCE_SLOTS
        || spec.instance_generation == 0
    {
        return Err(Error::Bounds);
    }
    let args_offset = HEADER_BYTES
        .checked_add(
            spec.capabilities
                .len()
                .checked_mul(CAPABILITY_DESCRIPTOR_BYTES)
                .ok_or(Error::Bounds)?,
        )
        .ok_or(Error::Bounds)?;
    let env_offset = args_offset
        .checked_add(
            spec.arguments
                .len()
                .checked_mul(STRING_DESCRIPTOR_BYTES)
                .ok_or(Error::Bounds)?,
        )
        .ok_or(Error::Bounds)?;
    let strings_offset = env_offset
        .checked_add(
            spec.environment
                .len()
                .checked_mul(STRING_DESCRIPTOR_BYTES)
                .ok_or(Error::Bounds)?,
        )
        .ok_or(Error::Bounds)?;
    let strings_len = spec
        .arguments
        .iter()
        .chain(spec.environment.iter())
        .try_fold(0usize, |sum, string| sum.checked_add(string.len()))
        .ok_or(Error::Bounds)?;
    let total_bytes = strings_offset
        .checked_add(strings_len)
        .ok_or(Error::Bounds)?;
    if strings_len > STRING_BYTES_MAX || total_bytes > BLOCK_BYTES {
        return Err(Error::Bounds);
    }
    page.fill(0);
    page[..4].copy_from_slice(MAGIC);
    put_u16(page, 4, VERSION);
    put_u16(page, 6, HEADER_BYTES as u16);
    put_u32(page, OFF_TOTAL, total_bytes as u32);
    put_u32(page, OFF_FLAGS, spec.flags);
    page[OFF_APP_ID..OFF_APP_ID + ID_BYTES].copy_from_slice(spec.application_id);
    put_u16(page, OFF_INSTANCE_SLOT, spec.instance_slot);
    put_u64(page, OFF_GENERATION, spec.instance_generation);
    put_u16(page, OFF_ARGC, spec.arguments.len() as u16);
    put_u16(page, OFF_ENVC, spec.environment.len() as u16);
    put_u16(page, OFF_CAPC, spec.capabilities.len() as u16);
    put_u32(page, OFF_ARGS, args_offset as u32);
    put_u32(page, OFF_ENV, env_offset as u32);
    put_u32(page, OFF_CAPS, HEADER_BYTES as u32);
    put_u32(page, OFF_STRINGS, strings_offset as u32);
    put_u32(page, OFF_STRINGS_LEN, strings_len as u32);
    put_u16(page, OFF_CWD, spec.cwd.unwrap_or(NONE));
    put_u16(page, OFF_STDIN, spec.stdin.unwrap_or(NONE));
    put_u16(page, OFF_STDOUT, spec.stdout.unwrap_or(NONE));
    put_u16(page, OFF_STDERR, spec.stderr.unwrap_or(NONE));
    put_u32(page, OFF_PAGE_SIZE, BLOCK_BYTES as u32);
    put_u64(page, OFF_ENTRY, spec.entry);
    put_u64(page, OFF_LOAD_BASE, spec.load_base);
    put_u64(page, OFF_CLOCK, spec.clock_us);

    for (index, capability) in spec.capabilities.iter().enumerate() {
        let at = HEADER_BYTES + index * CAPABILITY_DESCRIPTOR_BYTES;
        put_u16(page, at, capability.slot);
        page[at + 2] = capability.role as u8;
        page[at + 3] = capability.kind;
        put_u32(page, at + 4, capability.rights);
    }

    let mut string_offset = 0usize;
    for (index, string) in spec.arguments.iter().enumerate() {
        if string.contains(&0) || (index == 0 && string.is_empty()) {
            return Err(Error::InvalidString);
        }
        let at = args_offset + index * STRING_DESCRIPTOR_BYTES;
        write_string(page, at, strings_offset, &mut string_offset, string)?;
    }
    for (index, string) in spec.environment.iter().enumerate() {
        if string.is_empty() || string.contains(&0) {
            return Err(Error::InvalidString);
        }
        let at = env_offset + index * STRING_DESCRIPTOR_BYTES;
        write_string(page, at, strings_offset, &mut string_offset, string)?;
    }
    if string_offset != strings_len {
        return Err(Error::NonCanonical);
    }
    parse(page).map(|_| total_bytes)
}

fn write_string(
    page: &mut [u8; BLOCK_BYTES],
    descriptor: usize,
    strings_offset: usize,
    string_offset: &mut usize,
    string: &[u8],
) -> Result<(), Error> {
    let end = string_offset
        .checked_add(string.len())
        .ok_or(Error::Bounds)?;
    if end > STRING_BYTES_MAX || strings_offset + end > BLOCK_BYTES {
        return Err(Error::Bounds);
    }
    put_u32(page, descriptor, *string_offset as u32);
    put_u32(page, descriptor + 4, string.len() as u32);
    page[strings_offset + *string_offset..strings_offset + end].copy_from_slice(string);
    *string_offset = end;
    Ok(())
}

fn validate_strings(
    page: &[u8; BLOCK_BYTES],
    tables: StringTables,
    arguments: &mut [StringDescriptor; ARGUMENT_MAX],
    environment: &mut [StringDescriptor; ENVIRONMENT_MAX],
) -> Result<(), Error> {
    let mut expected_offset = 0usize;
    for (table, count, output, is_environment) in [
        (tables.args_offset, tables.argc, arguments, false),
        (tables.env_offset, tables.envc, environment, true),
    ] {
        for (index, output_item) in output.iter_mut().enumerate().take(count) {
            let at = table + index * STRING_DESCRIPTOR_BYTES;
            let offset = get_u32(page, at) as usize;
            let length = get_u32(page, at + 4) as usize;
            if offset != expected_offset {
                return Err(Error::NonCanonical);
            }
            let end = offset.checked_add(length).ok_or(Error::Bounds)?;
            if end > tables.strings_len {
                return Err(Error::Bounds);
            }
            let bytes = &page[tables.strings_offset + offset..tables.strings_offset + end];
            if bytes.contains(&0)
                || (index == 0 && !is_environment && bytes.is_empty())
                || (is_environment && bytes.is_empty())
            {
                return Err(Error::InvalidString);
            }
            *output_item = StringDescriptor {
                offset: offset as u32,
                length: length as u32,
            };
            expected_offset = end;
        }
    }
    if expected_offset != tables.strings_len {
        return Err(Error::NonCanonical);
    }
    Ok(())
}

fn validate_capabilities(
    page: &[u8; BLOCK_BYTES],
    caps_offset: usize,
    cap_count: usize,
    references: [Option<u16>; 4],
) -> Result<[CapabilityDescriptor; CAPABILITY_MAX], Error> {
    let mut capabilities = [EMPTY_CAPABILITY; CAPABILITY_MAX];
    let mut found = [None; 7];
    for (index, output) in capabilities.iter_mut().enumerate().take(cap_count) {
        let at = caps_offset + index * CAPABILITY_DESCRIPTOR_BYTES;
        let descriptor = read_capability(page, caps_offset, index)?;
        if descriptor.slot != (index + 1) as u16
            || !valid_cap_kind(descriptor.kind)
            || descriptor.rights == 0
            || descriptor.rights & !RIGHTS_MASK != 0
        {
            return Err(Error::InvalidCapability);
        }
        if descriptor.role == CapabilityRole::CurrentDirectory
            && (descriptor.kind != CAP_KIND_BADGED_ENDPOINT || descriptor.rights & RIGHT_WRITE == 0)
        {
            return Err(Error::InvalidCapability);
        }
        if descriptor.role == CapabilityRole::StandardStreamSet
            && (descriptor.kind != CAP_KIND_SHARED_REGION
                || descriptor.rights != (RIGHT_READ | RIGHT_WRITE))
        {
            return Err(Error::InvalidCapability);
        }
        if descriptor.role == CapabilityRole::StreamWake
            && (descriptor.kind != CAP_KIND_NOTIFICATION || descriptor.rights != RIGHT_WRITE)
        {
            return Err(Error::InvalidCapability);
        }
        if descriptor.role == CapabilityRole::SyncDomain
            && (descriptor.kind != CAP_KIND_SYNC_DOMAIN
                || descriptor.rights != (RIGHT_READ | RIGHT_WRITE))
        {
            return Err(Error::InvalidCapability);
        }
        if page[at + 8..at + CAPABILITY_DESCRIPTOR_BYTES]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(Error::NonCanonical);
        }
        let role_index = match descriptor.role {
            CapabilityRole::CurrentDirectory => Some(0),
            CapabilityRole::StandardInput => Some(1),
            CapabilityRole::StandardOutput => Some(2),
            CapabilityRole::StandardError => Some(3),
            CapabilityRole::Other => None,
            CapabilityRole::StandardStreamSet => Some(4),
            CapabilityRole::StreamWake => Some(5),
            CapabilityRole::SyncDomain => Some(6),
        };
        if let Some(role_index) = role_index
            && found[role_index].replace(index as u16).is_some()
        {
            return Err(Error::InvalidCapability);
        }
        *output = descriptor;
    }
    if found[0] != references[0] {
        return Err(Error::InvalidRoleReference);
    }
    if let Some(stream_set) = found[4] {
        if references[1..] != [Some(stream_set); 3]
            || found[1..4].iter().any(Option::is_some)
            || found[5].is_none()
        {
            return Err(Error::InvalidRoleReference);
        }
    } else if found[1..4] != references[1..] || found[5].is_some() {
        return Err(Error::InvalidRoleReference);
    }
    Ok(capabilities)
}

fn role_descriptor(
    capabilities: &[CapabilityDescriptor; CAPABILITY_MAX],
    cap_count: usize,
    role: CapabilityRole,
) -> Option<u16> {
    capabilities
        .iter()
        .take(cap_count)
        .position(|descriptor| descriptor.role == role)
        .map(|index| index as u16)
}

fn optional_index(
    page: &[u8; BLOCK_BYTES],
    offset: usize,
    cap_count: usize,
) -> Result<Option<u16>, Error> {
    let value = get_u16(page, offset);
    if value == NONE {
        return Ok(None);
    }
    (usize::from(value) < cap_count)
        .then_some(Some(value))
        .ok_or(Error::InvalidRoleReference)
}

fn read_capability(
    page: &[u8; BLOCK_BYTES],
    caps_offset: usize,
    index: usize,
) -> Result<CapabilityDescriptor, Error> {
    let at = caps_offset + index * CAPABILITY_DESCRIPTOR_BYTES;
    Ok(CapabilityDescriptor {
        slot: get_u16(page, at),
        role: CapabilityRole::from_byte(page[at + 2]).ok_or(Error::InvalidCapability)?,
        kind: page[at + 3],
        rights: get_u32(page, at + 4),
    })
}

fn valid_cap_kind(kind: u8) -> bool {
    (CAP_KIND_IMAGE..=CAP_KIND_RTC).contains(&kind) || kind == CAP_KIND_SYNC_DOMAIN
}

fn valid_application_id(id: &[u8; ID_BYTES]) -> bool {
    let Some(end) = id.iter().position(|byte| *byte == 0) else {
        return false;
    };
    (1..=31).contains(&end)
        && id[end..].iter().all(|byte| *byte == 0)
        && (id[0].is_ascii_lowercase() || id[0].is_ascii_digit())
        && id[..end].iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'.' || *byte == b'-'
        })
}

fn get_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn get_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4]))
}
fn get_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8]))
}
fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn test_application_id() -> [u8; 32] {
        let mut id = [0; 32];
        let bytes = b"com.arena.editor";
        let mut index = 0;
        while index < bytes.len() {
            id[index] = bytes[index];
            index += 1;
        }
        id
    }
    const TEST_APPLICATION_ID: [u8; 32] = test_application_id();

    fn sample_spec<'a>(
        args: &'a [&'a [u8]],
        env: &'a [&'a [u8]],
        caps: &'a [CapabilityDescriptor],
    ) -> StartupSpec<'a> {
        StartupSpec {
            application_id: &TEST_APPLICATION_ID,
            instance_slot: 3,
            instance_generation: 77,
            flags: FLAG_MULTI_INSTANCE,
            arguments: args,
            environment: env,
            capabilities: caps,
            cwd: Some(0),
            stdin: None,
            stdout: None,
            stderr: None,
            entry: 0x0040_0120,
            load_base: 0x0040_0000,
            clock_us: 1234,
        }
    }

    fn sample_caps() -> [CapabilityDescriptor; 2] {
        [
            CapabilityDescriptor {
                slot: 1,
                role: CapabilityRole::CurrentDirectory,
                kind: CAP_KIND_BADGED_ENDPOINT,
                rights: RIGHT_WRITE,
            },
            CapabilityDescriptor {
                slot: 2,
                role: CapabilityRole::Other,
                kind: CAP_KIND_IMAGE,
                rights: RIGHT_READ,
            },
        ]
    }

    fn encoded() -> [u8; BLOCK_BYTES] {
        let args: [&[u8]; 2] = [b"com.arena.editor", b"notes.txt"];
        let env: [&[u8]; 1] = [b"LANG=en"];
        let caps = sample_caps();
        let spec = sample_spec(&args, &env, &caps);
        let mut page = [0; BLOCK_BYTES];
        encode(&spec, &mut page).unwrap();
        page
    }

    #[test]
    fn canonical_startup_roundtrip_contains_data_not_authority() {
        let page = encoded();
        assert_eq!(&page[..], include_bytes!("../tests/data/startup-v2.bin"));
        let view = parse(&page).unwrap();
        assert_eq!(view.total_bytes(), 128 + 2 * 16 + 3 * 8 + 32);
        assert_eq!(view.application_id(), &TEST_APPLICATION_ID);
        assert_eq!(view.instance_slot(), 3);
        assert_eq!(view.instance_generation(), 77);
        assert_eq!(view.argument_count(), 2);
        assert_eq!(view.argument(0), Some(&b"com.arena.editor"[..]));
        assert_eq!(view.argument(1), Some(&b"notes.txt"[..]));
        assert_eq!(view.environment_count(), 1);
        assert_eq!(view.environment(0), Some(&b"LANG=en"[..]));
        assert_eq!(view.cwd_descriptor(), Some(0));
        assert_eq!(view.stdin_descriptor(), None);
        assert_eq!(view.capability(0), Some(sample_caps()[0]));
        assert_eq!(view.capability(1).unwrap().slot, 2);
        assert!(view.capability_matches(0, [12, 999, RIGHT_WRITE as u64]));
        assert!(!view.capability_matches(0, [12, 999, RIGHT_READ as u64]));
        assert_eq!(view.page_size(), 4096);
        assert_eq!(view.entry(), 0x0040_0120);
        assert_eq!(view.load_base(), 0x0040_0000);
        assert_eq!(view.clock_us(), 1234);
        // App IDs and cap descriptor slots carry no process/file authority.
    }

    #[test]
    fn standard_stream_roles_share_one_exact_region_and_writable_wake_cap() {
        let args: [&[u8]; 1] = [b"com.arena.editor"];
        let env: [&[u8]; 0] = [];
        let caps = [
            CapabilityDescriptor {
                slot: 1,
                role: CapabilityRole::StandardStreamSet,
                kind: CAP_KIND_SHARED_REGION,
                rights: RIGHT_READ | RIGHT_WRITE,
            },
            CapabilityDescriptor {
                slot: 2,
                role: CapabilityRole::StreamWake,
                kind: CAP_KIND_NOTIFICATION,
                rights: RIGHT_WRITE,
            },
        ];
        let mut spec = sample_spec(&args, &env, &caps);
        spec.flags |= FLAG_STANDARD_STREAMS;
        spec.cwd = None;
        spec.stdin = Some(0);
        spec.stdout = Some(0);
        spec.stderr = Some(0);
        let mut page = [0; BLOCK_BYTES];
        encode(&spec, &mut page).unwrap();
        let view = parse(&page).unwrap();
        assert_eq!(view.stdin_descriptor(), Some(0));
        assert_eq!(view.stdout_descriptor(), Some(0));
        assert_eq!(view.stderr_descriptor(), Some(0));
        assert_eq!(view.stream_set_descriptor(), Some(0));
        assert_eq!(view.stream_wake_descriptor(), Some(1));

        let mut roles_without_opt_in = page;
        put_u32(&mut roles_without_opt_in, OFF_FLAGS, 0);
        assert!(matches!(
            parse(&roles_without_opt_in),
            Err(Error::InvalidRoleReference)
        ));

        let no_stream_caps = sample_caps();
        let mut flag_without_roles = sample_spec(&args, &env, &no_stream_caps);
        flag_without_roles.flags |= FLAG_STANDARD_STREAMS;
        let mut missing_roles = [0; BLOCK_BYTES];
        assert_eq!(
            encode(&flag_without_roles, &mut missing_roles),
            Err(Error::InvalidRoleReference)
        );

        let mut wrong_wake = page;
        put_u32(
            &mut wrong_wake,
            HEADER_BYTES + CAPABILITY_DESCRIPTOR_BYTES + 4,
            RIGHT_READ,
        );
        assert!(matches!(parse(&wrong_wake), Err(Error::InvalidCapability)));

        let mut split_reference = page;
        put_u16(&mut split_reference, OFF_STDERR, 1);
        assert!(matches!(
            parse(&split_reference),
            Err(Error::InvalidRoleReference)
        ));
    }

    #[test]
    fn sync_domain_role_requires_exact_read_write_authority_and_is_unique() {
        let args: [&[u8]; 1] = [b"com.arena.editor"];
        let env: [&[u8]; 0] = [];
        let domain = CapabilityDescriptor {
            slot: 1,
            role: CapabilityRole::SyncDomain,
            kind: CAP_KIND_SYNC_DOMAIN,
            rights: RIGHT_READ | RIGHT_WRITE,
        };
        let caps = [domain];
        let mut spec = sample_spec(&args, &env, &caps);
        spec.flags |= FLAG_NATIVE_SYNC;
        spec.cwd = None;
        let mut page = [0; BLOCK_BYTES];
        encode(&spec, &mut page).unwrap();
        let view = parse(&page).unwrap();
        assert_eq!(
            view.capability_for_role(CapabilityRole::SyncDomain),
            Some(domain)
        );

        let mut missing_opt_in = page;
        put_u32(&mut missing_opt_in, OFF_FLAGS, 0);
        assert!(matches!(
            parse(&missing_opt_in),
            Err(Error::InvalidRoleReference)
        ));

        let mut missing_domain = sample_spec(&args, &env, &[]);
        missing_domain.flags |= FLAG_NATIVE_SYNC;
        missing_domain.cwd = None;
        let mut missing_domain_page = [0; BLOCK_BYTES];
        assert_eq!(
            encode(&missing_domain, &mut missing_domain_page),
            Err(Error::InvalidRoleReference)
        );

        let mut wrong_rights = page;
        put_u32(&mut wrong_rights, HEADER_BYTES + 4, RIGHT_READ);
        assert!(matches!(
            parse(&wrong_rights),
            Err(Error::InvalidCapability)
        ));

        let mut wrong_kind = page;
        wrong_kind[HEADER_BYTES + 3] = CAP_KIND_NOTIFICATION;
        assert!(matches!(parse(&wrong_kind), Err(Error::InvalidCapability)));

        let duplicate = [domain, CapabilityDescriptor { slot: 2, ..domain }];
        let mut duplicate_spec = sample_spec(&args, &env, &duplicate);
        duplicate_spec.cwd = None;
        let mut duplicate_page = [0; BLOCK_BYTES];
        assert_eq!(
            encode(&duplicate_spec, &mut duplicate_page),
            Err(Error::InvalidCapability)
        );
    }

    #[test]
    fn noncanonical_offsets_padding_and_tail_are_rejected() {
        let original = encoded();
        for (offset, value) in [(50, 1), (original.len() - 1, 1)] {
            let mut page = original;
            page[offset] = value;
            assert!(matches!(parse(&page), Err(Error::NonCanonical)));
        }
        let mut bad_argument_offset = original;
        let args_offset = get_u32(&bad_argument_offset, OFF_ARGS) as usize;
        put_u32(&mut bad_argument_offset, args_offset, 1);
        assert!(matches!(
            parse(&bad_argument_offset),
            Err(Error::NonCanonical)
        ));
        let mut bad_strings = original;
        let strings_offset = get_u32(&bad_strings, OFF_STRINGS) as usize;
        bad_strings[strings_offset] = 0;
        assert!(matches!(parse(&bad_strings), Err(Error::InvalidString)));
    }

    #[test]
    fn cap_slots_roles_rights_and_identity_are_fail_closed() {
        let original = encoded();
        let mut wrong_slot = original;
        put_u16(&mut wrong_slot, HEADER_BYTES, 2);
        assert!(matches!(parse(&wrong_slot), Err(Error::InvalidCapability)));
        let mut wrong_rights = original;
        put_u32(&mut wrong_rights, HEADER_BYTES + 4, 0);
        assert!(matches!(
            parse(&wrong_rights),
            Err(Error::InvalidCapability)
        ));
        let mut wrong_cwd = original;
        put_u16(&mut wrong_cwd, OFF_CWD, 1);
        assert!(matches!(
            parse(&wrong_cwd),
            Err(Error::InvalidRoleReference)
        ));
        let mut bad_id = original;
        bad_id[OFF_APP_ID] = b'!';
        assert!(matches!(parse(&bad_id), Err(Error::BadIdentity)));
    }

    #[test]
    fn startup_transport_requires_one_read_only_shared_page() {
        assert_eq!(
            validate_startup_cap([CAP_KIND_SHARED_REGION as u64, 1, 9], 1),
            Ok(())
        );
        assert_eq!(
            validate_startup_cap([CAP_KIND_SHARED_REGION as u64, 1, 11], 1),
            Err(Error::InvalidCapability)
        );
        assert_eq!(
            validate_startup_cap([CAP_KIND_SHARED_REGION as u64, 1, 9], 2),
            Err(Error::InvalidCapability)
        );
        assert_eq!(
            validate_startup_cap([CAP_KIND_ENDPOINT as u64, 1, 9], 1),
            Err(Error::InvalidCapability)
        );
    }

    #[test]
    fn bounds_refuse_before_encoding_or_application_entry() {
        let args: [&[u8]; 1] = [b"app"];
        let env: [&[u8]; 0] = [];
        let caps = sample_caps();
        let mut oversized_caps = [caps[0]; CAPABILITY_MAX + 1];
        oversized_caps[1..].copy_from_slice(&[caps[1]; CAPABILITY_MAX]);
        for index in 1..oversized_caps.len() {
            oversized_caps[index].slot = (index + 1) as u16;
            oversized_caps[index].role = CapabilityRole::Other;
        }
        let mut page = [0; BLOCK_BYTES];
        let spec = sample_spec(&args, &env, &oversized_caps);
        assert_eq!(encode(&spec, &mut page), Err(Error::Bounds));
        assert!(page.iter().all(|byte| *byte == 0));

        let caps = sample_caps();
        let mut bad_entry = sample_spec(&args, &env, &caps);
        bad_entry.entry = 0;
        assert_eq!(encode(&bad_entry, &mut page), Err(Error::BadEntry));
    }

    #[test]
    fn thirty_two_live_instance_slots_are_unique_and_the_next_refuses_mutation_free() {
        assert_eq!(INSTANCE_SLOTS, 32);
        let args: [&[u8]; 1] = [b"app"];
        let env: [&[u8]; 0] = [];
        let caps = sample_caps();
        let mut spec = sample_spec(&args, &env, &caps);
        let mut page = [0u8; BLOCK_BYTES];

        spec.instance_slot = (INSTANCE_SLOTS - 1) as u16;
        assert!(encode(&spec, &mut page).is_ok());
        assert_eq!(parse(&page).unwrap().instance_slot(), 31);

        spec.instance_slot = INSTANCE_SLOTS as u16;
        page.fill(0xa5);
        assert_eq!(encode(&spec, &mut page), Err(Error::Bounds));
        assert!(page.iter().all(|byte| *byte == 0xa5));
    }
}
