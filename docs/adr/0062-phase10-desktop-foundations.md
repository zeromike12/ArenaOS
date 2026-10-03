# ADR-0062 — Phase-10 desktop foundations

Status: proposed; implementation and guest qualification pending.

## Authority and ownership

The userspace desktop/compositor acts as a bounded launch broker. A launched
application receives the exact SharedRegion allocated by the broker and a
call-side compositor endpoint by existing SYS_SPAWN attenuation. The broker
retains the SYS_SPAWN-created Process/READ|DESTROY capability and its own backing
reference. It checks these held associations, not caller PID, app name, package
ID, title, window handle or position. A presentation handle never authorizes
mapping, focus, destruction or filesystem access.

This generalizes provisioning to ordinary child spawns, including an explicitly
delegated dynamic Image bearer. ImageRegistrar, raw display/input, SharedDma,
Power and unrelated Process authority are never inherited by graphical clients.
Function-specific filesystem/configuration/diagnostic authority is explicit in
APP-CONTRACTS.md. Metadata alone cannot substitute for a function capability.

The broker has a finite application table. Creation preflights a free application
slot, a region/cap reservation and the inheritance grant list before publishing
any window. Every failed launch rolls back its owned temporary backing and all
child state. A held Process capability supplies liveness. Retirement stops/reaps
that exact child, releases mappings, backing caps, focus, drag capture and queued
input before reusing a slot. Stale Process and window generations never revive.
Death of display/compositor remains fail-stop; live service restart is excluded.

## Concurrency decision still to measure

The existing one-unretired-dynamic-child rule must be replaced by a finite scan
of structurally tagged spawn records under IF=0. Exited, unreaped children still
count. Only successful Process-cap FINISH releases an occupied record. Image
revocation affects future spawns, not already copied child pages. No change to
Image reference/pin hooks, storage-slot generations or registrar trust is needed.

The useful desktop working set is five applications (terminal, files, editor,
settings, monitor) and a gallery fixture. The final bound must be selected from
actual full-fixture resource measurements, including transitional cap pressure
and service readiness workers, rather than deleting the guard or guessing a
large limit. Current Phase-9 guest measurements in this workspace:

- manager baseline 25 caps; old child 27; two live image IDs 28;
- after retirement 26 caps; four serial STOP/FINISH cycles return to 26;
- live old child costs exactly 14 frames and one process/spawn record;
- baseline 16 processes and 16 spawn records, 114443 free frames;
- packaged measured cap high-water 7; notification table 18/18.

These are Phase-9 observations, not Phase-10 capacity qualification.

Historical second-child refusal tests must be explicitly superseded by tests
that fill the new bound, refuse without changing frames/caps/records/processes,
retain refusal for exited-unreaped records, then retire/relaunch. Test multiple
Image IDs, revocation with existing children, stale Process witnesses and owner
service death. Production mutations must prove relevant guest tests go RED,
then restore exact source/artifact and prove GREEN.

## Input and window policy

Use the existing virtio-input transport and root-issued producer witness.
A client receives bounded routed events, never the raw input capability.
The driver must support a real QEMU pointer event queue. Screen coordinates,
button state and keyboard navigation cross a neutral typed wire. The compositor
verifies the input capability before hit testing or routing.

Native policy is userspace: half-open clipped hit regions, highest-z hit,
focus-on-click, activation, title-bar drag capture, pointer release, close,
client-relative pointer coordinates, bounded queues and accessible screen edges.
Window titles are descriptive metadata. Minimize/maximize are only advertised
if their production behavior and cleanup are tested.

## Presentation boundary

`userspace/ui` contains palettes, geometry, components and integer monotonic
motion. App models/service adapters do not embed literal colors or durations.
`userspace/desktop/src/model.rs` implements authority-free window policy; the
future service adapter must supply held-cap verification. A pure host model is
not proof of IPC authority, process retirement or graphical guest behavior.

Deterministic host gallery fixtures establish reusable presentation structure;
QMP gallery and app captures remain mandatory for the design handoff. Structural
functional tests do not pin the Sol palette. Unsupported primitives remain
explicit in UI-CAPABILITIES.md and DESIGN-REQUESTS.md.

## Persistent format boundary

APKG v1 retains its 4096-byte payload and dynamic images their existing 16-page
load budget. BootImage applications can be normal statically linked SYS_SPAWN
children without changing that signed wire contract. General installation of
larger graphical applications requires a separately reviewed persistent-format
proposal; it cannot be smuggled into a desktop image or an unsigned package path.

AFS1 WRITE currently modifies existing sectors in place. An editor must not
advertise transactional replacement using that operation. Any replacement
extension needs data CoW, bounded preflight, metadata commit, rollback, reference
retention for recoverable older commits and actual crash/byte-level tests. Its
on-disk representation and invariants must be reviewed before implementation.
