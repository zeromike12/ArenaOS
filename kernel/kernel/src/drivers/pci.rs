//! Kernel-side PCI enumeration (M5.1, ADR-0021) — the POLICY half of the
//! kernel/userspace driver split.
//!
//! At boot the kernel walks bus 0 through the architecture-guaranteed
//! config PIO pair (0xCF8 address / 0xCFC data), records every function it
//! finds, sizes its BARs, and — for each VirtIO function — walks the PCI
//! capability list for the VirtIO vendor capabilities (cap ID 0x09, virtio
//! 1.0 §4.1.4) and the MSI-X capability (0x11), then sets MEM|BUS MASTER
//! in the command register. That last write is the whole point of keeping
//! enumeration in the kernel: BUS MASTER is DMA authorization, and DMA is
//! a security decision (ADR-0004's driver-isolation promise). Config space
//! itself is NEVER exposed to ring 3 — a userspace driver receives the
//! *resolved records* (BAR windows as kernel-minted Mmio caps, capability
//! locations as plain data) and does all mechanism (virtqueues, doorbells)
//! itself from M5.2 on.
//!
//! Deliberately unsupported in v1, each with its reason:
//!
//! * buses > 0: the reference machine's VirtIO devices sit on bus 0;
//!   bridging is a Phase-6 concern (recorded in ADR-0021).
//! * legacy (PIO) VirtIO: the transitional QEMU device exposes BOTH the
//!   legacy IO BAR and modern capability structures; we drive the modern
//!   structures and ignore BAR0-IO. A device with ONLY the legacy
//!   interface (`virtio-pci` with `disable-modern=on`) is not supported
//!   by design.
//! * ECAM/MMCONFIG: the PIO pair is architecture-guaranteed present;
//!   MMCONFIG requires ACPI table parsing the kernel does not have yet.

use crate::arch::x86_64::{inl, outl};
use crate::log::log_info;
use crate::sync::{SyncCell, without_interrupts};

// ---- config-space mechanics ------------------------------------------------

/// PCI config address port (mechanism #1, PCI Local Bus Spec §3.2.2.3.2).
const CONFIG_ADDRESS: u16 = 0xCF8;
/// PCI config data port.
const CONFIG_DATA: u16 = 0xCFC;

const REG_VENDOR_DEVICE: u8 = 0x00;
const REG_COMMAND_STATUS: u8 = 0x04;
const REG_CLASS_REV: u8 = 0x08;
const REG_HEADER_TYPE: u8 = 0x0C;
const REG_BAR_BASE: u8 = 0x10;
const REG_SUBSYSTEM: u8 = 0x2C;
const REG_CAP_POINTER: u8 = 0x34;

/// Command register: memory-space decode enable.
const COMMAND_MEM: u16 = 1 << 1;
/// Command register: bus-master (DMA) enable — kernel policy (ADR-0021).
const COMMAND_BUS_MASTER: u16 = 1 << 2;
/// Status register: capability-list present.
const STATUS_CAP_LIST: u16 = 1 << 4;
/// Header type: multifunction device.
const HEADER_MULTIFUNCTION: u8 = 1 << 7;

// ---- VirtIO constants (OASIS virtio 1.0, §4.1) -----------------------------

/// The VirtIO vendor ID.
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
/// Transitional device IDs: 0x1000..=0x103F; the VirtIO device type comes
/// from the SUBSYSTEM device ID (legacy convention).
const VIRTIO_TRANSITIONAL_LO: u16 = 0x1000;
const VIRTIO_TRANSITIONAL_HI: u16 = 0x103F;
/// Modern device IDs: 0x1040 + the VirtIO device type.
const VIRTIO_MODERN_BASE: u16 = 0x1040;
/// VirtIO device type: block device.
pub const VIRTIO_TYPE_BLOCK: u16 = 2;

/// PCI capability ID: vendor-specific — VirtIO structures live in these.
const CAP_ID_VENDOR: u8 = 0x09;
/// PCI capability ID: MSI-X.
const CAP_ID_MSIX: u8 = 0x11;

/// `virtio_pci_cap.cfg_type` values (virtio 1.0 §4.1.4.3).
const VIRTIO_CFG_COMMON: u8 = 1;
const VIRTIO_CFG_NOTIFY: u8 = 2;
const VIRTIO_CFG_ISR: u8 = 3;
const VIRTIO_CFG_DEVICE: u8 = 4;

