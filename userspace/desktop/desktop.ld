/* Phase-9 standalone static ring-3 image; no host or kernel code linked. */
ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(0x5); data PT_LOAD FLAGS(0x6); }
SECTIONS {
    . = 0x200000;
    .text : ALIGN(0x1000) { *(.text .text.*) *(.rodata .rodata.*) } :text
    . = ALIGN(0x1000);
    .data : { *(.data .data.*) } :data
    .bss (NOLOAD) : ALIGN(0x1000) { *(.bss .bss.*) } :data
    /DISCARD/ : { *(.comment) *(.eh_frame) *(.eh_frame_hdr) *(.note .note.*) }
}
