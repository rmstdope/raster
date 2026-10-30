//! The zombie picture example end to end: `examples/zombie/zombie.raster` built and run, and every
//! pixel of the frame compared with the picture `raster-assets` converts the PNG into.
//!
//! The compiler converts with the same function, so this does not judge the conversion, which
//! `raster-assets`' own tests and `split_picture.rs` do. It proves the rest: the tiles laid out
//! across both pattern tables, the nametable, attributes and palette loaded, and the example's
//! two `ppu.ctrl` switches landing on the right lines.

use std::{fs, io::Cursor, num::NonZeroU32};

use raster_assets::{
    convert_background_split, NesBackground, NAMETABLE_WIDTH_TILES, SPLIT_TILE_ROW,
};
use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::run;

#[path = "common/demo.rs"]
mod demo;
#[path = "common/scratch.rs"]
mod scratch;
use demo::zombie_demo_path;
use scratch::Scratch;

/// The NES entry the converted picture shows at (`x`, `y`): rows from the split down draw from
/// the second pattern table.
fn shown(background: &NesBackground, x: usize, y: usize) -> u16 {
    let mut tile = background.nametable()[(y / 8) * NAMETABLE_WIDTH_TILES + x / 8] as usize;
    if y / 8 >= SPLIT_TILE_ROW {
        tile += 256;
    }
    let bits = &background.chr()[tile * 16..tile * 16 + 16];
    let bit = 7 - x % 8;
    let value = ((bits[y % 8] >> bit) & 1) | (((bits[8 + y % 8] >> bit) & 1) << 1);
    let attribute = background.attributes()[(y / 32) * 8 + x / 32];
    let palette = (attribute >> (((y / 16) % 2) * 4 + ((x / 16) % 2) * 2)) & 3;
    u16::from(if value == 0 {
        background.palettes()[0]
    } else {
        background.palettes()[usize::from(palette * 4 + value)]
    })
}

fn build() -> Vec<u8> {
    let directory = Scratch::new("zombie-demo");
    let output = directory.path().join("zombie.nes");
    let (mut stdout, mut stderr) = (Cursor::new(Vec::new()), Cursor::new(Vec::new()));
    let result = run(
        vec![
            zombie_demo_path().display().to_string(),
            "-o".into(),
            output.display().to_string(),
        ],
        &mut stdout,
        &mut stderr,
    );
    let stderr = String::from_utf8(stderr.into_inner()).unwrap();
    assert_eq!(result, Ok(()), "{stderr}");
    assert_eq!(stderr, "", "no warnings");
    fs::read(&output).expect("the ROM is written")
}

#[test]
fn every_pixel_of_the_frame_is_the_converted_picture() {
    let png = fs::read(
        zombie_demo_path()
            .parent()
            .expect("the example is in a directory")
            .join("../assets/zombie-nes.png"),
    )
    .expect("the picture is readable");
    let picture = convert_background_split(png.as_slice()).expect("the picture converts");
    let rom = build();

    // Two frames in a row: the switch at line 120 lands a tile later in one than in the other.
    for frames in [10, 11] {
        let frame = render_after_frames(
            "zombie.nes",
            &rom,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        let entries = frame.as_indices();
        for y in 0..FRAME_HEIGHT {
            for x in 0..FRAME_WIDTH {
                assert_eq!(
                    entries[y * FRAME_WIDTH + x],
                    shown(&picture, x, y),
                    "frame {frames}, ({x}, {y})"
                );
            }
        }
    }
}
