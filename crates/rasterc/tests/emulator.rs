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

/// The schedule the MMC3 counter placed, as the PPU actually emitted it.
///
/// The compiler's claim about `using irq` is that a handler runs at the top of the scanline the
/// source names, and nothing but an execution can settle whether the latch arithmetic, the
/// acknowledgement order and the vblank re-arm add up to that. The bands are read in NES palette
/// entries, so the fixture is judged against the colours its source names rather than against an
/// emulator's idea of what they look like.
#[test]
fn irq_frame_runs_events_at_the_requested_scanlines() {
    let rom =
        compile_source(&cycles_fixture("irq-colour-bars.raster")).expect("the fixture compiles");
    let render = |frames: u32| {
        let frame = render_after_frames(
            "irq-colour-bars.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        *frame.as_indices()
    };

    let first = render(8);
    assert_eq!(
        bands(&first),
        // Black throughout, emphasised red from scanline 60 and green from 120: the emphasis bits
        // `ppu.mask` asked for arrive in bits 6 and 7 of the palette entry the PPU emitted.
        vec![(0, 0x0f), (60, 0x4f), (120, 0x8f), (180, 0x0f)],
        "each handler runs at the top of the scanline its `at` names"
    );

    // Every scanline is one colour: a handler counted to the rise on the line before the one it
    // names finishes inside that line's hblank, so nothing is written part-way along a visible row.
    assert!(
        row_bands(&first).iter().all(|bands| bands.len() == 1),
        "a handler wrote part-way along a scanline"
    );

    // The chain is armed once and wraps, so a frame that differs is a link the counter dropped
    // rather than a loop that drifted. Frame 300 is five seconds in, which is where a chain
    // re-armed from a `$2002` poll would already have lost frames.
    for frames in [9, 10, 11, 61, 300] {
        let later = render(frames);
        assert_eq!(
            bands(&later),
            bands(&first),
            "frame {frames} shows different bands from frame 8"
        );
        assert_eq!(row_bands(&later), row_bands(&first));
    }
}

/// The window the compiler enforces is one the console honours.
///
/// `IRQ_HANDLER_BODY_CYCLES` was measured rather than derived, so the number is only worth what an
/// execution says it is. This runs the widest body the fixture can build — seven cycles, a register
/// store read from a variable — and asserts that not one pixel of any row carries the colour the
/// handler was replacing.
///
/// **Seven is the widest body this fixture can build, not the widest the compiler accepts.** That is
/// eight: `ppu.mask = ppu.status`, `LDA $2002` four cycles and `STA $2001` four. It cannot appear
/// here, because what `$2002` returns is not a mask — writing it would turn rendering off, and the
/// MMC3 counts filtered A12 rises, so the chain this fixture depends on would stop. Eight is
/// therefore reasoned rather than run: by the model the constant's doc comment records, a store
/// completing at body cycle eight lands at column `3 * 8 - 27 = -3`, still before dot 0. Nothing in
/// the language costs nine, so the window's last cycle is unreachable by construction; what admits
/// eight and turns away ten is pinned in `the_hblank_admits_a_body_up_to_its_window_and_no_wider`.
///
/// Every handler stores a value the mask does not already hold, so each store shows. A fixture
/// whose handlers rewrote the value already there would satisfy this assertion without the stores
/// having landed anywhere in particular.
#[test]
fn irq_handler_body_fits_the_hblank_it_lands_in() {
    let rom =
        compile_source(&cycles_fixture("irq-hblank-window.raster")).expect("the fixture compiles");
    let render = |frames: u32| {
        let frame = render_after_frames(
            "irq-hblank-window.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        *frame.as_indices()
    };

    let first = render(8);
    assert_eq!(
        bands(&first),
        // Black throughout, and the emphasis each handler's variable holds from the scanline its
        // `at` names: the stores land, and they land where the source asked.
        vec![(0, 0x0f), (40, 0x4f), (90, 0x8f), (140, 0x10f), (190, 0x0f)],
        "each handler runs at the top of the scanline its `at` names"
    );

    // The assertion this test exists for, and it is made pixel by pixel rather than through
    // `row_bands`: that helper drops any run shorter than eight pixels, so a store landing in
    // columns 1 to 7 would leave the row reading as a single band and a window of 11 would pass for
    // one of 9. A handler's row is checked whole — every column of it already carries the value the
    // handler stored, so the store finished before dot 0 rather than merely near it.
    //
    // Six consecutive frames, because a frame is 29780 CPU cycles and two dots: the phase between
    // the interrupt and the picture walks through every value it has over that many, and a body that
    // only just fits would tear in some of them and not others.
    for frames in 8..14 {
        let entries = render(frames);
        for (scanline, entry) in [(40usize, 0x4fu16), (90, 0x8f), (140, 0x10f), (190, 0x0f)] {
            let row = &entries[scanline * FRAME_WIDTH..(scanline + 1) * FRAME_WIDTH];
            let intact = row.iter().position(|&pixel| pixel != entry);
            assert_eq!(
                intact, None,
                "frame {frames}, scanline {scanline}: column {intact:?} does not carry the value \
                 this handler stored, so the store reached the picture after dot 0"
            );
        }
    }
}
