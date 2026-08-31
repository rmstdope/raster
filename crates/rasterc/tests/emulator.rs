use std::num::NonZeroU32;

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
