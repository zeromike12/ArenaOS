# ADR-0046 — Phase 8.1 transactional configuration store: design investigation

*Status: accepted for bounded 8.1 implementation (2026-09-29); read boundary qualified; transactional-core implementation and qualification in progress. Phase 8.0 remains complete and 8.1 is not yet closed. This is not a permission policy (8.2).*

## Existing facts and threat boundary

AFS1 (ADR-0023; `userspace/fsd/src/main.rs`, `tools/afs1.py`) has 32 flat-namespace objects, names shorter than 32 bytes, a single-threaded FS endpoint, and `CREATE`, `OPEN`, `READ`, `WRITE`, `CLOSE`, `LS`, `UNLINK`. It copies metadata into new sectors and flips a 512-byte ping-pong commit sector. **Existing file data is overwritten in place.** Therefore updating a configuration by overwriting a file is NOT atomic. `CREATE` commits an empty file separately from the subsequent `WRITE`; `CLOSE` is not a commit; there is no rename/truncate syscall. A failed `CREATE` may even have committed an empty object before it discovers the 8-slot open-file table is full. `WRITE` of a new one-sector file writes data first, then metadata, then the commit record. Failures must be reconciled by scanning the disk state, not assuming an error reply means nothing changed.

The existing crash model (M5.4) is SIGKILL of QEMU under ordered, submitted writes and atomic 512-byte sectors. Host volatile-cache power loss, malicious writes via a raw FS capability, and arbitrary block-device rollback are **not** covered. The current Power-holding shell has a raw FS `Endpoint/WRITE` cap for historical `ls`/`cat`/`write`; it cannot be treated as an untrusted config reader or used as evidence that the config namespace is protected from every process. No ordinary config reader will get that raw cap. A future permission system must separately protect the namespace from processes with filesystem write authority.

## Decision: bounded single-key v1

One built-in `configd` process serializes requests. The kernel boot root grants it the production fsd `Endpoint/WRITE`, a dedicated config `Endpoint/READ`, and a READ-only reference to an otherwise inert, newly minted Notification object. Read clients receive only the config endpoint's WRITE side. A separately named trusted **test updater** receives the same endpoint plus a READ|COPY|DESTROY reference to the Notification. The receiving service checks the *transferred* reference against its own boot-granted object and exact allowed rights, then discards it on all paths (ADR-0047). An endpoint, opcode, pid, file name or guessed object number is never update authority. No new kernel cap type, name-based grant, generic device request, or 8.2 approval UI is proposed. The updater and read client must be separate processes in the negative proof; the existing Power/raw-FS shell is a trusted bootstrap actor, not the unprivileged reader.

The production footprint is auditable before any code: the kernel registry currently has `MAX_IMAGES=21` with images 0..20 in use; one new service and one or two isolated test clients would require raising that limit with literal image mappings and an audit. `MAX_ENDPOINTS=8`: the full production fixture uses block, FS, net, rng, input, console and stack (7), so ONE config endpoint fits if there are no extra endpoints hidden in the final fixture. `MAX_NOTIFS=13` is filled by existing production objects; the inert authority marker needs a documented one-object increase. Every process has 16 cap slots. The fully equipped shell already reserves slots 0..6, transient child slot 7, file-window slots 8..9, protected-target proof slots 10..14 and diagnostic marker slot 15: do NOT silently add config grants to it or reuse a transient slot. A dedicated updater/read client avoids this conflict. Recheck the boot and the no-device fixture, grant counts and resource snapshots when introducing real processes.

For a deliberately bounded *single key*, reserve names `cfg8-01` through `cfg8-08`. Never overwrite a nonempty file. Version 1 has a fixed **512-byte** file: 8-byte magic `ARCFG8V1`, format u32=1 at offset 8, zero u32 at 12, sequence u64 at 16, payload length u16 (0..32) at 24, zeros at 26..31, 32 payload bytes at 32..63 (unused bytes zero), zeros at 64..503, then FNV-1a-64 over bytes 0..503 at 504. All integers are little-endian. Names and embedded sequences must agree, be contiguous from 1, and never wrap. Request/reply data fits the existing 64-byte inline IPC message; the receiver's *own* mapped, reusable buffer frame stages exactly one 512-byte FS write/read. A client never needs a map of a lent cap. Read returns explicit `UNSET`, `VALUE(seq,len,bytes)`, or `CORRUPT`; update returns `COMMITTED`, `NO_SPACE` or a typed failure. Empty payload is distinguishable from an empty pending file by its full 512-byte on-disk record.

