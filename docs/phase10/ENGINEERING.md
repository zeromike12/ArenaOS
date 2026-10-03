# Phase 10 engineering record

Status: functional implementation and focused proofs complete; final historical,
exact-image 100/100 and independently extracted archive qualification pending.
Sol supplies the reference skin; Opus owns the subsequent visual design branch.
No intermediate receipt is the engineering baseline.

## Provenance

Branch: `arena/phase10-sol-engineering`. Fetched starting main:
`491944da750888aa157070b2c580102d6a6dd9fc`, containing qualified Phase-9 parent
`832867e7511c5ba09051eb752c623fe87dd2d96c`. Both have tree
`764863b343a3297796a49b951e5826bfc6c2682b`; ancestry/tree equality were verified
before creating the engineering branch. Main receives no Phase-10 changes.
The unchanged Phase-9 source plus unlinked UI foundations passed 80/80 suites
(`/workspace/scratch/arena-phase9-full.log`), before production changes.

## Implemented decisions

| ADR | Mechanism |
|---|---|
| 0062 | Ordinary graphical spawn, held-cap ownership, userspace pointer/window policy, shared presentation |
| 0063 | Complete AFS1 CoW PUT with unchanged disk format and ordered-prefix crash proof |
| 0064 | Four unretired dynamic children, exact Process ownership, checked canceled-call reply |
| 0065 | Explicit function scopes/launch targets, six reusable private clocks, Pool/READ diagnostics |
| 0066 | Generic signed Image bearer launch, mutation-free spawn preflight, explicit production profile |
| 0067 | Private complete owned-frame snapshots; unpublished staging remains invisible |
| 0068 | Bounded clock measurement sampling and unconditional diagnostic Process cleanup |

The broker binds each fresh backing to its original Process before serving
requests. Backing generation, function rights, trusted resource scope and held
Process liveness authorize operations. Names, startup view kind, PIDs, titles
and surface IDs are descriptive. A static multicall ELF selects six ordinary
application models; trusted launch descriptors independently provision grants.
Generic signed dynamic apps receive graphics and pacing only. APKG's 4096-byte
payload and the accepted Image ledger remain intact.

The native dynamic roster scans under IF=0, counts exited-unreaped children and
releases records only after authorized FINISH. Image liveness precedes quota;
loader pins, revocation and stale generations retain their accepted semantics.
Four signed graphical processes and fifth refusal use actual package/Image/
process/IPC execution. Six desktop sessions are a separate bound.

Inputd owns the existing virtio keyboard and QEMU tablet queues. A root-issued
producer witness authenticates events; clients get bounded local key/button
state. Userspace supplies highest-z hit testing, focus, title drag, close,
clipped screen edges and pointer capture. F1..F6 launch, F7 cycles focus, F8
closes. The independent serial shell retains its existing authority.

The persistent desktop has an Arena-drawn background, active-app top bar, real
monotonic uptime, centered dock, actual launch/refusal state and bounded open/
close/focus motion. Shared tokens/components, pure views/layout and monotonic
motion separate presentation from IPC, lifecycle and filesystem mechanisms.

## Ordinary applications

Terminal has real help/echo/ls/cat/ps/put/rm/launch/clear, 64-byte line editing
and 32x64 scrollback. Files lists actual flat user-files, selects/reads, creates
without overwrite, deletes and launches an ordinary Editor. Editor supports
4096-byte ASCII documents, insertion/deletion/navigation, Open/New/Save/Save As
and save/discard/cancel on dirty close. Complete CoW PUT supplies durable save.
Settings persists light/dark/motion using a checksummed 16-byte application-data
record before live application. Monitor displays real process/thread rows and
native counters with read-only diagnostics. Gallery exposes supported controls,
states and shared motion samples. APP-CONTRACTS.md records exact grants/bounds.

## Focused proof and resources

Production mutations cover function scope, Files launch targets, pointer hit,
actual SYS_SPAWN, Process retirement, diagnostic attenuation, owner/stale
surfaces, generation reuse, frame publication, capacity preflight, dock-safe
title movement and live appearance delivery to all six clients. Separate
native controls break the dynamic quota, checked-reply cancellation and data
CoW. Mutants must compile and fail real guest oracles. Exact source/EFI/ESP
restoration precedes GREEN. The full suite reruns and preserves this evidence.
Native controls also reject early busy-wait return and persistent overshoot,
and prove that a bad signed-child receipt still returns Process/frame/record
counts to baseline. The first discovery run's 90/94 receipt remains preserved;
it is a failed attempt, not qualification evidence.
The checksummed `releases/checkpoints/phase10-discovery-1/` archive preserves
that complete run and its failed receipts and is included in the final bundle.
The second run reproduced the native/graphical controls and the previously
failing historical configuration-read gate, but exposed a signed-app snapshot
timing error and was interrupted before completing every historical suite.
`releases/checkpoints/phase10-discovery-2/` preserves that invalid attempt.
The corrected test waits for an actual fresh exact teardown sample before
shutdown, retaining the native leak oracle. Production cleanup is unchanged.

