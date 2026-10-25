/*
 * Phase 16 register-preservation probe (reviewed assembly; EXPERIMENTAL).
 *
 * Why assembly: under the C calling convention every XMM register is
 * caller-saved, so C code cannot keep a value in an XMM register across a
 * call such as thrd_yield() -- the compiler spills it to the stack first
 * (which is exactly what a high-level C loop measures, whatever it claims).
 * This probe follows the accepted Phase 15 fixture technique: it loads
 * known, distinct values into xmm0-xmm7 and three x87 stack slots, performs
 * ONE raw `syscall` transition, then captures FXSAVE64. Between the loads
 * and the FXSAVE nothing stores those values to memory, so any difference
 * in the captured image is a state-preservation failure.
 *
 * Syscall register contract (src/sysabi.h arena_syscall6): number in rax,
 * args in rdi/rsi/rdx/r10/r8/r9; clobbers rax/rcx/r11 and the arg registers.
 * The probe saves/restores callee-saved rbx and r12-r15, and re-initializes
 * the x87 unit before returning (caller's empty-stack assumption).
 *
 * int64_t fp_probe_run(uint64_t nr, uint64_t a0, uint64_t a1, uint64_t a2,
 *                      uint64_t a3, const void *xmm_vals (8 x 16 bytes),
 *                      void *fxsave_out (512 bytes, 16-byte aligned));
 * C arg registers: rdi=nr rsi=a0 rdx=a1 rcx=a2 r8=a3 r9=xmm_vals,
 * 7th argument fxsave_out at [rsp+8] on entry. nr == 0 = no-syscall control.
 * Returns the syscall's rax, or 0 for the control.
 *
 * Built with -DFP_PROBE_MUTATE only for the isolated negative-control ELF:
 * one byte of the captured XMM3 slot is flipped, and the comparator in
 * fp_image_check.h must reject the image.
 */
    .intel_syntax noprefix
    .section .text
    .globl fp_probe_run
    .type fp_probe_run, @function
fp_probe_run:
    push rbx
    push r12
    push r13
    push r14
    push r15
    sub rsp, 16
    /* Park everything in callee-saved regs / local slots before FP loads. */
    mov r12, rdi                /* nr         */
    mov rbx, rsi                /* a0         */
    mov r13, rdx                /* a1         */
    mov [rsp], rcx              /* a2         */
    mov [rsp + 8], r8           /* a3         */
    mov r14, r9                 /* xmm_vals   */
    mov r15, [rsp + 16 + 40 + 8] /* fxsave_out (past locals, 5 pushes, ret) */
    /* Load xmm0-xmm7 with the caller's distinct patterns. */
    movdqu xmm0, [r14]
    movdqu xmm1, [r14 + 16]
    movdqu xmm2, [r14 + 32]
    movdqu xmm3, [r14 + 48]
    movdqu xmm4, [r14 + 64]
    movdqu xmm5, [r14 + 80]
    movdqu xmm6, [r14 + 96]
    movdqu xmm7, [r14 + 112]
    /* Known x87 state: fninit (TOP=0), three pushes land in R7, R6, R5. */
    fninit
    fld tbyte ptr [rip + .Lx87_a]   /* 1.0 -> R7 */
    fld tbyte ptr [rip + .Lx87_b]   /* 2.5 -> R6 */
    fld tbyte ptr [rip + .Lx87_c]   /* 0.5 -> R5 */
    /* The single transition under test (or none for the control). */
    xor eax, eax
    test r12, r12
    jz .Lcapture
    mov rax, r12
    mov rdi, rbx
    mov rsi, r13
    mov rdx, [rsp]
    mov r10, [rsp + 8]
    syscall
.Lcapture:
    fxsave64 [r15]
#ifdef FP_PROBE_MUTATE
    xor byte ptr [r15 + 160 + 3 * 16], 0xFF
#endif
    mov r12, rax
    fninit
    mov rax, r12
    add rsp, 16
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    ret
    .size fp_probe_run, . - fp_probe_run

    .section .rodata
    .p2align 4
    /* 80-bit extended: 1 sign, 15 exp (bias 16383), 64 explicit-int mantissa. */
.Lx87_a: .byte 0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x80,0xFF,0x3F  /* 1.0 */
.Lx87_b: .byte 0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xA0,0x00,0x40  /* 2.5 */
.Lx87_c: .byte 0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x80,0xFE,0x3F  /* 0.5 */
