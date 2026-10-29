# raster-d5w — retrospective

- **Implementer:** Bishop
- **Date:** 2026-10-29
- **PR:** #92

## The bug bead's acceptance would have refused programs that display correctly

**What happened.** The bead asked for every loaded picture outside CHR bank 0, tiles 0–255, to be
refused, because "only CHR bank 0 is mapped, and only its first pattern table is selected". I
built exactly that, test-first, and the gate was green. The cold-read review then pointed at
`crates/raster-link/src/runtime.rs`: the reset map puts all 8 KiB of bank 0 at PPU
`$0000-$1FFF`, so a program that writes `ppu.ctrl` bit 4 shows tiles 256–511. Writes to
`mmc3.bank_data`, which `raster-ir` deliberately does not warn about for R0–R5, can map any other
bank. Both kinds of program built and displayed correctly before the fix and would have been
refused after it.

**Why.** Established. The bead was filed from a review finding on raster-fl4.5.3 that described
the default case (a program that neither selects nor maps the tiles) as if it were the only case,
and its acceptance carried that wording over. Neither the filing nor my reading of the bead
checked the claim against the reset map or the registers a program can write.

**Cost.** One rework of the fix, from a fixed refusal to one judged after lowering against
`ppu.ctrl` / `mmc3.bank_data` writes, plus two navigator questions and a second review.

**Prevent by.** In `fix-bug`, step 1 *Start from the bead's bug claim*: when the claim is that the
hardware "cannot" or "never" does something, check it against the code that sets up that hardware
(here `raster-link/src/runtime.rs`'s reset map, and which registers `raster-ir` lets a program
write) before writing the reproduction test.

**Seen before.** None found.
