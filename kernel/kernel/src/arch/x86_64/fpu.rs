//! Native x87/SSE setup and the bounded FXSAVE64 state format (proposed ADR-0112).
//!
//! Ordinary Rust kernel code remains integer-only. These three leaf routines
//! are the complete reviewed instruction surface: two preserve their caller
//! while probing/building the image, and `arena_fpu_install_clean` is an
//! explicit architectural state boundary used before scheduler startup.

use super::{CpuInfo, cr0, cr4, read_cr0, read_cr4, write_cr0, write_cr4};
use crate::sync::SyncCell;
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, Ordering};

pub const FX_STATE_BYTES: usize = 512;
pub const FX_STATE_ALIGN: usize = 16;
pub const MXCSR_DEFAULT: u32 = 0x0000_1F80;
pub const MXCSR_FALLBACK_MASK: u32 = 0x0000_FFBF;

/// Exact legacy FXSAVE64 image. Scheduler storage is kernel static memory and
/// this type is never exposed through the capability or user mapping APIs.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxState {
    bytes: [u8; FX_STATE_BYTES],
}

impl Default for FxState {
    fn default() -> Self {
        Self::zeroed()
    }
}

const _: [(); FX_STATE_BYTES] = [(); core::mem::size_of::<FxState>()];
const _: [(); FX_STATE_ALIGN] = [(); core::mem::align_of::<FxState>()];

impl FxState {
    pub const fn zeroed() -> Self {
        Self {
            bytes: [0; FX_STATE_BYTES],
        }
    }

    fn u16_at(&self, offset: usize) -> u16 {
        u16::from_le_bytes([self.bytes[offset], self.bytes[offset + 1]])
    }

    fn u32_at(&self, offset: usize) -> u32 {
        u32::from_le_bytes([
            self.bytes[offset],
            self.bytes[offset + 1],
            self.bytes[offset + 2],
            self.bytes[offset + 3],
        ])
    }
}

static PROBE_IMAGE: SyncCell<FxState> = SyncCell::new(FxState::zeroed());
static CLEAN_IMAGE: SyncCell<FxState> = SyncCell::new(FxState::zeroed());
static INITIAL_MXCSR: SyncCell<u32> = SyncCell::new(MXCSR_DEFAULT);
static SANITIZED_XCR0: SyncCell<Option<u64>> = SyncCell::new(None);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

global_asm!(
    ".section .text",
    // Win64 C ABI: RCX = output image. At entry RSP is 8 mod 16, so reserving
    // 520 bytes aligns the 512-byte FXSAVE work area without touching the
    // caller-provided 32-byte home area. FXSAVE/FXRSTOR preserve the full
    // incoming architectural state around this diagnostic probe.
    ".p2align 4",
    ".globl arena_fpu_probe_image",
    "arena_fpu_probe_image:",
    "sub rsp, 0x208",
    "fxsave64 [rsp]",
    "fninit",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fninit",
    "fxsave64 [rcx]",
    "fxrstor64 [rsp]",
    "add rsp, 0x208",
    "ret",
    ".globl arena_fpu_probe_image_end",
    "arena_fpu_probe_image_end:",
    // RCX = output image, RDX = aligned MXCSR word. Construct an empty-tag
    // x87 image only after writing zero into all eight physical x87 registers;
    // all sixteen XMM registers are cleared. Restore the caller after capture.
    ".p2align 4",
    ".globl arena_fpu_build_clean",
    "arena_fpu_build_clean:",
    "sub rsp, 0x208",
    "fxsave64 [rsp]",
    "fninit",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fldz",
    "fninit",
    "ldmxcsr [rdx]",
    "pxor xmm0, xmm0",
    "pxor xmm1, xmm1",
    "pxor xmm2, xmm2",
    "pxor xmm3, xmm3",
    "pxor xmm4, xmm4",
    "pxor xmm5, xmm5",
    "pxor xmm6, xmm6",
    "pxor xmm7, xmm7",
    "pxor xmm8, xmm8",
    "pxor xmm9, xmm9",
    "pxor xmm10, xmm10",
    "pxor xmm11, xmm11",
    "pxor xmm12, xmm12",
    "pxor xmm13, xmm13",
    "pxor xmm14, xmm14",
    "pxor xmm15, xmm15",
    "fxsave64 [rcx]",
    "fxrstor64 [rsp]",
    "add rsp, 0x208",
    "ret",
    ".globl arena_fpu_build_clean_end",
    "arena_fpu_build_clean_end:",
    // RCX = trusted kernel-owned image. This is an explicit state-transition
    // leaf, like the scheduler switch, and intentionally changes FP state.
    ".p2align 4",
    ".globl arena_fpu_install_clean",
    "arena_fpu_install_clean:",
    "fxrstor64 [rcx]",
    "ret",
    ".globl arena_fpu_install_clean_end",
    "arena_fpu_install_clean_end:",
    // These two tiny leaves are called only when firmware left OSXSAVE set.
    // Read XCR0, reduce it to the legacy x87+SSE components, verify the
    // result, then clear CR4.OSXSAVE so AVX-family instructions fault.
    ".p2align 4",
    ".globl arena_fpu_read_xcr0",
    "arena_fpu_read_xcr0:",
    "xor ecx, ecx",
    "xgetbv",
    "shl rdx, 32",
    "or rax, rdx",
    "ret",
    ".globl arena_fpu_read_xcr0_end",
    "arena_fpu_read_xcr0_end:",
    ".p2align 4",
    ".globl arena_fpu_set_legacy_xcr0",
    "arena_fpu_set_legacy_xcr0:",
    "xor ecx, ecx",
    "mov eax, 3",
    "xor edx, edx",
    "xsetbv",
    "ret",
    ".globl arena_fpu_set_legacy_xcr0_end",
    "arena_fpu_set_legacy_xcr0_end:",
);

