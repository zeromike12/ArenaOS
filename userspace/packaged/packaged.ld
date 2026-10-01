/* ADR-0054: the verifier's bounded AINS/AACT scanner extends static RX
 * text beyond configd's 64 KiB gap. Keep the same two W^X PT_LOADs and
 * derived guest stack, but give packaged its OWN static layout. The gap
 * is unmapped; this does not increase AFS1, cap, or dynamic ELF bounds.
 */
ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(0x5); data PT_LOAD FLAGS(0x6); }
SECTIONS
{
    . = 0x400000;
    .text : ALIGN(0x1000) { *(.text .text.*) *(.rodata .rodata.*) } :text
    . = 0x430000;
    .data : ALIGN(0x1000) { *(.data .data.*) *(.got .got.*) } :data
    .bss (NOLOAD) : { *(.bss .bss.*) } :data
    /DISCARD/ : { *(.comment) *(.eh_frame) *(.eh_frame_hdr) *(.note .note.*) }
}