// ---- recorded tables -------------------------------------------------------

/// How many bus-0 functions the kernel records (the reference machine has
/// a handful; overflow is logged, not fatal).
pub const MAX_PCI_FUNCTIONS: usize = 8;
/// How many VirtIO functions the kernel records.
pub const MAX_VIRTIO_DEVICES: usize = 4;
/// Capability-list walk bound (a corrupt loop must not hang the boot).
const CAP_WALK_BOUND: usize = 48;

/// One resolved VirtIO structure location: which BAR, at what offset,
/// how long (virtio 1.0 `virtio_pci_cap`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VirtioCapLoc {
    pub present: bool,
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
}

const NO_CAP: VirtioCapLoc = VirtioCapLoc {
    present: false,
    bar: 0,
    offset: 0,
    length: 0,
};

/// The MSI-X capability record (needed by M5.2's relay arming).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MsixInfo {
    pub present: bool,
    /// Table size in entries (the register holds size-1).
    pub table_size: u16,
    pub table_bar: u8,
    pub table_offset: u32,
}

const NO_MSIX: MsixInfo = MsixInfo {
    present: false,
    table_size: 0,
    table_bar: 0,
    table_offset: 0,
};

/// One recorded bus-0 function.
#[derive(Clone, Copy)]
pub struct PciFunction {
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    /// Raw dword at 0x08: revision in the low byte, 24-bit class above.
    pub class_rev: u32,
    /// Raw dword at 0x2C: subsystem vendor (low 16) / device (high 16).
    pub subsystem: u32,
    /// Resolved BAR bases (64-bit BARs merged; IO BARs recorded with
    /// their raw base, flagged in `bar_is_io`; absent BARs are 0).
    pub bar_base: [u64; 6],
    /// Sized BAR lengths in bytes (0 for absent BARs).
    pub bar_size: [u64; 6],
    /// True where the BAR is an IO-port BAR (legacy; not driven — see the
    /// module header).
    pub bar_is_io: [bool; 6],
}

const NO_FUNCTION: PciFunction = PciFunction {
    dev: 0,
    func: 0,
    vendor_id: 0,
    device_id: 0,
    class_rev: 0,
    subsystem: 0,
    bar_base: [0; 6],
    bar_size: [0; 6],
    bar_is_io: [false; 6],
};

/// One recorded VirtIO function with its resolved capability structures.
#[derive(Clone, Copy)]
pub struct VirtioDevice {
    /// Index into the recorded function table.
    pub pci_index: usize,
    pub dev: u8,
    pub func: u8,
    pub device_id: u16,
    /// The VirtIO device TYPE (2 = block): from the subsystem ID for
    /// transitional devices, `device_id - 0x1040` for modern ones.
    pub virtio_type: u16,
    /// Transitional devices drive the modern structures here too, but
    /// the flag matters for revision checks and legacy-BAR expectations.
    pub transitional: bool,
    pub common: VirtioCapLoc,
    pub notify: VirtioCapLoc,
    pub isr: VirtioCapLoc,
    pub device_cfg: VirtioCapLoc,
    /// `notify_off_multiplier` (virtio 1.0 §4.1.4.7): queue N's doorbell
    /// lives at `notify.offset + queue_notify_off * multiplier`.
    pub notify_off_multiplier: u32,
    pub msix: MsixInfo,
}

const NO_VIRTIO: VirtioDevice = VirtioDevice {
    pci_index: 0,
    dev: 0,
    func: 0,
    device_id: 0,
    virtio_type: 0,
    transitional: false,
    common: NO_CAP,
    notify: NO_CAP,
    isr: NO_CAP,
    device_cfg: NO_CAP,
    notify_off_multiplier: 0,
    msix: NO_MSIX,
};

static PCI_FUNCS: SyncCell<[PciFunction; MAX_PCI_FUNCTIONS]> =
    SyncCell::new([NO_FUNCTION; MAX_PCI_FUNCTIONS]);
static PCI_COUNT: SyncCell<usize> = SyncCell::new(0);
static VIRTIO_DEVS: SyncCell<[VirtioDevice; MAX_VIRTIO_DEVICES]> =
    SyncCell::new([NO_VIRTIO; MAX_VIRTIO_DEVICES]);
