# Application contracts

Status: implemented contracts; final engineering qualification is pending.

Every application is an ordinary userspace ELF launched through capability-gated
SYS_SPAWN. Presentation never authorizes an operation. The launch broker retains
exact child Process authority and associates the backing it allocated before
launch; clients receive only their backing and call endpoint for graphics.

Implemented function grants:

| Client | Function authority | Required behavior |
|---|---|---|
| Terminal | Scoped, mediated user-file and launch access as implemented | Real command results, bounded input and scrollback |
| Files | Scoped user-file and Editor launch mediation | Flat AFS1 list/read/create/delete; no invented folders |
| Editor | Scoped text read/save mediation | Real bytes, cursor and explicit durable save errors |
| Settings | Specific appearance configuration access | Real running theme change and durable settings |
| Monitor | Read-only observations | Real processes/resources; labels only descriptive |
| UI Gallery | Own graphics only | Deterministic reusable components and states |

No graphical client receives Power, raw GPU/display/input, another client's
backing, unrelated Process destruction, ImageRegistrar or package installation
rights. Any implemented departure must be documented and reviewed before claim.

The raw fsd endpoint is a trusted writer boundary (ADR-0048/0053). Ordinary
clients must not receive it: it can change system permission/configuration and
package records. The desktop needs a real enforcing file-service boundary, not
a filename filter in a GUI. Names select data within an explicitly granted
scope; names, app titles and window IDs do not confer that grant. Complete-file copy-on-write replacement and broker-enforced client mediation
are implemented (ADRs 0063 and 0065).

Appearance persistence must preserve configd's accepted single-key semantics.
A desktop preference store needs a separate scoped durable record or a reviewed
generalization; overwriting the Phase-8.1 key is not an implementation shortcut.

## Scoped broker service

The raw fsd endpoint remains only in the trusted broker. The broker provisions
function references as attenuated copies of the exact fresh session region,
with DESTROY explicitly delegated as the service-function marker. A graphical
READ|WRITE|COPY reference has no such bit and cannot amplify its rights.
Receiving code must independently require the marker, the operation's rights,
the explicitly provisioned resource scope, exact backing generation and the
held original Process witness's liveness. App names, startup view kind, titles,
PIDs and window handles cannot supply any of these checks.

Read functions require READ|COPY|DESTROY; write/configuration functions require
WRITE|COPY|DESTROY; launch functions require COPY|DESTROY. Scope is also checked:
a file grant cannot acquire configuration or launch scope by attenuation.
Public file operations are limited to actual flat names beginning `user-`.
Trust/package/configd records and `ui10-prefs` lie outside that resource scope.
The preferences grant accesses the dedicated appearance/motion record only.
The record is ordinary application data; configd's accepted single key remains
unchanged. Persisted PUT must succeed before applying a setting to running UI.

Each client also receives a separate private Notification/READ|WRITE for
monotonic pacing, inherited from one of six broker-held delegable clocks.
Process death sweeps its timers. Reuse must drain pending clock badges after
retirement. The production notification bound becomes 25, including the
compositor clock and six client clocks; full-fixture refusal tests must check
actual 25/25 occupancy and mutation-free twenty-sixth refusal.

Monitor may additionally receive MemoryPool/READ only. Additive SYS_OBSERVE
reports nine real scalar counts through this held reference. WRITE is still
required to create SharedRegions; READ grants no allocation, physical address,
Power, Process destruction or filesystem operation. Existing admin resource
snapshot semantics remain unchanged. Native startup audits and QMP Monitor tests exercise positive counts, wrong
capability kinds, reserved arguments, invalid output pointers and allocation
refusal. Complete historical/exact-image qualification is still pending.

## Exact client slots and bounds

Every built-in client receives slot 0 Endpoint/WRITE, slot 1 its fresh
SharedRegion/READ|WRITE|COPY, slot 2 an attenuated function reference to that
same region, and slot 3 its private Notification/READ|WRITE. Terminal's scope
is file read/write plus the six built-in launch targets. Files' trusted launch
target mask permits only Editor. Receiver enforcement checks the target mask
as well as the function rights and launch scope. Editor receives file read/write.
Settings receives preferences scope. Monitor and Gallery receive no function
scope. Slot 2 rights are 15 for file/launch roles, 14 for Settings and 5 for
Monitor/Gallery; Monitor alone receives MemoryPool/READ in slot 4. All other
slots are empty at spawn. Native startup audits precede window creation.

An ordinary signed dynamic Image-cap launch receives slots 0..3 only, no
function scope and no diagnostic pool. The broker chooses scope from trusted
launch descriptors; descriptive bootstrap metadata cannot choose a grant.
The signed graphical probe exercises this generic path with four independent
processes, forged/stale surface requests, exact pixels and live-code Image
revocation. APKG's existing 4096-byte payload ceiling is unchanged. Built-in
static ELFs are BootImage-backed; dynamic linking is absent.

Six desktop sessions may coexist; at most four unretired dynamic children may
exist system-wide. Each session has one 127-page backing (one I/O page and
126 pixel pages), one owned window up to 448x288 and a 32-event queue.
Close first delivers an owned event. Editor offers save/discard/cancel for
unsaved changes. A repeated close forcibly finishes the exact held Process.
Dead applications are swept and retired before their backing is released.

Files are flat AFS1 objects in the explicit `user-*` namespace. New uses CREATE
and refuses existing names. Save/Save As use complete transactional PUT. The
editor accepts ASCII printable text, spaces, tab and LF up to 4096 bytes;
unsupported content and full-buffer operations refuse without changing the
model. There are no directories, POSIX terminal semantics or ambient fsd caps.
Terminal supports help, echo, ls, cat, ps, put, rm, launch and clear with bounded
32x64 scrollback and a 64-byte input line. Settings persists light/dark and
motion choices in a checksummed 16-byte `ui10-prefs` application-data record.
