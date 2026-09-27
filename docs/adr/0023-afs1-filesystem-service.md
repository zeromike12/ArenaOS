# ADR-0023: AFS1 — the own-design filesystem v1, IPC v1.1 inline messages, and the end-to-end zero-copy file-I/O chain

Status: accepted (M5.3)

Date: 2026-09-26

Related: ADR-0018 (IPC v1), ADR-0019 (spawn protocol), ADR-0020 (the
shell), ADR-0021 (driver substrate), ADR-0022 (the userspace block
service), ARCHITECTURE §7 (IPC), §10 (files: paths are UI)

## Problem

ROADMAP 5.3 requires a filesystem of our own design — extent-based
file data, copy-on-write transactional metadata — served entirely from
ring 3: `create/write/read/close` through a userspace daemon on top of
the 5.2 block service, an on-disk layout that survives re-open, a
host-side `mkfs`, and shell builtins (`ls`/`cat`/`write`) against it.

The constraints that shape the design:

1. **The kernel stays mechanism.** No filesystem code in ring 0, and
   no new syscall numbers: the FS is a capability endpoint like any
   other service (ARCHITECTURE §7/§10).
2. **Two words cannot carry a name.** IPC v1 messages are `[w0, w1]`
   plus one cap — a 32-byte filename or a directory entry does not
   fit. ARCHITECTURE §7 always anticipated "small messages copied
   inline"; 5.3 is where that becomes load-bearing.
3. **fsd must never map a client buffer.** A LENT Untyped cap cannot
   be mapped (`owned = false` is enforced by the kernel, ADR-0022) —
   by design. So a filesystem server holding only a lent cap has
   exactly one honest way to move bulk data: forward the cap to the
   block service and let the device DMA the CLIENT's page.
4. **Crash direction: leaks are acceptable, corruption is not.** The
   5.4 crash gate will kill QEMU mid-write and demand recovery
   without an fsck ritual; the metadata discipline has to make that
   true by construction, starting now.
5. **The usual ring-3 budget:** no allocator, one 4 KiB stack page,
   .bss carried by the image, every constant shared with the host
   tools instead of duplicated.

## Decision

### 1. AFS1: the on-disk layout (own design; `tools/afs1.py` is the host-side mirror)

The scratch disk is 8 MiB = 16384 × 512 B sectors. One Python module
(`tools/afs1.py`) owns the layout for the host side — `mkfs`, and the
post-boot parser `test_m5.py` uses to verify the real committed bytes
— and `userspace/fsd` mirrors it byte-for-byte in ring 3.

```text
sector 0        superblock: magic "ARENAFS1", version 1, geometry
                (sector size, total sectors, 32 objects × 64 B,
                4+4 metadata sectors, commit base), FNV-1a-64
                checksum over bytes [0, 504) stored at 504
sectors 1..2    commit records, ping-pong: slot = 1 + (seq % 2).
                {magic "AFSC", version, seq u64, objtab_head u32,
                bitmap_head u32, checksum} — the ONLY in-place writes
                on the whole disk; one 512 B sector flips the
                filesystem between generations (sector-write
                atomicity is the documented v1 assumption)
sectors 3..6    initial object table: 32 records × 64 B
                {type u8, name_len u8, flags, pad, extent_head u32,
                size u64, mtime u64 (reserved — no clock in v1),
                reserved, name[32]}
sectors 7..10   initial allocation bitmap: 1 bit per sector
sectors 11..    free pool: data sectors, extent blocks, and each
                commit's fresh CoW object-table/bitmap runs
```

- **Extent blocks** are `{next u32, count u32, 62 × (phys_start u32,
  len u32)}` chained sectors. An entry's FILE position is the
  cumulative sum of the lengths before it — the classic
  implicit-offset extent tree, ordered by file position. The reader
  walks `next` chains generically; the v1 server keeps depth ≤ 1
  (≤ 62 extents per file) and CoWs the whole (single) block on
  append, merging physically contiguous runs.
