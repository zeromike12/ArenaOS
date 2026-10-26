# Phase 16 corrected demonstration artifact

Classification: **CANONICAL CORRECTED QUALIFICATION CANDIDATE — NOT
ACCEPTED and not a release.** Phase 16 remains pending M/C acceptance.

## Download

- Archive:
  [`ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz`](https://github.com/zeromike12/ArenaOS/blob/arena/2b4ae1a5-arenaos/docs/phase16/artifacts/ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz)
- SHA-256 sidecar:
  [`ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz.sha256`](https://github.com/zeromike12/ArenaOS/blob/arena/2b4ae1a5-arenaos/docs/phase16/artifacts/ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz.sha256)
- Bytes: 5,016,910
- SHA-256:
  `106088522042fb501a00554442ae97133127e53a9141d5e95069f112c9a9898b`
- Final tested OS/test source:
  `5aacbc88c50d4565a7bb9f4075be27e40df27779`
- Final EFI:
  `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`
- Final ESP:
  `04ffaf4681e92fe87886bdcff29b175df032ae9d88bd6cae59b34ad070ea674a`

## Verify

Download both files into the same directory, then:

```sh
sha256sum -c ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz.sha256
mkdir -p /home/user/p16f
tar -xzf ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz -C /home/user/p16f
cd /home/user/p16f/ArenaOS-Phase16-Demonstration-5aacbc8
python3 verify_package.py
```

For the representative guest launch/verification with the pinned
canonical backend wrapper:

```sh
ARENA_QEMU=/home/user/phase16-backend/qemu-patched \
ARENA_GLIBC_DIR=/home/user/glibc-2.40 \
python3 verify_package.py --guest
```

The archive's internal `README.md` contains the complete contents,
backend identities, short-prefix requirement, and rebuild caveats.
Fresh-extraction receipts for the exact sealed bytes are outside the
archive at
`../receipts/canonical-corrected-5aacbc8-2026-10-26/fresh-archive/`.
