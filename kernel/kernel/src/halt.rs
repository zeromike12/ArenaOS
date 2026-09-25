//! Terminal failure path: diagnostics, then a safe machine halt.
//!
//! "Halt safely" means the firmware's `ResetSystem(EfiResetShutdown)`
//! *runtime* service (ADR-0003) — runtime services survive
//! `ExitBootServices` by design (UEFI 2.10 §8.6), so this path works
//! identically in the boot stage and in the kernel proper after the M2.7
//! handoff. The boot stage deposits the entry point here as a raw
//! function pointer (`set_reset_system`); if it was never deposited
//! (failure before the firmware tables were captured) or firmware refuses
//! the call, we fall back to a CLI+HLT park loop: with IF=0 the CPU stays
//! parked forever.

use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

/// EFI_RESET_TYPE `EfiResetShutdown` (UEFI 2.10 §8.6.11).
pub const RESET_SHUTDOWN: u32 = 2;

/// `EFI_SUCCESS` for the ResetStatus argument (UEFI 2.10 §8.6.11).
const EFI_SUCCESS: usize = 0;

/// EFI_RESET_SYSTEM (UEFI 2.10 §8.6.11): (ResetType, ResetStatus,
/// DataSize, ResetData).
type ResetSystemFn = unsafe extern "efiapi" fn(u32, usize, usize, *const u8);

static RESET_FN: AtomicU64 = AtomicU64::new(0);

/// Firmware's own CR3, captured at boot entry (M2.7). Runtime services
/// execute correctly only in firmware's world: its page tables map the RT
/// drivers' private data writable (ours observed live to write-fault
/// inside `ResetSystem`, #PF ec=0x3 under the kernel view). 0 = never
/// captured; the direct-call fallback below then applies.
static FW_CR3: AtomicU64 = AtomicU64::new(0);

/// Deposit firmware's entry-time CR3 for the shutdown farewell.
///
/// # Safety
/// `cr3` must be the firmware page-table root read at boot entry; those
/// tables stay valid for the machine's remaining lifetime (we never free
/// firmware's frames — our allocations come from the bitmap, ADR-0008).
pub unsafe fn set_firmware_cr3(cr3: u64) {
    FW_CR3.store(cr3, Ordering::Relaxed);
}

global_asm!(
    // Shutdown farewell island (M2.7). Called at its IDENTITY address —
    // the one page of the kernel image the kernel view deliberately keeps
    // identity-mapped (build_kernel_view) — so the CR3 switch mid-body
    // survives: firmware's tables map this page, the kernel view maps it,
    // and the boot-stage tables map all identity. Win64/"C" arguments:
    //   rcx = firmware CR3, rdx = ResetSystem entry point.
    // After `mov cr3` only register instructions run until RSP is lowered
    // back to its identity alias (kernel-view stacks live at
    // phys+KERNEL_OFFSET; firmware's tables map the identity address).
    // Page-aligned; the whole body (~50 bytes) never crosses its page.
    ".section .text",
    ".p2align 12",
    ".globl arena_reset_island",
    "arena_reset_island:",
    "mov r10, rdx", // ResetSystem entry point
    "mov cr3, rcx", // firmware's own tables live
    "test rsp, rsp",
    "jns 1f",              // identity stack (bit63 clear): leave it
    "mov eax, 0x80000000", // +2 GiB wraps the high alias back to
    "add rsp, rax",        // identity (KERNEL_OFFSET = -2 GiB)
    "1:",
    "mov ecx, 2",   // EfiResetShutdown
    "xor edx, edx", // ResetStatus = EFI_SUCCESS
    "xor r8d, r8d", // DataSize = 0
    "xor r9d, r9d", // ResetData = NULL
    "sub rsp, 40",  // win64 shadow space + alignment
    "call r10",
    "2:", // ResetSystem returned: park for good
    "cli",
    "hlt",
    "jmp 2b",
);

unsafe extern "C" {
    /// The farewell island (see asm above); never returns.
    fn arena_reset_island(fw_cr3: u64, reset_fn: u64) -> !;
}

/// Address of the farewell island as this code sees it (identity during
/// boot, high alias under the kernel view — `kernel_view_phys` normalizes
/// either to the physical page; boot identity VA == phys makes that the
/// value `build_kernel_view` maps).
pub fn reset_island_addr() -> u64 {
    arena_reset_island as *const () as u64
}

/// Deposit the firmware's `ResetSystem` entry point (handoff ABI; see
/// [`crate::handoff`]).
///
/// # Safety
/// `ptr` must be the address of the firmware runtime services'
/// ResetSystem function (efiapi ABI), live for the machine's remaining
/// lifetime — the boot stage passes the typed table field directly.
pub unsafe fn set_reset_system(ptr: u64) {
    RESET_FN.store(ptr, Ordering::Relaxed);
}

/// Request a full system shutdown. Returns only if firmware never handed
/// over (or refuses) — and then parks the CPU instead, so from the
/// caller's perspective this never returns.
pub fn reset_shutdown() -> ! {
    // Canonical clean-halt declaration (tools/mtest.py, docs/TESTING.md):
    // this exact line is printed only on paths that reach ResetSystem, so
    // the harnesses can tell a clean shutdown from a triple fault (which
    // also exits QEMU with rc=0 under -no-reboot).
    crate::log::log_info!("halt", "halting via UEFI ResetSystem(shutdown)");
    let ptr = RESET_FN.load(Ordering::Relaxed);
    let fw_cr3 = FW_CR3.load(Ordering::Relaxed);
    if ptr != 0 && fw_cr3 != 0 {
        // Farewell-island path (M2.7+): hand the machine back to firmware
        // for the actual reset — its tables, its stack alias, its world.
        // SAFETY: the island is entered at its identity address, mapped
        // executable in every view this runs under (firmware/dual tables
        // map all identity; the kernel view maps the island page by
        // design). Args: firmware CR3 (set_firmware_cr3 contract) and the
        // captured ResetSystem entry (set_reset_system contract). It never
        // returns.
        let island = crate::arch::x86_64::paging::kernel_view_phys(reset_island_addr());
        let go: unsafe extern "C" fn(u64, u64) -> ! =
            unsafe { core::mem::transmute::<u64, unsafe extern "C" fn(u64, u64) -> !>(island) };
        unsafe { go(fw_cr3, ptr) };
    }
    if ptr != 0 {
        // Fallback: no firmware CR3 captured (failure earlier than boot
        // step 2) — call ResetSystem directly under whatever tables are
        // live (firmware's or the dual view; the M2.6-proven path).
        // SAFETY: `ptr` was deposited by the boot stage from the live
        // runtime-services table (contract on `set_reset_system`);
        // ResetSystem is legal at any time and does not return on success.
        let reset: ResetSystemFn = unsafe { core::mem::transmute::<u64, ResetSystemFn>(ptr) };
        unsafe { reset(RESET_SHUTDOWN, EFI_SUCCESS, 0, core::ptr::null()) };
    }

    // Never deposited, or firmware misbehaved: park the CPU.
    // SAFETY: CLI+HLT with interrupts masked parks the single CPU
    // permanently; nothing else is runnable in the boot/kernel context.
    unsafe {
        crate::arch::x86_64::cli();
        loop {
            core::arch::asm!("hlt", options(nostack, nomem, preserves_flags));
        }
    }
}

/// Log the terminal reason, then halt safely.
pub fn halt_machine(reason: &str) -> ! {
    crate::log::log_error!("halt", "halting machine: {reason}");
    reset_shutdown()
}
