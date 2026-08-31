use std::{fs, num::NonZeroU32, path::PathBuf};

use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::compile_source;

mod common;
use common::demo_source;

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
    let offset = (y * FRAME_WIDTH + x) * 4;
    pixels[offset..offset + 4]
        .try_into()
        .expect("four channels")
}

/// The one colour the whole screen shows, having asserted it is one colour.
/// The frame is an unfiltered palette lookup, so column 0 is the backdrop like
/// every other column and is checked with them.
fn backdrop_colour(name: &str, source: &str) -> [u8; 4] {
    let rom = compile_source(source).expect("the source compiles");
    let frame = render_after_frames(name, &rom.image, NonZeroU32::new(5).expect("five frames"))
        .expect("the ROM runs");
    let pixels = frame.as_rgba();

    let expected = pixel(pixels, 0, 0);
    for y in 0..FRAME_HEIGHT {
        for x in 0..FRAME_WIDTH {
            assert_eq!(
                pixel(pixels, x, y),
                expected,
                "{name}: pixel ({x}, {y}) is not the backdrop"
            );
        }
    }
    expected
}

#[test]
fn renders_the_backdrop_colour_the_source_asks_for() {
    let demo = demo_source();
    let lighter = demo.replace("const BACKDROP: u8 = $12", "const BACKDROP: u8 = $21");
    assert_ne!(demo, lighter, "the demo declares the backdrop as $12");

    let blue = backdrop_colour("demo.nes", &demo);
    let light_blue = backdrop_colour("lighter.nes", &lighter);

    // No RGBA constant is written down: the differential is what proves the
    // source's colour reaches the screen, rather than a palette entry the PPU
    // happened to be pointing at.
    assert_ne!(blue, light_blue);
}

/// The one palette entry the whole screen shows, having asserted it is one
/// entry. The entry is returned whole, emphasis bits included, so a frame that
/// unexpectedly set emphasis fails here rather than comparing equal.
fn backdrop_entry(name: &str, source: &str) -> u16 {
    let rom = compile_source(source).expect("the source compiles");
    let frame = render_after_frames(name, &rom.image, NonZeroU32::new(5).expect("five frames"))
        .expect("the ROM runs");
    let entries = frame.as_indices();

    let expected = entries[0];
    for y in 0..FRAME_HEIGHT {
        for x in 0..FRAME_WIDTH {
            assert_eq!(
                entries[y * FRAME_WIDTH + x],
                expected,
                "{name}: pixel ({x}, {y}) is not the backdrop entry"
            );
        }
    }
    expected
}

#[test]
fn renders_the_exact_nes_colour_the_source_names() {
    let demo = demo_source();
    let lighter = demo.replace("const BACKDROP: u8 = $12", "const BACKDROP: u8 = $21");
    assert_ne!(demo, lighter, "the demo declares the backdrop as $12");

    // No RGB constant anywhere: the source names an NES colour and the PPU is
    // asked what colour it emitted.
    assert_eq!(backdrop_entry("demo.nes", &demo), 0x12);
    assert_eq!(backdrop_entry("lighter.nes", &lighter), 0x21);
}

/// A `tests/cycles` fixture, read from the repository so a test can never drift from the source an
/// author actually compiles.
fn cycles_fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cycles")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()))
}

/// Where the picture changes going down it, and to what: one entry per band, from the top.
///
/// The NES palette entry, not a colour, so the fixture is judged against the entries its source
/// names rather than against whatever RGB an emulator chose for them. Column 128 is sampled rather
/// than the whole row, because a handler runs part-way along its scanline: the row it lands on
/// carries the old entry to the left of the write and the new one to its right.
fn bands(entries: &[u16]) -> Vec<(usize, u16)> {
    let mut bands = Vec::new();
    let mut previous = None;
    for y in 0..FRAME_HEIGHT {
        let entry = entries[y * FRAME_WIDTH + 128];
        if previous != Some(entry) {
            bands.push((y, entry));
            previous = Some(entry);
        }
    }
    bands
}

/// The bands each row of the picture is made of, in order, one entry per row.
///
/// A run shorter than eight pixels is ignored: a handler runs part-way along its scanline, so the
/// row it lands on carries the old entry, then the one dot the write straddles, then the new entry
/// — and that dot moves by one or two between frames, because a frame is 29780 CPU cycles and two
/// dots and only a whole pass of three is a whole number of them. Which bands a row is made of is
/// what stability means here, and it is exact.
fn row_bands(entries: &[u16]) -> Vec<Vec<u16>> {
    const SHORTEST_RUN: usize = 8;
    (0..FRAME_HEIGHT)
        .map(|y| {
            let row = &entries[y * FRAME_WIDTH..(y + 1) * FRAME_WIDTH];
            let mut bands = Vec::new();
            let mut run = (row[0], 0usize);
            for &entry in row {
                if entry == run.0 {
                    run.1 += 1;
                    continue;
                }
                if run.1 >= SHORTEST_RUN {
                    bands.push(run.0);
                }
                run = (entry, 1);
            }
            if run.1 >= SHORTEST_RUN {
                bands.push(run.0);
            }
            bands
        })
        .collect()
}

#[test]
fn timed_colour_bars_are_identical_across_consecutive_frames() {
    let rom = compile_source(&cycles_fixture("colour-bars.raster")).expect("the fixture compiles");
    let render = |frames: u32| {
        let frame = render_after_frames(
            "colour-bars.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        *frame.as_indices()
    };

    // The frame the source declares, in the entries it names: black to scanline 60, `$12` to 120,
    // `$21` to 180, black below. Nothing here is an RGB constant — the source names NES colours and
    // the PPU is asked which ones it emitted, and where.
    let first = render(8);
    assert_eq!(
        bands(&first),
        vec![(0, 0x0f), (60, 0x12), (120, 0x21), (180, 0x0f)],
        "the bars land on the scanlines the source names, in the colours it names"
    );

    // Six frames running, and one three hundred frames later. The distant one is the point: a loop
    // whose pass is a cycle longer than the picture slides a scanline every three seconds, which
    // five consecutive frames cannot see and five seconds of them cannot miss.
    for frames in [9, 10, 11, 12, 13, 300] {
        let later = render(frames);
        assert_eq!(
            bands(&later),
            bands(&first),
            "frame {frames} shows different bands from frame 8"
        );
        assert_eq!(
            row_bands(&later),
            row_bands(&first),
            "frame {frames} shows different bands in some row than frame 8"
        );
    }
}
