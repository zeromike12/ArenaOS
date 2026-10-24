#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

// Standalone CPL3 probe for #XM/#XF delivery. Argument 0 executes only the
// MXCSR setup/readback and invalid SQRTSS; argument 1 exits successfully so
// the same process can prove that a surviving sibling remains runnable.
global_asm!(
    ".section .text",
    ".p2align 4",
    ".globl _start",
    "_start:",
    "cmp edi, 1",
    "je .Lxf_probe_sibling",
    "sub rsp, 0x20",
    // All six sticky flags clear; invalid-operation mask bit 7 clear.
    "mov dword ptr [rsp + 0x10], 0x00001F00",
    "ldmxcsr dword ptr [rsp + 0x10]",
    "mov eax, 0xBF800000",
    "movd xmm0, eax",
    // Record the actual state immediately before SQRTSS. The compare and
    // branch below are integer-only and cannot alter MXCSR.
    "stmxcsr dword ptr [rsp + 0x14]",
    "cmp dword ptr [rsp + 0x14], 0x00001F00",
    "jne .Lxf_probe_bad_prestate",
    "sqrtss xmm0, xmm0",
    // If execution reaches here, return both the recorded pre-state and the
    // actual post-state so the guest harness can distinguish fallthrough.
    "stmxcsr dword ptr [rsp + 0x18]",
    "mov eax, dword ptr [rsp + 0x14]",
    "shl rax, 32",
    "mov ecx, dword ptr [rsp + 0x18]",
    "or rax, rcx",
    "mov rdi, rax",
    "jmp .Lxf_probe_exit",
    ".Lxf_probe_bad_prestate:",
    "mov edi, 0x200",
    "jmp .Lxf_probe_exit",
    ".Lxf_probe_sibling:",
    "mov edi, 42",
    ".Lxf_probe_exit:",
    "mov eax, 2",
    "syscall",
    "ud2",
);

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
