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

Two fixtures are compiled to be **refused** rather than measured: `over-budget.raster`, whose
block costs more than its budget, and `return-in-a-block.raster`, whose block leaves before it has
spent one. Their tests assert the diagnostic rather than a cycle count, because there is no ROM.

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

**Two of the penalties a prediction charges are charged and unverified, and nothing here proves
them.** `delay-bracketed.raster`'s loop happens to sit within one page, so the reserved cycle is
never spent and the measurement's upper bound is never reached; forcing that branch across a page
needs alignment control the linker cannot yet express. And no `.raster` source can reach the
indexed-read page penalty at all, because under the legal ISA `raster-codegen` emits no indexed
addressing mode. Both are charged by `worst_case_cycles` and measured by nothing — a source
construct that reaches either one is the point at which a fixture becomes writable.

## Frame fixtures

`colour-bars.raster`, `frame-schedule.raster` and `frame-over-budget.raster` are the
`frame ... using timed` ones. Their handlers are placed by `raster_timing::plan_timed_frame`, which
pads each to a whole scanline where the next is far enough away and to the exact distance to the
next where it is not — 114, 113, 114, so three lines are 341 cycles and nothing accumulates.

`colour-bars.raster` is judged in the emulator by `crates/rasterc/tests/emulator.rs`: the bars have
to land on the scanlines the source names and stay there three hundred frames later. A loop that
spends one cycle more than the picture slides a scanline every three seconds, which five
consecutive frames cannot see.

`irq-colour-bars.raster` and `irq-hblank-window.raster` are the `frame ... using irq` ones, and
both are judged in the emulator rather than counted. The first places three bands and holds them
three hundred frames later; the second is made of the widest handler body the language can express
inside `raster_timing::IRQ_HANDLER_BODY_CYCLES` — seven cycles, a register store read from a
variable — and proves that a body the compiler accepts finishes inside the hblank the MMC3 leaves
it. Every handler in both stores a value the mask does not already hold, which is what makes the
picture evidence: a store of the value already there paints nothing, and the row would read as one
band whether it landed inside the window or half-way along the line.

`unsynchronized.raster` is the negative of spec section 6.6: a timed region writing a PPU register
with nothing to align it. Inside a `frame` the same region is fine, because the frame emits the
synchronization itself.
