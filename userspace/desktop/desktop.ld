/* Phase-9 standalone static ring-3 image; no host or kernel code linked. */
ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(0x5); data PT_LOAD FLAGS(0x6); stack PT_LOAD FLAGS(0x6); }
SECTIONS {
    . = 0x200000;
    .text : ALIGN(0x1000) { *(.text .text.*) *(.rodata .rodata.*) } :text
    . = ALIGN(0x1000);
    .data : { *(.data .data.*) *(.got .got.*) } :data
    .bss (NOLOAD) : ALIGN(0x1000) { *(.bss .bss.*) } :data
    /* One unmapped guard page, then the dedicated stack (entry! macro). */
    . = ALIGN(0x1000) + 0x1000;
    .stack (NOLOAD) : ALIGN(0x1000) { *(.stack) } :stack
    /DISCARD/ : { *(.comment) *(.eh_frame) *(.eh_frame_hdr) *(.note .note.*) }
}
