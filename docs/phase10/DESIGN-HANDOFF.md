# Phase 10 design handoff

Status: NOT READY. Phase-10 engineering is in progress. This is the handoff
contract, not a qualified baseline or authorization to change service internals.

The exact qualified baseline commit, artifact hashes, suite count and 100/100
receipt must be recorded here before the Opus design branch starts.

## Architecture and editing boundary

```mermaid
flowchart TD
    Root[Boot root: explicit capabilities] --> Display[displayd]
    Root --> Input[inputd: device authority]
    Root --> Desktop[userspace desktop / compositor / launch broker]
    Input -->|authenticated bounded input| Desktop
    Desktop -->|typed frame publication| Display
    Desktop -->|own backing + endpoint + function grants| App[ordinary userspace application]
    App -->|owned surface + bounded events| Desktop
    App -->|explicit function capability| Services[fsd / configuration / read-only diagnostics]
    Desktop -->|held exact Process cap| Life[liveness and stop / retire]
```

Names, PIDs, window handles and visual placement never confer authority.
Presentation must remain in `userspace/ui` and app view/layout files. Opus may
edit tokens, components and presentation after the qualified checkpoint. Kernel,
IPC, lifecycle, fsd/configd/packaged, input transport and display architecture
require engineering review. Record missing primitives in DESIGN-REQUESTS.md.

## Required checkpoint inventory

- Exact Sol baseline commit and branch; diagram updated to implemented topology.
- Every app's screenshot and deterministic gallery capture with hashes.
- Drawing primitives, XRGB/framebuffer details and alpha support status.
- Font, icon/image, pointer, keyboard and window-management capabilities.
- Monotonic motion primitives, frame scheduling and preference behavior.
- Measured surface/owner/region/process/cap limits and supported resolutions.
- Token paths, presentation files safe to edit, protected mechanism files.
- Known visual limitations; UI-only test commands and screenshot instructions.
- Full qualification receipt and independently extracted graphical bundle proof.

No screenshot or Phase-10 capability is claimed by this initial contract.
