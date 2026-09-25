//! Hand-written UEFI ABI bindings — the only firmware interface in the tree.
//!
//! Policy (ADR-0004): zero third-party crates, so these definitions are ours.
//! Every `#[repr(C)]` struct mirrors the UEFI 2.10 specification exactly and
//! carries a static size assertion plus a spec citation. Only the table
//! entries the boot stage actually uses are typed; later entries are declared
//! as opaque `usize` placeholders so offsets stay correct, or omitted when
//! nothing beyond them is read (reading a prefix of a C struct is safe).
//!
//! Calling convention: all firmware functions are `extern "efiapi"`
//! (win64-style on x86_64). Status codes are `usize` per spec (UINTN).

use core::fmt;

/// `EFI_STATUS` (UEFI 2.10 §7.1). UINTN-sized; high bit set = error.
pub type Status = usize;

pub const EFI_SUCCESS: Status = 0;
/// (MAX_BIT | 5) — UEFI 2.10 Table 31 error encoding.
pub const EFI_BUFFER_TOO_SMALL: Status = 0x8000_0000_0000_0005;

/// EFI_TABLE_HEADER — UEFI 2.10 §11.2.1 (Table 15). 24 bytes.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}
const _: () = assert!(core::mem::size_of::<TableHeader>() == 24);

/// EFI_SYSTEM_TABLE — UEFI 2.10 §3.6.2 (Table 10).
/// We declare fields up to `ConfigurationTable`; nothing beyond is read.
#[repr(C)]
pub struct SystemTable {
    pub hdr: TableHeader,
    /// UCS-2 string, owned by firmware.
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: usize,
    pub con_in: usize,
    pub console_out_handle: usize,
    pub con_out: usize,
    pub standard_error_handle: usize,
    pub std_err: usize,
    pub runtime_services: *const RuntimeServices,
    pub boot_services: *const BootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: usize,
}
const _: () = assert!(core::mem::size_of::<SystemTable>() == 24 + 96);

/// EFI_RUNTIME_SERVICES — UEFI 2.10 §8.6 (Table 17).
/// Typed through `reset_system` (the 10th function); the M1 boot stage uses
/// nothing beyond it. UEFI 2.0+ adds more entries after it which we never
/// touch, so they are not declared.
#[repr(C)]
pub struct RuntimeServices {
    pub hdr: TableHeader,
    pub get_time: usize,
    pub set_time: usize,
    pub get_wakeup_time: usize,
    pub set_wakeup_time: usize,
    pub set_virtual_address_map: usize,
    pub convert_pointer: usize,
    pub get_variable: usize,
    pub get_next_variable_name: usize,
    pub set_variable: usize,
    pub get_next_high_mono_count: usize,
    /// EFI_RESET_SYSTEM — UEFI 2.10 §8.6.11.
    pub reset_system: unsafe extern "efiapi" fn(
        reset_type: u32,
        status: Status,
        data_size: usize,
        data: *const u8,
    ),
}
const _: () = assert!(core::mem::size_of::<RuntimeServices>() == 24 + 88);

/// EFI_RESET_TYPE (UEFI 2.10 §8.6.11): 0=Cold, 1=Warm, 2=Shutdown, ...
pub const EFI_RESET_SHUTDOWN: u32 = 2;

/// EFI_BOOT_SERVICES — UEFI 2.10 §7.2 (Table 14).
/// Typed through `get_memory_map` (the 5th function). Everything after it is
/// declared as opaque placeholders to keep the struct layout honest about
/// there being more table (not needed by M1, listed for documentation).
#[repr(C)]
pub struct BootServices {
    pub hdr: TableHeader,
    pub raise_tpl: usize,
    pub restore_tpl: usize,
    pub allocate_pages: usize,
    pub free_pages: usize,
    /// EFI_GET_MEMORY_MAP — UEFI 2.10 §7.2.10.
    pub get_memory_map: unsafe extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut u8,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> Status,
}
const _: () = assert!(core::mem::size_of::<BootServices>() == 24 + 40);

/// EFI_MEMORY_DESCRIPTOR — UEFI 2.10 §7.2.10 (Table 18). 40 bytes, v1.
/// Firmware may pad descriptors (GetMemoryMap reports the actual stride in
/// `descriptor_size`; EDK2 uses 48) — consumers must stride by that value.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryDescriptor {
    pub memory_type: u32,
    /// Spec-mandated pad (UEFI 2.10 Table 18).
    pub pad: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}
const _: () = assert!(core::mem::size_of::<MemoryDescriptor>() == 40);

/// EFI_MEMORY_TYPE — UEFI 2.10 §7.2.10 (Table 19).
pub mod memory_type {
    pub const RESERVED: u32 = 0;
    pub const LOADER_CODE: u32 = 1;
    pub const LOADER_DATA: u32 = 2;
    pub const BOOT_SERVICES_CODE: u32 = 3;
    pub const BOOT_SERVICES_DATA: u32 = 4;
    pub const RUNTIME_SERVICES_CODE: u32 = 5;
    pub const RUNTIME_SERVICES_DATA: u32 = 6;
    pub const CONVENTIONAL: u32 = 7;
    pub const UNUSABLE: u32 = 8;
    pub const ACPI_RECLAIM: u32 = 9;
    pub const ACPI_NVS: u32 = 10;
    pub const MMIO: u32 = 11;
    pub const MMIO_PORT_SPACE: u32 = 12;
    pub const PAL_CODE: u32 = 13;
    pub const PERSISTENT: u32 = 14;

