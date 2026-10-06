/* ADR-0083 independent ring-3 guest. Fixed ET_EXEC base keeps the startup
 * record's entry/load-base facts independently checkable. W^X and NX remain
 * explicit; the initial process stack stays the kernel's derived 4 KiB page. */
ENTRY(_start)
PHDRS
{
    text PT_LOAD FLAGS(0x5);   /* R+X */
    data PT_LOAD FLAGS(0x6);   /* R+W */
}
SECTIONS
{
    . = 0x00200000;
    .text : ALIGN(0x1000) {
        KEEP(*(.text._start))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } :text

    . = 0x00210000;
    .data : ALIGN(0x1000) {
        *(.data .data.*)
        *(.got .got.*)
    } :data
    .bss (NOLOAD) : ALIGN(0x1000) {
        *(.bss .bss.*)
    } :data

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note .note.*)
    }
}
