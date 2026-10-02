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

/// EFI_INVALID_PARAMETER (UEFI 2.10 §7.1) — what `ExitBootServices`
/// returns when the map key is stale (the map changed since the
/// GetMemoryMap that produced it).
pub const EFI_INVALID_PARAMETER: Status = 0x8000_0000_0000_0002;

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
    // Slots 5..=16 — the UEFI 2.10 §7.3 table order. NOTE: CloseEvent and
    // CheckEvent (slots 11/12) are long-standing boot services that are
    // easy to misremember as absent; omitting them shifts HandleProtocol
    // from its true slot 16 onto ReinstallProtocolInterface (slot 14),
    // which answers every protocol query with a plausible-looking
    // EFI_NOT_FOUND. Verified against EDK2's DxeCore mBootServices table
    // and this firmware's own HeaderSize (376 = 24 + 44 slots).
    pub allocate_pool: usize,
    pub free_pool: unsafe extern "efiapi" fn(buffer: *const u8) -> Status,
    pub create_event: usize,
    pub set_timer: usize,
    pub wait_for_event: usize,
    pub signal_event: usize,
    pub close_event: usize,
    pub check_event: usize,
    pub install_protocol_interface: usize,
    pub reinstall_protocol_interface: usize,
    pub uninstall_protocol_interface: usize,
    /// EFI_HANDLE_PROTOCOL — UEFI 2.10 §7.3 (table slot 16).
    pub handle_protocol: unsafe extern "efiapi" fn(
        handle: usize,
        protocol: *const Guid,
        interface: *mut usize,
    ) -> Status,
    // Slots 17..=25 in spec order (UEFI 2.10 §7.3). Slot 17 is Reserved
    // (NULL) in every spec version — it is counted here, as always.
    pub reserved_17: usize,
    pub register_protocol_notify: usize,
    pub locate_handle: usize,
    pub locate_device_path: usize,
    pub install_configuration_table: usize,
    pub load_image: usize,
    pub start_image: usize,
    pub exit: usize,
    pub unload_image: usize,
    /// EFI_EXIT_BOOT_SERVICES — UEFI 2.10 §7.3.16 (table slot 26). The
    /// point of no return (M2.7): on success every slot above this one is
    /// dead; only runtime services survive.
    pub exit_boot_services:
        unsafe extern "efiapi" fn(image_handle: usize, map_key: usize) -> Status,
    pub get_next_monotonic_count: usize,
    pub stall: usize,
    /// EFI_SET_WATCHDOG_TIMER — UEFI 2.10 §7.3.19 (table slot 29).
    /// Disabled (timeout 0) before ExitBootServices so nothing external
    /// can reset the machine mid-handoff.
    pub set_watchdog_timer: unsafe extern "efiapi" fn(
        timeout: usize,
        watchdog_code: usize,
        data_size: usize,
        watchdog_data: *const u16,
    ) -> Status,
    // Slots 30..36, in UEFI 2.10 §7.3 order: ConnectController,
    // DisconnectController, OpenProtocol, CloseProtocol,
    // OpenProtocolInformation, ProtocolsPerHandle, LocateHandleBuffer.
    pub graphics_preceding_services: [usize; 7],
    /// Slot 37: EFI_LOCATE_PROTOCOL — called only before EBS.
    pub locate_protocol: unsafe extern "efiapi" fn(
        protocol: *const Guid,
        registration: *const u8,
        interface: *mut *const u8,
    ) -> Status,
    // Slots 38..43 remain unused.
}
const _: () = assert!(core::mem::size_of::<BootServices>() == 24 + 38 * 8);

impl BootServices {
    /// The table's own claimed spec revision (this firmware reports 0x20046,
    /// the 2.70-era encoding (2 << 16) | 70).
    pub fn spec_revision(&self) -> u32 {
        self.hdr.revision
    }
}

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
    deposit_reset_fn();
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
/// Hand the kernel's halt path the runtime `ResetSystem` entry point
/// (handoff ABI, ADR-0011): runtime services survive `ExitBootServices`,
/// so this is the one firmware function the kernel proper may call.
fn deposit_reset_fn() {
    // SAFETY: called from `init` with firmware's live system table;
    // `set_reset_system`'s contract is exactly this typed field's address.
    unsafe {
        if let Some(st) = system_table() {
            let rt = &*(*st).runtime_services;
            #[allow(clippy::fn_to_numeric_cast_with_truncation)]
            let ptr = rt.reset_system as usize as u64;
            arena_kernel::halt::set_reset_system(ptr);
        }
    }
}