A successful read scans **all 32 directory entries**, rejects any malformed name in the reserved `cfg8-` prefix, duplicates, gaps, invalid sizes/checksums/versions/reserved bytes, or more than one empty generation. All nonempty generations are read and validated, not merely the highest candidate; a 0-byte file is only a pending `max_valid+1` generation. If no valid record exists and no pending file, return `UNSET`; an empty first generation also leaves the value unset. On an update, resume the one pending empty next generation or CREATE it, WRITE the complete new record once at offset 0, CLOSE, rescan and verify the *exact* requested value before acknowledging it. A failed write or reply is not evidence of non-commit: the next read/reboot scans again. On the eighth committed generation, all subsequent updates return `NO_SPACE` with the eighth record intact. There is deliberately NO GC or UNLINK in v1; the table limit and disk fullness are reported, never papered over by deleting the prior good value. The old record is never mutated, including on an aborted update.

Under the stated AFS1 crash model the observable states around one update would be:

| SIGKILL point | Recovery candidate |
| --- | --- |
| Before CREATE's commit record | Prior complete generation (or UNSET) |
| After CREATE's commit, before WRITE's commit | Prior complete generation plus one empty pending file |
| During new data/extent/metadata writes, before WRITE's commit | Same pending empty file; unreachable new blocks may leak |
| After WRITE's commit, before/after CLOSE/reply | Fully validated new generation |

Do **not** conflate a pending 0-byte file with corruption or a returned configuration, and do not turn a nonempty malformed record into a silent fallback. A full table/disk, lost FS service or unrecognized on-disk format must be explicit, not a success with a different value. This contract covers fail-closed *visible record* corruption under AFS1's ordered-write/atomic-sector crash model, not the separate commit-selection problem below. Any ambiguous FS I/O or unexpected return leaves configd DEGRADED until reboot; it must not continue writes with potentially inconsistent in-memory fsd state.

## Integrity limit and rejected alternative: invisible newest generation

`fsd` and `tools/afs1.py` select the highest **valid** AFS1 commit record. An invalid newer commit slot is ignored and the older valid slot wins. If a committed newer config's AFS1 commit sector is subsequently corrupted, fsd can mount the older metadata and **hide the newer generation completely**. A config service scanning only fsd cannot distinguish that disk from one where the newer commit was never submitted; a checksum on the now-invisible config file does not help. Thus an unqualified claim that *all* corrupt states fail closed instead of returning an older configuration would be false. The host probe `tools/probe_config_commit_fallback.py` demonstrates the actual commit-selection behavior without changing the kernel or trusting a timing retry.

**Scope decision:** 8.1 adopts the existing AFS1 crash model and requires visible record corruption to fail closed. Arbitrary corruption of an AFS1 commit sector, malicious raw-FS writes and device rollback are explicitly outside this guarantee. Detecting a lost committed generation would require a separate trusted monotonic anchor or a stronger storage protocol; replicating checksums inside files hidden by fsd cannot help. This is an accepted limitation, not a claimed proof of general media integrity. Do not silently broaden it in test names or release notes. If general corruption detection becomes a requirement, accept a new storage ADR and qualify its implementation separately. The existing v1 `FS_OP_CREATE` post-commit handle failure and lack of per-file ownership are exercised within the chosen crash/authority model.

## Read-boundary boot integration (partial 8.1 checkpoint)

This checkpoint implements only FS-backed reads and receiver-side SET
refusal; it does **not** create a trusted updater, grant anyone the marker
update right, or write any config generation. The boot root allocates one
endpoint and one inert notification; `configd` receives the literal grants
`fsd Endpoint/WRITE` (slot 0), `config Endpoint/READ` (slot 1) and
`marker Notification/READ` (slot 2). Its data-only client (image 22)
receives `config Endpoint/WRITE|COPY`, no fsd endpoint or notification.
The server checks transferred markers against its own slot-2 object;
missing and wrong-kind references are refused, consumed and destroyed at
the receiving boundary. Even a correctly marked SET returns `NOT_READY`
at this stage. The separately privileged updater/positive case and the
write transaction remain REQUIRED before 8.1 can close. The ordinary
reader uses a copyable endpoint as intentional data-authority delegation,
not as update authority.

