.section .text,"ax"
.code64
.global efi_main
efi_main:
    cli
    cld

    /* Serial COM1, 115200 8N1. */
    movw $0x3f9, %dx
    xorb %al, %al
    outb %al, %dx
    movw $0x3fb, %dx
    movb $0x80, %al
    outb %al, %dx
    movw $0x3f8, %dx
    movb $1, %al
    outb %al, %dx
    movw $0x3f9, %dx
    xorb %al, %al
    outb %al, %dx
    movw $0x3fb, %dx
    movb $3, %al
    outb %al, %dx
    movw $0x3fa, %dx
    movb $0xc7, %al
    outb %al, %dx
    movw $0x3fc, %dx
    movb $0x0b, %al
    outb %al, %dx

    /* Install independent #UD and #XM handlers for this test application. */
    movw %cs, %cx
    leaq ud_handler(%rip), %rax
    movw %ax, idt + 6*16(%rip)
    movw %cx, idt + 6*16+2(%rip)
    movb $0, idt + 6*16+4(%rip)
    movb $0x8e, idt + 6*16+5(%rip)
    shrq $16, %rax
    movw %ax, idt + 6*16+6(%rip)
    shrq $16, %rax
    movl %eax, idt + 6*16+8(%rip)
    movl $0, idt + 6*16+12(%rip)

    leaq xm_handler(%rip), %rax
    movw %ax, idt + 19*16(%rip)
    movw %cx, idt + 19*16+2(%rip)
    movb $0, idt + 19*16+4(%rip)
    movb $0x8e, idt + 19*16+5(%rip)
    shrq $16, %rax
    movw %ax, idt + 19*16+6(%rip)
    shrq $16, %rax
    movl %eax, idt + 19*16+8(%rip)
    movl $0, idt + 19*16+12(%rip)

    leaq idt(%rip), %rax
    movq %rax, idtr+2(%rip)
    lidt idtr(%rip)

    /* Enable OSFXSR and OSXMMEXCPT, preserving firmware control bits. */
    movq %cr4, %rax
    orq $((1 << 9) | (1 << 10)), %rax
    movq %rax, %cr4

begin_valid:
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    /* Valid SQRTSS with invalid unmasked must not fault. */
    movl $0x1f00, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0x40800000, %eax          /* 4.0f */
    movd %eax, %xmm1
    pxor %xmm0, %xmm0
    leaq after_valid_sqrt(%rip), %rax
    movq %rax, resume_rip(%rip)
valid_sqrtss:
    sqrtss %xmm1, %xmm0
after_valid_sqrt:
    stmxcsr mxcsr_out(%rip)
    movd %xmm0, %eax
    cmpl $0x40000000, %eax          /* 2.0f */
    jne fail_valid
    cmpl $0x1f00, mxcsr_out(%rip)
    jne fail_valid
    cmpl $0, xm_count(%rip)
    jne fail_valid
    leaq msg_valid(%rip), %rsi
    call puts

begin_masked:
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    /* Masked invalid completes, sets IE, and does not deliver vector 19. */
    movl $0x1f80, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0xbf800000, %eax          /* -1.0f */
    movd %eax, %xmm1
    movl $0x41200000, %eax          /* sentinel 10.0f */
    movd %eax, %xmm0
    leaq after_masked_sqrt(%rip), %rax
    movq %rax, resume_rip(%rip)
masked_sqrtss:
    sqrtss %xmm1, %xmm0
