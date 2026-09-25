//! 16550A UART driver (COM1), polled TX — the kernel's console from the first
//! instruction that can print to the moment of panic.
//!
//! Reference: "PC16550D Universal Asynchronous Receiver/Transmitter"
//! datasheet; registers below cite it (LSR §, LCR §...). QEMU's `-serial`
//! isa device emulates this part faithfully, including loopback mode, which
//! the M1 self-test uses to prove the port I/O path really works.
//!
//! Why polled: UEFI forbids interrupt-driven models before ExitBootServices
//! (ADR-0003), and a console that works with interrupts disabled is exactly
//! what panic diagnostics need forever after.

use core::fmt;

use crate::arch::x86_64::{inb, outb};

/// COM1 base port (legacy BIOS-assigned; OVMF wires it to QEMU `-serial`).
const COM1_BASE: u16 = 0x03F8;

// Register offsets from base (PC16550D datasheet, Table "Register Description")
const RBR_THR_DLL: u16 = 0; // Receiver Buffer (read) / Transmit Holding (write) / Divisor Latch Low (DLAB=1)
const IER_DLM: u16 = 1; // Interrupt Enable Register / Divisor Latch High (DLAB=1)
const IIR_FCR: u16 = 2; // Interrupt ID (read) / FIFO Control (write)
const LCR: u16 = 3; // Line Control Register
const MCR: u16 = 4; // Modem Control Register
const LSR: u16 = 5; // Line Status Register
const SCRATCH: u16 = 7; // Scratch Register (datasheet: general purpose R/W)

// Line Control Register bits (PC16550D §LCR)
const LCR_DLAB: u8 = 1 << 7; // Divisor Latch Access Bit
const LCR_8N1: u8 = 0x03; // 8 data bits, no parity, 1 stop bit

// Line Status Register bits (PC16550D §LSR)
const LSR_THR_EMPTY: u8 = 1 << 5; // Transmitter Holding Register is empty

// Modem Control Register bits (PC16550D §MCR)
const MCR_DTR: u8 = 1 << 0;
const MCR_RTS: u8 = 1 << 1;
const MCR_OUT2: u8 = 1 << 3; // gates interrupts on real hardware
const MCR_LOOPBACK: u8 = 1 << 4; // internal loopback test mode
const MCR_NORMAL: u8 = MCR_DTR | MCR_RTS | MCR_OUT2;

/// 115200 / 38400 = 3. Clock is the standard 1.8432 MHz.
const BAUD_DIVISOR_38400: u16 = 3;

/// FIFO enable + clear both FIFOs + 14-byte trigger (PC16550D §FCR).
const FCR_INIT: u8 = 0xC7;

/// Bound on spin-waits for THR-empty. A present UART empties a byte in
/// microseconds; a *missing* UART on real hardware typically reads 0xFF
/// (floating bus) which passes the check immediately, but a wedged device
/// must never hang the boot path. On exhaustion we drop the byte — the
/// console is best-effort by contract; correctness of the system never
/// depends on a byte reaching the screen.
const TX_WAIT_BOUND: u32 = 10_000_000;

/// Initialize COM1: 38400 baud, 8N1, FIFO on, interrupts off, DTR/RTS on.
///
/// # Safety
/// Requires ring 0 port I/O privileges (we are the boot stage) and that
/// COM1 either exists or its port range is inert (reads 0xFF / writes
/// ignored — true for PC-class IO space).
pub unsafe fn init() {
    unsafe {
        outb(COM1_BASE + IER_DLM, 0x00); // disable all UART interrupts
        outb(COM1_BASE + LCR, LCR_DLAB); // enter divisor-latch mode
        outb(COM1_BASE + RBR_THR_DLL, (BAUD_DIVISOR_38400 & 0xFF) as u8);
        outb(COM1_BASE + IER_DLM, (BAUD_DIVISOR_38400 >> 8) as u8);
        outb(COM1_BASE + LCR, LCR_8N1); // 8N1 also clears DLAB
        outb(COM1_BASE + IIR_FCR, FCR_INIT); // enable + clear FIFOs
        outb(COM1_BASE + MCR, MCR_NORMAL);
    }
}

/// Blocking (bounded) transmit of one byte.
///
/// # Safety
/// Port I/O contract as [`init`].
pub unsafe fn putc(c: u8) {
    unsafe {
        let mut spin = 0u32;
        while inb(COM1_BASE + LSR) & LSR_THR_EMPTY == 0 {
            spin += 1;
            if spin >= TX_WAIT_BOUND {
                return; // console is best-effort; never hang the boot path
            }
            core::hint::spin_loop();
        }
        outb(COM1_BASE + RBR_THR_DLL, c);
    }
}

/// Hardware self-test: scratch-register round-trip + loopback round-trip.
///
/// The scratch register (offset 7) is a pure R/W byte inside the UART; the
/// loopback bit (MCR bit 4) internally wires THR output back to RBR input.
/// A successful round-trip through *both* proves: our port I/O reaches a real
/// 16550-compatible device, and reads return what the device holds — not
/// values we merely wrote into our own memory. (PC16550D §MCR: "when bit 4 is
/// set, the transmitter output is connected to the receiver input".)
///
/// # Safety
/// Port I/O contract as [`init`]. Temporarily reprograms UART mode; callers
/// must be the only console user (true at boot).
pub unsafe fn loopback_selftest() -> Result<(), &'static str> {
    unsafe {
        // 1. Scratch register round-trip with two complementary patterns.
        let orig = inb(COM1_BASE + SCRATCH);
        for pattern in [0x5Au8, 0xA5] {
            outb(COM1_BASE + SCRATCH, pattern);
            let read = inb(COM1_BASE + SCRATCH);
            if read != pattern {
                return Err("scratch register round-trip mismatch (no 16550 at COM1?)");
            }
        }
        outb(COM1_BASE + SCRATCH, orig);

        // 2. Loopback round-trip.
        outb(COM1_BASE + MCR, MCR_NORMAL | MCR_LOOPBACK);
        for probe in [0x41u8, 0xFF, 0x00] {
            outb(COM1_BASE + RBR_THR_DLL, probe);
            let mut spin = 0u32;
            while inb(COM1_BASE + LSR) & 0x01 == 0 {
                // LSR bit 0: data ready
                spin += 1;
                if spin >= TX_WAIT_BOUND {
                    outb(COM1_BASE + MCR, MCR_NORMAL);
                    return Err("loopback: transmitter never delivered to receiver");
                }
                core::hint::spin_loop();
            }
            let echo = inb(COM1_BASE + RBR_THR_DLL);
            if echo != probe {
                outb(COM1_BASE + MCR, MCR_NORMAL);
                return Err("loopback: byte came back corrupted");
            }
        }
        outb(COM1_BASE + MCR, MCR_NORMAL);
        Ok(())
    }
}

/// The console writer used by `log.rs` and `fmt`.
pub struct SerialConsole;

impl fmt::Write for SerialConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                // SAFETY: boot stage owns COM1; port contract as `init`.
                unsafe { putc(b'\r') };
            }
            // SAFETY: as above.
            unsafe { putc(b) };
        }
        Ok(())
    }
}
