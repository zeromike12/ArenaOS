/* One RX file-backed PT_LOAD starts at offset zero, covering ELF+PHDR
 * headers. No writable segment: the one-page spawn stack holds all IPC
 * buffers. This is a purpose-built static ET_EXEC, never a packed/truncated
 * ELF or a change to APKG v1's 4096-byte signed payload ceiling. */
ENTRY(_start)
PHDRS { text PT_LOAD FILEHDR PHDRS FLAGS(0x5); }
SECTIONS {
    . = 0x400000 + SIZEOF_HEADERS;
    .text : { *(.text._start) *(.text .text.*) *(.rodata .rodata.*) } :text
    /DISCARD/ : { *(.comment) *(.eh_frame) *(.eh_frame_hdr) *(.note.*) }
}
