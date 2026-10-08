# ADR-0077 — filesd: AFS2 file service and file capabilities

Status: accepted (Phase 11.5 service and records; the trusted chooser,
11.6, is proven separately and recorded in the Phase 11 progress log).

## Problem

AFS2 (ADR-0076) gives the disk a hierarchical namespace. Applications must
open, list and change files in it **without ambient authority**. A path an
application sends is a presentation string. It must never decide what
the application may touch.
`/System` (imported system records) must be unreachable from anything an
application can hold. A dead application's access must end with it, and
one application must not be able to exhaust the service for the others.

## Decision

### The service

`filesd` is boot image 9, spawned in the desktop profile with four
capabilities: storaged's block endpoint (write: whole-block
`OP_READ_BLOCK`/`OP_WRITE_BLOCK`), its own endpoint (read), the Rtc
capability (read) and fsd's endpoint (write, used only for the one-shot
AFS1 import). It is the only process that addresses the AFS2 region, and
the only user of the whole-block storage ops (log word `BLKW4K`).

Bring-up follows ADR-0076: a never-committed region is formatted and AFS1
is imported read only. A committed volume without the import marker is
formatted again and the import redone. Any other volume mounts or
**fails closed**: filesd reports the file service offline and writes
nothing. On a disk without the region (8 MiB, every historical fixture)
filesd says so and stays offline. It writes nothing there either.

### Authority: badged endpoint capabilities

Every file capability is a **BadgedEndpoint** on filesd's endpoint
(ADR-0074). filesd reads the badge the kernel delivers with each call
(`SYS_IPC_RECV_BADGED`). The badge names one **record**: `index | generation << 16`.
A record holds an AFS2 object id (index and generation), rights, a lineage
and, for a lineage head, a registered I/O page. Nothing in a request
grants anything. The request names an operation, a byte range and a name
relative to the called record's object.

* **Root.** The only record created without a request is record 1 =
  `/Users/user` with all rights. The kernel mints its badge
  (`USER_ROOT_BADGE = 1 | 1 << 16`, rights write|copy) into the desktop
  broker's slot 20. No record for `/` or `/System` ever exists.
* **Reachability.** New records come only from `OPEN` on an existing
  record: the record itself (attenuation), or one *child name* of a
  directory record. Names are validated by the engine. A name is never
  `.` or `..` and never contains `/`, and there is no parent operation.
  So the records reachable from record 1 are exactly the subtree of
  `/Users/user`, and `/System` is unreachable from any capability.
* **Rights.** READ, WRITE (write, truncate), LIST (list, stat a child, open
  a child by name: naming reveals existence), CREATE (create, mkdir; also
  required on a rename destination), DELETE (unlink, rmdir), RENAME. A
  record opened from another gets `requested & parent` rights:
  attenuation only.
* **Rename.** The source directory is the called record (RENAME). The
  destination is the called record, or another record of this service
  that the caller lends with the request. filesd reads its badge with
  `SYS_ENDPOINT_BADGE` (below) and requires CREATE on it. Moves are atomic
  (ADR-0076). Moving a directory into its own subtree is refused.
* **Stale.** A record names an object *generation*. When the object is
  deleted, every operation on the record fails, and a new object at the
  same index has a new generation. A retired record's badge is stale:
  `DENIED`. A record index whose 16-bit generation is exhausted retires
  for good, so a badge is never valid twice.

### Names and data: the I/O page

A lineage head registers one page of a SharedRegion it owns
(`OP_SESSION`, the region lent with the call). Names, listings and
file data travel through it, at most one page per call. filesd copies
every name and every byte to write into private memory before checking or
using it. The client cannot change a name after it is checked. Paths are
never sent.

### Lineages, quotas and revocation

