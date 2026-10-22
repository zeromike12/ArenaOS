//! Native AHCI fallback for the interactive VirtualBox profile.
//!
//! The kernel validates the native AHCI PCI class, sizes BAR5, and grants
//! storaged only that MMIO window after enabling the controller's memory
//! decode and bus-master bits. This module performs all AHCI protocol work in
//! ring 3. Requests retain storaged's existing endpoint and lent-frame block
//! ABI. A single command slot is used and completion is polled with a finite
//! bound; the driver never receives PCI configuration or arbitrary port I/O.

#![allow(dead_code)]

use crate::abi::*;
#[path = "ahci_protocol.rs"]
mod protocol;

const SLOT_ABAR: u64 = 0;
const SLOT_ENDPOINT: u64 = 1;
const SLOT_DIAGNOSTIC: u64 = 3;
const SLOT_DMA: u64 = 8;
const EXIT_AHCI: u64 = 69;
const EXIT_RECV: u64 = 65;
const EXIT_PHYS: u64 = 66;
const EXIT_REPLY: u64 = 68;
use protocol::{FRAME_BYTES, SECTOR_BYTES};
const CI_TIMEOUT_SPINS: usize = 100_000_000;

fn log(s: &str) {
    log_line(|o| o.str(s));
}

fn fail(reason: &str) -> ! {
    log_line(|o| {
        o.str("storaged: AHCI fatal: ");
        o.str(reason);
    });
    // SAFETY: thread_exit terminates this ring-3 service.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_AHCI) };
    loop {
        core::hint::spin_loop();
    }
}

fn reg_read(base: u64, offset: u64) -> u32 {
    // SAFETY: `base` is the kernel-minted, self-mapped AHCI ABAR capability;
    // every offset below is bounded by the one-page VirtualBox target profile.
    unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
}

fn reg_write(base: u64, offset: u64, value: u32) {
    // SAFETY: `base` is the exact writable ABAR capability granted to this
    // driver and all register offsets are within the bounded profile.
    unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
}

