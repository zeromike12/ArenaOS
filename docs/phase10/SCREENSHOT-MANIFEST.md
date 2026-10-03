# Screenshot manifest

Status: no Phase-10 captures yet; gallery and application capture implementation
pending. Reference screenshots are presentation evidence, separate from semantic
pixel-region tests. Never use temporary theme RGB values as security or lifecycle
proof.

At qualification record: source commit, EFI SHA-256, guest resolution, fixture
seed/data, input sequence, QMP capture command, screenshot path and SHA-256 for
desktop, every app, gallery and significant interactive states.

## Host reference fixture, before guest integration

`python3 tools/test_m10_ui.py` renders each fixture twice and requires identical
bytes. Source is `userspace/ui/src/components/mod.rs`, with owned raster output
from `userspace/ui/examples/gallery.rs`. These are **host raster fixtures**,
not screenshots of a guest application and not the required QMP handoff proof.

| Fixture | Dimensions | SHA-256 |
|---|---|---|
| `build/phase10-host-gallery-light.ppm` | 448x288 | `dd27df16eae40c64bbe2509de71c1a16ba14620a96d358ee3685697d8ed97886` |
| `build/phase10-host-gallery-dark.ppm` | 448x288 | `6ab7b468ddab86a2d7fe4a8ecec4132ee4a371d01a7437a3df5660626c1d5b99` |

Structural checks compare component relationships, not particular RGB tokens.
