# Screenshot manifest

Qualified guest capture source: `11a45c34deae9a7a539a99fd7d3d6ea196ea6025`.
Exact EFI: `18357914fbfca7c7af87bd846c0f12f645e7f9a78d4816395d688b295ba90c7f`.
All 12 guest PNGs and the source-bound `screenshot-manifest.json` are preserved
in `releases/checkpoints/phase10-engineering-baseline/`. Captures used the
supplied shipping ESP without rebuilding; owned light/dark gallery crops
matched byte-for-byte across independent boots.

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

## Qualified PNG inventory

| Capture | Size | Deterministic owned golden | SHA-256 |
|---|---|---|---|
| [desktop-light.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/desktop-light.png) | 800x600 | no; live values | `ad13695214685489938be51c76bce5cd042bdb3fe030cbc82af8b42f10d57b88` |
| [terminal.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/terminal.png) | 800x600 | no; live values | `0d66160dfe4fb28e89783a3b93ae0bad4751916c2a9b6882f84a1002b7cf51d2` |
| [files.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/files.png) | 800x600 | no; live values | `6ae2c154279e8eda5c6cb0932209d7b76ad641ebdc629493c38267351b09c378` |
| [editor.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/editor.png) | 800x600 | no; live values | `f26eeddd6e13c948c8f6f00a20321102bff0594281296926d3eacce20db198c4` |
| [settings.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/settings.png) | 800x600 | no; live values | `a939b058775b8fa660246e230dc68b8af56bc90458ffb09c895295ab44ae9b0e` |
| [monitor.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/monitor.png) | 800x600 | no; live values | `571a9d01cdafce79f09844f7c20ad4d8724784e6cbb79ded09290e870a42e585` |
| [gallery.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/gallery.png) | 800x600 | no; live values | `f678554dfae8c14693cba3bdb3c8ce612619813f0e080c6c3fb1e669e3cb5401` |
| [gallery-owned-light.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/gallery-owned-light.png) | 448x288 | yes | `68fc2300b36538eaab37f4c1a80c7d617b1527833416def7193e3694ab5b186e` |
| [gallery-owned-dark.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/gallery-owned-dark.png) | 448x288 | yes | `c0762ffc57ee5dc02a92156af6c2c50cc8923bec2b5734574b79cf6c50128392` |
| [desktop-six-apps.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/desktop-six-apps.png) | 800x600 | no; live values | `ca0a52e4e2cb2c9eec58c926dbf31090e4f879662a52b64de0ddb0fb3d79fe8a` |
| [settings-dark.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/settings-dark.png) | 800x600 | no; live values | `6b5ac460b90d48716b87d9b5ca6b2c643bb44bcab5ded9968b5580eecc582d37` |
| [desktop-dark.png](../../releases/checkpoints/phase10-engineering-baseline/screenshots/desktop-dark.png) | 800x600 | no; live values | `dba07c55272533fcb4521fb17a82ef41686455eb428552842d5c1253d50ffcd7` |
