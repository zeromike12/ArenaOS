# EFI/ESP Evidence Field Erratum

## Affected artifact

- `docs/v1/evidence-main-current-package/evidence.json` (committed)
- `docs/v1/evidence-main-current-package.tar.gz` (sealed archive, sha256 sidecar)

## Error

The `efi` field in `evidence.json` was incorrectly populated with the SHA-256 of the entire ESP disk image rather than the SHA-256 of the `BOOTX64.EFI` file contained within it. Both `esp` and `efi` were set to:

```
d5aa4cc4dc6f468687daa2212db3ad78c4cc0b1941a8b3256f640e112ee1160a
```

This is the ESP image hash, not the EFI binary hash.

## Correct values (verified)

| Field | Meaning | Correct SHA-256 | Verification method |
|-------|---------|-----------------|---------------------|
| `esp` | ESP disk image (`build/arena-esp.img`) | `d5aa4cc4dc6f468687daa2212db3ad78c4cc0b1941a8b3256f640e112ee1160a` | `sha256sum build/arena-esp.img` |
| `efi` | EFI bootloader (`/EFI/BOOT/BOOTX64.EFI` extracted from ESP) | `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6` | `sha256sum BOOTX64.EFI` (11038720 bytes, extracted from ESP cluster 4 via direct sequential FAT read) |

## How verified

1. BOOTX64.EFI was extracted from the ESP disk image using a Python FAT parser (loop device mount unavailable in sandbox; mtools not installed).
2. Direct sequential read from cluster 4 using directory-entry file size (recursive cluster walk loops on self-referencing `.` entries was avoided).
3. SHA-256 computed independently on both the ESP image and the extracted BOOTX64.EFI binary.
4. The two hashes differ, confirming they are distinct identities:
   - ESP: 16777216 bytes (16 MiB FAT12 filesystem)
   - BOOTX64.EFI: 11038720 bytes (PE32+ UEFI executable)

## Disposition

- **Original sealed archive and receipt are preserved unchanged.** No silent rewrite, no resealing of the old archive.
- This erratum document is the explicit correction. The original `evidence.json` remains as the historical record with the erroneous `efi` value.
- **New evidence runs** (including the AFS2-enabled baseline at `v1-boot/evidence-afs2/`) record `efi` and `esp` as separate, correctly computed identities.
- Backend-control raw outputs are included in the next evidence package (see `phase15-pkg/controls/` for patched/unpatched QEMU control binaries and exit codes: patched exit 33 PASS, unpatched exit 35 expected fail, verifier exit 0).

## Backend identities (for new runs)

| Identity | Value |
|----------|-------|
| QEMU binary | `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff` (QEMU 11.1.2, TCG only) |
| QEMU wrapper | `92ec1c12cc174d549dfdbedf921984784252d66630bdeca32d248085b42c76a1` |
| glibc libc.so.6 | `eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f` |
| glibc ld-linux | `63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458` |
| OVMF_CODE_4M.fd | `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94` |
| Source commit | `d2bfd660040ddface1cd6e791301fb8ea73d181c` (main) |
