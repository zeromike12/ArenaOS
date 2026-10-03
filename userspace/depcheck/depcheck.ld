/* ADR-0037: the manager is an ordinary no_std static ring-3 process.
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
