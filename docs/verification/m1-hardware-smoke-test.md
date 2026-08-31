# M1 hardware smoke test

This PAL-only check verifies the hand-built M1 ROM on a PAL NES using an
EverDrive N8 Pro. It provides hardware evidence for mapper and initialization
behaviour that an emulator cannot establish.

## Procedure

1. Run the full Rust gate:

   ```sh
   cargo fmt --check && cargo clippy -- -D warnings && cargo test
   ```

2. Generate the ROM outside the repository:

   ```sh
   cargo run -p raster-link --bin m1_solid_backdrop -- /tmp/m1-solid-backdrop.nes
   ```

3. Copy `/tmp/m1-solid-backdrop.nes` to an EverDrive N8 Pro, insert it into a
   PAL NES, and cold-boot the console.
4. Observe the display for 60 seconds. A pass is a uniform blue display with
   no reset loop, visual artifact, or unexpected audio during that interval.
5. On failure, record the observed symptom and withhold the
   hardware-proven conclusion.

## Accepted historical result

| Field | Value |
| --- | --- |
| Date | 2026-08-30 |
| Console revision | NES PAL |
| EverDrive firmware | 26.0625 |
| Display | UHD TV |
| ROM SHA-256 | Not recorded for the accepted historical run |
| Measured observation duration | Not recorded for the accepted historical run |
| Result | Pass (navigator accepted) |
| Observation | Everything looks ok |

The unrecorded checksum and measured duration make this historical evidence,
not a bit-for-bit reproducible artifact. It also predates the reset runtime
programming the MMC3 CHR bank registers, so the ROM it describes is not the ROM
this repository builds today. Nothing here is hardware evidence for the current
ROM.
