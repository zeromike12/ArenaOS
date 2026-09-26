//! Kernel-owned device drivers. M1: the 16550 serial console. M2: the 8254
//! PIT (kernel tick + real-time reference for TSC calibration) and, from
//! M2.7 on, the interrupt-controller pair (IOAPIC/LAPIC) whose routing the
//! firmware tears down at ExitBootServices. M5.1 adds the one driver that
//! STAYS in the kernel by design (ADR-0021): the PCI enumerator — it owns
//! config space, BAR sizing, the virtio capability walk, and DMA
//! authorization (MEM|BUS MASTER), recording what the userspace driver
//! servers need. Since M5.2 (ADR-0022) the mechanism side is real: the
//! VirtIO protocol itself runs in ring 3 (`userspace/storaged`), and
//! this module's PCI layer adds the arming half of `SYS_IRQ_RELAY` —
//! MSI-X table writes through paging's pre-wired kernel PCI window and
//! the config-space enable bit. Everything beyond panic diagnostics and
//! this policy layer lives in userspace driver servers (ADR-0002).

pub mod intc;
pub mod pci;
pub mod pit;
pub mod serial;
