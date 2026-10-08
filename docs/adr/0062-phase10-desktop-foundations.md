# ADR-0062 — Phase-10 desktop foundations

Status: implemented engineering decision; full historical/exact-image qualification pending.

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

## Bounded concurrency

ADR-0064 replaces the existing one-unretired-dynamic-child rule with a finite scan
of structurally tagged spawn records under IF=0. Exited, unreaped children still
count. Only successful Process-cap FINISH releases an occupied record. Image
revocation affects future spawns, not already copied child pages. No change to
Image reference/pin hooks, storage-slot generations or registrar trust is needed.

The working set is five applications and the gallery. Six desktop sessions and
four unretired dynamic children are separately enforced. Six real apps use 762
backing pages plus scanout, 28 steady broker caps and 29 transient caps. Eight
SharedRegions, 2048 pages and 32 mappings remain unchanged. GOP800 and GPU800/640
full-working-set measurements are recorded in ENGINEERING.md.

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
production service adapter supplies held-cap verification. A pure host model is
not proof of IPC authority, process retirement or graphical guest behavior.

Host fixtures and independent-boot byte-identical QMP gallery captures establish
reusable presentation structure. test_m10_handoff produces app references. Structural
functional tests do not pin the Sol palette. Unsupported primitives remain
explicit in UI-CAPABILITIES.md and DESIGN-REQUESTS.md.

## Persistent format boundary

APKG v1 retains its 4096-byte payload and dynamic images their existing 16-page
load budget. BootImage applications can be normal statically linked SYS_SPAWN
children without changing that signed wire contract. General installation of
larger graphical applications requires a separately reviewed persistent-format
proposal; it cannot be smuggled into a desktop image or an unsigned package path.

AFS1 WRITE modifies existing sectors in place. An editor must not
advertise transactional replacement using that operation. Any replacement
extension needs data CoW, bounded preflight, metadata commit, rollback, reference
retention for recoverable older commits and actual crash/byte-level tests. Its
on-disk representation and invariants must be reviewed before implementation.
ADR-0063 implements complete CoW PUT without changing AFS1's disk format;
the editor uses PUT and actual crash/byte-level tests prove its boundary.

## Implemented service refinements

The ordinary app and signed dynamic paths demonstrate this topology in ring 3. The broker
receives an explicit BootImage/READ reference, Pool/WRITE, client Endpoint/READ,
delegable client Endpoint/WRITE|COPY, display Endpoint/WRITE, input comparator
and a private frame Notification/READ|WRITE. The ordinary launched gallery
receives only its endpoint caller and its own backing. Owned Process capability
retirement consumes the lifecycle handle; input/window policy remains userspace.

Additive TRY_RECV uses the existing receiver delivery and cancellation path;
empty work returns BUSY without publishing a kernel waiter. A private timer
notification drives idle cleanup and shared animations. Six pending app callers
plus the producer justify eight queued calls. The desktop and six reusable app
clocks justify 25 notifications; historical full-table tests fill 25 and refuse
the 26th without mutation. Native multi-child production mutations and dynamic
graphical ownership are exercised by Phase-10 guest gates.