after_masked_sqrt:
    stmxcsr mxcsr_out(%rip)
    cmpl $0x1f81, mxcsr_out(%rip)
    jne fail_masked_mxcsr
    cmpl $0, xm_count(%rip)
    jne fail_masked_vector
    movd %xmm0, %eax
    movl %eax, %ecx
    andl $0x7f800000, %ecx          /* NaN has all-ones exponent */
    cmpl $0x7f800000, %ecx
    jne fail_masked_result
    andl $0x007fffff, %eax
    testl %eax, %eax                /* and a non-zero payload */
    jz fail_masked_result
    leaq msg_masked(%rip), %rsi
    call puts

    /* A pre-existing IE flag is now unmasked. A valid SQRTSS must not trap. */
    stmxcsr mxcsr_before(%rip)
    cmpl $0x1f81, mxcsr_before(%rip)
    jne fail_sticky_seed
    movl mxcsr_before(%rip), %eax
    andl $0xffffff7f, %eax           /* clear only the invalid-operation mask */
    movl %eax, mxcsr_tmp(%rip)       /* 0x1f81 -> 0x1f01 */
    ldmxcsr mxcsr_tmp(%rip)
    stmxcsr mxcsr_before(%rip)
    cmpl $0x1f01, mxcsr_before(%rip)
    jne fail_sticky_seed
begin_sticky_valid:
    movl $0x1f01, mxcsr_tmp(%rip)  /* seed old IE with IE unmasked */
    ldmxcsr mxcsr_tmp(%rip)
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    movl $0x40800000, %eax
    movd %eax, %xmm1
    movdqa xmm_sentinel(%rip), %xmm0
    leaq op_sticky_valid_sqrtss(%rip), %rax
    movq %rax, expected_rip(%rip)
    leaq after_sticky_valid_sqrtss(%rip), %rax
    movq %rax, resume_rip(%rip)
op_sticky_valid_sqrtss:
    sqrtss %xmm1, %xmm0
after_sticky_valid_sqrtss:
    stmxcsr mxcsr_out(%rip)
    movd %xmm0, %eax
    cmpl $0x40000000, %eax
    jne fail_sticky_valid
    cmpl $0x1f01, mxcsr_out(%rip)   /* old IE sticky flag remains set */
    jne fail_sticky_valid
    cmpl $0, xm_count(%rip)         /* no spurious fault on valid operation */
    jne fail_sticky_valid
    cmpl $0, rip_checks(%rip)
    jne fail_sticky_valid
    leaq msg_sticky_valid(%rip), %rsi
    call puts

begin_sticky_invalid:
    /* IE is already sticky and unmasked: another invalid op must fault again. */
    movl $0x1f01, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    stmxcsr mxcsr_before(%rip)
    cmpl $0x1f01, mxcsr_before(%rip)
    jne fail_sticky_seed
    movdqa xmm_sentinel(%rip), %xmm0
    movl $0xbf800000, %eax
    movd %eax, %xmm1
    leaq op_sticky_invalid_sqrtss(%rip), %rax
    movq %rax, expected_rip(%rip)
    leaq after_sticky_invalid_sqrtss(%rip), %rax
    movq %rax, resume_rip(%rip)
op_sticky_invalid_sqrtss:
    sqrtss %xmm1, %xmm0
after_sticky_invalid_sqrtss:
    stmxcsr mxcsr_out(%rip)
    cmpl $1, xm_count(%rip)
    jne fail_sticky_invalid
    cmpl $1, rip_checks(%rip)
    jne fail_sticky_invalid
    cmpl $0, rip_mismatches(%rip)
    jne fail_sticky_invalid
    cmpl $0x1f01, mxcsr_out(%rip)
    jne fail_sticky_invalid
    movdqa %xmm0, xmm_actual(%rip)
    cmpl $0x3f800000, xmm_actual+0(%rip)
    jne fail_sticky_invalid
    cmpl $0x89abcdef, xmm_actual+4(%rip)
    jne fail_sticky_invalid
    cmpl $0x55aa55aa, xmm_actual+8(%rip)
    jne fail_sticky_invalid
    cmpl $0xa55aa55a, xmm_actual+12(%rip)
    jne fail_sticky_invalid
    leaq msg_sticky_invalid(%rip), %rsi
    call puts

begin_clean_invalid:
    /* Clean-status unmasked invalid SQRTSS preserves all 128 XMM0 bits. */
    movl $0x1f00, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    movdqa xmm_sentinel(%rip), %xmm0
    movl $0xbf800000, %eax
    movd %eax, %xmm1
    leaq op_clean_invalid_sqrtss(%rip), %rax
    movq %rax, expected_rip(%rip)
    leaq after_clean_invalid_sqrtss(%rip), %rax
    movq %rax, resume_rip(%rip)
