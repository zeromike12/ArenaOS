OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

PHDRS
{
    text PT_LOAD FILEHDR PHDRS FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
    dynamic PT_DYNAMIC FLAGS(6);
    stack PT_GNU_STACK FLAGS(6);
}

SECTIONS
{
    . = SIZEOF_HEADERS;
    .text : ALIGN(16)
    {
        KEEP(*(.text._start))
        *(.text .text.*)
    } :text

    . = ALIGN(0x1000);
    .rodata :
    {
        *(.rodata .rodata.*)
    } :rodata

    .rela.dyn : { *(.rela.dyn) } :rodata

    . = ALIGN(0x1000);
    .data :
    {
        *(.data.rel.ro .data.rel.ro.*)
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .dynamic : { *(.dynamic) } :data :dynamic

    .bss (NOLOAD) :
    {
        *(.bss .bss.*)
        *(COMMON)
        *(.stack)
    } :data

    /DISCARD/ :
    {
        *(.comment)
        *(.eh_frame .eh_frame.*)
        *(.note .note.*)
    }
}