- **Data sectors are written in place** — once a sector belongs to a
  file offset it is overwritten there (extent-based data, exactly the
  ROADMAP wording). **Metadata is copy-on-write**: the object table
  and bitmap are mutated in RAM during a transaction and written to
  FRESH contiguous 4-sector runs at commit — the live generation is
  never overwritten.
- **The transaction**: `begin` (reclaim the dead list into the RAM
  bitmap) → mutate (extent-block CoWs and forwarded data writes hit
  the disk immediately — they only allocate) → `commit` (objtab run,
  bitmap run, then the ONE commit-record sector). Data lands before
  the record that makes it reachable.
- **Two-generation-delayed freeing**: each commit rotates two dead
  lists — the runs of the generation before last are freed inside the
  current transaction, so no sector a valid commit points at is ever
  overwritten, even by the allocator of the very next transaction. A
  crash mid-transaction leaves the previous generation valid and the
  new sectors merely leaked; a bounded dead list (64/generation)
  degrades to a logged leak, never to damage.
- **v1 limits, honestly stated**: 32 objects; names ≤ 31 bytes +
  NUL in a 32-byte field; flat namespace (no directories, `/`
  refused); ≤ 62 extents per file; no gap writes (`off > size`
  answers `FS_ERR_RANGE` — the cumulative mapping only stays honest
  when appends happen at EOF); no truncate/delete/append-as-op; no
  clock; disks ≤ 16384 sectors (the RAM bitmap is image-sized).

### 2. IPC v1.1: the optional 64-byte inline message

`SYS_IPC_CALL` gains `a5`, `SYS_IPC_RECV` `a2`, `SYS_IPC_REPLY` `a4`:
a pointer to a 64-byte buffer, or NULL. The kernel snapshots the
buffer at call/reply time **in the owner's context** (validated
against the caller's regions, STAC-bracketed), rides it inside the
call slot, and copies it out to the receiving side — the resumed
caller's buffer is IN/OUT (its request bytes are overwritten by the
server's reply bytes). NULL means exactly the v1.0 behavior, so every
existing caller is wire-compatible — with one discipline lesson, now
encoded everywhere: **the new argument register must be explicitly
zeroed**. A stale `r9`/`rdx`/`r8` is validated as a pointer and
refused (`STATUS_BAD_ADDRESS`); the m4 emitters, storaged, blktest,
and the shared `syscall6` stub all pin it.

This is the last kernel change the filesystem needs: names and
dirents ride the inline buffer; bulk data rides caps; the registry
gains no numbers.

### 3. Block protocol v1.1: the in-frame buffer offset

Request word 1 becomes `op | (buf_offset << 8)`: the sector's landing
spot INSIDE the caller's lent 4 KiB frame. storaged **must** refuse
`buf_offset + 512 > 4096` — without that bound a caller could aim the
device's DMA into the frame AFTER its buffer (a security seam, not a
robustness nicety). One frame can now stage up to 8 sectors, and a
file transfer at any 512-aligned start maps into the client's frame
at `sector*512 - offset`. `blktest` keeps offset 0 (wire-identical to
v1.0) and moves its raw-block cycle to the LAST scratch sector: the
host now formats the disk (below), and a raw write must not touch FS
structures.

### 4. Host-side `mkfs` — and the disk as a test artifact

`arena_env.make_scratch_disk` formats `build/scratch.img` with
`afs1.mkfs` every run: superblock, first commit (`seq 1` in slot
sector 2; sector 1 stays invalid so mount can never pick a stale
slot), empty object table, bitmap marking sectors 0..10. Fresh per
run — cross-BOOT persistence is 5.4's exit criterion; cross-RE-OPEN
persistence inside one boot is 5.3's, and the production fsd mount
proves it (below).

After every m5 boot, `test_m5.py` parses the image with the same
module: newest valid commit (seq 3 after CREATE + WRITE),
`arena.txt` in the committed object table with size 512, its extents
resolving to real sectors marked used in the committed bitmap, and
those sectors holding fstest's exact pattern bytes — the on-disk
layout proven from the host side of the wire.

