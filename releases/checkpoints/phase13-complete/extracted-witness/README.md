# Extracted archive witness

These files were captured from a fresh extraction of
`arenaos-phase13-complete-qemu-x86_64.tar.gz`, SHA-256
`7f9fca01dcd3b3ae7e133531cf345300506a3b88248ea676daf19e9bf1ce5d2a`.
The final archive was assembled after the report and progress docs were
complete. The independent boot ran
with `PYTHONPATH` unset and read the EFI, ESP, OVMF, scratch image, APB1
fixtures, and Python witness modules only from the extracted directory. QEMU
10.0.11 was the only external runtime tool.

`archive-boot-output.txt` is the semantic pass receipt. `serial.log.gz` and
`console.log.gz` contain the full guest transcripts; `screen.ppm` and
`pixels.txt` bind the observed desktop pixels; `tcp.log.gz` and `dns.log.gz`
record the bundled historical stream fixtures. The logs are gzip-compressed
byte-for-byte copies of the captured output.