unsafe extern "C" {
    pub(super) fn arena_fpu_probe_image(output: *mut FxState);
    pub(super) fn arena_fpu_probe_image_end();
    pub(super) fn arena_fpu_build_clean(output: *mut FxState, mxcsr: *const u32);
    pub(super) fn arena_fpu_build_clean_end();
    pub(super) fn arena_fpu_install_clean(state: *const FxState);
    pub(super) fn arena_fpu_install_clean_end();
    pub(super) fn arena_fpu_read_xcr0() -> u64;
    pub(super) fn arena_fpu_read_xcr0_end();
    pub(super) fn arena_fpu_set_legacy_xcr0();
    pub(super) fn arena_fpu_set_legacy_xcr0_end();
}

/// Pure part of the mandatory profile, also exercised by host tests.
pub const fn required_features_present(
    has_fpu: bool,
    has_fxsr: bool,
    has_sse: bool,
    has_sse2: bool,
) -> bool {
    has_fpu && has_fxsr && has_sse && has_sse2
}

pub const fn inherited_osxsave_is_consistent(cpuid_osxsave: bool, cr4_osxsave: bool) -> bool {
    cpuid_osxsave == cr4_osxsave
}

pub const fn xcr0_supports_legacy_profile(supported: u64) -> bool {
    supported & 0x3 == 0x3
}

/// Normalize the architectural FXSAVE MXCSR mask field. A zero field means
/// the legacy architectural fallback, not that no MXCSR bits are writable.
pub const fn effective_mxcsr_mask(reported: u32) -> Option<u32> {
    let mask = if reported == 0 {
        MXCSR_FALLBACK_MASK
    } else {
        reported
    };
    if mask & 0xFFFF_0000 != 0 {
        None
    } else {
        Some(mask)
    }
}

/// Reset a scheduler slot from an already validated template. Kept pure so
/// host tests exercise the same lifecycle operation used by spawn and reap.
pub fn reset_state(destination: &mut FxState, template: &FxState) {
    *destination = *template;
}

/// Validate stable architectural fields in a captured clean image.
pub fn valid_clean_image(image: &FxState, mxcsr_mask: u32, initial_mxcsr: u32) -> bool {
    image.u16_at(0) == 0x037F
        && image.u16_at(2) == 0
        && image.bytes[4] == 0 // abridged FTW: all x87 stack entries empty
        && image.u32_at(24) == initial_mxcsr
        && image.u32_at(28) == mxcsr_mask
        && initial_mxcsr & !mxcsr_mask == 0
        && image.bytes[32..160].iter().all(|byte| *byte == 0)
        && image.bytes[160..416].iter().all(|byte| *byte == 0)
}

