# Phase 10 engineering record

Status: implementation in progress; no Phase-10 qualification claimed.

## Ordinary application checkpoint

ADR-0065 documents scoped function references, six private client pacing clocks
and MemoryPool/READ diagnostics. `tools/test_m10_apps.py` passes six actual
ordinary processes, pointer-driven launch/close, capacity refusal, terminal
command/file operations, Files create, Editor exact complete-file save and
unsaved-close cancel/discard, Settings durable theme/motion and a fresh reboot
whose pixels reflect the preference. Two additional full six-app cycles and
a gallery relaunch return to identical warmed frames/records/processes/shared
regions/pages/maps/caps. Cold-to-warm overhead is precisely six retained
intermediate page tables under the existing ADR-0056 unmap policy.

Workspace receipt: `/workspace/scratch/arena-apps-gate.log`. The observed cold
baseline in that build is `(115241,14,14,1,469,2,16)`; six sessions require 1231
shared pages including the 469-page GOP scanout, 14 maps and 28 steady broker
caps. Actual transient peaks and additional display modes remain to qualify.

The four-dynamic-child quota and checked-reply cancellation production mutants
both go RED. Restored exact EFI runs the full guest resource gate GREEN:
`e6ff0fe9439c57ed45d5015e31f98331b4f97f37ad02f1ead90aec19ed95516a`.
Receipts: `/workspace/scratch/arena-multi-red.log` and
`/workspace/scratch/arena-multi-exact-green.log`. Notification capacity now
includes six app clocks (25); those earlier receipts predate this addition.

This checkpoint is not the engineering baseline. Generic dynamic graphical
launch, adversarial/new-boundary controls, production boot integration,
additional UI workflows, historical/full gates, screenshots/handoff and
exact-EFI stability/archive proof remain outstanding.

Engineering branch: `arena/phase10-sol-engineering`.

Exact starting main: `491944da750888aa157070b2c580102d6a6dd9fc`.
Qualified Phase-9 parent: `832867e7511c5ba09051eb752c623fe87dd2d96c`.
Both commits have tree `764863b343a3297796a49b951e5826bfc6c2682b`.
Verified with `git fetch origin main`, `git merge-base --is-ancestor` and
`git show -s --format='%H %T %P %s'`. Main is a merge of the qualified parent.
The engineering branch was created locally and through the connected GitHub API.

## Source audit

- Production graphics service is `phase9-work/compositord-main.rs`; the crate
  under `userspace/compositord` supplies its policy, wire and rendering code.
- `kernel/kernel/src/spawn.rs` tags dynamic spawn records and refuses a second
  unretired dynamic child before record/process/frame reservation.
- Dynamic images retain two registry slots, 4096-byte ELF storage, a 16-page
  loadable-image budget, monotonic IDs, loader pins and exact reference hooks.
- SYS_SPAWN produces a held Process/READ|DESTROY capability. SYS_PROC_FINISH
  consumes it only after successful lifecycle retirement. No PID grants this.
- SharedRegion creation requires MemoryPool/WRITE; mapping and IPC reference
  ownership already have exact hooks. Eight regions, 2048 aggregate pages,
  512 pages/region and 32 mappings are the current kernel bounds.
- Graphics clients currently require root-installed matching Process/region
  comparator caps. Arbitrary spawned clients are deliberately refused.
- Inputd currently decodes US-ASCII keyboard input, feeds the serial shell's
  existing line discipline, and forwards printable keys with a ProofToken.
- fsd is flat AFS1: 32 objects, bounded names, append-only sector writes and
  transactional metadata/unlink. Safe existing-file replacement is absent.
- Configd uses eight immutable generations of up to 32 payload bytes. Its
  write endpoint requires a distinct receiving-service approval capability.
- Process listing is read-only and currently available without Power; resource
  snapshots require Power and cannot be granted to graphical apps as-is.

## Work order and evidence discipline

1. Establish reproducible host/guest build and rerun the Phase-9 graphics gate.
2. ADR and production/guest proofs for bounded multi-child ownership and
   general broker-provisioned graphical children; keep image ledger unchanged.
3. Real virtio pointer input, hit testing and userspace window policy.
4. Shared presentation toolkit, deterministic gallery, desktop and launch UI.
5. Ordinary terminal/files/editor/settings/monitor clients with documented
   least grants and real data; filesystem extensions need crash tests.
6. Historical suite, mutation controls, resource/capacity/lifecycle evidence,
   deterministic captures, exact-EFI 100 graphical boots, archive extraction.

No intermediate checkpoint is the qualified engineering baseline. The baseline
must include all requested applications and proof, then presentation work stops
for the Opus branch. See DESIGN-HANDOFF.md for the checkpoint contract.

## Environment

This workspace has no sudo or preinstalled Rust/QEMU. The pinned repository
bootstrap is being reproduced under `/workspace/scratch/arena-tools` rather than
changing host system directories. Git HTTPS clone/fetch works; HTTPS push lacks
credentials. Connected GitHub Git-data APIs are available for preservation.

## Initial reproduced evidence

With unchanged Phase-9 production source and the pinned toolchain:

- `tools/test_m9_compositor_input.py`: PASS, distinct owned guest surfaces and
  real QMP-injected key changed actual pixels.