Unexpected client death retires its Process/backing/caps/maps while sibling
pixels/input survive. A live full-queue client can be force-closed by repeating
Close. Actual compositor death answers blocked input then root fail-stops;
there is no service-restart claim.

Six GOP800 clients use 20 processes/records, seven regions, 1231 shared pages,
14 mappings and 28 steady broker caps; incoming/spawn high-water is 29.
GPU800 uses eight regions, 1233 pages and 15 maps; GPU640 uses eight regions,
1064 pages and 15 maps. Full-suite receipts record frozen-source frame minima.
Client backings cost 762 pages; scanout costs 469 at 800x600 or 300 at 640x480.
GPU adds two SharedDma pages and a private pixel DMA buffer. Private complete
snapshots reserve another 756 broker pages. Kernel limits remain eight regions,
2048 pages, 512 pages/region, 32 maps and 32 cap slots/process. Spawn records are
24, processes 32 and notifications 25.

SharedUnmap retains empty intermediate page tables until address-space teardown
(ADR-0056). Warming the six fixed slots retains exactly six frames. Two further
complete working-set cycles and relaunch return identical warmed frames,
records/processes/regions/pages/maps/caps. This resident VM overhead is distinct
from application allocation leaks.

## Bugs caught by actual execution

- A late manager grant hit an occupied worker cap slot; reserved slot 31 fixes
  the refusal without broadening package authority.
- Canceled nested FS calls could fail-stop packaged; checked reply consumes the
  tombstone and disposes an unsent provisional Image.
- Files New used replacement; CREATE now refuses duplicates and actual bytes
  prove the existing document survives.
- Six-device discovery missed the GPU in the seven-device shipping topology;
  bounded discovery includes all seven and actual GPU matrix tests pass.
- Direct client staging reads exposed unpublished drawing; private publication
  fixes it. The RED oracle also proves visible cursor redraws because identical
  QEMU pointer coordinates can coalesce.
- Full-queue Close lost force-close intent; pending lifecycle state now survives
  independently of event delivery.
- QMP can sample a GOP copy in progress; settled owned captures prevent a mixed
  scanout from masquerading as submitted content. No vsync is claimed.
- A small title could slide behind the dock; creation and drag now retain it
  above the dock, with exact signed-app chrome/close proof at the bottom edge.
- A diagnostic child receipt short-circuited FINISH on failure; the original
  Process witness is now retired before checking the combined result.
- Clock upper-bound measurement could include host descheduling; bounded
  three-window sampling retains strict early-return and persistent-stall gates.
- Host workflows could observe the first frame before its initial accounting
  record completed. Fixtures now wait for native baseline readiness and consume
  only newline-complete serial records; every possible byte split is tested.
  The invalid partial run is preserved in `phase10-discovery-3`, alongside the
  earlier discovery archives. No partial run qualifies the baseline.

## Qualification and handoff

Freeze/push source, run `tools/run_tests.sh`, build shipping Desktop with
`tools/build.sh --image`, capture the handoff fixture with that supplied ESP
(no rebuild), then run `tools/stability_loop.sh 100` against that exact EFI.
The linker embeds a real PE timestamp; rebuilding afterward changes the EFI
hash even with identical source. Every boot checks real app spawn, keyboard pixels, tablet drag,
Process/resource retirement, relaunch and serial shutdown, alongside historical
boot/device/network verdicts. Any failure restarts the attempt at zero.

`tools/test_m10_handoff.py` captures all apps and proves byte-identical owned
light/dark gallery rasters across independent boots. The bundle tool requires
clean source-bound suite evidence, matching EFI/ESP/100 receipt and screenshot
metadata, then independently extracts and graphically boots using bundled
firmware/tools and standard-library Python. Final hashes and receipt values go
in DESIGN-HANDOFF.md and the checkpoint QUALIFICATION.json.

Michael's MANUAL-SMOKE.md steps are supplementary; no human session is claimed
by automated results. UI-CAPABILITIES.md and DESIGN-REQUESTS.md expose limits
and the review path for missing presentation primitives.