/// EFI_GUID — UEFI 2.10 §2.2 (mixed-endian layout, 16 bytes).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Guid(pub u32, pub u16, pub u16, pub [u8; 8]);
const _: () = assert!(core::mem::size_of::<Guid>() == 16);

/// gEfiLoadedImageProtocolGuid — UEFI 2.10 §9.1.
pub const LOADED_IMAGE_PROTOCOL_GUID: Guid = Guid(
    0x5B1B_31A1,
    0x9562,
    0x11D2,
    [0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
);

/// UEFI 2.10 §12.9: GOP discovery only; no protocol pointer survives EBS.
pub const GRAPHICS_OUTPUT_PROTOCOL_GUID: Guid = Guid(
    0x9042_A9DE,
    0x23DC,
    0x4A38,
    [0x96, 0xFB, 0x7A, 0xDE, 0xD0, 0x80, 0x51, 0x6A],
);

#[repr(C)]
struct GraphicsOutput {
    query_mode: unsafe extern "efiapi" fn(
        *const GraphicsOutput,
        u32,
        *mut usize,
        *mut *const GraphicsModeInfo,
    ) -> Status,
    set_mode: unsafe extern "efiapi" fn(*const GraphicsOutput, u32) -> Status,
    blt: usize,
    mode: *const GraphicsMode,
}
const _: () = assert!(core::mem::size_of::<GraphicsOutput>() == 32);

#[repr(C)]
struct GraphicsMode {
    max_mode: u32,
    mode: u32,
    info: *const GraphicsModeInfo,
    size_of_info: usize,
    framebuffer_base: u64,
    framebuffer_size: usize,
}
const _: () = assert!(core::mem::size_of::<GraphicsMode>() == 40);

#[repr(C)]
struct GraphicsModeInfo {
    version: u32,
    horizontal: u32,
    vertical: u32,
    format: u32,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    reserved_mask: u32,
    pitch_pixels: u32,
}
const _: () = assert!(core::mem::size_of::<GraphicsModeInfo>() == 36);

/// Capture a bounded direct-color GOP mode, preferring 800×600 and then
/// 640×480. The firmware owns all pointers here: copy scalar facts and
/// release every QueryMode buffer BEFORE ExitBootServices. Unsupported
/// machines return None rather than fabricating a display. This routine
/// never paints and never gives the kernel a firmware protocol pointer.
pub fn graphics_output_info() -> Option<arena_kernel::handoff::Display> {
    // SAFETY: called in the boot stage before EBS, with a live system table.
    unsafe {
        let bs = boot_services()?;
        if bs.hdr.header_size < core::mem::size_of::<BootServices>() as u32 {
            return None;
        }
        let mut interface = core::ptr::null();
        if (bs.locate_protocol)(
            &GRAPHICS_OUTPUT_PROTOCOL_GUID,
            core::ptr::null(),
            &mut interface,
        ) != EFI_SUCCESS
            || interface.is_null()
        {
            return None;
        }
        let gop = &*interface.cast::<GraphicsOutput>();
        if gop.mode.is_null() {
            return None;
        }
        let current = &*gop.mode;
        if current.max_mode == 0 || current.max_mode > 64 {
            return None;
        }
        // First valid preferred resolution, never a blind SetMode.
        for &(w, h) in &[(800, 600), (640, 480)] {
            for idx in 0..current.max_mode {
                let mut size = 0usize;
                let mut info: *const GraphicsModeInfo = core::ptr::null();
                let status = (gop.query_mode)(gop, idx, &mut size, &mut info);
                let wanted = status == EFI_SUCCESS
                    && !info.is_null()
                    && size >= core::mem::size_of::<GraphicsModeInfo>()
                    && (*info).horizontal == w
                    && (*info).vertical == h
                    && (*info).format <= 1
                    && (*info).pitch_pixels >= w
                    && (*info).pitch_pixels <= 1024;
                if !info.is_null() {
                    let _ = (bs.free_pool)(info.cast::<u8>());
                }
                if wanted && (gop.set_mode)(gop, idx) == EFI_SUCCESS {
                    return validated_graphics_mode(&*gop.mode);
                }
            }
        }
        // Some firmware exposes only its current mode. If it is within
        // the bounded 2 MiB scanout budget, use it without resetting it.
        validated_graphics_mode(current)
    }
}

fn validated_graphics_mode(mode: &GraphicsMode) -> Option<arena_kernel::handoff::Display> {
    // SAFETY: mode/info are firmware-owned and valid through EBS; caller is
    // still inside this pre-EBS boot routine.
    let info = unsafe { mode.info.as_ref()? };
    let (w, h, pitch) = (info.horizontal, info.vertical, info.pitch_pixels);
    if info.format > 1
        || w == 0
        || h == 0
        || w > 800
        || h > 600
        || pitch < w
        || pitch > 1024
        || mode.framebuffer_base & 4095 != 0
    {
        return None;
    }
    let bytes = u64::from(pitch).checked_mul(u64::from(h))?.checked_mul(4)?;
    if bytes > 2 * 1024 * 1024
        || bytes > mode.framebuffer_size as u64
        || mode.framebuffer_base.checked_add(bytes).is_none()
    {
        return None;
    }
    Some(arena_kernel::handoff::Display {
        phys: mode.framebuffer_base,
        bytes,
        width: w,
        height: h,
        pitch_pixels: pitch,
        format: info.format,
    })
}

/// EFI_LOADED_IMAGE_PROTOCOL — UEFI 2.10 §9.1 (fields up to ImageSize; the
/// rest is opaque to us). Offsets are spec-mandated and asserted below —
/// this is how paging (M2.4) learns where firmware loaded our image.
#[repr(C)]
pub struct LoadedImageProtocol {
    pub revision: u32,
    _pad0: u32,
    pub parent_handle: usize,
    pub system_table: usize,
    pub device_handle: usize,
    pub file_path: usize,
    pub reserved: usize,
    pub load_options_size: u32,
    _pad1: u32,
    pub load_options: usize,
    /// Where the image lives in physical memory (identity-mapped pre-EBS).
    pub image_base: usize,
    /// In-memory size including headers and all sections (.bss included).
    pub image_size: u64,
    // ImageCodeType/ImageDataType/Unload follow; not consumed at M2.
}
const _: () = assert!(core::mem::offset_of!(LoadedImageProtocol, image_base) == 64);
const _: () = assert!(core::mem::offset_of!(LoadedImageProtocol, image_size) == 72);

/// Query the Loaded Image Protocol for our own image: (base, size).
/// None if firmware or the handle is not what we expect — callers treat a
/// missing answer as a hard error (no guessing where we are loaded).
pub fn loaded_image_info(image_handle: usize) -> Option<(u64, u64)> {
    // SAFETY: single-CPU boot context; `boot_services()` was captured from
    // the live system table at entry; the handle is the one firmware passed
    // to `efi_main`; HandleProtocol writes one pointer-sized output.
    unsafe {
        let boot = boot_services()?;
        let mut interface = 0usize;
        let status =
            (boot.handle_protocol)(image_handle, &LOADED_IMAGE_PROTOCOL_GUID, &mut interface);
        if status != EFI_SUCCESS || interface == 0 {
            arena_kernel::log::log_warn!(
                "uefi",
                "loaded_image_info: handle_protocol status={status:#x} interface={interface:#x} handle={image_handle:#x}"
            );
            return None;
        }
        let li = interface as *const LoadedImageProtocol;
        let base = (*li).image_base as u64;
        let size = (*li).image_size;
        if base == 0 || size == 0 || !base.is_multiple_of(4096) {
            arena_kernel::log::log_warn!(
                "uefi",
                "loaded_image_info: implausible base={base:#x} size={size:#x}"
            );
            return None;
        }
        Some((base, size))
    }
}