/// Initialize the CPU's FXSAVE profile and build the canonical per-thread
/// image. Called once at the beginning of `kmain`, before any scheduling or
/// user execution. Every control-register write is read back before use.
pub fn init(cpu: &CpuInfo) -> Result<(), &'static str> {
    if INITIALIZED.load(Ordering::Acquire) {
        return Err("FPU initialization already completed");
    }
    if !required_features_present(cpu.has_fpu, cpu.has_fxsr, cpu.has_sse, cpu.has_sse2) {
        return Err("CPU lacks required x87/FXSR/SSE/SSE2 support");
    }

    let old_cr0 = read_cr0();
    let want_cr0 = (old_cr0 & !(cr0::EM | cr0::TS)) | cr0::MP | cr0::NE;
    // SAFETY: the required profile is present; unrelated paging/protection
    // bits are preserved and each modified bit is verified immediately.
    unsafe { write_cr0(want_cr0) };
    let live_cr0 = read_cr0();
    let cr0_mask = cr0::MP | cr0::EM | cr0::TS | cr0::NE;
    if live_cr0 & cr0_mask != want_cr0 & cr0_mask {
        return Err("CR0 FP policy readback mismatch");
    }

    let old_cr4 = read_cr4();
    let inherited_osxsave = old_cr4 & cr4::OSXSAVE != 0;
    if !inherited_osxsave_is_consistent(cpu.has_osxsave, inherited_osxsave) {
        return Err("firmware CR4.OSXSAVE and CPUID.OSXSAVE disagree");
    }
    if inherited_osxsave {
        let max_basic = super::cpuid(0).eax;
        if max_basic < 0xD {
            return Err("firmware enabled OSXSAVE without CPUID leaf 0xD");
        }
        let supported = super::cpuid(0xD);
        let xcr0_supported = (supported.edx as u64) << 32 | supported.eax as u64;
        if !xcr0_supports_legacy_profile(xcr0_supported) {
            return Err("CPUID XCR0 mask lacks required x87/SSE components");
        }
        // SAFETY: firmware left CR4.OSXSAVE enabled, so XGETBV/XSETBV are
        // available at CPL0. Restrict the active state bitmap to the legacy
        // components that FXSAVE64 preserves, then read it back.
        let _firmware_xcr0 = unsafe { arena_fpu_read_xcr0() };
        unsafe { arena_fpu_set_legacy_xcr0() };
        if unsafe { arena_fpu_read_xcr0() } != 0x3 {
            return Err("XCR0 did not reduce to the x87/SSE legacy profile");
        }
        unsafe { *SANITIZED_XCR0.get() = Some(0x3) };
    } else {
        // XGETBV is not legal once OSXSAVE is clear. With no inherited
        // OSXSAVE, extended-vector state is not active or addressable.
        unsafe { *SANITIZED_XCR0.get() = None };
    }
    let want_cr4 = (old_cr4 | cr4::OSFXSR | cr4::OSXMMEXCPT) & !cr4::OSXSAVE;
    // SAFETY: FXSR/SSE are mandatory above; preserve all unrelated features.
    unsafe { write_cr4(want_cr4) };
    let live_cr4 = read_cr4();
    let cr4_mask = cr4::OSFXSR | cr4::OSXMMEXCPT | cr4::OSXSAVE;
    if live_cr4 & cr4_mask != want_cr4 & cr4_mask {
        return Err("CR4 FP policy readback mismatch");
    }
    if super::cpuid(1).ecx & (1 << 27) != 0 {
        return Err("CPUID still reports OSXSAVE after CR4.OSXSAVE was cleared");
    }

    // SAFETY: the aligned static image is writable kernel memory and CR0/CR4
    // now enable FXSAVE64. The leaf saves and restores its caller state.
    unsafe { arena_fpu_probe_image(PROBE_IMAGE.get()) };
    let raw_mask = unsafe { (*PROBE_IMAGE.get()).u32_at(28) };
    let mask = effective_mxcsr_mask(raw_mask).ok_or("FXSAVE reported reserved MXCSR mask bits")?;
    let initial_mxcsr = MXCSR_DEFAULT & mask;
    unsafe { *INITIAL_MXCSR.get() = initial_mxcsr };

    // SAFETY: both pointers are aligned, kernel-owned static storage. The
    // builder preserves the state live at this call boundary.
    unsafe { arena_fpu_build_clean(CLEAN_IMAGE.get(), INITIAL_MXCSR.get()) };
    if !valid_clean_image(unsafe { &*CLEAN_IMAGE.get() }, mask, initial_mxcsr) {
        return Err("captured clean FXSAVE64 template is invalid");
    }

    // Establish the same clean state for the bootstrap thread. This is an
    // intentional architectural state transition before scheduler startup;
    // kernel code is compiled/audited to keep no live FP/SIMD values here.
    unsafe { arena_fpu_install_clean(CLEAN_IMAGE.get()) };
    INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

