# ADR-0046 — 8.1 transactional configuration store: design investigation

*Status: proposed; Phase 8.1 opened after the qualified 8.0 manager checkpoint. No storage implementation or permission policy is approved by this draft.*

## Problem and existing contracts

8.1 needs versioned, bounded update/read and multi-boot verification with
QEMU SIGKILL at every commit boundary. A corrupt state must fail closed,
not silently return an older or different configuration. This is a
configuration store, **not** 8.2's permission-grant UI. No service may
turn a file's symbolic name or an application's pid into authority.

AFS1 (ADR-0023) has an atomic metadata commit-record flip under the
*process-crash / ordered submitted writes / atomic 512-byte sector*
model, but its `WRITE` path overwrites **existing data sectors in place**.
Therefore overwriting an existing config file, even behind AFS1's CoW
metadata, is NOT an atomic config update. There is no rename syscall.
`CREATE` commits an empty file before a later `WRITE` commits its data;
`UNLINK` is its own transaction. A crash may leave the empty new file.
Power loss with volatile write caches is outside AFS1's existing
contract; do not claim it without FLUSH/FUA and new qualification.

## Candidate boundary to validate before implementation

A single built-in config service, started with a fixed, kernel-audited
filesystem `Endpoint/WRITE` capability, owns the records. A client
needs a separately granted config-service endpoint capability to read;
updates require a **different** trusted WRITE authority. The service
must not infer write rights from a pid, filename or a manifest. It
serializes requests and has no path for ordinary clients to reach the
FS write endpoint. Exactly which kernel bootstrap grant and two
public endpoint objects fit existing bounds must be audited before
accepting this design; no new mint syscall is assumed.

Consider immutable, uniquely named generation files. Each has a
versioned, fixed-size record (magic, format, monotonic sequence,
length, payload, checksum, reserved zeros) in one <=512-byte payload
written by one `FS_OP_WRITE` into a newly created *empty* file. The
committed AFS1 metadata flip either leaves the file empty (not a
committed config), or exposes its full checked bytes. Recovery scans
the bounded namespace, refuses any malformed nonempty generation,
selects the unique highest valid sequence, and rejects duplicates,
gaps, impossible length/version, unexpected name or sequence overflow.
An empty generation after a crash is only a pending transaction, never
a returned value. Reclaim of old generations via `UNLINK` must preserve
at least one durable prior generation and be crash-tested separately;
the 32-object AFS1 table and competing user files make this a real
capacity/GC design question, not a reason to silently overwrite data.

## Decisions still required before code

1. Specify the trusted config updater's cap grant and whether a new
   bounded service/image fits the kernel registry, endpoint and cap
   tables without granting FS write access to untrusted readers.
2. Nail down the exact permitted recovery shapes at CREATE, WRITE,
   CLOSE and UNLINK boundaries, including full-table/IO errors and
   the distinction between an aborted empty file and corrupted bytes.
3. Fix the maximum record size, generation count, sequence/name format,
   GC ordering and behavior on capacity exhaustion; no wraparound or
   fallback to an older value on corruption.
4. Run multi-boot host SIGKILL at **each** observable commit boundary,
   check returned bytes and the actual AFS1 platter, and boot the same
   final EFI under the full historical suite and fresh 100/100 gate.

No implementation begins until these choices are made in an accepted
ADR. This investigation deliberately does not invent transactional
rename, overwrite atomicity, crash-safe disk flushes, or a permissions
UI that AFS1 and the current authority model do not provide.
