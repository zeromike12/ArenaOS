/* ArenaOS minimal shell — fixed layout, same strict-subset contract as
 * the test payload (ADR-0016), own address window (ADR-0020):
 *
 *   0x400000  _start, the shell program    (PT_LOAD #1: R+X)
 *   0x410000  .data / .bss                 (PT_LOAD #2: R+W; .bss is
 *                                           NOLOAD — the loader must
 *                                           zero-fill it)
 *
 * The stack page is NOT laid out here: the spawn protocol derives it
 * from the image's top segment VA (first page above — 0x411000 for the
 * current data+bss extent). Keep the image at two PT_LOAD segments.
 */
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(0x5);   /* R+X — never W+X (ADR-0008/0016) */
    data PT_LOAD FLAGS(0x6);   /* R+W */
}

SECTIONS
{
    . = 0x400000;

    .text : ALIGN(0x1000) {
        *(.text .text.*)
        *(.rodata .rodata.*)
    } :text

    . = 0x410000;
    /* .got is explicit, not orphaned: the shell's slice indexing pulls
     * in core's panic-fmt machinery, whose indirect calls resolve
     * through GOT slots. lld would otherwise place .got AFTER the
     * NOLOAD .bss (the exact orphan trap M4.3 caught in the payload).
     * ET_EXEC + relocation-model=static: the entries are absolute and
     * filled at link time — the loader just copies them, no runtime
     * relocation processing exists or is needed. */
    .data : ALIGN(0x1000) {
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss (NOLOAD) : {
        *(.bss .bss.*)
    } :data

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note .note.*)
    }
}
