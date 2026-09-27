/* ArenaOS network service (netd + nettest) — fixed layout, same
 * strict-subset contract as every other userspace image (ADR-0016),
 * own address window:
 *
 *   0x400000  _start, the program            (PT_LOAD #1: R+X)
 *   0x410000  .data / .bss                   (PT_LOAD #2: R+W; .bss is
 *                                             NOLOAD — the loader must
 *                                             zero-fill it)
 *
 * The stack page is NOT laid out here: the spawn protocol derives it
 * from the image's top segment VA (first page above). Keep the image
 * at two PT_LOAD segments. Both binaries of this crate share the
 * layout — each is loaded into its OWN address space, never both into
 * one, so the identical VAs are safe.
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
    /* .got is explicit, not orphaned (the exact orphan trap M4.3
     * caught): ET_EXEC + relocation-model=static means the entries are
     * absolute and filled at link time — the loader just copies them. */
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
