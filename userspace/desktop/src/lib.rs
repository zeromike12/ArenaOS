//! Desktop userspace policy. Receiver adapters must verify capabilities.
#![no_std]
pub mod model;

pub mod perf;

pub mod compose;
pub mod desk;
pub mod shell;

pub mod wire;

pub mod client;

pub mod input_wire;

pub mod associations;
pub mod favorites;
pub mod fs_backend;
pub mod preferences;
pub mod scope;
pub mod service_wire;

#[path = "../../abi.rs"]
mod abi;
pub mod files;
#[path = "../../filesd_wire.rs"]
pub mod filesd_wire;
pub mod package;

pub mod app_client;
pub mod apps;
pub mod session_auth;

/// Entry point with a dedicated stack (Phase 11.3).
///
/// The kernel starts a process on one derived 4 KiB page directly above
/// its image, so an overflow silently writes into `.bss`. Desktop
/// binaries hold window scenes and band tables on the stack; this gives
/// them `$bytes` of stack in the `.stack` segment, which `desktop.ld`
/// places above an unmapped guard page: an overflow is a page fault, never
/// corruption.
#[macro_export]
macro_rules! entry {
    ($main:path, $bytes:expr) => {
        #[repr(C, align(4096))]
        struct ArenaStack([u8; $bytes]);
        #[unsafe(link_section = ".stack")]
        static mut ARENA_STACK: ArenaStack = ArenaStack([0; $bytes]);
        #[unsafe(naked)]
        #[unsafe(no_mangle)]
        pub extern "C" fn _start() -> ! {
            core::arch::naked_asm!(
                "lea rsp, [rip + {stack} + {size}]",
                "call {main}",
                "ud2",
                stack = sym ARENA_STACK,
                size = const $bytes,
                main = sym $main,
            )
        }
    };
}
