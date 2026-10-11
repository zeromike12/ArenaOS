/* Native ET_EXEC fixture. RX text and RW data remain W^X-separated. */
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(0x5);
    data PT_LOAD FLAGS(0x6);
}

SECTIONS
{
    . = 0x300000;
    .text : ALIGN(0x1000) {
        *(.text .text.*)
        *(.rodata .rodata.*)
    } :text

    . = 0x301000;
    .data : ALIGN(0x1000) {
        *(.data .data.*)
        KEEP(*(.phase15_data))
    } :data

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note .note.*)
    }
}
