//! A `frame ... using timed` whose `every 1 scanlines` run covers the whole picture: too many
//! handlers to unroll into the fixed bank three times over, so the schedule runs them as a loop,
//! and the loop has to land every handler exactly where the unrolled schedule would have.

use std::num::NonZeroU32;

use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::compile_source;

#[path = "common/cycles.rs"]
mod cycles_fixtures;
use cycles_fixtures::cycles_fixture;

const BLACK: u16 = 0x0f;
const WHITE: u16 = 0x30;

/// Where along each row the stripe changes colour, and the colour it changes to.
///
/// Row `y` starts in the colour row `y - 1` was left showing and changes once its handler's second
/// `ppu.addr` write lands.
fn edges(entries: &[u16]) -> Vec<(usize, u16)> {
    (0..FRAME_HEIGHT)
        .map(|y| {
            let row = &entries[y * FRAME_WIDTH..(y + 1) * FRAME_WIDTH];
            let x = (1..FRAME_WIDTH)
                .find(|&x| row[x] != row[0])
                .unwrap_or_else(|| panic!("row {y} never changes colour: {row:02x?}"));
            assert!(
                row[x..].iter().all(|&entry| entry == row[x]),
                "row {y} changes colour more than once"
            );
            (x, row[x])
        })
        .collect()
}

#[test]
fn a_handler_on_every_scanline_fits_the_fixed_bank() {
    let rom = compile_source(&cycles_fixture("stripes.raster")).expect("the fixture compiles");
    assert!(
        rom.code_len < 4096,
        "a looped schedule is a few handlers, not 720: {} bytes",
        rom.code_len
    );
}

#[test]
fn the_loop_charges_its_own_overhead_to_the_handlers_it_runs() {
    let rom = compile_source(&cycles_fixture("stripes.raster")).expect("the fixture compiles");
    let budget = rom.frame.expect("the fixture has a frame");
    assert_eq!(budget.rows.len(), 1);
    // A scanline is 113 or 114 cycles, and the loop's closing jump comes out of the last handler
    // of each turn: the narrowest window any occurrence gets is a scanline less that overhead.
    assert!(
        (90..=100).contains(&budget.rows[0].window_cycles),
        "{:?}",
        budget.rows[0]
    );
}

#[test]
fn every_looped_handler_lands_at_the_same_point_of_its_scanline() {
    let rom = compile_source(&cycles_fixture("stripes.raster")).expect("the fixture compiles");
    // The three frames of one pass, each with its own copy of the loop, and one long after.
    for frames in [8, 9, 10, 300] {
        let frame = render_after_frames(
            "stripes.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        let edges = edges(frame.as_indices());

        for (y, &(_, colour)) in edges.iter().enumerate() {
            let expected = if y % 2 == 0 { WHITE } else { BLACK };
            assert_eq!(colour, expected, "frame {frames}, row {y}");
        }
        // A scanline is 341 dots and a handler starts on a whole cycle, three dots, so where a
        // write lands wanders by up to a cycle from row to row. A loop a cycle out per turn would
        // wander three dots further every three rows, and a whole picture of it would cross the row.
        let columns: Vec<usize> = edges.iter().map(|&(x, _)| x).collect();
        let (earliest, latest) = (columns.iter().min().unwrap(), columns.iter().max().unwrap());
        assert!(
            latest - earliest <= 3,
            "frame {frames}: writes land between columns {earliest} and {latest}: {columns:?}"
        );
    }
}
