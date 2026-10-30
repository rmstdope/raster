//! A picture of more tiles than one pattern table holds, split across both with `tiles: 512`,
//! built into a ROM that switches pattern tables halfway down the screen, and run: every pixel
//! of the frame is the picture's.

use std::num::NonZeroU32;

use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::{compile_source_with_assets, AssetSource};

const PATH: &str = "art/picture.png";

/// One file, by the name the source asks for.
struct Fixture(Vec<u8>);

impl AssetSource for Fixture {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        if path == PATH {
            Ok(self.0.clone())
        } else {
            Err(format!("no such file `{path}`"))
        }
    }
}

/// White noise on black, tile by tile: the upper 15 rows of tiles draw 200 noise tiles and the
/// lower 15 rows 200 others. 400 distinct tiles are too many for one pattern table, and each
/// half fits one. White is entry $20, and black is transparent, the backdrop $0F: the strict
/// importer reads opaque black as $0D (raster-bt8).
fn noise_halves() -> Vec<bool> {
    let mut seed = 0x1234_5679u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let noise: Vec<u64> = (0..400)
        .map(|_| u64::from(next()) << 32 | u64::from(next()))
        .collect();
    let mut white = vec![false; 256 * 240];
    for tile in 0..960usize {
        let bits = noise[usize::from(tile / 32 >= 15) * 200 + tile % 200];
        for bit in 0..64 {
            white[((tile / 32) * 8 + bit / 8) * 256 + (tile % 32) * 8 + bit % 8] =
                bits >> bit & 1 == 1;
        }
    }
    white
}

fn png(white: &[bool]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, 256, 240);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let bytes: Vec<u8> = white
        .iter()
        .flat_map(|&white| {
            if white {
                [0xf8, 0xf8, 0xf8, 0xff]
            } else {
                [0; 4]
            }
        })
        .collect();
    writer.write_image_data(&bytes).unwrap();
    drop(writer);
    output
}

/// A program showing the picture, imported with `fields`, whose frame is `frame`.
fn program(fields: &str, frame: &str) -> String {
    format!(
        "target nes {{ mapper: mmc3  region: ntsc }}\n\
         asset image picture = png(\"{PATH}\") {{ kind: background  {fields} }}\n\
         chrrom bank 0 {{ picture.tiles }}\n\
         main {{\n\
         \x20   load_palette(picture.palette)\n\
         \x20   load_nametable(picture.nametable, picture.attributes)\n\
         \x20   ppu.ctrl = $08   // background tiles 0-255, sprites 256-511\n\
         \x20   ppu.mask = $0a\n\
         }}\n\
         {frame}"
    )
}

/// Halfway down, the background takes the second pattern table and the sprites the first, so
/// that the MMC3 still sees A12 rise once a line; after the last line, at the post-render line
/// 240, they go back for the next frame.
const SWITCH_HALFWAY: &str = "frame halves using irq {\n\
                              \x20   at scanline 120 { ppu.ctrl = $10 }\n\
                              \x20   at scanline 240 { ppu.ctrl = $08 }\n\
                              }\n";

fn assert_shows(fields: &str) {
    let white = noise_halves();
    let rom = compile_source_with_assets(&program(fields, SWITCH_HALFWAY), &Fixture(png(&white)))
        .unwrap_or_else(|diagnostics| panic!("it compiles: {diagnostics:?}"));

    let frame = render_after_frames("split.nes", &rom.image, NonZeroU32::new(10).unwrap())
        .expect("the ROM runs");

    let shown = frame.as_indices();
    for y in 0..FRAME_HEIGHT {
        for x in 0..FRAME_WIDTH {
            let expected = if white[y * 256 + x] { 0x20 } else { 0x0f };
            assert_eq!(
                shown[y * FRAME_WIDTH + x],
                expected,
                "({x}, {y}) with `{fields}`"
            );
        }
    }
}

#[test]
fn a_split_picture_shows_whole_when_the_frame_switches_pattern_tables_halfway() {
    assert_shows("tiles: 512");
}

#[test]
fn a_converted_split_picture_shows_whole_too() {
    assert_shows("convert: true  tiles: 512");
}

#[test]
fn a_split_picture_in_a_program_that_never_selects_the_second_pattern_table_is_refused() {
    let found =
        compile_source_with_assets(&program("tiles: 512", ""), &Fixture(png(&noise_halves())))
            .expect_err("the lower half could never show");

    assert_eq!(
        found[0].message,
        "a picture at tiles 256–511 is not supported yet"
    );
}

#[test]
fn without_tiles_512_a_picture_of_400_tiles_is_refused() {
    let found =
        compile_source_with_assets(&program("", SWITCH_HALFWAY), &Fixture(png(&noise_halves())))
            .expect_err("one pattern table holds 256");

    assert_eq!(
        found[0].message,
        "`art/picture.png` needs 401 distinct tiles; a background allows 256"
    );
}
