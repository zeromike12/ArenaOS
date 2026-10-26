/* ArenaOS C prototype guest link script (EXPERIMENTAL).
 *
 * Static, non-PIE, fixed base 0x200000 (the window the repository's other
 * static guests use; Luna's PIE/ASLR work owns any change to placement).
 * Only PT_LOAD is consumed by the kernel loader (elf.rs); PT_TLS is emitted
 * for the runtime's benefit and is inert to the loader.
 *
 * W^X: text (R+X) and data (R+W) are separate PT_LOADs. Nothing is W+X.
 */
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5);   /* R+X */
    data PT_LOAD FLAGS(6);   /* R+W */
    tls  PT_TLS;
}

SECTIONS
{
    . = 0x00200000;

    .text : ALIGN(0x1000) {
        KEEP(*(.text._start))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } :text

    .init_array : ALIGN(8) {
        PROVIDE_HIDDEN(__init_array_start = .);
        KEEP(*(SORT_BY_INIT_PRIORITY(.init_array.*) SORT_BY_INIT_PRIORITY(.ctors.*)))
        KEEP(*(.init_array))
        PROVIDE_HIDDEN(__init_array_end = .);
    } :text

    . = ALIGN(0x1000);

    /* KEEP: --gc-sections (LLD, zig cc) would otherwise drop the unreferenced
     * layout anchor in crt0.c and change the TLS segment per toolchain. */
    .tdata : ALIGN(16) {
        KEEP(*(.tdata .tdata.*))
    } :data :tls

    .tbss (NOLOAD) : ALIGN(16) {
        KEEP(*(.tbss .tbss.*))
    } :data :tls

    .data : ALIGN(16) {
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss (NOLOAD) : ALIGN(16) {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /* TLS layout facts read by crt0.c (values, not addresses). */
    PROVIDE(__arena_tls_image  = ADDR(.tdata));
    PROVIDE(__arena_tls_filesz = SIZEOF(.tdata));
    /* memsz must span .tdata..end-of-.tbss INCLUDING the alignment padding
     * between them (PT_TLS p_memsz). SIZEOF(.tdata)+SIZEOF(.tbss) omits that
     * padding and under-reports memsz; the guest run caught the resulting
     * 16-byte TP placement error (experiments/c-runtime/build notes). */
    PROVIDE(__arena_tls_memsz  = ADDR(.tbss) + SIZEOF(.tbss) - ADDR(.tdata));
    PROVIDE(__arena_tls_align  = MAX(ALIGNOF(.tdata), ALIGNOF(.tbss)));

    /DISCARD/ : {
        *(.comment)
        *(.note .note.*)
        *(.eh_frame)
        *(.eh_frame_hdr)
    }
}