### 5. `fsd`: the filesystem server (registry image 4) and the zero-copy chain

One crate, two images (the storaged/blktest pattern): `fsd` (the
resident server) and `fstest` (the suite's client, image 5). fsd is
spawned with kernel-literal grants — slot 0 the block endpoint's call
side (WRITE), slot 1 its own FS endpoint's serve side (READ). **No
Mmio cap, no notification**: fsd never sees a device register and has
no interrupt of its own; every disk operation is one synchronous
forwarded block call whose completion wakes it through the call.

The data path, end to end:

```text
client (shell/fstest)          fsd                    storaged         device
  ALLOC_FRAME + CAP_COPY  ─┐
  fill frame, CALL ────────┼─▶ recv (name/len in the inline msg,
    (LENT cap attached)    │     client cap landed in fsd's space)
                           └─▶ CALL block ep ────────▶ descriptor at
                                (the SAME lent cap       client-frame
                                 forwarded untouched)    phys + offset
                                                          ──▶ DMA
```

The file's bytes travel disk ↔ the CLIENT's page. fsd cannot map the
lent cap (the kernel refuses), so zero copy here is not an
optimization — it is the only thing the capability system allows.
fsd's own metadata I/O uses one owned scratch frame in the
lend-keep pattern (ADR-0022).

The FS protocol (shared `userspace/abi.rs` — one wire contract, four
programs): `CREATE`/`OPEN` (name in the inline message) → handle;
`READ`/`WRITE` (`w1 = fh | offset << 8`, length word in the inline
message, client's LENT frame attached, ≤ 3584 B per call); `CLOSE`;
`LS` (cursor in, dirent out: next cursor, size, name); `SHUTDOWN`
(reply carries fsd's lifetime disk-operation count, then fsd exits by
its own hand — a parked server is never destroyed). Typed negative
statuses (`FS_ERR_NOT_FOUND`, `EXISTS`, `BAD_FH`, `NO_SPACE`,
`CORRUPT`, `RANGE`, …); a refused commit rolls the mutation back
(CREATE erases the record; WRITE restores the size — extents beyond
EOF stay unreachable, the honest leak).

### 6. The proof: m5 `fs_service` (suite 6/6) and the production boot

The suite spawns THREE children — a fresh storaged test instance (its
reset-driven handshake re-initializes the device; `relay::register`
restarts the delivery count), fsd, and fstest — and asserts:

- fstest's script: CREATE `arena.txt` → WRITE 512 pattern bytes →
  CLOSE → **re-OPEN by name** → clear frame → READ → verify
  byte-for-byte → LS walk (exactly one file, right name and size) →
  both services shut down cleanly, all in ring 3;
- three exit badges exact, three exit codes 42 (the diagnostic
  contracts 69–78 / 80–84 / 60–68 mapped per stage);
- relay-vector deliveries **exactly 33** — not an observed-then-frozen
  number but a derived contract: mount 11 (superblock + 2 commit slots
  + 4 objtab + 4 bitmap) + CREATE commit 9 + WRITE 11 (extent block +
  data sector + commit 9) + READ 2 (extent lookup + data) — and fsd's
  reported disk-op count and storaged's completion count must equal
  it: three independent counters, one number;
- the dead driver's relay swept by `proc::destroy`; teardown
  frame-exact across all three address spaces.

Production boot: `entry.rs` spawns storaged → **fsd** → shell, and
the shell's grants gain slot 3 (the FS endpoint's call side). The
production fsd mounts the volume the suite's fs_service committed
EARLIER IN THE SAME BOOT — that mount, serving the shell's `ls`, is
the in-boot persistence-across-re-open proof.

### 7. The shell grows filesystem verbs (and joins the shared ABI)

The shell migrates to `userspace/abi.rs` (its private syscall stubs
and formatters deleted — one ABI surface for every ring-3 image) and
gains `ls`, `cat NAME`, `write NAME TEXT`. It allocates ONE 4 KiB
file window lazily (lend-keep) and reuses it for the rest of its
life — a resident process must not leak a frame per command. `cat`
streams in ≤ 3584-byte chunks; `write` CREATES and refuses to
overwrite (no truncate in v1 — an honest refusal beats a silent
clobber). Its `help` text fits one console chunk: resident services
share the wire, and a chunked help can be split mid-verb by fsd's
mount log — an interleaving the shell session test now documents.

The kernel's exit-status log became a newest-wins 64-entry ring: one
full boot (five suites + production spawn) records more exits than
the original 16-entry prefix log held, and suites always query their
own just-exited children.

## Approaches considered

1. **A kernel filesystem.** Rejected: the kernel is mechanism
   (ARCHITECTURE §7); 5.3's exit criterion says ring 3; and it would
   put a storage stack inside the trusted base.
2. **Log-structured everything / full CoW including data (ZFS-like).**
   Rejected for v1: the block protocol moves one sector per call, so
   data CoW multiplies calls and write amplification; the ROADMAP
   asks for extent-based data. The commit record + CoW metadata give
   the transactional property where it matters; data CoW/snapshots
   can layer on later without a layout revolution (the `next`-chained
   extent format already outlives the v1 server's depth-1 use).
3. **Journalling metadata (ext4-style).** Rejected: a journal plus a
   replay path is more machinery than a ping-pong commit for a
   32-object v1 — the commit record already IS the atomic flip, and
   mount is "pick the highest valid seq".
4. **Checksums on every block.** Deferred: the superblock and both
   commit slots are checksummed (the load-bearing decisions); objtab/
   bitmap runs are protected by commit ordering + the bitmap
   self-consistency checks at mount. Per-run checksums are a 5.4
   hardening candidate — the crash gate will tell us what it buys.
5. **New syscalls for file operations.** Rejected: capability
   endpoints are the interface; the only kernel change is the inline
   message buffer the IPC design always anticipated.
6. **fsd staging file data through its own buffer (one copy).**
   Rejected: it would require mapping lent caps — exactly what the
   cap system forbids — or copying through the reply path, which the
   2-word + inline-64 message cannot carry. Forwarding is both the
   only lawful and the fastest design.
7. **Directories in v1.** Deferred: flat object table; ARCHITECTURE
   §10 says paths are UI — 5.4's file caps and path→cap resolution
   are the namespace story.

## Consequences

**What this buys:** a filesystem whose every layer — layout, mkfs,
server, client, shell verbs — is our own design and provably running
in ring 3; transactional metadata with a crash story that degrades to
leaks; a derived (not observed) machine-event contract asserted by
the suite; host-side verification of the actual committed platter
bytes; a wire-compatible ABI-growth pattern (NULL = old behavior)
that future extensions can copy; and one shared ABI file binding
every ring-3 image to one frozen contract.

## Downsides accepted

The v1 limits listed above (32 files, flat names, ≤ 62 extents, no
truncate/delete, no gap writes, commit-per-write with no batching,
16384-sector ceiling); the single-atomic-sector-write assumption for
the commit flip; leak-on-abort semantics; HELP/console interleaving
between resident services (mitigated by chunk-sized atomicity, not
eliminated); fsd's per-write commit costs 9 metadata sectors (fine at
this scale, obvious future batching point).

## Future implications

5.4 must prove: two-boot persistence (the same `scratch.img` written
in boot N, read in boot N+1 — the harness stops re-formatting between
the paired boots), and the crash gate (kill QEMU mid-write, reboot,
recover WITHOUT fsck — the ping-pong commit + two-generation delay
are designed to make that a mount-time choice, not a repair). File
caps + path→cap resolution replace name-per-call. Truncate/append/
delete need the dead-list machinery to grow file-extent freeing (the
lists already exist). Larger disks resize the bitmap run; more files
grow the object-table run (both are superblock geometry, not code).
v0.5.0 ships after 5.4.
