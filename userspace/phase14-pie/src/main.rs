#![no_std]
#![no_main]

use arena_lib::abi::{SYS_DEBUG_WRITE, SYS_THREAD_EXIT, syscall1, syscall2};
use arena_runtime::startup;
use arena_startup_abi::startup::StartupView;
use core::arch::naked_asm;

const STACK_BYTES: usize = 16 * 1024;
const EXPECTED_INIT: u64 = 0x1357_9bdf_2468_ace0;
const EXPECTED_CONST: &[u8] = b"ArenaOS static PIE relocation passed";

#[repr(C, align(4096))]
struct InitialStack([u8; STACK_BYTES]);

#[unsafe(link_section = ".stack")]
static mut INITIAL_STACK: InitialStack = InitialStack([0; STACK_BYTES]);

#[unsafe(link_section = ".rodata.phase14")]
#[used]
static READ_ONLY_VALUE: [u8; 36] = *b"ArenaOS static PIE relocation passed";

#[unsafe(link_section = ".data.phase14")]
#[used]
static mut INITIALIZED_VALUE: u64 = EXPECTED_INIT;

#[unsafe(link_section = ".bss.phase14")]
#[used]
static mut ZERO_INITIALIZED_VALUE: u64 = 0;

unsafe extern "C" fn relocated_function() -> u64 {
    42
}

#[unsafe(link_section = ".data.phase14")]
#[used]
static mut RELOCATED_FUNCTION: unsafe extern "C" fn() -> u64 = relocated_function;

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    naked_asm!(
        "lea rsp, [rip + {stack} + {size}]",
        "and rsp, -16",
        "call {main}",
        "ud2",
        stack = sym INITIAL_STACK,
        size = const STACK_BYTES,
        main = sym main,
    )
}

extern "C" fn main() -> ! {
    match startup::run(run_application) {
        Ok(()) => exit(42),
        Err(_) => exit(70),
    }
}

fn run_application(view: StartupView<'_>) {
    let actual_entry = _start as *const () as usize as u64;
    let actual_base = view.load_base();
    let initialized = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(INITIALIZED_VALUE)) };
    let mut zeroed =
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!(ZERO_INITIALIZED_VALUE)) };
    let relocated = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(RELOCATED_FUNCTION)) };
    let relocated_value = unsafe { relocated() };
    let const_ok = unsafe {
        core::ptr::read_volatile(core::ptr::addr_of!(READ_ONLY_VALUE)) == *EXPECTED_CONST
    };

    if actual_base == 0
        || !actual_base.is_multiple_of(2 * 1024 * 1024)
        || view.entry() != actual_entry
        || initialized != EXPECTED_INIT
        || zeroed != 0
        || !const_ok
        || relocated as usize != relocated_function as *const () as usize
        || relocated_value != 42
    {
        write(b"[phase14-pie] FAIL: startup, data, BSS, constant, or RELATIVE relocation\n");
        exit(71);
    }

    zeroed = 0x55aa_aa55_1234_4321;
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(ZERO_INITIALIZED_VALUE), zeroed);
    }
    if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(ZERO_INITIALIZED_VALUE)) } != zeroed {
        write(b"[phase14-pie] FAIL: BSS write/read\n");
        exit(72);
    }

    write(b"[phase14-pie] PASS base=0x");
    write_hex(actual_base);
    write(b" entry=0x");
    write_hex(actual_entry);
    write(b" relocated=0x");
    write_hex(relocated as usize as u64);
    write(b" data=0x");
    write_hex(initialized);
    write(b" bss=0x");
    write_hex(zeroed);
    write(b" exit=42\n");
}

fn write(bytes: &[u8]) {
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, bytes.as_ptr() as u64, bytes.len() as u64) };
}

fn write_hex(mut value: u64) {
    let mut bytes = [b'0'; 16];
    for index in (0..16).rev() {
        let digit = (value & 0xf) as u8;
        bytes[index] = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        value >>= 4;
    }
    write(&bytes);
}

fn exit(code: u64) -> ! {
    let _ = unsafe { syscall1(SYS_THREAD_EXIT, code) };
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(99)
}