    pub fn name(t: u32) -> &'static str {
        match t {
            RESERVED => "Reserved",
            LOADER_CODE => "LoaderCode",
            LOADER_DATA => "LoaderData",
            BOOT_SERVICES_CODE => "BootServicesCode",
            BOOT_SERVICES_DATA => "BootServicesData",
            RUNTIME_SERVICES_CODE => "RuntimeServicesCode",
            RUNTIME_SERVICES_DATA => "RuntimeServicesData",
            CONVENTIONAL => "ConventionalMemory",
            UNUSABLE => "UnusableMemory",
            ACPI_RECLAIM => "ACPIReclaim",
            ACPI_NVS => "ACPIMemoryNVS",
            MMIO => "MemoryMappedIO",
            MMIO_PORT_SPACE => "MMIOPortSpace",
            PAL_CODE => "PalCode",
            PERSISTENT => "PersistentMemory",
            _ => "Unknown",
        }
    }
}

/// A UCS-2 firmware string, displayed as ASCII (non-ASCII → '?').
/// Bounded iteration: firmware strings are NUL-terminated, but we never trust
/// an unterminated one — at most `MAX_UCS2_SCAN` code units are read.
pub struct Ucs2Str {
    ptr: *const u16,
}

const MAX_UCS2_SCAN: usize = 128;

impl Ucs2Str {
    /// # Safety
    /// `ptr` must be null-terminated UCS-2 readable for at most
    /// MAX_UCS2_SCAN units, or null.
    pub const unsafe fn new(ptr: *const u16) -> Self {
        Self { ptr }
    }
}

impl fmt::Display for Ucs2Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ptr.is_null() {
            return f.write_str("<null>");
        }
        for i in 0..MAX_UCS2_SCAN {
            // SAFETY: caller guarantees readability up to NUL within bound;
            // we stop at NUL or at MAX_UCS2_SCAN, whichever comes first.
            let c = unsafe { *self.ptr.add(i) };
            if c == 0 {
                break;
            }
            let ch = if (0x20..0x7f).contains(&c) {
                c as u8 as char
            } else {
                '?'
            };
            f.write_fmt(format_args!("{ch}"))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Firmware state capture & services we invoke
// ---------------------------------------------------------------------------

use core::sync::atomic::{AtomicUsize, Ordering};

/// Captured EFI_SYSTEM_TABLE pointer. Stored once at entry, before anything
/// that could panic; read by the panic handler (reset) and boot-info capture.
static SYSTEM_TABLE: AtomicUsize = AtomicUsize::new(0);

/// Capture the system table pointer. Called once from `efi_main` before any
/// other work.
pub fn init(system_table: *const SystemTable) {
    SYSTEM_TABLE.store(system_table as usize, Ordering::Release);
}

fn system_table() -> Option<*const SystemTable> {
    match SYSTEM_TABLE.load(Ordering::Acquire) {
        0 => None,
        p => Some(p as *const SystemTable),
    }
}

/// Firmware vendor string (UCS-2, borrowed from firmware) + revision.
///
/// # Safety
/// Requires `init()` to have been called with a valid system table.
pub unsafe fn firmware_info() -> Option<(Ucs2Str, u32)> {
    // SAFETY: caller guarantees init() ran with a valid table; firmware
    // keeps the system table (and its vendor string) alive until
    // ExitBootServices (UEFI 2.10 §3.6).
    unsafe {
        let st = system_table()?;
        Some((Ucs2Str::new((*st).firmware_vendor), (*st).firmware_revision))
    }
}

/// Boot-services function table pointer.
///
/// # Safety
/// As [`firmware_info`]; valid only until ExitBootServices (M2+).
pub unsafe fn boot_services() -> Option<&'static BootServices> {
    // SAFETY: caller contract as above; the table outlives boot services use.
    unsafe { system_table().map(|st| &*(*st).boot_services.cast::<BootServices>()) }
}

/// Halt the machine via UEFI ResetSystem(EfiResetShutdown).
///
/// Returns only if the system table was never captured (panic-before-init) or
/// firmware misbehaves; callers must have a fallback (see `panic.rs`).
pub fn reset_shutdown() {
    // SAFETY: we call this single-CPU, boot-services-active; ResetSystem is
    // legal at any time per UEFI 2.10 §8.6.11 and does not return on success.
    unsafe {
        if let Some(st) = system_table() {
            let rt = &*(*st).runtime_services;
            (rt.reset_system)(EFI_RESET_SHUTDOWN, EFI_SUCCESS, 0, core::ptr::null());
        }
    }
}
