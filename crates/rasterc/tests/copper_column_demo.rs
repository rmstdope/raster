//! The copper bar behind a picture end to end: `examples/copper-column/copper-column.raster`
//! built and run. The picture's own pixels are the converted picture's, its black is the bar's
//! colour on that line, and the column on the right is the bar, swinging.

use std::{fs, io::Cursor, num::NonZeroU32};

use raster_assets::{convert_background_with, Conversion, NesBackground, NAMETABLE_WIDTH_TILES};
use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::run;

#[path = "common/demo.rs"]
mod demo;
#[path = "common/scratch.rs"]
mod scratch;
use demo::copper_column_demo_path;
use scratch::Scratch;

/// Where the picture ends and the column side begins.
const PICTURE_WIDTH: usize = 128;

/// The palette value the converted picture draws at (`x`, `y`), or `None` where it shows the
/// backdrop.
fn drawn(background: &NesBackground, x: usize, y: usize) -> Option<u16> {
    let tile = background.nametable()[(y / 8) * NAMETABLE_WIDTH_TILES + x / 8] as usize;
    let bits = &background.chr()[tile * 16..tile * 16 + 16];
    let bit = 7 - x % 8;
    let value = ((bits[y % 8] >> bit) & 1) | (((bits[8 + y % 8] >> bit) & 1) << 1);
    let attribute = background.attributes()[(y / 32) * 8 + x / 32];
    let palette = (attribute >> (((y / 16) % 2) * 4 + ((x / 16) % 2) * 2)) & 3;
    (value != 0).then(|| u16::from(background.palettes()[usize::from(palette * 4 + value)]))
}

/// The numbers of the example's `const NAME: [256]u8 = [...]`, read from the source.
fn table(source: &str, name: &str) -> Vec<u8> {
    let declaration = format!("const {name}: [256]u8 = [");
    let start = source.find(&declaration).expect("the table is declared") + declaration.len();
    let body = &source[start..];
    body[..body.find(']').expect("the table is closed")]
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| match entry.strip_prefix('$') {
            Some(hex) => u8::from_str_radix(hex, 16).unwrap(),
            None => entry.parse().unwrap(),
        })
        .collect()
}

fn build() -> Vec<u8> {
    let directory = Scratch::new("copper-column-demo");
    let output = directory.path().join("copper-column.nes");
    let (mut stdout, mut stderr) = (Cursor::new(Vec::new()), Cursor::new(Vec::new()));
    let result = run(
        vec![
            copper_column_demo_path().display().to_string(),
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

/// The line the bar's top is on in a frame whose right-hand column reads `column`, having asserted
/// the column is the bar and black everywhere else.
fn bar_top(column: &[u16], bar: &[u8], frames: u32) -> usize {
    let top = column
        .iter()
        .position(|&entry| entry != 0x0f)
        .unwrap_or_else(|| panic!("frame {frames}: the bar is somewhere in the column"));
    for (line, &entry) in column.iter().enumerate() {
        let expected = u16::from(bar[line.wrapping_sub(top) & 0xff]);
        assert_eq!(entry, expected, "frame {frames}: the column on line {line}");
    }
    top
}

#[test]
fn the_bar_swings_behind_the_picture_and_across_the_column() {
    let source = fs::read_to_string(copper_column_demo_path()).expect("the example is readable");
    let bar = table(&source, "BAR");
    let png = fs::read(
        copper_column_demo_path()
            .parent()
            .expect("the example is in a directory")
            .join("../assets/zombie-nes.png"),
    )
    .expect("the picture is readable");
    let picture = convert_background_with(
        png.as_slice(),
        Conversion {
            width: PICTURE_WIDTH,
            ..Conversion::default()
        },
    )
    .expect("the picture converts");
    let rom = build();

    let mut tops = Vec::new();
    for frames in [40, 41, 100] {
        let frame = render_after_frames(
            "copper-column.nes",
            &rom,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        let entries = frame.as_indices();
        let at = |x: usize, y: usize| entries[y * FRAME_WIDTH + x];

        // A line's handler writes the colour of the line after it while its own line is drawn, so
        // the right edge shows that colour, and the backdrop behind the next line is it.
        let column: Vec<u16> = (0..FRAME_HEIGHT).map(|y| at(FRAME_WIDTH - 1, y)).collect();
        tops.push(bar_top(&column, &bar, frames));

        for y in 0..FRAME_HEIGHT {
            let behind = if y == 0 { 0x0f } else { column[y - 1] };
            for x in 0..PICTURE_WIDTH {
                assert_eq!(
                    at(x, y),
                    drawn(&picture, x, y).unwrap_or(behind),
                    "frame {frames}, ({x}, {y}): the picture, or the bar behind its black"
                );
            }
            // Right of the picture the backdrop shows: this line's bar colour until the handler
            // writes the next one, inside the column, and that one after.
            for x in PICTURE_WIDTH..FRAME_WIDTH {
                assert!(
                    at(x, y) == behind || at(x, y) == column[y],
                    "frame {frames}, ({x}, {y}) shows ${:02X}, neither ${behind:02X} nor ${:02X}",
                    at(x, y),
                    column[y]
                );
            }
        }
    }
    assert_ne!(
        tops[0], tops[2],
        "the bar moves: line {} at frames 40 and 100",
        tops[0]
    );
}
