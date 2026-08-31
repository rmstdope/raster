# Cycle fixtures

Raster sources whose timed regions have a known, hand-checkable cost. They are compiled by
`crates/rasterc/tests/cycles.rs`, which asserts the cost the compiler predicts, and are measured
against a real emulator by the timing-measurement bead's own test — the same fixtures, judged twice,
which is the point: a prediction only means something if something independent agrees with it.

`ppu.mask = 1` is the unit these are built from. It generates `LDA #$01` (two cycles) and
`STA $2001` (four), so it costs six.

A region's budget also pays for the interrupt masking the compiler puts around it — `PHP` 3, `SEI` 2
and `PLP` 4, nine cycles — unless the region is `interruptible`. That is what makes the annotation
the region's wall-clock cost, so a scanline loop does not drift.