The live budget is 23 images (0..22, up from 21), 8 endpoints (the one
remaining full-fixture endpoint is config), and 14 notifications (up from
13, the sole added object is the update marker). The full-fixture boot
checks that allocating a fifteenth notification fails; without optional
devices the table is intentionally not full. Existing shell slots remain
untouched; the boot-root reader is destroyed, its exit notice consumed and
spawn record forgotten before shell start. Neither it nor the shell is
given an update marker. One resident configd process remains; no second
filesystem owner is introduced.

The reader's pre-shell proof needs fsd's real interrupt-driven block
completion. Blocking the boot thread on its exit notification left all
threads blocked with an empty scheduler ready ring and prevented the
pending device interrupt from finishing: **do not block the boot thread
here**. Instead, keep it runnable, enable interrupts, yield while
watching the reader's exact exit status, and enforce an HPET-derived
21-second wall-clock deadline; restore its prior interrupt state, then
consume the matching exit badge with nonblocking `try_wait` and reap the
reader. This is a deadline, not a yield-count retry or a missing-badge
success path. Start the manager before the reader drain, as in the historical
boot: it may finish its initial device probe before shell input begins,
so its logs cannot split a typed line or a file streamed to the console.
The first full regression exposed this real interleaving when the manager
was moved AFTER the reader: filesystem marker bytes and keyboard echo
were split by manager startup output. The reader drain must instead
poll `audit_manager_child` while runnable and record any short-lived
initial probe's exact installed caps immediately. Waiting to audit only
in the later idle loop misses that worker if it has exited by then.
The idle loop subsequently audits the production child and installs
the shell's READ-only foreign Process reference. This preserves both
the first-probe audit and the historical quiet shell handoff without
weakening either regression test. Manager readiness badges persist.
`test_m8_dependencies.py` checks both workers and
the full historical suite preserves this ordering.

Host-prepared fixture disks exercise UNSET, a valid committed 512-byte
`cfg8-01` value through real fsd/storaged DMA, malformed reserved names,
and a corrupt newest checksum with a valid older value: the guest must
return CORRUPT rather than silently downgrade. The offline host fixture
is not a guest SET transaction or a crash-recovery proof. No in-guest
writer or positive update authority proof exists in this checkpoint.

## Authorized update probe and opt-in boot trigger (next implementation slice)

The production service accepts an arbitrary 0..32-byte SET payload from
**a separately granted updater** that transfers a fresh COPY of the
Notification reference on *each* request. The receiver compares the
object and exact rights to its boot anchor, consumes the landing on every
path, then performs the full CREATE/WRITE/CLOSE/rescan transaction.
The source marker remains with the updater and is never copied to the
ordinary reader or shell. A malformed SET without it is always DENIED;
a correct marker with malformed bytes is BAD_INPUT, never a write.

To make the positive path opt-in **without filling the shell's already
occupied cap table or giving it update authority**, the trusted raw-FS
shell may create exactly one test-intent file, `cfg-intent-one` or
`cfg-intent-two` (the latter requests a distinct test payload). The
service exposes a marker-gated `TEST_PLAN` read of those names via its
existing fsd cap; the updater holds only the config endpoint and marker,
asks for the plan, then constructs and sends a normal SET payload.
The intent file is input chosen by an already trusted filesystem writer,
NOT authority; the service MUST still reject SET when the transferred
marker is missing, forged or wrong-kind. An absent intent is a verified
SKIP with no config writes. Both intents present is a typed refusal.
This staging avoids automatically consuming config generations on every
ordinary boot and preserves all historical fresh-disk fixtures. It is a
bounded destructive test interface, not a permission-policy UI (8.2).
The shell never receives a config cap and cannot cause a SET by sending
an opcode or guessing an object id.

