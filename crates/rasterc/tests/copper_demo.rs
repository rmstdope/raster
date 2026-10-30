//! The copper bar example end to end: `examples/copper/copper.raster` built the way an author builds
//! it and run, so the bar is judged line by line as the PPU drew it, on frames from every phase of
//! the timed pass and one long after.

use std::{
    fs,
    io::Cursor,
    num::NonZeroU32,
    sync::atomic::{AtomicUsize, Ordering},
};

use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::run;

#[path = "common/demo.rs"]
mod demo;
#[path = "common/scratch.rs"]
mod scratch;
use demo::copper_demo_path;
use scratch::Scratch;

const BACKDROP: u16 = 0x0f;

/// The bar from its top line down, in the NES colours the example loads: three lines of each
/// shade, four of white in the middle.
fn bar() -> Vec<u16> {
    let shades = [0x02, 0x12, 0x22, 0x32, 0x30, 0x32, 0x22, 0x12, 0x02];
    shades
        .iter()
        .flat_map(|&shade| vec![shade; if shade == 0x30 { 4 } else { 3 }])
        .collect()
}

/// Builds the example into a scratch directory; the summary, the diagnostics and the ROM.
fn build() -> (String, String, Vec<u8>) {
    static BUILDS: AtomicUsize = AtomicUsize::new(0);
    let directory = Scratch::new(&format!(
        "copper-demo-{}",
        BUILDS.fetch_add(1, Ordering::Relaxed)
    ));
    let output = directory.path().join("out.nes");
    let (mut stdout, mut stderr) = (Cursor::new(Vec::new()), Cursor::new(Vec::new()));
    let result = run(
        vec![
            copper_demo_path().display().to_string(),
            "-o".into(),
            output.display().to_string(),
        ],
        &mut stdout,
        &mut stderr,
    );
    let stderr = String::from_utf8(stderr.into_inner()).unwrap();
    assert_eq!(result, Ok(()), "{stderr}");
    (
        String::from_utf8(stdout.into_inner()).unwrap(),
        stderr,
        fs::read(&output).expect("the ROM is written"),
    )
}

/// Each line's colour after `frames` frames, having asserted every line is one colour from edge
/// to edge: a change that landed inside the picture would split its row.
fn lines(rom: &[u8], frames: u32) -> Vec<u16> {
    let frame = render_after_frames(
        "copper.nes",
        rom,
        NonZeroU32::new(frames).expect("at least one frame"),
    )
    .expect("the ROM runs");
    let entries = frame.as_indices();
    (0..FRAME_HEIGHT)
        .map(|y| {
            let row = &entries[y * FRAME_WIDTH..(y + 1) * FRAME_WIDTH];
            assert!(
                row.iter().all(|&entry| entry == row[0]),
                "frame {frames}: line {y} changes colour inside the picture: {row:02x?}"
            );
            row[0]
        })
        .collect()
}

/// The bar's top line, having asserted the bar is whole and alone on a black screen.
fn bar_top(lines: &[u16]) -> usize {
    let top = lines
        .iter()
        .position(|&entry| entry != BACKDROP)
        .expect("the bar is on screen");
    let bar = bar();
    assert_eq!(lines[top..top + bar.len()], bar[..], "{lines:02x?}");
    assert!(
        lines[top + bar.len()..]
            .iter()
            .all(|&entry| entry == BACKDROP),
        "nothing below the bar: {lines:02x?}"
    );
    top
}

#[test]
fn the_example_builds_with_a_handler_on_every_line() {
    let (stdout, stderr, _) = build();

    assert_eq!(stderr, "", "no warnings");
    let table = stdout
        .split("\n\n")
        .nth(1)
        .expect("a budget table follows the summary");
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines[0], "frame copper (using timed, mmc3)");
    assert_eq!(lines.len(), 3, "{table}");
    assert!(
        lines[1].starts_with("  scanline  0-238 every 1 handler"),
        "{table}"
    );
    assert!(
        lines[2].starts_with("  scanline  239           handler"),
        "{table}"
    );
}

#[test]
fn the_bar_is_set_line_by_line_and_swings() {
    let (_, _, rom) = build();

    // Three consecutive frames are the three phases of the timed pass, each with its own loop.
    let tops: Vec<usize> = [10, 11, 12, 40, 300]
        .into_iter()
        .map(|frames| bar_top(&lines(&rom, frames)))
        .collect();

    assert!(
        tops.windows(2).all(|pair| pair[0] != pair[1]),
        "the bar moves every frame: {tops:?}"
    );
    for top in tops {
        assert!((16..=196).contains(&top), "the bar's top is line {top}");
    }
}
