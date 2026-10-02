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
