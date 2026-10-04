# ADR-0076 — AFS2: hierarchical copy-on-write filesystem

Status: accepted (Phase 11.5). Host model `tools/afs2.py` is the format's
single source of truth on the host; `fsd2` mirrors it in ring 3.

## Problem

AFS1 (ADR-0023/0063) is flat, holds 32 objects in total, names of at most
31 bytes, files of at most 62 extents, an 8 MiB disk, no clock, no rename,
no truncate and no directories. A real file explorer and a desktop backed
by the filesystem need a hierarchical store with stable object identity,
atomic rename/move and honest timestamps — without giving up AFS1's crash
discipline (copy-on-write metadata, a single-sector commit written last,
exact old-or-new recovery with no repair tool).

## Decision

### Placement and coexistence

The scratch/system disk keeps its first 8 MiB (sectors 0..16384) as the
**AFS1 legacy area**, untouched: the historical system services (configd,
permissiond, packaged, the shell's `ls/cat/write`) and their crash proofs
keep operating on AFS1 exactly as before. The **AFS2 volume** occupies the
rest of the disk, starting at sector 16384 (byte offset 8 MiB); all AFS2
block numbers below are relative to that base. Test and shipping disks are
72 MiB (AFS2: 64 MiB = 16384 blocks).

### Geometry

4096-byte blocks (8 sectors). Sector writes are the atomic unit, as in
AFS1; nothing else is assumed atomic.

```text
block 0   superblock — sector 0 only: magic "ARENAFS2", version 1,
          block size, total blocks, bitmap block count, object
          capacity, volume id, FNV-1a-64 checksum of bytes 0..504 at 504
block 1   commit slot A — sector 0 only (one 512-byte record)
block 2   commit slot B — sector 0 only
block 3.. free pool: every metadata and data block, always written fresh
```

### Commit record (one sector; slot = 1 + seq % 2)

`magic "AFS2COMT"`, version, `seq u64`, `objtab_root u64`, `free_blocks
u64`, `used_objects u32`, `wall_us u64` (commit wall time, 0 = unknown),
`bitmap_count u32`, `bitmap[52] u64` block numbers, FNV-1a-64 checksum of
bytes 0..504 at 504. Mount picks the valid record with the highest `seq`.

### Metadata blocks

Every metadata block ends with a 24-byte trailer `{kind u32, pad u32,
block u64, checksum u64}` (checksum over bytes 0..4088, which include
`kind` and the block's own number, so a misdirected or stale block is
detected), leaving 4072 payload bytes.

| Kind | Payload |
|---|---|
| 1 OBJROOT | 509 × u64 child pointers to OBJLEAF blocks (0 = none) |
| 2 OBJLEAF | 31 × 128-byte object records |
| 3 MAPNODE | 509 × u64 block pointers (data blocks, or MAPNODEs one level down) |
| 4 DIRBLK | packed directory entries, sorted by name bytes |
| 5 BITMAP | allocation bits: 1 = used; 32,576 blocks per bitmap block |

Object capacity: 509 leaves × 31 records = 15,779 objects (≥ 10,000).

### Object record (128 bytes)

`type u8` (0 free, 1 file, 2 directory), `depth u8` (map depth 0..2),
`flags u16`, `generation u32`, `parent u64` (object id of the containing
directory), `size u64`, `map u64`, `ctime u64`, `mtime u64` (wall
microseconds since 1970, **0 = unknown**), `entries u32` (directories),
`version u64` (advanced on every change: change notification), reserved
zero. An **object id** is `generation << 32 | index`; index 0 is the root
directory. Freed records keep their generation; reuse advances it, so a
stale id never names a new object.

File data is reached through the block map: depth 1 = `map` is one MAPNODE
of data pointers (≤ 509 blocks, ~2 MiB); depth 2 = `map` points to a
MAPNODE of MAPNODEs (≤ 259,081 blocks, ~1 GiB). Files ≥ 16 MiB are
ordinary depth-2 files. Pointer 0 is a hole (reads zero).

### Directories

A directory's data (through the same block map) is a sequence of DIRBLKs;
each holds `count u16, used u16` and packed entries `{index u32, generation
u32, type u8, name_len u8, name}`. Entries are globally sorted by name
bytes: every name in block k sorts before every name in block k+1. Names
are 1..255 bytes of valid UTF-8 without `/`, NUL or control characters,
and never `.` or `..`. Listing is by **name cursor**: "entries strictly
after this name", which is stable across concurrent changes and bounded.

### Transactions and the crash model

Every mutating operation is one transaction ending in one commit:

1. Allocation uses a working copy of the committed bitmap; a block freed by
   this transaction stays allocated in the working copy until the commit,
   so **no block referenced by the committed generation is ever written**.
2. New and changed metadata and data blocks are written to fresh blocks
   (copy-on-write all the way to a new OBJROOT; data written by `write`
   goes to fresh blocks, so a partial overwrite is transactional too).
3. The new bitmap blocks are written fresh.
4. The commit record sector is written last, into the slot not holding the
   current generation.

A crash at any point before step 4 completes leaves the previous commit as
the newest valid one: **exactly the old state**. After step 4: exactly the
new state. There is no intermediate namespace, ever: a rename/move removes
the old entry, inserts the new one and updates the object's parent inside
the same transaction.

**Fail closed.** If the newest valid commit references a metadata block
whose trailer kind, block number or checksum is wrong, that is corruption,
not a crash (the commit is written last): mount refuses (`CORRUPT`) instead
of silently falling back to an older generation. A superblock or both
commit slots invalid also refuses.

### Operations

`lookup`, `stat`, `statfs`, `create` (file), `mkdir`, `write(offset,
bytes)` (extends; holes read as zero), `read`, `truncate(size)`,
`unlink` (files), `rmdir` (empty directories), `rename(src dir, name, dst
dir, new name)` (atomic within the volume, across directories, refuses an
existing target, refuses moving a directory into itself or a descendant),
`list(dir, after-name, max)`, `set_times`. Copy is a service-layer
operation (read + create + write). No-space and capacity refusals happen
before anything is written (preflight), so a refused operation leaves the
volume byte-identical.

### Time

Timestamps come from a userspace CMOS RTC service. When it is absent or
invalid, `ctime`/`mtime` are written as 0 and every presentation says
"unknown". Wall time is data, never authority.

### Migration from AFS1

A one-shot import, crash-atomic by construction: it builds the AFS2
volume (or adds to a freshly created one) in transactions and writes a
final commit containing `/System/imported-afs1` (system records: config,
permission, package and preference records) and `/Users/user/Documents`
(user files, `user-*` names) plus `/Users/user/Desktop` and
`/Users/user/.Trash`. The AFS1 legacy area is **read only** during import;
until the import's last commit lands, a crash leaves the AFS2 volume
either empty-formatted or at a previous complete state, and a restarted
import begins again. System services remain authoritative on AFS1 in
Phase 11; `/System` holds the imported copy and is reachable only from the
filesystem service's private system root, never from a user capability.

## Proof obligations

* Host model (`tools/afs2.py`): randomized operation sequences compared
  against an independent dictionary model of the namespace; encode,
  remount and compare after every step; corruption and no-space refusal.
* Host crash prefixes: for every mutating operation, every prefix of its
  block writes (and every torn commit sector) remounts to exactly the old
  or exactly the new model state.
* RED controls: in-place metadata mutation, early free (reuse of a block
  freed in the same transaction), a rename that omits the parent update,
  and a half-published rename each fail the proofs.
* Guest: the same crash-prefix property against the real `fsd2` through
  QEMU kills at named write triggers, with the trigger required to have
  fired.
