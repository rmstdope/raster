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

/// The example's `PHASE`, read from the source.
fn phase(source: &str) -> u8 {
    let value = source
        .lines()
        .find_map(|line| line.strip_prefix("const PHASE: u8 = "))
        .expect("the phase is declared");
    value.split("//").next().unwrap().trim().parse().unwrap()
}

/// The three bars' top lines at `step` of the swing.
fn tops(source: &str, step: u8) -> [u8; 3] {
    let (swing, phase) = (table(source, "SWING"), phase(source));
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

#[test]
fn every_frame_is_the_three_bars_at_the_step_its_frame_has_reached() {
    let source = fs::read_to_string(copper_bars_demo_path()).expect("the example is readable");
    let rom = build();

    // Three consecutive frames are the three phases of the timed pass, each with its own loop.
    // The others are where bars overlap: two partly, then two exactly on top of each other, then
    // long after, when `step` has wrapped.
    let frames: [u32; 6] = [10, 11, 12, 35, 57, 300];
    let shown: Vec<Vec<u16>> = frames.iter().map(|&frames| lines(&rom, frames)).collect();
    let step = |frames: u32, offset: u8| (frames as u8).wrapping_sub(offset);

    // Frame `n` shows step `n - offset`, for the frames the ROM spent starting up. One picture can
    // be more than one step, because the swing mirrors itself, but only one offset explains every
    // frame — counted within one swing, since SWING holds two identical ones.
    let offsets: Vec<u8> = (0..128u8)
        .filter(|&offset| {
            frames
                .iter()
                .zip(&shown)
                .all(|(&frames, shown)| expected(&source, step(frames, offset)) == *shown)
        })
        .collect();
    assert_eq!(
        offsets.len(),
        1,
        "offsets explaining every frame: {offsets:?}"
    );

    let overlapping = frames.iter().any(|&frames| {
        let mut tops = tops(&source, step(frames, offsets[0]));
        tops.sort_unstable();
        tops.windows(2).any(|pair| pair[1] - pair[0] < 9)
    });
    assert!(overlapping, "some frame checked shows bars overlapping");
}