static VIRTIO_COUNT: SyncCell<usize> = SyncCell::new(0);
static ENUMERATED: SyncCell<bool> = SyncCell::new(false);

// ---- config access ----------------------------------------------------------

/// Build the mechanism-#1 config address: enable bit | bus | device |
/// function | dword-aligned register.
fn config_address(bus: u8, dev: u8, func: u8, reg: u8) -> u32 {
    0x8000_0000
        | (u32::from(bus) << 16)
        | (u32::from(dev) << 11)
        | (u32::from(func) << 8)
        | u32::from(reg & 0xFC)
}

/// Aligned dword read from config space.
///
/// # Safety
/// Ring 0 with IOPL permitting CLI ports (the kernel's normal state).
unsafe fn config_read32(bus: u8, dev: u8, func: u8, reg: u8) -> u32 {
    // SAFETY: the config pair is the architecture-guaranteed window; the
    // caller owns the boot-time scan (single accessor under IF=0 — the
    // whole enumeration runs inside `without_interrupts`).
    unsafe {
        outl(CONFIG_ADDRESS, config_address(bus, dev, func, reg));
        inl(CONFIG_DATA)
    }
}

/// Aligned dword write to config space.
///
/// # Safety
/// As [`config_read32`].
unsafe fn config_write32(bus: u8, dev: u8, func: u8, reg: u8, value: u32) {
    // SAFETY: as above.
    unsafe {
        outl(CONFIG_ADDRESS, config_address(bus, dev, func, reg));
        outl(CONFIG_DATA, value);
    }
}

/// Byte read from config space (dword fetch + shift — the mechanism only
/// has dword granularity at the data port anyway).
///
/// # Safety
/// As [`config_read32`].
unsafe fn config_read8(bus: u8, dev: u8, func: u8, reg: u8) -> u8 {
    // SAFETY: as above.
    unsafe {
        let dword = config_read32(bus, dev, func, reg);
        (dword >> (8 * u32::from(reg & 3))) as u8
    }
}

/// Word read from config space (dword fetch + shift; `reg` must be
/// 2-aligned — every field this scan reads is).
///
/// # Safety
/// As [`config_read32`].
unsafe fn config_read16(bus: u8, dev: u8, func: u8, reg: u8) -> u16 {
    // SAFETY: as above.
    unsafe {
        let dword = config_read32(bus, dev, func, reg & 0xFE);
        (dword >> (8 * u32::from(reg & 2))) as u16
    }
}

/// Public re-read for the suites: 16-bit config field of a RECORDED
/// function (e.g. the command register, to prove MEM|BUS MASTER stuck).
/// Returns `None` for an unrecorded index.
pub fn config_read16_of(pci_index: usize, reg: u8) -> Option<u16> {
    let f = pci_function(pci_index)?;
    without_interrupts(|| {
        // SAFETY: ring 0, IF=0, recorded function on bus 0.
        unsafe { Some(config_read16(0, f.dev, f.func, reg)) }
    })
}

// ---- BAR sizing -------------------------------------------------------------

