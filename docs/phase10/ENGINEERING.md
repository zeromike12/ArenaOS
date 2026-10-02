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
