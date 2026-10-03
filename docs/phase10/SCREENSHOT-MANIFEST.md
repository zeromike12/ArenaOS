# Screenshot manifest

Actual guest capture is implemented and independently repeatable. Final frozen
source/EFI hashes and PNG inventory are preserved in the qualified checkpoint
`screenshot-manifest.json`; prequalification captures are not baseline receipts.

Run `python3 tools/test_m10_handoff.py` in the pinned development environment.
It emits `build/phase10-screenshots/` containing desktop-light/dark, all six apps,
settings-dark, desktop-six-apps and gallery-owned-light/dark PNGs plus manifest.
PNG encoding is lossless RGB using standard-library zlib; pixels come from real
QMP screendump. No serial string stands in for graphical evidence.

Fixture: 800x600 GOP, fresh AFS1, canonical light/motion-disabled UI10 record,
`user-note` containing two known ASCII lines, actual dock launches, cursor parked
at 780,500. Terminal receives real echo/help keys; Files selects the real text;
Editor opens its actual bytes; Settings changes the real persisted theme.
Every app is closed through its held Process lifecycle. The full working set
capture has six independent actual processes.

Owned gallery captures crop 448x288 at 70,60 after the raster settles. A second
independent boot of the same seeded platter must produce identical light and
local-dark gallery bytes. These are deterministic reference goldens. Fullscreen
references include real uptime/monitor values and are checksummed evidence,
not a promise of identical entire-frame bytes on every boot. Functional tests
compare semantic regions/process/resources independently of the Sol palette.

The gallery displays chrome, title, labels, buttons, fields, rows/selection,
sidebar, toolbar/icon controls, focus/unfocus, disabled/refused, progress/status,
terminal and shared motion samples. Menus and scrollbars are absent primitives.

Host fixture command: `python3 tools/test_m10_ui.py`. Host PPMs under build are
supplementary component proofs and never replace the guest screenshot inventory.