/// Size one memory/IO BAR by the all-ones write-back dance (PCI Local
/// Bus Spec §6.2.5). Returns (base, size, is_io); 64-bit BARs consume
/// their neighbor slot (`skip_next` tells the caller).
///
/// # Safety
/// As [`config_read32`]; `bar_reg` is the config offset of BAR N.
unsafe fn size_bar(bus: u8, dev: u8, func: u8, bar_reg: u8) -> (u64, u64, bool, bool) {
    // SAFETY: caller contract; the original value is always restored
    // before returning — a device left with an all-ones BAR would stop
    // decoding, and firmware-assigned windows must survive the scan.
    unsafe {
        let original = config_read32(bus, dev, func, bar_reg);
        if original == 0 {
            return (0, 0, false, false); // BAR not implemented
        }
        let is_io = original & 1 == 1;
        let is_64 = !is_io && (original >> 1) & 0b11 == 0b10;
        config_write32(bus, dev, func, bar_reg, 0xFFFF_FFFF);
        let sized = config_read32(bus, dev, func, bar_reg);
        config_write32(bus, dev, func, bar_reg, original);

        if is_io {
            let mask = 0xFFFF_FFFCu32;
            let base = u64::from(original & 0xFFFC);
            let size = u64::from((!(sized & mask)).wrapping_add(1) & mask);
            return (base, size, true, false);
        }
        let mask = 0xFFFF_FFF0u32;
        let mut base = u64::from(original & mask);
        // The size comes from inverting the ALL-ONES read-back — for a
        // 64-bit BAR the inversion must see the full merged 64-bit value
        // (inverting each half separately leaves the high half's ones in
        // the size; observed live as size 0xffffffff_00004000).
        let mut sized64 = u64::from(sized & mask);
        let mut skip_next = false;
        if is_64 {
            // The high half lives in the NEXT BAR register; size it the
            // same way (its own all-ones dance) and merge.
            let hi_reg = bar_reg + 4;
            let hi_original = config_read32(bus, dev, func, hi_reg);
            config_write32(bus, dev, func, hi_reg, 0xFFFF_FFFF);
            let hi_sized = config_read32(bus, dev, func, hi_reg);
            config_write32(bus, dev, func, hi_reg, hi_original);
            base |= u64::from(hi_original) << 32;
            sized64 |= u64::from(hi_sized) << 32;
            skip_next = true;
        }
        // Invert at the BAR's own width: 64-bit bars invert the merged
        // value; 32-bit bars invert within u32 (inverting a zero-extended
        // read-back would smear the high half's ones into the size).
        let size = if is_64 {
            (!sized64).wrapping_add(1)
        } else {
            u64::from(!(sized & mask)) + 1
        };
        (base, size, false, skip_next)
    }
}

// ---- the scan ----------------------------------------------------------------

/// Read every recorded field of one function and size its BARs.
///
/// # Safety
/// As [`config_read32`]; BAR sizing restores every original value.
unsafe fn record_function(dev: u8, func: u8, vendor_device: u32) -> PciFunction {
    // SAFETY: caller contract.
    unsafe {
        let mut f = NO_FUNCTION;
        f.dev = dev;
        f.func = func;
        f.vendor_id = vendor_device as u16;
        f.device_id = (vendor_device >> 16) as u16;
        f.class_rev = config_read32(0, dev, func, REG_CLASS_REV);
        f.subsystem = config_read32(0, dev, func, REG_SUBSYSTEM);
        let mut bar = 0u8;
        while bar < 6 {
            let (base, size, is_io, skip) = size_bar(0, dev, func, REG_BAR_BASE + bar * 4);
            f.bar_base[bar as usize] = base;
            f.bar_size[bar as usize] = size;
            f.bar_is_io[bar as usize] = is_io;
            // A 64-bit BAR consumed the next register slot as its high half.
            bar += if skip { 2 } else { 1 };
        }
        f
    }
}

