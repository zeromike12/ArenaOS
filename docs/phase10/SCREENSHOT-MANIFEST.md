# Screenshot manifest

Current references: **Opus design pass**. The Sol engineering-baseline
inventory below is retained unchanged as history.

## Opus design capture

Design source: `cf311719e08d07cf765d7799524bce7067bd8419` (branch `arena/phase10-opus-design`).
Capture EFI (development build of that source, not a qualified image):
`d2b6068c8d4e6a48cc482e54cde30664e18fbd0a15f53e2614f6c0dfe0a8eed4`. Captured by `python3 tools/test_m10_handoff.py`
with the unchanged fixture: user-note text; canonical light/motion-disabled UI10 preference; fresh AFS1; cursor parked at 780,500. QMP screendump, settled owned raster; lossless RGB PNG.
The owned 448x288 gallery crops at 70,60 matched byte-for-byte across an
independent boot of the same seeded platter, in both palettes. PNGs and the
source-bound `screenshot-manifest.json` are preserved in
`releases/checkpoints/phase10-opus-design/`. Pixels changed intentionally:
this pass replaces the Sol reference skin; semantic suites were not
weakened (see OPUS-DESIGN-RETURN.md). Sol requalifies the post-design image.

| Capture | Size | Deterministic owned golden | SHA-256 |
|---|---|---|---|
| [desktop-light.png](../../releases/checkpoints/phase10-opus-design/screenshots/desktop-light.png) | 800x600 | no; live values | `45a0cb8eaf89e862d2f1c28af4a0ff64cd31ad2276e2dbe634db9a08838251a9` |
| [terminal.png](../../releases/checkpoints/phase10-opus-design/screenshots/terminal.png) | 800x600 | no; live values | `2bab4f05ef0414ede6b1921adb190dfb986ce39e77dc137b130a206b4de597f0` |
| [files.png](../../releases/checkpoints/phase10-opus-design/screenshots/files.png) | 800x600 | no; live values | `645fcd4bbaed4079996257b9aaaae5a34a75189e41b9bd36679bd3acffb76285` |
| [editor.png](../../releases/checkpoints/phase10-opus-design/screenshots/editor.png) | 800x600 | no; live values | `8451b921efaa1c707ecc9dcde71fba6136dda428152e69b5989b3d7ad2207998` |
| [settings.png](../../releases/checkpoints/phase10-opus-design/screenshots/settings.png) | 800x600 | no; live values | `bafc754ebd17e6634a81e3c7674121aebffd026e06fec0bfb1ce36ced6d6d25e` |
| [monitor.png](../../releases/checkpoints/phase10-opus-design/screenshots/monitor.png) | 800x600 | no; live values | `f680611323f42a7b44af9fba62ca5fb057a40855c69099946a19bad1bf6611bf` |
| [gallery.png](../../releases/checkpoints/phase10-opus-design/screenshots/gallery.png) | 800x600 | no; live values | `1772a84883fb1fd73055a90fb219ea7e1263010f8ea028bc2b62bf8d99a402be` |
| [gallery-owned-light.png](../../releases/checkpoints/phase10-opus-design/screenshots/gallery-owned-light.png) | 448x288 | yes | `1343a8eca755711e0d32ee57efb0c19d882339870ff26b34c7f7f7eaa948ba10` |
| [gallery-owned-dark.png](../../releases/checkpoints/phase10-opus-design/screenshots/gallery-owned-dark.png) | 448x288 | yes | `7e2cb2d8587c56152274c0925a031a461f37ab29e26b9299ec6d476dcccee549` |
| [desktop-six-apps.png](../../releases/checkpoints/phase10-opus-design/screenshots/desktop-six-apps.png) | 800x600 | no; live values | `e0fed21376e28922c2de58a48a175b4e9120f7765b5792dd4bd8beaf499cf227` |
| [settings-dark.png](../../releases/checkpoints/phase10-opus-design/screenshots/settings-dark.png) | 800x600 | no; live values | `ba654fee642a6aa0a7421151800fec0b038832c4e54acc0bc3acdf6a8e08e697` |
| [desktop-dark.png](../../releases/checkpoints/phase10-opus-design/screenshots/desktop-dark.png) | 800x600 | no; live values | `c071d9cb49ae9186e4c09d806faceb8b54ade9cffbb696c441a707b78532ac0d` |

The gallery now shows: focused/resting chrome swatches, button kinds and
states (normal, primary, focused, destructive, disabled, refused), focused/
resting/refused fields, sidebar rows (normal, selected, disabled, refused),
console sample, switches, progress and meter, status chips, the real Smooth
curve over OPEN_US, and the six application tiles. Menus and scrollbars
remain absent primitives.

# Sol engineering baseline (historical)

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