op_clean_invalid_sqrtss:
    sqrtss %xmm1, %xmm0
after_clean_invalid_sqrtss:
    stmxcsr mxcsr_out(%rip)
    cmpl $1, xm_count(%rip)
    jne fail_xm
    cmpl $1, rip_checks(%rip)
    jne fail_clean_invalid_rip
    cmpl $0, rip_mismatches(%rip)
    jne fail_clean_invalid_rip
    movq last_fault_rip(%rip), %rax
    cmpq expected_rip(%rip), %rax
    jne fail_clean_invalid_rip
    cmpl $0x1f01, mxcsr_out(%rip)
    jne fail_xm
    leaq msg_rip_sqrtss(%rip), %rsi
    call puts
    movdqa %xmm0, xmm_actual(%rip)
    cmpl $0x3f800000, xmm_actual+0(%rip)
    jne fail_xm
    cmpl $0x89abcdef, xmm_actual+4(%rip)
    jne fail_xm
    cmpl $0x55aa55aa, xmm_actual+8(%rip)
    jne fail_xm
    cmpl $0xa55aa55a, xmm_actual+12(%rip)
    jne fail_xm
    leaq msg_scalar_128(%rip), %rsi
    call puts

begin_osxmmexcpt:
    /* CR4.OSXMMEXCPT=0 must select #UD and preserve the saved instruction RIP. */
    movq %cr4, %rax
    andq $~(1 << 10), %rax
    movq %rax, %cr4
    movl $0x1f00, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    movdqa xmm_sentinel(%rip), %xmm0
    movl $0xbf800000, %eax
    movd %eax, %xmm1
    leaq op_osxmmexcpt_clear(%rip), %rax
    movq %rax, expected_rip(%rip)
    leaq after_osxmmexcpt_clear(%rip), %rax
    movq %rax, resume_rip(%rip)
op_osxmmexcpt_clear:
    sqrtss %xmm1, %xmm0
after_osxmmexcpt_clear:
    stmxcsr mxcsr_out(%rip)
    cmpl $1, ud_count(%rip)
    jne fail_osxmmexcpt
    cmpl $0, xm_count(%rip)
    jne fail_osxmmexcpt
    cmpl $1, rip_checks(%rip)
    jne fail_osxmmexcpt_rip
    cmpl $0, rip_mismatches(%rip)
    jne fail_osxmmexcpt_rip
    movq last_fault_rip(%rip), %rax
    cmpq expected_rip(%rip), %rax
    jne fail_osxmmexcpt_rip
    cmpl $0x1f01, mxcsr_out(%rip)
    jne fail_osxmmexcpt
    movdqa %xmm0, xmm_actual(%rip)
    cmpl $0x3f800000, xmm_actual+0(%rip)
    jne fail_osxmmexcpt
    cmpl $0x89abcdef, xmm_actual+4(%rip)
    jne fail_osxmmexcpt
    cmpl $0x55aa55aa, xmm_actual+8(%rip)
    jne fail_osxmmexcpt
    cmpl $0xa55aa55a, xmm_actual+12(%rip)
    jne fail_osxmmexcpt
    movq %cr4, %rax
    orq $(1 << 10), %rax
    movq %rax, %cr4
    leaq msg_osxmmexcpt(%rip), %rsi
    call puts
    leaq msg_rip_ud(%rip), %rsi
    call puts

begin_divzero:
    /* Original DIVSS divide-by-zero control: only ZE is unmasked. */
    movq %cr4, %rax
    orq $(1 << 10), %rax
    movq %rax, %cr4
    movl $0x1d80, mxcsr_tmp(%rip)   /* all masked except divide-by-zero */
    ldmxcsr mxcsr_tmp(%rip)
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    movl $0x3f800000, %eax
    movd %eax, %xmm0
    xorl %eax, %eax
    movd %eax, %xmm1
    leaq op_divss_zero(%rip), %rax
    movq %rax, expected_rip(%rip)
    leaq after_divss_zero(%rip), %rax
    movq %rax, resume_rip(%rip)