fn wait_clear(base: u64, offset: u64, mask: u32) -> bool {
    for _ in 0..CI_TIMEOUT_SPINS {
        if reg_read(base, offset) & mask == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn alloc_dma_frame(slot: u64) -> (u64, u64) {
    // SAFETY: the kernel allocates an owned frame into this empty driver slot;
    // mapping consumes that ownership into this process's registered region.
    let phys = unsafe { syscall1(SYS_ALLOC_FRAME, slot) };
    if phys <= 0 {
        fail("owned DMA frame allocation refused");
    }
    // SAFETY: exact owned frame cap just returned above; write access required.
    let va = unsafe { syscall2(SYS_MAP_MEMORY, slot, 1) };
    if va <= 0 {
        fail("owned DMA frame self-map refused");
    }
    (phys as u64, va as u64)
}

fn phys_pair(phys: u64) -> (u32, u32) {
    (phys as u32, (phys >> 32) as u32)
}

struct Controller {
    base: u64,
    port: u64,
    cap: u32,
    dma_phys: u64,
    dma_va: u64,
}

impl Controller {
    unsafe fn start() -> Self {
        // SAFETY: callers hold the exact Mmio cap the kernel minted for the
        // validated AHCI BAR5 function; map_memory enforces that cap's rights.
        let base_phys = unsafe { syscall1(SYS_CAP_PHYS, SLOT_ABAR) };
        if base_phys <= 0 || base_phys as u64 & 4095 != 0 {
            fail("validated ABAR capability is absent or unaligned");
        }
        let base = unsafe { syscall2(SYS_MAP_MEMORY, SLOT_ABAR, 1) };
        if base <= 0 {
            fail("ABAR self-map refused");
        }
        let base = base as u64;

        // BIOS/OS handoff (AHCI 1.3 §10.6), if this HBA advertises it.
        let cap2 = reg_read(base, 0x24);
        if cap2 & 1 != 0 {
            reg_write(base, 0x28, reg_read(base, 0x28) | (1 << 1)); // OOS
            let mut handed_off = false;
            for _ in 0..10_000_000usize {
                if reg_read(base, 0x28) & ((1 << 0) | (1 << 4)) == 0 {
                    handed_off = true;
                    break;
                }
                core::hint::spin_loop();
            }
            if !handed_off {
                fail("AHCI BIOS ownership handoff timed out");
            }
        }

        let cap = reg_read(base, 0x00);
        // GHC.AE must be on before accessing port registers. Interrupts stay
        // disabled; completion is observed in PxCI with a finite spin bound.
        reg_write(base, 0x04, (reg_read(base, 0x04) | (1 << 31)) & !(1 << 1));
        let implemented = reg_read(base, 0x0C);
        let ports = ((cap & 0x1F) + 1).min(8);
        let mut selected = None;
        for number in 0..ports {
            if implemented & (1 << number) == 0 {
                continue;
            }
            let port = base + 0x100 + u64::from(number) * 0x80;
            let ssts = reg_read(port, 0x28);
            let signature = reg_read(port, 0x24);
            if ssts & 0xF == 3 && signature == 0x0000_0101 {
                selected = Some((number, port));
                break;
            }
        }
        let Some((number, port)) = selected else {
            fail("no attached ATA disk found in the first eight AHCI ports");
        };

        let (dma_phys, dma_va) = alloc_dma_frame(SLOT_DMA);
        if cap & (1 << 31) == 0 && dma_phys > u64::from(u32::MAX) {
            fail("HBA lacks 64-bit DMA but the allocated frame is above 4 GiB");
        }
        // SAFETY: this is the driver's newly owned, mapped DMA page; zero it
        // before the HBA can observe command list/FIS/table structures.
        unsafe { core::ptr::write_bytes(dma_va as *mut u8, 0, FRAME_BYTES as usize) };

        // Stop the firmware's command engine before installing our DMA pages.
        let mut cmd = reg_read(port, 0x18);
        cmd &= !1; // ST
        reg_write(port, 0x18, cmd);
        if !wait_clear(port, 0x18, 1 << 15) {
            fail("AHCI command engine did not stop (CR stuck)");
        }
        cmd = reg_read(port, 0x18) & !(1 << 4); // FRE off before changing FB
        reg_write(port, 0x18, cmd);
        if !wait_clear(port, 0x18, 1 << 14) {
            fail("AHCI received-FIS engine did not stop (FR stuck)");
        }

        // One 4 KiB owned frame contains a 1 KiB command list at +0, the
        // received-FIS area at +1024, and a 128-byte-aligned command table at
        // +2048. These offsets satisfy AHCI's alignment requirements.
        let (clb, clbu) = phys_pair(dma_phys);
        let (fb, fbu) = phys_pair(dma_phys + 1024);
        reg_write(port, 0x00, clb);
        reg_write(port, 0x04, clbu);
        reg_write(port, 0x08, fb);
        reg_write(port, 0x0C, fbu);
        reg_write(port, 0x30, u32::MAX); // clear stale SERR
        reg_write(port, 0x10, u32::MAX); // clear stale PxIS
        cmd = reg_read(port, 0x18) | (1 << 4); // FRE, after FB is valid
        reg_write(port, 0x18, cmd);
        let mut fis_ready = false;
        for _ in 0..CI_TIMEOUT_SPINS {
            if reg_read(port, 0x18) & (1 << 14) != 0 {
                fis_ready = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !fis_ready {
            fail("AHCI received-FIS engine did not start");
        }
        // SUD enables spin-up when the HBA advertises staggered spin-up.
        cmd = reg_read(port, 0x18);
        if cap & (1 << 27) != 0 {
            cmd |= 1 << 1;
        }
        cmd |= 1; // ST
        reg_write(port, 0x18, cmd);

        log_line(|o| {
            o.str("storaged: native AHCI ready on port ");
            o.u64(u64::from(number));
            o.str("; CAP ");
            o.hex(u64::from(cap));
            o.str(", BAR5 ");
            o.hex(base_phys as u64);
            o.str(", DMA page ");
            o.hex(dma_phys);
        });
        Self {
            base,
            port,
            cap,
            dma_phys,
            dma_va,
        }
    }

    fn transfer(&mut self, op: u64, lba: u64, buffer_phys: u64, bytes: u64) -> bool {
        let write = op == OP_WRITE || op == OP_WRITE_BLOCK;
        let Some(encoded) = protocol::encode_transfer(lba, buffer_phys, bytes, write) else {
            return false;
        };
        if self.cap & (1 << 31) == 0
            && encoded.buffer_phys + encoded.bytes - 1 > u64::from(u32::MAX)
        {
            return false;
        }
        if !wait_clear(self.port, 0x38, u32::MAX) {
            return false;
        }
        reg_write(self.port, 0x10, u32::MAX);
        reg_write(self.port, 0x30, u32::MAX);

        let table = self.dma_va + 2048;
        // SAFETY: command table is in the driver's single owned/mapped DMA
        // frame; the port is stopped from issuing another command until the
        // completion below.
        unsafe { core::ptr::write_bytes(table as *mut u8, 0, 512) };
        let command_header = self.dma_va;
        write_u16(command_header, encoded.header_flags);
        write_u16(command_header + 2, 1); // one PRDT entry
        write_u32(command_header + 4, 0); // PRDBC
        write_u32(command_header + 8, (self.dma_phys + 2048) as u32);
        write_u32(command_header + 12, ((self.dma_phys + 2048) >> 32) as u32);

        // Register Host-to-Device FIS, READ/WRITE DMA EXT (48-bit LBA).
        for (offset, byte) in encoded.command.iter().copied().enumerate() {
            write_u8(table + offset as u64, byte);
        }

        // PRDT at command-table offset 128. IOC is set so future interrupt
        // support can use the same descriptor; this profile deliberately
        // polls completion and does not install an AHCI interrupt route.
        let prdt = table + 128;
        write_u32(prdt, encoded.buffer_phys as u32);
        write_u32(prdt + 4, (encoded.buffer_phys >> 32) as u32);
        write_u32(prdt + 8, 0);
        write_u32(prdt + 12, 0x8000_0000 | (encoded.bytes as u32 - 1));

        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        reg_write(self.port, 0x38, 1); // PxCI issue command slot 0
        if !wait_clear(self.port, 0x38, 1) {
            return false;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let is = reg_read(self.port, 0x10);
        let tfd = reg_read(self.port, 0x20);
        reg_write(self.port, 0x10, is);
        let bytes_done = unsafe { core::ptr::read_volatile((self.dma_va + 4) as *const u32) };
        let completed = is & ((1 << 30) | (1 << 29) | (1 << 28) | (1 << 27)) == 0
            && tfd & 0x89 == 0
            && u64::from(bytes_done) == encoded.bytes;
        completed && (!write || self.flush_cache())
    }

    fn flush_cache(&mut self) -> bool {
        if !wait_clear(self.port, 0x38, u32::MAX) {
            return false;
        }
        reg_write(self.port, 0x10, u32::MAX);
        reg_write(self.port, 0x30, u32::MAX);
        let table = self.dma_va + 2048;
        unsafe { core::ptr::write_bytes(table as *mut u8, 0, 512) };
        let command_header = self.dma_va;
        write_u16(command_header, 5);
        write_u16(command_header + 2, 0);
        write_u32(command_header + 4, 0);
        write_u32(command_header + 8, (self.dma_phys + 2048) as u32);
        write_u32(command_header + 12, ((self.dma_phys + 2048) >> 32) as u32);
        for (offset, byte) in protocol::encode_flush().iter().copied().enumerate() {
            write_u8(table + offset as u64, byte);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        reg_write(self.port, 0x38, 1);
        if !wait_clear(self.port, 0x38, 1) {
            return false;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let is = reg_read(self.port, 0x10);
        let tfd = reg_read(self.port, 0x20);
        reg_write(self.port, 0x10, is);
        is & ((1 << 30) | (1 << 29) | (1 << 28) | (1 << 27)) == 0 && tfd & 0x89 == 0
    }
}

fn write_u8(address: u64, value: u8) {
    // SAFETY: address is in the driver's mapped command table or header.
    unsafe { core::ptr::write_volatile(address as *mut u8, value) }
}
fn write_u16(address: u64, value: u16) {
    // SAFETY: address is in a properly aligned mapped AHCI structure.
    unsafe { core::ptr::write_volatile(address as *mut u16, value) }
}
fn write_u32(address: u64, value: u32) {
    // SAFETY: address is in a properly aligned mapped AHCI structure.
    unsafe { core::ptr::write_volatile(address as *mut u32, value) }
}

fn reply(status: u64, value: u64) {
    // SAFETY: this service is the sole server on its granted endpoint.
    let r = unsafe { syscall5(SYS_IPC_REPLY, SLOT_ENDPOINT, status, value, CAP_NONE, 0) };
    if r < 0 {
        fail("block reply refused");
    }
}

/// Start the AHCI-backed form of the ordinary block service. It intentionally
/// keeps the existing block request ABI, FS clients, and caller-owned DMA
/// frames unchanged.
pub fn run() -> ! {
    log("storaged: no modern virtio-blk device record; trying authorized AHCI BAR5");
    // SAFETY: feature profile and kernel boot policy provide MMIO/READ|WRITE
    // only when a native AHCI function with a bounded BAR5 was validated.
    let mut controller = unsafe { Controller::start() };
    // Before advertising a usable disk to fsd, perform one real sector read
    // and prove the virtual disk accepts FLUSH CACHE EXT. The scratch disk's
    // first sector is not assumed to contain a particular filesystem.
    let (probe_phys, _probe_va) = alloc_dma_frame(SLOT_DMA + 1);
    if !controller.transfer(OP_READ, 0, probe_phys, SECTOR_BYTES) {
        fail("initial AHCI sector read failed");
    }
    if !controller.flush_cache() {
        fail("ATA FLUSH CACHE EXT is unavailable or failed");
    }
    crate::signal_storage_ready();
    log("storaged: AHCI read and durable-write capability probes passed; storage service ready");
    let mut message = [0u64; 3];
    loop {
        // SAFETY: slot 1 is this service's exact READ endpoint capability.
        let r = unsafe { syscall3(SYS_IPC_RECV, SLOT_ENDPOINT, message.as_mut_ptr() as u64, 0) };
        if r < 0 {
            let _ = r;
            unsafe { syscall1(SYS_THREAD_EXIT, EXIT_RECV) };
            fail("IPC_RECV refused on AHCI block service");
        }
        let (lba, word, landed) = (message[0], message[1], message[2]);
        let (op, offset) = (word & 0xFF, word >> 8);
        if op == OP_SHUTDOWN {
            if take_diagnostic(landed, SLOT_DIAGNOSTIC) {
                reply(VIRTIO_BLK_S_OK, 0);
                // SAFETY: a valid diagnostic request orders a clean service exit.
                unsafe { syscall1(SYS_THREAD_EXIT, 42) };
                fail("thread_exit returned after diagnostic shutdown");
            }
            if landed != CAP_NONE {
                unsafe { syscall1(SYS_CAP_DESTROY, landed) };
            }
            reply(VIRTIO_BLK_S_UNSUPP, 0);
            continue;
        }
        let block = op == OP_READ_BLOCK || op == OP_WRITE_BLOCK;
        if (op > OP_WRITE && !block) || landed == CAP_NONE {
            if landed != CAP_NONE {
                unsafe { syscall1(SYS_CAP_DESTROY, landed) };
            }
            reply(VIRTIO_BLK_S_UNSUPP, 0);
            continue;
        }
        let bytes = if block { FRAME_BYTES } else { SECTOR_BYTES };
        if offset
            .checked_add(bytes)
            .is_none_or(|end| end > FRAME_BYTES)
        {
            unsafe { syscall1(SYS_CAP_DESTROY, landed) };
            reply(VIRTIO_BLK_S_UNSUPP, 0);
            continue;
        }
        // The physical buffer derives only from the LENT Untyped cap, never a
        // caller-provided address. Destroy it after the synchronous DMA.
        let buffer = unsafe { syscall1(SYS_CAP_PHYS, landed) };
        if buffer <= 0 {
            unsafe { syscall1(SYS_CAP_DESTROY, landed) };
            unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PHYS) };
            fail("SYS_CAP_PHYS refused the lent request frame");
        }
        let ok = controller.transfer(op, lba, buffer as u64 + offset, bytes);
        unsafe { syscall1(SYS_CAP_DESTROY, landed) };
        reply(
            if ok {
                VIRTIO_BLK_S_OK
            } else {
                VIRTIO_BLK_S_IOERR
            },
            if ok { bytes } else { 0 },
        );
    }
}
