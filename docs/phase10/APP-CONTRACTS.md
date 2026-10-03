# Application contracts

Status: design constraints; final grants and guest proofs pending.

Every application is an ordinary userspace ELF launched through capability-gated
SYS_SPAWN. Presentation never authorizes an operation. The launch broker retains
exact child Process authority and associates the backing it allocated before
launch; clients receive only their backing and call endpoint for graphics.

Planned function grants:

| Client | Function authority | Required behavior |
|---|---|---|
| Terminal | Scoped, mediated user-file and launch access as implemented | Real command results, bounded input and scrollback |
| Files | Scoped user-file mediation | Flat AFS1 list/read/create/delete; no invented folders |
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
scope; names, app titles and window IDs do not confer that grant. General
filesystem replacement and client mediation remain unimplemented.

Appearance persistence must preserve configd's accepted single-key semantics.
A desktop preference store needs a separate scoped durable record or a reviewed
generalization; overwriting the Phase-8.1 key is not an implementation shortcut.

## Scoped broker service (implementation in progress)

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
snapshot semantics remain unchanged. This diagnostics path needs guest rights,
output/refusal and live-data qualification before being claimed complete.
