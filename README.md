# raster
A high level language for writing timing critical NES ROMs.

## The MVP demo

`examples/mvp/demo.raster` shows a picture imported from a PNG, with the
background tinted red from line 60, green from line 120, and back to plain
from line 180.

    rasterc examples/mvp/demo.raster

Open `examples/mvp/demo.nes` in Mesen2. Each tint starts on its yellow mark
on the ruler down the left edge; the build prints how many cycles each bar's
handler has to spare.

On a PAL console the red and green tints swap places: the
console wires those two bits the other way round.