/// Copy the verified template into a stable scheduler slot. The caller holds
/// the scheduler's IF=0 ownership and must assign the slot before it is Ready.
pub fn clean_state() -> FxState {
    if !INITIALIZED.load(Ordering::Acquire) {
        crate::halt::halt_machine("scheduler requested FP state before FPU initialization");
    }
    unsafe { *CLEAN_IMAGE.get() }
}

pub fn initial_mxcsr() -> u32 {
    unsafe { *INITIAL_MXCSR.get() }
}

pub fn mxcsr_mask() -> u32 {
    unsafe { (*CLEAN_IMAGE.get()).u32_at(28) }
}

/// XCR0 after firmware state was reduced to legacy x87+SSE components. `None`
/// means OSXSAVE was not enabled on entry, so XCR0 was not accessible.
pub fn sanitized_xcr0() -> Option<u64> {
    unsafe { *SANITIZED_XCR0.get() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fxsave_image_has_architectural_size_and_alignment() {
        assert_eq!(core::mem::size_of::<FxState>(), 512);
        assert_eq!(core::mem::align_of::<FxState>(), 16);
        assert_eq!((FxState::zeroed().bytes.as_ptr() as usize) & 0xF, 0);
    }

    #[test]
    fn required_feature_policy_rejects_each_missing_baseline_bit() {
        assert!(required_features_present(true, true, true, true));
        assert!(!required_features_present(false, true, true, true));
        assert!(!required_features_present(true, false, true, true));
        assert!(!required_features_present(true, true, false, true));
        assert!(!required_features_present(true, true, true, false));
    }

    #[test]
    fn inherited_osxsave_and_xcr0_policy_is_fail_closed() {
        assert!(inherited_osxsave_is_consistent(false, false));
        assert!(inherited_osxsave_is_consistent(true, true));
        assert!(!inherited_osxsave_is_consistent(true, false));
        assert!(!inherited_osxsave_is_consistent(false, true));
        assert!(xcr0_supports_legacy_profile(0x3));
        assert!(xcr0_supports_legacy_profile(0x7));
        assert!(!xcr0_supports_legacy_profile(0x1));
        assert!(!xcr0_supports_legacy_profile(0x2));
    }

    #[test]
    fn mxcsr_zero_mask_uses_fallback_and_reserved_bits_are_refused() {
        assert_eq!(effective_mxcsr_mask(0), Some(MXCSR_FALLBACK_MASK));
        assert_eq!(effective_mxcsr_mask(0x0000_FFFF), Some(0x0000_FFFF));
        assert_eq!(effective_mxcsr_mask(0x0001_0000), None);
        assert_eq!(MXCSR_DEFAULT & MXCSR_FALLBACK_MASK, MXCSR_DEFAULT);
    }

    #[test]
    fn clean_image_validator_checks_control_words_payload_and_all_xmm() {
        let mut image = FxState::zeroed();
        image.bytes[0..2].copy_from_slice(&0x037Fu16.to_le_bytes());
        image.bytes[24..28].copy_from_slice(&MXCSR_DEFAULT.to_le_bytes());
        image.bytes[28..32].copy_from_slice(&MXCSR_FALLBACK_MASK.to_le_bytes());
        assert!(valid_clean_image(
            &image,
            MXCSR_FALLBACK_MASK,
            MXCSR_DEFAULT
        ));
        image.bytes[160 + 15 * 16 + 8] = 1;
        assert!(!valid_clean_image(
            &image,
            MXCSR_FALLBACK_MASK,
            MXCSR_DEFAULT
        ));
    }

    #[test]
    fn slot_reinitialization_uses_the_clean_template_value() {
        let template = FxState::zeroed();
        let mut recycled = FxState::zeroed();
        recycled.bytes[160] = 0xA5;
        reset_state(&mut recycled, &template);
        assert_eq!(recycled, template);
    }
}