/// Resolve a VirtIO function: device type, MEM|BUS MASTER enable (the
/// kernel's DMA-authorization policy write), the vendor-capability walk
/// (common/notify/isr/device structures + notify multiplier), and the
/// MSI-X record M5.2 arms relays from.
///
/// # Safety
/// As [`config_read32`]; writes touch only the command register.
unsafe fn record_virtio(pci_index: usize, f: &PciFunction) -> VirtioDevice {
    // SAFETY: caller contract.
    unsafe {
        let mut v = NO_VIRTIO;
        v.pci_index = pci_index;
        v.dev = f.dev;
        v.func = f.func;
        v.device_id = f.device_id;
        v.transitional = (VIRTIO_TRANSITIONAL_LO..=VIRTIO_TRANSITIONAL_HI).contains(&f.device_id);
        v.virtio_type = if v.transitional {
            // Legacy convention: the subsystem DEVICE id carries the type.
            (f.subsystem >> 16) as u16
        } else if f.device_id >= VIRTIO_MODERN_BASE {
            f.device_id - VIRTIO_MODERN_BASE
        } else {
            u16::MAX // vendor matched but the ID is nowhere in the spec
        };

        // The policy write: memory decode + bus master. This is the one
        // config-space mutation ring 3 never performs (ADR-0021).
        let cs = config_read32(0, f.dev, f.func, REG_COMMAND_STATUS);
        let command = cs as u16;
        let status = (cs >> 16) as u16;
        let want = command | COMMAND_MEM | COMMAND_BUS_MASTER;
        if want != command {
            config_write32(
                0,
                f.dev,
                f.func,
                REG_COMMAND_STATUS,
                (cs & 0xFFFF_0000) | u32::from(want),
            );
        }
        let now = config_read16(0, f.dev, f.func, REG_COMMAND_STATUS);
        log_info!(
            "pci",
            "  command {command:#06x} -> {now:#06x} (MEM|BUS MASTER — DMA authorized by kernel policy)"
        );

        if status & STATUS_CAP_LIST == 0 {
            log_info!(
                "pci",
                "  virtio device WITHOUT a capability list — modern structures unreachable"
            );
            return v;
        }
        let mut next = config_read8(0, f.dev, f.func, REG_CAP_POINTER) & 0xFC;
        for _ in 0..CAP_WALK_BOUND {
            if next == 0 {
                break;
            }
            match config_read8(0, f.dev, f.func, next) {
                CAP_ID_VENDOR => {
                    // virtio_pci_cap (virtio 1.0 §4.1.4): [3]=cfg_type,
                    // [4]=bar, [8..12]=offset, [12..16]=length; the notify
                    // cap appends notify_off_multiplier at [16..20].
                    let cfg_type = config_read8(0, f.dev, f.func, next + 3);
                    let loc = VirtioCapLoc {
                        present: true,
                        bar: config_read8(0, f.dev, f.func, next + 4),
                        offset: config_read32(0, f.dev, f.func, next + 8),
                        length: config_read32(0, f.dev, f.func, next + 12),
                    };
                    match cfg_type {
                        VIRTIO_CFG_COMMON => v.common = loc,
                        VIRTIO_CFG_NOTIFY => {
                            v.notify = loc;
                            v.notify_off_multiplier = config_read32(0, f.dev, f.func, next + 16);
                        }
                        VIRTIO_CFG_ISR => v.isr = loc,
                        VIRTIO_CFG_DEVICE => v.device_cfg = loc,
                        _ => {}
                    }
                }
                CAP_ID_MSIX => {
                    // [2..4] message control (bits 10:0 = table size-1),
                    // [4..8] table offset/BIR (bits 2:0 = the BAR).
                    let ctl = config_read16(0, f.dev, f.func, next + 2);
                    let tbl = config_read32(0, f.dev, f.func, next + 4);
                    v.msix = MsixInfo {
                        present: true,
                        table_size: (ctl & 0x7FF) + 1,
                        table_bar: (tbl & 7) as u8,
                        table_offset: tbl & 0xFFFF_FFF8,
                    };
                }
                _ => {}
            }
            next = config_read8(0, f.dev, f.func, next + 1) & 0xFC;
        }
        v
    }
}

fn log_virtio(v: &VirtioDevice) {
    log_info!(
        "pci",
        "  virtio device_id {:#06x} → type {} ({}){}",
        v.device_id,
        v.virtio_type,
        if v.virtio_type == VIRTIO_TYPE_BLOCK {
            "block"
        } else {
            "other"
        },
        if v.transitional {
            ", transitional"
        } else {
            ", modern"
        },
    );
    for (name, c) in [
        ("common", &v.common),
        ("notify", &v.notify),
        ("isr", &v.isr),
        ("device", &v.device_cfg),
    ] {
        if c.present {
            log_info!(
                "pci",
                "    {name}: bar{} + {:#x} len {:#x}",
                c.bar,
                c.offset,
                c.length
            );
        } else {
            log_info!("pci", "    {name}: ABSENT");
        }
    }
    log_info!(
        "pci",
        "    notify_off_multiplier = {:#x}",
        v.notify_off_multiplier
    );
    if v.msix.present {
        log_info!(
            "pci",
            "    msix: {} entries, table bar{} + {:#x}",
            v.msix.table_size,
            v.msix.table_bar,
            v.msix.table_offset
        );
    } else {
        log_info!("pci", "    msix: ABSENT");
    }
}

