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

/// The NES colour each palette entry the example loads holds.
fn colour(entry: u8) -> u16 {
    match entry {
        0x00 => 0x0f,
        0x01 => 0x02,
        0x03 => 0x12,
        0x07 => 0x22,
        0x0f => 0x32,
        0x1f => 0x30,
        other => panic!("the example loads no palette entry ${other:02X}"),
    }
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

/// What every line shows at `step` of the swing: the brightest of the three bars it is in.
fn expected(source: &str, step: u8) -> Vec<u16> {
    let (bar, swing) = (table(source, "BAR"), table(source, "SWING"));
    let phase: u8 = source
        .lines()
        .find_map(|line| line.strip_prefix("const PHASE: u8 = "))
        .expect("the phase is declared")
        .parse()
        .unwrap();
    let tops = [0, 1, 2].map(|bar| swing[usize::from(step.wrapping_add(phase.wrapping_mul(bar)))]);
    (0..FRAME_HEIGHT as u8)
        .map(|line| {
            colour(tops.iter().fold(0, |shade, &top| {
                shade | bar[usize::from(line.wrapping_sub(top))]
            }))
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

#[test]
fn every_frame_is_the_three_bars_at_one_step_and_the_next_frame_is_the_next_step() {
    let source = fs::read_to_string(copper_bars_demo_path()).expect("the example is readable");
    let rom = build();

    // Three consecutive frames are the three phases of the timed pass, each with its own loop.
    let steps: Vec<u8> = [10, 11, 12, 40]
        .into_iter()
        .map(|frames| {
            let shown = lines(&rom, frames);
            (0..=255u8)
                .find(|&step| expected(&source, step) == shown)
                .unwrap_or_else(|| panic!("frame {frames} is no step of the swing: {shown:02x?}"))
        })
        .collect();

    assert_eq!(steps[1], steps[0].wrapping_add(1), "{steps:?}");
    assert_eq!(steps[2], steps[1].wrapping_add(1), "{steps:?}");
    assert_eq!(steps[3], steps[0].wrapping_add(30), "{steps:?}");
}
