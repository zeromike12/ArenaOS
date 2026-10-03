# Phase 10 engineering record

Status: implementation in progress; no Phase-10 qualification claimed.

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