The boot root allocates no additional endpoint or notification. It adds
image 23 (24 images in the registry) for the separately privileged
updater. It launches and reaps that child after the ordinary reader's
pre-shell proof but before the shell prompt; both use the same boot-root
exit notification with distinct badges, consumed one at a time. This
retains the manager's short-lived probe audit throughout both
interruptible, wall-clock-bounded drains; it also prevents its log from
splitting console command output. The updater's exit is checked against
a success/skip protocol, not assumed from a debug line. Production
services and ordinary boots gain no ambient update authority.

Test driver kills QEMU at observed CREATE, WRITE-commit, CLOSE and
pre-reply markers, audits the actual committed AFS1 disk **before**
reboot, then verifies the same disk recovers through fsd/configd and
an authorized retry without a repair tool. The test-intent file is
left in place between the kill and recovery boots so the updater can
retry. A second update exercises immutable old generation plus new
value; full-table and I/O ambiguity must remain explicit, never a
silent fallback or invented success.

## Transactional-core implementation/proof scope (partial checkpoint)

The service now uses its single mapped reusable 512-byte buffer and
LENT copy for a canonical `encode`, CREATE or OPEN the one pending
empty generation, WRITE once, CLOSE, rescan all fsd-visible reserved
entries, and compare the exact desired value before `COMMITTED`.
Identical input with no pending generation is `UNCHANGED`, spending no
slot. An ambiguous CREATE/OPEN/WRITE/CLOSE/readback error latches
DEGRADED against further SET until reboot; physical allocation failure
is a typed `NO_SPACE` with no success claim. Table exhaustion is also
`NO_SPACE` but returns the bound (8) as a distinct detail. A correct
marker with oversized input is `BAD_INPUT` and is consumed; a second,
valid SET transfers a new marker copy. Test-only intent filenames
select two distinct payloads but convey no update authority.

Guest test `test_m81_update.py` commits two distinct records via the
service, then SIGKILLs QEMU after CREATE submission, committed-empty
CREATE, WRITE submission, fsd's WRITE commit record, WRITE reply, CLOSE,
and just before SET reply; it audits the SAME AFS1 platter before each
recovery boot and checks old/new bytes after. Visible predecessor
corruption and conflicting test intents refuse writes; a host-prepared
allocator-full bitmap produces a real FS `NO_SPACE` without mutating the
old visible record. `test_m81_update_table.py` checks eight actual
guest generations and a typed ninth refusal. The ordinary reader never
gets the marker, the updater never gets fsd or Power, and the boot root
reaps both before shell input. This is a transactional **core** proof,
not a claim that numeric resource/fault-path accounting is complete.
The original crash-model and media-integrity limits above still apply.

The first full regression found two historical console scripts pacing
commands to **fsd mount** or **manager READY** rather than to the shell's
real prompt. The added pre-shell updater drain exposed their hidden
assumption: the keyboard's first character and the stackstop command
could be echoed before the shell existed, splitting byte-exact serial
proof. The typing script now waits for `arena>`; the stop script requires
both READY and `arena>` before sending input. The commands, authority
checks and exact output assertions remain unchanged. This is a
readiness-boundary repair, not a timing retry or weakened regression.

## Required proof before 8.1 closure

1. Audit actual image, endpoint, notification, slot and process bounds and grant provenance with both device-present and absent fixtures before allocating boot grants. Keep raw FS writers explicitly trusted; never give the config reader a raw FS endpoint.
2. Unit-test the exact record parser, reserved namespace scan, empty/contiguous/gap/duplicate/unknown-version/corrupt cases, sequence exhaustion and full-table refusal. Prove ordinary endpoint without marker and wrong/forged markers cannot update at the receiver; only the genuine separately granted updater can.
3. Multi-boot QEMU test with the SAME scratch disk: exact old/new bytes and on-disk `afs1.audit()` after SIGKILL at every CREATE, WRITE commit-record, CLOSE and reply boundary. Test failure return paths, full table/disk and bounded memory/capability accounting; do not treat a reboot after a transient failure as a fix for a flake.
4. Keep all historical tests. At any **completed checkpoint**, run the full historical suite and a fresh final-image-bound 100/100 QEMU qualification, package and boot its own deployable archive and verify its receipt. A separately qualified partial read-boundary checkpoint does not close 8.1; no unqualified guest image is a deployable checkpoint.
