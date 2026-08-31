# Cycle fixtures

Raster sources whose timed regions have a known, hand-checkable cost. They are compiled by
`crates/rasterc/tests/cycles.rs`, which asserts both the cost the compiler predicts *and* the cost
a 6502 spends running the ROM it produced — the same fixtures, judged twice, which is the point: a
prediction only means something if something independent agrees with it.

The measurement comes from `raster_emu::cycles_between`, which counts CPU cycles between two
opcodes rather than two addresses, so nothing in the emulator crate has to know where the compiler
put its code. `PHP` to `PLP` is the bracket every timed region carries, and therefore the window
the timing analysis judged.

`delay.raster` carries no timed region and so no bracket of its own; `delay-bracketed.raster` is
the same delay between two `cycles(?)` regions, measured from the `PLP` that closes the first to
the `PHP` that opens the second.

`ppu.mask = 1` is the unit these are built from. It generates `LDA #$01` (two cycles) and
`STA $2001` (four), so it costs six.

A region's budget also pays for the interrupt masking the compiler puts around it — `PHP` 3, `SEI` 2
and `PLP` 4, nine cycles — unless the region is `interruptible`. That is what makes the annotation
the region's wall-clock cost, so a scanline loop does not drift.

A prediction is a **worst case**: `worst_case_cycles` charges every branch as taken and
page-crossing, because a flat instruction sequence cannot prove otherwise. The fixtures measured
for exact agreement are all straight-line, which is not a convenience — the compiler refuses a
branch or a `wait` inside a timed region precisely so that a region's cost is provable. The one
branch a prediction has to cover is the `BEQ` closing a delay loop, and the compiler reserves it
one cycle for crossing a page.