op_divss_zero:
    divss %xmm1, %xmm0
after_divss_zero:
    stmxcsr mxcsr_out(%rip)
    cmpl $1, xm_count(%rip)
    jne fail_divzero
    cmpl $1, rip_checks(%rip)
    jne fail_divzero_rip
    cmpl $0, rip_mismatches(%rip)
    jne fail_divzero_rip
    movq last_fault_rip(%rip), %rax
    cmpq expected_rip(%rip), %rax
    jne fail_divzero_rip
    cmpl $0x1d84, mxcsr_out(%rip)
    jne fail_divzero
    movd %xmm0, %eax
    cmpl $0x3f800000, %eax
    jne fail_divzero
    leaq msg_divzero(%rip), %rsi
    call puts
    leaq msg_rip_divss(%rip), %rsi
    call puts

begin_sticky_div:
    /* Existing IE must survive while a new, unmasked ZE is raised.
       Keep IE unmasked, but clear the ZE mask bit (MXCSR bit 9). */
    movl $0x1d01, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    stmxcsr mxcsr_before(%rip)
    cmpl $0x1d01, mxcsr_before(%rip)
    jne fail_sticky_div
    leaq msg_sticky_div_before(%rip), %rsi
    call puts
    movdqa xmm_div_sentinel(%rip), %xmm0
    pxor %xmm1, %xmm1
    leaq op_sticky_divss_zero(%rip), %rax
    movq %rax, expected_rip(%rip)
    leaq after_sticky_divss_zero(%rip), %rax
    movq %rax, resume_rip(%rip)
op_sticky_divss_zero:
    divss %xmm1, %xmm0
after_sticky_divss_zero:
    stmxcsr mxcsr_out(%rip)
    cmpl $1, xm_count(%rip)
    jne fail_sticky_div_vector
    cmpl $1, rip_checks(%rip)
    jne fail_sticky_div_rip
    cmpl $0, rip_mismatches(%rip)
    jne fail_sticky_div_rip
    movq last_fault_rip(%rip), %rax
    cmpq expected_rip(%rip), %rax
    jne fail_sticky_div_rip
    leaq msg_sticky_div_vector(%rip), %rsi
    call puts
    cmpl $0x1d05, mxcsr_out(%rip)   /* old IE plus newly raised ZE */
    jne fail_sticky_div_flags
    movdqa %xmm0, xmm_actual(%rip)
    cmpl $0x3f800000, xmm_actual+0(%rip)
    jne fail_sticky_div_dest
    cmpl $0x76543210, xmm_actual+4(%rip)
    jne fail_sticky_div_dest
    cmpl $0xaabbccdd, xmm_actual+8(%rip)
    jne fail_sticky_div_dest
    cmpl $0x55aa55aa, xmm_actual+12(%rip)
    jne fail_sticky_div_dest
    leaq msg_sticky_div_dest(%rip), %rsi
    call puts
    leaq msg_sticky_div(%rip), %rsi
    call puts

begin_packed:
    /* Packed sqrt: lane 0 is valid before exceptional lane 1; no lane commits. */
    movl $0x1f00, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0, xm_count(%rip)
    movl $0, ud_count(%rip)
    movl $0, rip_checks(%rip)
    movl $0, rip_mismatches(%rip)
    movdqa xmm_packed_sentinel(%rip), %xmm0
    movdqa xmm_packed_input(%rip), %xmm1 /* [4.0, -1.0, 9.0, 16.0] */
    leaq op_packed_sqrtps(%rip), %rax
    movq %rax, expected_rip(%rip)
    leaq after_packed_sqrtps(%rip), %rax
    movq %rax, resume_rip(%rip)
op_packed_sqrtps:
    sqrtps %xmm1, %xmm0
