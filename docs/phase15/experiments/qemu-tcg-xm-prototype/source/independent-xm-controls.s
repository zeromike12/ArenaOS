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

    /* A finite positive square root under an unmasked invalid mask is exact. */
    movl $0x1f00, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0x40800000, %eax          /* 4.0f */
    movd %eax, %xmm1
    pxor %xmm0, %xmm0
    leaq after_valid_sqrt(%rip), %rax
    movq %rax, resume_rip(%rip)
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

    /* Masked invalid completes normally and sets MXCSR.IE, without #XM. */
    movl $0x1f80, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0xbf800000, %eax          /* -1.0f */
    movd %eax, %xmm1
    movl $0x41200000, %eax          /* sentinel 10.0f */
    movd %eax, %xmm0
    leaq after_masked_sqrt(%rip), %rax
    movq %rax, resume_rip(%rip)
    sqrtss %xmm1, %xmm0
 after_masked_sqrt:
    stmxcsr mxcsr_out(%rip)
    cmpl $0x1f81, mxcsr_out(%rip)
    jne fail_masked_mxcsr
    cmpl $0, xm_count(%rip)
    jne fail_masked_vector
    movd %xmm0, %eax
    movl %eax, %ecx
    andl $0x7f800000, %ecx
    cmpl $0x7f800000, %ecx         /* NaN has an all-ones exponent */
    jne fail_masked_result
    andl $0x007fffff, %eax
    testl %eax, %eax                /* and a non-zero payload */
    jz fail_masked_result
    leaq msg_masked(%rip), %rsi
    call puts

    /* Unmasked invalid delivers #XM, sets IE, and suppresses destination write. */
    movl $0x1f00, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0xbf800000, %eax
    movd %eax, %xmm1
    movl $0x41200000, %eax
    movd %eax, %xmm0
    leaq after_unmasked_sqrt(%rip), %rax
    movq %rax, resume_rip(%rip)
    sqrtss %xmm1, %xmm0
 after_unmasked_sqrt:
    stmxcsr mxcsr_out(%rip)
    cmpl $1, xm_count(%rip)
    jne fail_xm
    cmpl $0x1f01, mxcsr_out(%rip)
    jne fail_xm
    movd %xmm0, %eax
    cmpl $0x41200000, %eax
    jne fail_xm
    leaq msg_xm(%rip), %rsi
    call puts

    /* CR4.OSXMMEXCPT=0 must select #UD for this unmasked exception. */
    movq %cr4, %rax
    andq $~(1 << 10), %rax
    movq %rax, %cr4
    movl $0x1f00, mxcsr_tmp(%rip)
    ldmxcsr mxcsr_tmp(%rip)
    movl $0xbf800000, %eax
    movd %eax, %xmm1
    movl $0x41200000, %eax
    movd %eax, %xmm0
    leaq after_osxmmexcpt_clear(%rip), %rax
    movq %rax, resume_rip(%rip)
    sqrtss %xmm1, %xmm0
 after_osxmmexcpt_clear:
    stmxcsr mxcsr_out(%rip)
    cmpl $1, ud_count(%rip)
    jne fail_osxmmexcpt
    cmpl $1, xm_count(%rip)
    jne fail_osxmmexcpt
    cmpl $0x1f01, mxcsr_out(%rip)
    jne fail_osxmmexcpt
    movd %xmm0, %eax
    cmpl $0x41200000, %eax
    jne fail_osxmmexcpt
    movq %cr4, %rax
    orq $(1 << 10), %rax
    movq %rax, %cr4
    leaq msg_osxmmexcpt(%rip), %rsi
    call puts

    /* Distinct arithmetic exception: unmasked 1.0/0.0 raises #XM with ZE. */
    movl $0x1d80, mxcsr_tmp(%rip)    /* all masked except divide-by-zero */
    ldmxcsr mxcsr_tmp(%rip)
    movl $0x3f800000, %eax
    movd %eax, %xmm0
    xorl %eax, %eax
    movd %eax, %xmm1
    leaq after_divzero(%rip), %rax
    movq %rax, resume_rip(%rip)
    divss %xmm1, %xmm0
 after_divzero:
    stmxcsr mxcsr_out(%rip)
    cmpl $2, xm_count(%rip)
    jne fail_divzero
    cmpl $0x1d84, mxcsr_out(%rip)
    jne fail_divzero
    movd %xmm0, %eax
    cmpl $0x3f800000, %eax
    jne fail_divzero
    leaq msg_divzero(%rip), %rsi
    call puts

    leaq msg_done(%rip), %rsi
    call puts
    movl $0x10, %eax
    outl %eax, $0xf4
1:  hlt
    jmp 1b

fail_valid:          leaq msg_fail_valid(%rip), %rsi;       jmp fail
fail_masked_mxcsr:   leaq msg_fail_masked_mxcsr(%rip), %rsi; jmp fail
fail_masked_vector:  leaq msg_fail_masked_vector(%rip), %rsi; jmp fail
fail_masked_result:  leaq msg_fail_masked_result(%rip), %rsi; jmp fail
fail_xm:             leaq msg_fail_xm(%rip), %rsi;          jmp fail
fail_osxmmexcpt:     leaq msg_fail_osxmmexcpt(%rip), %rsi;  jmp fail
fail_divzero:        leaq msg_fail_divzero(%rip), %rsi;     jmp fail
fail:
    call puts
    movl $0x11, %eax
    outl %eax, $0xf4
2:  hlt
    jmp 2b

ud_handler:
    incl ud_count(%rip)
    movq resume_rip(%rip), %rax
    movq %rax, (%rsp)
    iretq

xm_handler:
    incl xm_count(%rip)
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
msg_valid:           .asciz "PASS: valid SQRTSS, unmasked invalid mask\r\n"
msg_masked:          .asciz "PASS: masked invalid SQRTSS sets IE without vector 19\r\n"
msg_xm:              .asciz "PASS: unmasked invalid SQRTSS delivered vector 19, IE=1, dest preserved\r\n"
msg_osxmmexcpt:      .asciz "PASS: CR4.OSXMMEXCPT=0 selected vector 6\r\n"
msg_divzero:         .asciz "PASS: unmasked DIVSS divide-by-zero delivered vector 19, ZE=1\r\n"
msg_done:            .asciz "RESULT PASS: independent TCG #XM controls\r\n"
msg_fail_valid:      .asciz "FAIL: valid SQRTSS control\r\n"
msg_fail_masked_mxcsr:  .asciz "FAIL: masked invalid MXCSR status\r\n"
msg_fail_masked_vector: .asciz "FAIL: masked invalid unexpectedly trapped\r\n"
msg_fail_masked_result: .asciz "FAIL: masked invalid destination\r\n"
msg_fail_xm:         .asciz "FAIL: unmasked invalid #XM control\r\n"
msg_fail_osxmmexcpt: .asciz "FAIL: OSXMMEXCPT-disabled control\r\n"
msg_fail_divzero:    .asciz "FAIL: unmasked divide-by-zero control\r\n"

.section .data,"aw"
.align 8
idtr:       .word 32*16 - 1
            .quad 0
mxcsr_tmp:  .long 0
mxcsr_out:  .long 0
resume_rip: .quad 0
ud_count:   .long 0
xm_count:   .long 0

.section .bss,"aw",@nobits
.align 16
idt: .zero 32*16
