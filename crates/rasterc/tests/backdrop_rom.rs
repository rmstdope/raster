//! The end of the pipeline: the ROM `rasterc` writes for the backdrop fixture is
//! run in an emulator, and the screen it produces is the colour the source asked for.

use std::{
    fs,
    io::Cursor,
    num::NonZeroU32,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::run;

/// NES colour `$12` as tetanes renders it — the blue the fixture writes to `$3F00`.
const EXPECTED_BACKDROP: [u8; 4] = [53, 51, 228, 255];

/// The palette write lands in the second frame, so a later frame is settled.
const SETTLED_FRAME: u32 = 10;

fn compile_backdrop_fixture() -> Vec<u8> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("backdrop.raster");
    let directory = std::env::temp_dir().join(format!(
        "rasterc-backdrop-rom-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&directory).expect("the scratch directory is creatable");
    let input = directory.join("backdrop.raster");
    fs::copy(&fixture, &input).expect("the fixture is copyable");

    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());
    let outcome = run(vec![input.display().to_string()], &mut stdout, &mut stderr);
    assert_eq!(
        outcome,
        Ok(()),
        "the fixture should compile: {}",
        String::from_utf8_lossy(&stderr.into_inner())
    );

    let rom =
        fs::read(directory.join("backdrop.nes")).expect("the ROM is written beside its source");
    fs::remove_dir_all(&directory).expect("the scratch directory is removable");
    rom
}

#[test]
fn compiled_backdrop_rom_reaches_expected_ppu_colour() {
    let rom = compile_backdrop_fixture();
    let frame = render_after_frames(
        "backdrop.nes",
        &rom,
        NonZeroU32::new(SETTLED_FRAME).expect("the settled frame number is non-zero"),
    )
    .expect("the compiled ROM should load and render");
    let pixels = frame.as_rgba();

    // Column zero is the emulator's first-dot artifact and carries no backdrop.
    for y in 0..FRAME_HEIGHT {
        for x in 1..FRAME_WIDTH {
            let offset = (y * FRAME_WIDTH + x) * 4;
            assert_eq!(
                &pixels[offset..offset + 4],
                EXPECTED_BACKDROP,
                "pixel ({x}, {y}) is not the backdrop the source asked for"
            );
        }
    }
}