after_packed_sqrtps:
    stmxcsr mxcsr_out(%rip)
    cmpl $1, xm_count(%rip)
    jne fail_packed
    cmpl $1, rip_checks(%rip)
    jne fail_packed_rip
    cmpl $0, rip_mismatches(%rip)
    jne fail_packed_rip
    movq last_fault_rip(%rip), %rax
    cmpq expected_rip(%rip), %rax
    jne fail_packed_rip
    cmpl $0x1f01, mxcsr_out(%rip)
    jne fail_packed
    movdqa %xmm0, xmm_actual(%rip)
    cmpl $0x3f800000, xmm_actual+0(%rip)
    jne fail_packed
    cmpl $0x89abcdef, xmm_actual+4(%rip)
    jne fail_packed
    cmpl $0x55aa55aa, xmm_actual+8(%rip)
    jne fail_packed
    cmpl $0xa55aa55a, xmm_actual+12(%rip)
    jne fail_packed
    leaq msg_packed_128(%rip), %rsi
    call puts

finish_controls:
    cmpl $5, xm_total(%rip)
    jne fail_totals
    cmpl $1, ud_total(%rip)
    jne fail_totals
    cmpl $6, rip_total(%rip)
    jne fail_totals
    cmpl $0, rip_mismatch_total(%rip)
    jne fail_totals
    leaq msg_all_rip(%rip), %rsi
    call puts
    cmpl $0, failure_count(%rip)
    jne report_failures
    leaq msg_done(%rip), %rsi
    call puts
    movl $0x10, %eax
    outl %eax, $0xf4
1:  hlt
    jmp 1b

report_failures:
    leaq msg_done_fail(%rip), %rsi
    call puts
    movl $0x11, %eax
    outl %eax, $0xf4
2:  hlt
    jmp 2b

fail_valid:
    leaq msg_fail_valid(%rip), %rsi
    call record_failure
    jmp begin_masked
fail_masked_mxcsr:
    leaq msg_fail_masked_mxcsr(%rip), %rsi
    call record_failure
    jmp begin_sticky_valid
fail_masked_vector:
    leaq msg_fail_masked_vector(%rip), %rsi
    call record_failure
    jmp begin_sticky_valid
fail_masked_result:
    leaq msg_fail_masked_result(%rip), %rsi
    call record_failure
    jmp begin_sticky_valid
fail_sticky_seed:
    leaq msg_fail_sticky_seed(%rip), %rsi
    call record_failure
    jmp begin_sticky_valid
fail_sticky_valid:
    leaq msg_fail_sticky_valid(%rip), %rsi
    call record_failure
    jmp begin_sticky_invalid
fail_sticky_invalid_seed:
    leaq msg_fail_sticky_seed(%rip), %rsi
    call record_failure
    jmp begin_clean_invalid
fail_sticky_invalid:
    leaq msg_fail_sticky_invalid(%rip), %rsi
    call record_failure
    jmp begin_clean_invalid
fail_xm:
    leaq msg_fail_xm(%rip), %rsi
    call record_failure
    jmp begin_osxmmexcpt
fail_clean_invalid_rip:
    leaq msg_fail_rip(%rip), %rsi
    call record_failure
    jmp begin_osxmmexcpt
fail_osxmmexcpt:
    leaq msg_fail_osxmmexcpt(%rip), %rsi
    call record_failure
    jmp begin_divzero
fail_osxmmexcpt_rip:
    leaq msg_fail_rip(%rip), %rsi
    call record_failure
    jmp begin_divzero
fail_divzero:
    leaq msg_fail_divzero(%rip), %rsi
    call record_failure
    jmp begin_sticky_div
fail_divzero_rip:
    leaq msg_fail_rip(%rip), %rsi
    call record_failure
    jmp begin_sticky_div
fail_sticky_div:
    leaq msg_fail_sticky_div(%rip), %rsi
    call record_failure
    jmp begin_packed
