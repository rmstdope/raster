use std::{num::NonZeroU32, path::PathBuf};

use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::compile_source;

fn demo_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp/demo.raster");
    std::fs::read_to_string(path).expect("the demo example is readable")
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
    let offset = (y * FRAME_WIDTH + x) * 4;
    pixels[offset..offset + 4]
        .try_into()
        .expect("four channels")
}

/// The one colour the whole visible screen shows, having asserted it is one
/// colour. Column 0 is excluded: tetanes renders it black in every frame,
/// whatever the backdrop, so a whole-frame uniformity check fails on a correct
/// ROM.
fn backdrop_colour(name: &str, source: &str) -> [u8; 4] {
    let rom = compile_source(source).expect("the source compiles");
    let frame = render_after_frames(name, &rom.image, NonZeroU32::new(5).expect("five frames"))
        .expect("the ROM runs");
    let pixels = frame.as_rgba();

    let expected = pixel(pixels, 1, 0);
    for y in 0..FRAME_HEIGHT {
        for x in 1..FRAME_WIDTH {
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
