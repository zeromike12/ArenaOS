# Application contracts

Status: design constraints; final grants and guest proofs pending.

Every application is an ordinary userspace ELF launched through capability-gated
SYS_SPAWN. Presentation never authorizes an operation. The launch broker retains
exact child Process authority and associates the backing it allocated before
launch; clients receive only their backing and call endpoint for graphics.

Planned function grants:

| Client | Function authority | Required behavior |
|---|---|---|
| Terminal | Explicit filesystem and mediated launch access as implemented | Real command results, bounded input and scrollback |
| Files | Explicit fsd endpoint | Flat AFS1 list/read/create/delete; no invented folders |
| Editor | Explicit text read/save endpoint | Real bytes, cursor and explicit durable save errors |
| Settings | Specific appearance configuration access | Real running theme change and durable settings |
| Monitor | Read-only observations | Real processes/resources; labels only descriptive |
| UI Gallery | Own graphics only | Deterministic reusable components and states |

No graphical client receives Power, raw GPU/display/input, another client's
backing, unrelated Process destruction, ImageRegistrar or package installation
rights. Any implemented departure must be documented and reviewed before claim.