fail_sticky_div_vector:
    leaq msg_fail_sticky_div_vector(%rip), %rsi
    call record_failure
    jmp begin_packed
fail_sticky_div_rip:
    leaq msg_fail_rip(%rip), %rsi
    call record_failure
    jmp begin_packed
fail_sticky_div_flags:
    leaq msg_fail_sticky_div_flags(%rip), %rsi
    call record_failure
    jmp begin_packed
fail_sticky_div_dest:
    leaq msg_fail_sticky_div_dest(%rip), %rsi
    call record_failure
    jmp begin_packed
fail_packed:
    leaq msg_fail_packed(%rip), %rsi
    call record_failure
    jmp finish_controls
fail_packed_rip:
    leaq msg_fail_rip(%rip), %rsi
    call record_failure
    jmp finish_controls
fail_totals:
    leaq msg_fail_totals(%rip), %rsi
    call record_failure
    jmp report_failures
record_failure:
    call puts
    incl failure_count(%rip)
    ret

ud_handler:
    /* Same-CPL exception frame begins with saved RIP at (%rsp).
       Inspect and compare it before replacing the frame's RIP. */
    movq (%rsp), %rax
    movq %rax, last_fault_rip(%rip)
    incl rip_checks(%rip)
    incl rip_total(%rip)
    cmpq expected_rip(%rip), %rax
    je 6f
    incl rip_mismatches(%rip)
    incl rip_mismatch_total(%rip)
6:
    incl ud_count(%rip)
    incl ud_total(%rip)
    movq resume_rip(%rip), %rax
    movq %rax, (%rsp)
    iretq

xm_handler:
    /* Same-CPL exception frame begins with saved RIP at (%rsp).
       Inspect and compare it before replacing the frame's RIP. */
    movq (%rsp), %rax
    movq %rax, last_fault_rip(%rip)
    incl rip_checks(%rip)
    incl rip_total(%rip)
    cmpq expected_rip(%rip), %rax
    je 7f
    incl rip_mismatches(%rip)
    incl rip_mismatch_total(%rip)
7:
    incl xm_count(%rip)
    incl xm_total(%rip)
    movq resume_rip(%rip), %rax
    movq %rax, (%rsp)
    iretq

puts:
    pushq %rsi
3:  lodsb
    testb %al, %al
    jz 4f
    call putc
    jmp 3b
4:  popq %rsi
    ret

putc:
    pushq %rax
    pushq %rdx
5:  movw $0x3fd, %dx
    inb %dx, %al
    testb $0x20, %al
    jz 5b
    movw $0x3f8, %dx
    movb 8(%rsp), %al
    outb %al, %dx
    popq %rdx
    popq %rax
    ret

