/* ArenaOS userspace test payload — fixed, self-describing layout
 * (ADR-0016).
 *
 *   0x200000  _start, the real program     (PT_LOAD #1: R+X)
 *   0x200800  MSG_BUF, pinned message      (inside PT_LOAD #1: R+X)
 *   0x201000  META manifest                (PT_LOAD #2: R+W, file-backed)
 *   0x202000  4 KiB .bss, NOLOAD           (inside PT_LOAD #2: memsz only —
 *                                           the loader must zero-fill it)
 *
 * The kernel's loader/process tests assert these addresses; change them
 * only together with kernel/kernel/src/m4.rs and the payload's META.
 */
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(0x5);   /* R+X — never W+X (ADR-0008/0016) */
    data PT_LOAD FLAGS(0x6);   /* R+W */
}

SECTIONS
{
    . = 0x200000;

    .text : ALIGN(0x1000) {
        *(.text .text.*)
        *(.rodata .rodata.*)
    } :text

    /* The message the program writes through SYS_DEBUG_WRITE, pinned so
     * META can describe it as a plain (va, len) fact and the kernel can
     * locate its bytes in the image file. */
    ASSERT(. <= 0x200800, "text overflowed into the pinned message at 0x200800")
    . = 0x200800;
    .msg : {
        *(.payload_msg)
    } :text

    . = 0x201000;
    .meta : {
        *(.payload_meta)
    } :data

    . = 0x202000;
    .bss (NOLOAD) : ALIGN(0x1000) {
        *(.payload_bss)
        *(.bss .bss.*)
    } :data

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note .note.*)
    }
}