- `tools/test_m85_resources.py`: PASS, baseline/live/retired resources
  `(114443,16,16) / (114429,17,17) / (114443,16,16)` for free frames,
  spawn records and process slots. Manager caps 25 baseline, 27 with old child,
  28 at two-image transition, 26 after finish and before/after four cycles.
- Existing Python FAT dependency needs `setuptools<81` for `pkg_resources` in
  this Python 3.12 workspace; fixed in workspace tooling, not guest code.
- The complete historical suite is being rerun before kernel mutation.

New source foundations (not running guest services): `userspace/ui` reference
palettes/metrics/components/motion and `userspace/desktop/src/model.rs` bounded
window policy. `tools/test_m10_ui.py` builds host and no_std targets, checks
fmt/clippy, exercises policy/motion and deterministic palette-independent
component structure. Host gallery fixtures are explicitly separate from QMP
handoff screenshots. ADR-0062 is proposed, not accepted production evidence.

## First linked desktop foundation (2026-10-03)

The original production source plus unlinked presentation tests completed the
entire historical gate: **80/80 suites**, including all prior mutation controls.
Receipt: `/workspace/scratch/arena-phase9-full.log` in the engineering workspace.
This predates the production changes below and is not Phase-10 qualification.

The linked Phase-10 broker now launches a real gallery via ordinary SYS_SPAWN,
allocates a distinct 127-page backing, retains the returned Process/READ|DESTROY
cap, checks backing generation plus held original-child liveness on every
request, and reaps/stops the child before releasing its region/mapping. App
capabilities are Endpoint/WRITE and SharedRegion/READ|WRITE|COPY. No raw input,
display, pool, Power, registrar or Process authority enters this child.

The prototype boot selects this path when two virtio-input functions are
present (keyboard and QEMU tablet). Single-keyboard historical fixtures retain
the original Phase-9 path. This is a temporary integration boundary, not a
claim of a complete production desktop. Production launch/bundle selection is
still pending. Both graphics services retain boot-root fail-stop supervision.

`SYS_IPC_TRY_RECV=42` is an additive, Endpoint/READ-gated operation. It shares
blocking receive's cap-delivery/cancellation code and returns BUSY on an empty
queue without parking. Explicit zero reserved arguments are required. The
broker combines it with an explicitly delegated private timer notification.
Queues are now eight deep (six application callers, one input producer, spare).
The notification budget is nineteen including this clock; full-fixture tests
now check actual 19/19 occupancy and mutation-free twentieth refusal. Spawn
records are provisionally 24, allowing the useful six-window working set plus
service-readiness transitions; the dynamic-child bound has not changed yet.

The one inputd process owns both actual MMIO caps and split queues. Discovery
matches the exact held BAR, both IRQs reach its existing notification, and
EV_ABS/button updates publish on SYN_REPORT. Desktop keystrokes bypass the
kernel console feeder; serial retains its original shell authority. The
producer token is root-issued and distinct from graphical client backings.

`python3 tools/test_m10_desktop.py` passes actual QMP dock spawn, owned client
pixels, keyboard palette change, exact content translation during title drag,
close to original desktop pixels and relaunch. Captures live under
`build/m10-desktop-*.ppm`. The initial framebuffer/type-width and syscall
reserved-argument bugs were caught by guest failure, fixed, and rerun GREEN.
This gate does not yet prove six-app capacity, dynamic Images or leak cycles.

## Complete-file replacement and dynamic-child concurrency

ADR-0063 adds named complete CoW PUT using the existing AFS1 format. Guest tests
cover create, replace, empty replacement, four crash checkpoints, exact file
bytes after remount, both valid commit generations and full-bitmap refusal with
the entire disk unchanged. A production mutant deliberately reused the old
first data sector: crash observation returned `new replacement with di` under
the old 23-byte metadata and went RED. Exact source and saved EFI/ESP were
restored, then the full guest gate ran GREEN against that exact EFI:
`30a5b713214296991d09d8dd4342b4fd6cd16d5f061e71387dcaae2ccea21523`.
This is an intermediate mutation receipt, not baseline qualification.

ADR-0064 sets four unretired dynamic children, preserving the two Image storage
slots, full-ID revocation checks, loader pins and Process-cap-only retirement.
Historical single-child BUSY probes now fill the additional three records,
allow actual ring-3 execution, refuse the fifth without changing held caps or
process/thread rows, and retire the extras. They run with live and exited but
unreaped original children and through repeated cutover cycles.

The first stress run found packaged fail-stopping on a reply to a canceled
caller during nested FS IPC. Additive checked reply distinguishes CALLER_GONE
from invalid reply usage and credits no response references on cancellation.
Packaged revokes/disposes an unsent provisional Image and continues receiving.
Legacy REPLY's cancellation contract remains unchanged.

The updated real cutover/resource gate passes: baseline/live/retired
`(114412,16,16) / (114398,17,17) / (114412,16,16)` for free frames, spawn records
and processes. Manager reported caps 25 baseline, 27 with original child,
28 with two Images, 26 after retirement and 26 before/after repeated cycles;
packaged actual descriptor-count peak is 8. The new LENT-frame descriptor
includes a previously unreported held buffer in these counts. Further
multi-child mutation and cross-parent dynamic graphical tests are still needed.

Pure terminal/editor models are present but unlinked. Host tests cover insertion,
deletion, navigation, capacity refusal, terminal line editing and bounded
scrollback. No guest terminal/editor functionality is claimed yet.