.section .rdata,"a"
msg_valid:             .asciz "PASS: valid SQRTSS, invalid unmasked, no fault\r\n"
msg_masked:            .asciz "PASS: masked invalid SQRTSS sets IE without vector 19\r\n"
msg_sticky_valid:      .asciz "PASS: old IE unmasked; valid SQRTSS did not fault and kept IE\r\n"
msg_sticky_invalid:    .asciz "PASS: invalid SQRTSS faulted again with IE already sticky; XMM0[127:0] preserved\r\n"
msg_rip_sqrtss:        .asciz "PASS: saved RIP matched the exact unmasked SQRTSS opcode\r\n"
msg_scalar_128:        .asciz "PASS: unmasked scalar SQRTSS preserved all 128 XMM0 bits\r\n"
msg_osxmmexcpt:        .asciz "PASS: CR4.OSXMMEXCPT=0 selected vector 6\r\n"
msg_rip_ud:            .asciz "PASS: saved RIP matched the exact OSXMMEXCPT-disabled opcode\r\n"
msg_divzero:           .asciz "PASS: unmasked DIVSS divide-by-zero delivered vector 19, ZE=1\r\n"
msg_rip_divss:         .asciz "PASS: saved RIP matched the exact DIVSS opcode\r\n"
msg_sticky_div_before: .asciz "PASS: sticky IE=1 was present before the new ZE operation\r\n"
msg_sticky_div_vector: .asciz "PASS: a new unmasked ZE fault delivered vector 19 with old IE already set\r\n"
msg_sticky_div_dest:   .asciz "PASS: DIVSS exception preserved all 128 destination bits\r\n"
msg_sticky_div:        .asciz "PASS: sticky IE preserved with new ZE; full XMM0 destination preserved\r\n"
msg_packed_128:        .asciz "PASS: packed SQRTPS exceptional lane faulted; all 128 XMM0 bits preserved\r\n"
msg_all_rip:           .asciz "PASS: actual saved RIP matched all six faulting opcode addresses\r\n"
msg_done:              .asciz "RESULT PASS: expanded independent TCG #XM controls\r\n"
msg_done_fail:         .asciz "RESULT FAIL: expanded independent TCG #XM controls; one or more controls failed\r\n"
msg_fail_valid:        .asciz "FAIL: valid SQRTSS control\r\n"
msg_fail_masked_mxcsr: .asciz "FAIL: masked invalid MXCSR status\r\n"
msg_fail_masked_vector:.asciz "FAIL: masked invalid unexpectedly trapped\r\n"
msg_fail_masked_result:.asciz "FAIL: masked invalid destination\r\n"
msg_fail_sticky_seed:  .asciz "FAIL: sticky IE seed/unmask sequence\r\n"
msg_fail_sticky_valid: .asciz "FAIL: old sticky IE spuriously faulted on valid SQRTSS\r\n"
msg_fail_sticky_invalid:.asciz "FAIL: newly raised IE did not fault with IE already sticky\r\n"
msg_fail_xm:           .asciz "FAIL: unmasked invalid #XM or XMM0 preservation control\r\n"
msg_fail_rip:          .asciz "FAIL: saved exception RIP did not match its faulting opcode\r\n"
msg_fail_osxmmexcpt:   .asciz "FAIL: OSXMMEXCPT-disabled control\r\n"
msg_fail_divzero:      .asciz "FAIL: unmasked divide-by-zero control\r\n"
msg_fail_sticky_div:   .asciz "FAIL: sticky IE preservation/new ZE/destination control\r\n"
msg_fail_sticky_div_vector: .asciz "FAIL: sticky IE plus new ZE did not deliver vector 19\r\n"
msg_fail_sticky_div_flags:  .asciz "FAIL: sticky IE/new ZE MXCSR flag merge\r\n"
msg_fail_sticky_div_dest:   .asciz "FAIL: sticky DIVSS did not preserve all 128 destination bits\r\n"
msg_fail_packed:       .asciz "FAIL: packed SQRTPS precise exception/destination control\r\n"
msg_fail_totals:       .asciz "FAIL: expected total vector or precise-RIP receipt count\r\n"

.section .data,"aw"
.align 8
idtr:           .word 32*16 - 1
                .quad 0
mxcsr_tmp:      .long 0
mxcsr_out:      .long 0
mxcsr_before:   .long 0
resume_rip:     .quad 0
expected_rip:   .quad 0
last_fault_rip: .quad 0
ud_count:       .long 0
xm_count:       .long 0
rip_checks:     .long 0
rip_mismatches: .long 0
xm_total:       .long 0
ud_total:       .long 0
rip_total:      .long 0
rip_mismatch_total: .long 0
failure_count:  .long 0

.section .rdata,"a"
.align 16
xmm_sentinel:
    .long 0x3f800000, 0x89abcdef, 0x55aa55aa, 0xa55aa55a
xmm_div_sentinel:
    .long 0x3f800000, 0x76543210, 0xaabbccdd, 0x55aa55aa
xmm_packed_sentinel:
    .long 0x3f800000, 0x89abcdef, 0x55aa55aa, 0xa55aa55a
xmm_packed_input:
    .long 0x40800000, 0xbf800000, 0x41100000, 0x41800000

.section .bss,"bw"
.align 16
xmm_actual: .zero 16
.align 16
idt: .zero 32*16
