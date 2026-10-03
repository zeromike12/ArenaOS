# UI capabilities

Status: Phase-9 source audit; Phase-10 implementation pending.

Available today: opaque XRGB8888 pixels, clipped rectangles and blits, an owned
5x7 bitmap font with six-pixel advance, bounded text, surfaces, z-order and
printable keyboard focus/routing. No alpha blending, vector paths, image decoder,
scalable fonts, pointer routing or live compositor restart is claimed.

Phase-9 limits: 4 owners, 6 surfaces, 320x200 client surfaces. Those values are
qualification policy and will be replaced only with measured desktop bounds.
The existing graphical guest gate uses 800x600. Larger resolutions must be
explicitly verified against region/page limits before this file lists them.

This inventory will record exact Phase-10 primitives and UI-only validation
commands at the qualified design checkpoint.

## Presentation foundations added, pending guest integration

Tokens: `userspace/ui/src/theme.rs`, `metrics.rs`, `motion.rs`. Components:
`userspace/ui/src/components/mod.rs`. The host gallery covers buttons (normal,
focused, disabled and refused), fields/cursor, plain/selected/disabled rows,
sidebar, geometric icons, progress and terminal styling. Integer motion supports
linear/smooth interpolation, retargeting from the sampled state, explicit zero
duration and bounded next-frame deadlines. The service must still implement
real frame scheduling and guest animation proof.

`python3 tools/test_m10_ui.py` checks host policy/motion/wire tests, formatting,
Clippy and bare-metal library builds. Gallery ELF is built separately from
`userspace/desktop` with `cargo build --offline --locked --release`; it is an
ordinary static ET_EXEC, currently unlinked to the production boot. Its measured
26,088-byte artifact exceeds APKG v1's 4096-byte ceiling. No package-format change
or successful guest launch is claimed.
