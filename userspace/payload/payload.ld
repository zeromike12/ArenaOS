/* ArenaOS M4 test payload — fixed, self-describing layout (ADR-0016).
 *
 *   0x200000  _start entry stub            (PT_LOAD #1: R+X)
 *   0x201000  META manifest                (PT_LOAD #2: R+W, file-backed)
 *   0x202000  4 KiB .bss, NOLOAD           (inside PT_LOAD #2: memsz only —
 *                                           the loader must zero-fill it)
 *
 * The kernel's loader tests assert these addresses; change them only
 * together with kernel/kernel/src/m4.rs.
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
