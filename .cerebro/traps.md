# Traps

The facts this project has already paid for, read by planners and implementers before they start.
One entry per trap: what happened, what it cost, and what to do about it. Empty until the first one.

## With rendering off, the PPU shows the palette entry its address points at

Writing the universal backdrop at `$3F00` leaves the PPU address register at
`$3F01`, because a `$2007` write auto-increments it. With rendering disabled the
PPU emits **the palette entry `v` points at**, not the backdrop — and `$3F01` is
zero on a cold console, which is grey.

Measured with `raster-emu` while planning `raster-6sl.5`: without a reset of the
address register, `$12`, `$21` and `$0F` all render `rgb(83,83,83)`. Pointing the
address out of palette space first (`ppu.addr = $00` twice) gives
`rgb(53,51,228)`, `rgb(92,160,255)` and black respectively.

**This is what made milestone 1's "shows a solid blue screen in Mesen2" untrue
for a year.** The ROM showed grey whatever colour its source wrote.

Any bead that writes a palette entry with rendering disabled meets this again.
The two `ppu.addr = $00` writes belong to the author, not to the compiler —
decided with the navigator, because a compiler that inserts PPU stores the source
did not ask for is a compiler whose cycle counts you cannot trust.

## The NTSC filter is a composite simulation, not the frame

`tetanes_core`'s `Config::filter` defaults to `VideoFilter::Ntsc`, which simulates
a CRT: it blends neighbouring pixels, its colours match no palette table, and it
forces column 0 to `rgb(0,0,0)` in every frame. `raster-emu` therefore asks for
`VideoFilter::Pixellate`, one RGBA pixel per PPU pixel straight from the palette,
and also exposes the raw palette entries.

Measured on the demo ROM's `$12` backdrop: filtered `rgb(53,51,228)` with a black
column 0; unfiltered `rgb(58,56,255)` in all 256 columns. Neither is the FCEUX
`rgb(0,0,188)` that `raster-assets` quantises source PNGs to, so a test must
compare palette entries and never an RGB constant.

## Hexadecimal is `$3f`, never `0x3f`

The lexer tokenises `0x3f` happily, and then `raster-sema` rejects it with
`invalid numeric literal`: `parse_number` handles only a `$` prefix and plain
decimal. Any fixture or example written with `0x` fails in a way that looks like
a parser bug and is not.
