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