A record opened from record 1 heads a **lineage**: the broker opens one
per application grant. Records opened from it, and from those, inherit
the lineage. All records of a lineage share the head's I/O page, so filesd
maps at most one page per application grant. A lineage holds at most 16
live records, and the table holds 256. `RELEASE` of a head, or `REVOKE` of
a head that someone lends to filesd, retires the whole lineage. When an
application dies, the broker revokes the head it granted, and every
capability the application derived goes stale with it. Record 1 cannot
be revoked.

**Placement (the powerbox rule).** `OPEN` may carry another record of this
service, lent with the call. The new record then joins *that* record's
lineage, with rights still `requested & called record`. The broker keeps
each application's lineage head itself, with no rights, and never hands
it out. It walks the user's chosen names through **its own** I/O page,
on record 1, and places only the final record in the application's
lineage. An application therefore never sees or influences the names
the broker resolves. Its grant still counts against its quota, and the
grant dies with the lineage.

**Creating a lineage (amendment, found by the guest).** Records opened
from record 1 stay in the broker's own lineage, so they use the broker's
I/O page. A new lineage head is created only by `OP_NEW_LINEAGE` through
record 1. Its record names `/Users/user` and has no rights. The first
design made every open from record 1 a new head with no I/O page, so the
chooser's own folder walk failed ("no file session").

**Rights along a walk.** Attenuation is transitive: a record opened from
a folder never has more rights than that folder. A client walking a path
therefore opens each intermediate folder with LIST plus the rights it
wants at the end. The chooser opens its folder with the rights it
grants. Two guest failures found this: a `mv` into a subfolder lost
CREATE, and a read/write grant came out with no rights at all.

### `SYS_ENDPOINT_BADGE` (51)

`endpoint_badge(server_slot, cap_slot) -> badge`. It succeeds only when
`server_slot` is a plain Endpoint the caller may receive on (READ) and
`cap_slot` is a live BadgedEndpoint of that same endpoint (same eid and
generation). It lets a server identify one of its own capabilities lent
back to it: a rename destination, or a grant to revoke. It never reveals
anything about another server's capabilities.

### Time: the Rtc capability

`CapObj::Rtc` with `SYS_RTC_READ` (50): the kernel reads the CMOS clock.
It waits for no update in progress, reads twice until both reads agree,
handles BCD and 12-hour modes and the century register, and accepts
2000..2199. It returns Unix seconds, or `BUSY` when the clock is absent,
inconsistent or out of range. filesd bases wall time on one read plus the
monotonic clock. When the read fails, every timestamp it writes is 0 and
its log says "unknown". Wall time is data, never authority.

## Security review

No external runtime library. New kernel surface: two syscalls and one
capability kind, each refusing every slot or kind it does not expect.
Abuse cases and their answers:

| Abuse | Answer |
|---|---|
| forged badge or path | The badge comes from the kernel; paths are not accepted |
| `..`, `/` or a control character in a name | The engine rejects it before lookup |
| a request for `/System` | No record can name it |
| a name swapped after the check | filesd copies it first |
| a stale record after delete or revoke | The generation no longer matches |
| exhausting the table | The lineage quota caps it |
| a reply that cannot be delivered | The minted record retires |
| an application dies holding records | The broker revokes its lineage |

## Proof obligations

* Host: engine proofs and RED controls (ADR-0076).
* Guest (`tools/test_m11_afs2.py`): migration exactness and AFS1 byte
  identity, a zero-write remount, interrupted import at verified kill
  points, fail closed on a damaged volume, unknown time when the RTC is
  out of range.
* Guest (11.6, `tools/test_m11_files.py`, PASS): chooser cancel; read-write
  and read-only grants (a refused save checked by a Save As positive
  control); stale capability after delete; rename semantics (the
  capability follows the object); malicious `/System` requests from the
  terminal; a signed hostile probe that sends raw forged requests; and
  cleanup on application death (lineage retirement counted).
  `tools/test_m11_files_red.py`: the no-attenuation and shallow-revoke
  mutants each fail it; the source and EFI are restored byte-exactly.
