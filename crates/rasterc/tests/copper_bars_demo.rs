//! The three-bar copper example end to end: `examples/copper-bars/copper-bars.raster` built and
//! run, and each frame compared line by line with the picture its own tables describe.

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
use demo::copper_bars_demo_path;
use scratch::Scratch;

/// The palette entries the example loads, darkest first, and the NES colour each holds.
const SHADES: [(u8, u16); 6] = [
    (0x00, 0x0f),
    (0x01, 0x02),
    (0x03, 0x12),
    (0x07, 0x22),
    (0x0f, 0x32),
    (0x1f, 0x30),
];

/// How bright a palette entry is: its place in [`SHADES`].
fn brightness(entry: u8) -> usize {
    SHADES
        .iter()
        .position(|&(shade, _)| shade == entry)
        .unwrap_or_else(|| panic!("the example loads no palette entry ${entry:02X}"))
}

/// The numbers of the example's `const NAME: [256]u8 = [...]`, read from the source so this test
/// describes the tables the ROM is built from.
fn table(source: &str, name: &str) -> Vec<u8> {
    let declaration = format!("const {name}: [256]u8 = [");
    let start = source.find(&declaration).expect("the table is declared") + declaration.len();
    let body = &source[start..];
    let body = &body[..body.find(']').expect("the table is closed")];
    body.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| match entry.strip_prefix('$') {
            Some(hex) => u8::from_str_radix(hex, 16).unwrap(),
            None => entry.parse().unwrap(),
        })
        .collect()
}

/// The example's `const NAME: u8`, read from the source.
fn constant(source: &str, name: &str) -> u8 {
    let declaration = format!("const {name}: u8 = ");
    let value = source
        .lines()
        .find_map(|line| line.strip_prefix(declaration.as_str()))
        .unwrap_or_else(|| panic!("`{name}` is declared"));
    value.split("//").next().unwrap().trim().parse().unwrap()
}

/// The three bars' top lines at `step` of the swing.
fn tops(source: &str, step: u8) -> [u8; 3] {
    let (swing, phase) = (table(source, "SWING"), constant(source, "PHASE"));
    [0, 1, 2].map(|bar| swing[usize::from(step.wrapping_add(phase.wrapping_mul(bar)))])
}

/// What every line shows at `step` of the swing: the brightest shade of the bars it is in, or
/// black. Said as the brightest, not as the OR the ROM works it out with, so the test holds the
/// example to what it means.
fn expected(source: &str, step: u8) -> Vec<u16> {
    let bar = table(source, "BAR");
    let tops = tops(source, step);
    (0..FRAME_HEIGHT as u8)
        .map(|line| {
            let brightest = tops
                .iter()
                .map(|&top| brightness(bar[usize::from(line.wrapping_sub(top))]))
                .max()
                .unwrap();
            SHADES[brightest].1
        })
        .collect()
}

fn build() -> Vec<u8> {
    static BUILDS: AtomicUsize = AtomicUsize::new(0);
    let directory = Scratch::new(&format!(
        "copper-bars-demo-{}",
        BUILDS.fetch_add(1, Ordering::Relaxed)
    ));
    let output = directory.path().join("out.nes");
    let (mut stdout, mut stderr) = (Cursor::new(Vec::new()), Cursor::new(Vec::new()));
    let result = run(
        vec![
            copper_bars_demo_path().display().to_string(),
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

/// Each line's colour after `frames` frames, having asserted every line is one colour from edge
/// to edge.
fn lines(rom: &[u8], frames: u32) -> Vec<u16> {
    let frame = render_after_frames(
        "copper-bars.nes",
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

/// The swing step the bars have reached after `frames` frames of moving: `SPEED` is sixteenths of
/// a step a frame.
fn step_after(speed: u8, frames: u32) -> u8 {
    (u32::from(speed) * frames / 16) as u8
}

/// Whether any two bars overlap at `step`.
fn overlapping(source: &str, step: u8) -> bool {
    let mut tops = tops(source, step);
    tops.sort_unstable();
    tops.windows(2).any(|pair| pair[1] - pair[0] < 9)
}

#[test]
fn every_frame_is_the_three_bars_at_the_step_its_frame_has_reached() {
    let source = fs::read_to_string(copper_bars_demo_path()).expect("the example is readable");
    let speed = constant(&source, "SPEED");
    let rom = build();

    // Three consecutive frames are the three phases of the timed pass, each with its own loop; the
    // rest are spread over a long run of the swing.
    let frames = [10, 11, 12, 70, 200, 600];
    let shown: Vec<Vec<u16>> = frames.iter().map(|&frames| lines(&rom, frames)).collect();

    // The ROM spends its first frames starting up, so the first frame showing the bars where they
    // start is where the count of frames they have moved begins. Every frame after it shows the
    // bars `SPEED` sixteenths of a step further on.
    let offset = (1..frames[0])
        .find(|&frames| lines(&rom, frames) == expected(&source, 0))
        .expect("the bars appear where they start");
    for (&frames, shown) in frames.iter().zip(&shown) {
        assert_eq!(
            *shown,
            expected(&source, step_after(speed, frames - offset)),
            "frame {frames}, {} frames after the bars appeared",
            frames - offset
        );
    }

    // Where bars overlap, a line shows the brightest of them. The first frame they do is found
    // from the tables, so it is checked whatever SPEED and PHASE are set to — if they overlap at
    // all in the first thousand frames.
    if let Some(moved) = (1..1000).find(|&moved| overlapping(&source, step_after(speed, moved))) {
        assert_eq!(
            lines(&rom, offset + moved),
            expected(&source, step_after(speed, moved)),
            "frame {}, the first where bars overlap",
            offset + moved
        );
    }
}
