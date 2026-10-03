/* ADR-0046: configd and its reader are static no_std ring-3 processes.
 * Its image has only RX text and RW data/bss; the kernel derives its
 * stack page from the top loaded segment (ADR-0016/0019).
 */
ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(0x5); data PT_LOAD FLAGS(0x6); }
SECTIONS
{
    . = 0x400000;
    .text : ALIGN(0x1000) { *(.text .text.*) *(.rodata .rodata.*) } :text
    . = 0x410000;
    .data : ALIGN(0x1000) { *(.data .data.*) *(.got .got.*) } :data
    .bss (NOLOAD) : { *(.bss .bss.*) } :data
    /DISCARD/ : { *(.comment) *(.eh_frame) *(.eh_frame_hdr) *(.note .note.*) }
}
