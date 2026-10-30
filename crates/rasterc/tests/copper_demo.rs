//! The copper bar example end to end: `examples/copper/copper.raster` built the way an author builds
//! it and run, so the bar is judged where the PPU drew it on two different frames.

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
const DARKEST: u16 = 0x06;
const BRIGHTEST: u16 = 0x30;
/// Lines of bar in `art/bar.png`.
const BAR_HEIGHT: usize = 48;

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

/// The palette entry of each line down the middle column, after `frames` frames.
fn column(rom: &[u8], frames: u32) -> Vec<u16> {
    let frame = render_after_frames(
        "copper.nes",
        rom,
        NonZeroU32::new(frames).expect("at least one frame"),
    )
    .expect("the ROM runs");
    let entries = frame.as_indices();
    (0..FRAME_HEIGHT)
        .map(|y| entries[y * FRAME_WIDTH + FRAME_WIDTH / 2])
        .collect()
}

/// The first line of the bar, having asserted the bar is whole: 48 lines, darkest at both edges,
/// brightest in the middle, and backdrop everywhere else.
fn bar_top(lines: &[u16]) -> usize {
    let top = lines
        .iter()
        .position(|&entry| entry != BACKDROP)
        .expect("the bar is on screen");
    let bar = &lines[top..top + BAR_HEIGHT];
    assert_eq!(bar[0], DARKEST, "{lines:02x?}");
    assert_eq!(bar[BAR_HEIGHT - 1], DARKEST, "{lines:02x?}");
    assert_eq!(bar[BAR_HEIGHT / 2], BRIGHTEST, "{lines:02x?}");
    assert!(
        bar.iter().all(|&entry| entry != BACKDROP),
        "the bar has no gap: {lines:02x?}"
    );
    assert!(
        lines[top + BAR_HEIGHT..]
            .iter()
            .all(|&entry| entry == BACKDROP),
        "nothing below the bar: {lines:02x?}"
    );
    top
}

#[test]
fn the_example_builds_with_one_handler_on_the_last_line() {
    let (stdout, stderr, _) = build();

    assert_eq!(stderr, "", "no warnings");
    let table = stdout
        .split("\n\n")
        .nth(1)
        .expect("a budget table follows the summary");
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines[0], "frame copper (using timed, mmc3)");
    assert_eq!(lines.len(), 2, "{table}");
    assert!(lines[1].starts_with("  scanline  239 handler"), "{table}");
}

#[test]
fn the_bar_swings_and_stays_on_screen() {
    let (_, _, rom) = build();

    let early = bar_top(&column(&rom, 10));
    let later = bar_top(&column(&rom, 30));

    assert_ne!(early, later, "the bar moved between frames 10 and 30");
    for top in [early, later] {
        assert!((16..=176).contains(&top), "the bar's top is line {top}");
    }
}