/// Walk bus 0, recording functions and VirtIO structures. Runs once
/// (repeat calls return the recorded counts); the whole scan holds IF=0
/// — config PIO is a shared two-port window that must not interleave
/// with any other accessor. Returns (recorded functions, recorded VirtIO
/// devices). Never fatal: a machine with nothing on bus 0 boots fine —
/// the m5 suite is what asserts the fixture disk.
pub fn enumerate() -> (usize, usize) {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        if unsafe { *ENUMERATED.get() } {
            return (pci_count(), virtio_count());
        }
        let mut pci_n = 0usize;
        let mut virtio_n = 0usize;
        for dev in 0..32u8 {
            // SAFETY: ring-0 boot scan under IF=0 (this fn's contract).
            let multifunction = unsafe {
                if config_read32(0, dev, 0, REG_VENDOR_DEVICE) & 0xFFFF == 0xFFFF {
                    continue; // nothing at this slot
                }
                config_read8(0, dev, 0, REG_HEADER_TYPE) & HEADER_MULTIFUNCTION != 0
            };
            let funcs: u8 = if multifunction { 8 } else { 1 };
            for func in 0..funcs {
                // SAFETY: as above.
                let vd = unsafe { config_read32(0, dev, func, REG_VENDOR_DEVICE) };
                if vd & 0xFFFF == 0xFFFF {
                    continue;
                }
                if pci_n >= MAX_PCI_FUNCTIONS {
                    log_info!("pci", "function table full — 00:{dev:02}.{func} dropped");
                    continue;
                }
                // SAFETY: as above; BAR sizing restores every original.
                let f = unsafe { record_function(dev, func, vd) };
                log_info!(
                    "pci",
                    "00:{:02}.{} {:04x}:{:04x} class {:06x} rev {:02x} subsys {:08x}",
                    f.dev,
                    f.func,
                    f.vendor_id,
                    f.device_id,
                    f.class_rev >> 8,
                    f.class_rev & 0xFF,
                    f.subsystem,
                );
                for b in 0..6 {
                    if f.bar_base[b] != 0 {
                        log_info!(
                            "pci",
                            "  bar{b}: base {:#x} size {:#x}{}",
                            f.bar_base[b],
                            f.bar_size[b],
                            if f.bar_is_io[b] {
                                " (io — legacy, not driven)"
                            } else {
                                ""
                            }
                        );
                    }
                }
                // SAFETY: single writer under IF=0.
                unsafe {
                    (*PCI_FUNCS.get())[pci_n] = f;
                    *PCI_COUNT.get() = pci_n + 1;
                }
                pci_n += 1;
                if f.vendor_id == VIRTIO_VENDOR_ID {
                    // SAFETY: as above; the command-register write is
                    // this scan's only config mutation.
                    let v = unsafe { record_virtio(pci_n - 1, &f) };
                    if virtio_n < MAX_VIRTIO_DEVICES {
                        // SAFETY: single writer under IF=0.
                        unsafe {
                            (*VIRTIO_DEVS.get())[virtio_n] = v;
                            *VIRTIO_COUNT.get() = virtio_n + 1;
                        }
                        virtio_n += 1;
                        log_virtio(&v);
                    } else {
                        log_info!("pci", "virtio table full — device not recorded");
                    }
                }
            }
        }
        // SAFETY: single writer under IF=0.
        unsafe { *ENUMERATED.get() = true };
        log_info!(
            "pci",
            "bus 0 scan complete: {pci_n} function(s) recorded, {virtio_n} virtio"
        );
        (pci_n, virtio_n)
    })
}

// ---- recorded-table accessors (copy-out; the tables are kernel-private) ----

/// Number of recorded bus-0 functions.
pub fn pci_count() -> usize {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { *PCI_COUNT.get() }
    })
}

/// Copy-out of recorded function `i` (None past the recorded count).
pub fn pci_function(i: usize) -> Option<PciFunction> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (i < *PCI_COUNT.get()).then(|| (*PCI_FUNCS.get())[i]) }
    })
}

/// Number of recorded VirtIO devices.
pub fn virtio_count() -> usize {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { *VIRTIO_COUNT.get() }
    })
}

/// Copy-out of recorded VirtIO device `i` (None past the recorded count).
pub fn virtio_device(i: usize) -> Option<VirtioDevice> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (i < *VIRTIO_COUNT.get()).then(|| (*VIRTIO_DEVS.get())[i]) }
    })
}

/// The first recorded VirtIO device of the given type (e.g.
/// [`VIRTIO_TYPE_BLOCK`]).
pub fn find_virtio(virtio_type: u16) -> Option<VirtioDevice> {
    (0..virtio_count()).find_map(|i| virtio_device(i).filter(|v| v.virtio_type == virtio_type))
}
