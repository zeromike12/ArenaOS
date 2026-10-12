//! Link-time address table consumed by `tools/audit_kernel_isa.py`.
//!
//! The deterministic EFI image intentionally omits COFF symbols. This small
//! retained `.rdata` record gives the build audit exact start/end VAs for the
//! six reviewed instruction leaves without keeping a symbol table or debug
//! directory in the release artifact.

use super::context::{arena_context_switch, arena_context_switch_end};
use super::fpu::{
    arena_fpu_build_clean, arena_fpu_build_clean_end, arena_fpu_install_clean,
    arena_fpu_install_clean_end, arena_fpu_probe_image, arena_fpu_probe_image_end,
    arena_fpu_read_xcr0, arena_fpu_read_xcr0_end, arena_fpu_write_xcr0, arena_fpu_write_xcr0_end,
};

#[repr(transparent)]
#[derive(Clone, Copy)]
struct Address(*const ());

// This is an immutable link-time record of code addresses, never dereferenced
// at runtime. The build script consumes its bytes from the PE `.rdata` section.
unsafe impl Sync for Address {}

#[repr(C)]
struct Range {
    start: Address,
    end: Address,
}

#[repr(C)]
struct Table {
    magic: [u8; 8],
    version: u32,
    range_count: u32,
    ranges: [Range; 6],
}

#[used]
#[unsafe(link_section = ".rdata$arena_fp_audit")]
static TABLE: Table = Table {
    magic: *b"ARFP15M1",
    version: 1,
    range_count: 6,
    ranges: [
        Range {
            start: Address(arena_context_switch as *const ()),
            end: Address(arena_context_switch_end as *const ()),
        },
        Range {
            start: Address(arena_fpu_probe_image as *const ()),
            end: Address(arena_fpu_probe_image_end as *const ()),
        },
        Range {
            start: Address(arena_fpu_build_clean as *const ()),
            end: Address(arena_fpu_build_clean_end as *const ()),
        },
        Range {
            start: Address(arena_fpu_install_clean as *const ()),
            end: Address(arena_fpu_install_clean_end as *const ()),
        },
        Range {
            start: Address(arena_fpu_read_xcr0 as *const ()),
            end: Address(arena_fpu_read_xcr0_end as *const ()),
        },
        Range {
            start: Address(arena_fpu_write_xcr0 as *const ()),
            end: Address(arena_fpu_write_xcr0_end as *const ()),
        },
    ],
};
