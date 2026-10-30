use std::num::NonZeroU32;

use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::compile_source;

#[path = "common/cycles.rs"]
mod cycles_fixtures;
#[path = "common/demo.rs"]
mod demo;
use cycles_fixtures::cycles_fixture;
use demo::backdrop_source;

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
    let demo = backdrop_source();
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
    let demo = backdrop_source();
    let lighter = demo.replace("const BACKDROP: u8 = $12", "const BACKDROP: u8 = $21");
    assert_ne!(demo, lighter, "the demo declares the backdrop as $12");

    // No RGB constant anywhere: the source names an NES colour and the PPU is
    // asked what colour it emitted.
    assert_eq!(backdrop_entry("demo.nes", &demo), 0x12);
    assert_eq!(backdrop_entry("lighter.nes", &lighter), 0x21);
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

/// An event at scanline 240 runs after the picture: everything it changes is already in place
/// when the next frame starts, and nothing it writes lands on a visible line.
#[test]
fn an_irq_event_at_scanline_240_runs_after_the_last_visible_line() {
    let source = cycles_fixture("irq-colour-bars.raster").replace(
        "    at scanline 180 {\n        ppu.mask = $1e          // none again\n    }",
        "    at scanline 240 {\n        ppu.mask = $1e          // none again\n    }",
    );
    assert!(
        source.contains("at scanline 240"),
        "the fixture still reads as expected"
    );
    let rom = compile_source(&source).expect("the program compiles");

    for frames in [8, 9, 61] {
        let frame = render_after_frames(
            "irq-240.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        let entries = *frame.as_indices();
        assert_eq!(
            bands(&entries),
            vec![(0, 0x0f), (60, 0x4f), (120, 0x8f)],
            "frame {frames}: green to the last line, and none again from the top of the next frame"
        );
        assert!(
            row_bands(&entries).iter().all(|bands| bands.len() == 1),
            "frame {frames}: a handler wrote part-way along a scanline"
        );
    }
}

/// Line 240 is never drawn, so its handler may put back several registers for the next frame: two
/// stores, twelve cycles, which a visible line's hblank would refuse.
///
/// The mask store must land: it makes the next frame start without emphasis. The control store
/// runs before it, in order, and rewrites the value `main` set, so it is proven to run rather than
/// to change anything.
#[test]
fn an_irq_event_at_scanline_240_may_put_back_several_registers() {
    let source = cycles_fixture("irq-colour-bars.raster").replace(
        "    at scanline 180 {\n        ppu.mask = $1e          // none again\n    }",
        "    at scanline 240 {\n        ppu.ctrl = $08\n        ppu.mask = $1e\n    }",
    );
    assert!(source.contains("ppu.ctrl = $08\n        ppu.mask = $1e"));
    let rom = compile_source(&source).expect("two stores fit the undrawn line");

    for frames in 8..14 {
        let frame = render_after_frames(
            "irq-240-two-stores.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        let entries = *frame.as_indices();
        assert_eq!(
            bands(&entries),
            vec![(0, 0x0f), (60, 0x4f), (120, 0x8f)],
            "frame {frames}: green to the last line, and none again from the top of the next frame"
        );
        assert!(
            row_bands(&entries).iter().all(|bands| bands.len() == 1),
            "frame {frames}: a handler wrote part-way along a scanline"
        );
    }

    let at_239 = source.replace("at scanline 240", "at scanline 239");
    assert!(
        compile_source(&at_239).is_err(),
        "the same two stores on a visible line are over its hblank"
    );
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

/// The hblank window was measured with the background at `$0000`, where A12 rises in the sprite
/// fetches at about dot 260. A `tiles: 512` picture's lower half runs with the background at
/// `$1000` and the sprites at `$0000`, which moves the rise to the background prefetch at about
/// dot 324 — late enough that a handler's store reaches the picture part-way along its row.
///
/// This is the shape `examples/zombie` has, plus one event inside the lower half: the switch at
/// 120, a mask change at 180 and back at 200, the switch back at 240. Rows 180 and 200 must each
/// read whole, or the program must not compile.
#[test]
fn an_irq_store_under_the_background_at_1000_never_lands_on_the_picture() {
    let source = "var red: u8 = $3e\nvar plain: u8 = $1e\nmain {\n    ppu.addr = $3f\n    ppu.addr = $00\n    ppu.data = $0f\n    ppu.addr = $00\n    ppu.addr = $00\n    ppu.ctrl = $08\n    ppu.mask = $1e\n}\nframe halves using irq {\n    at scanline 120 { ppu.ctrl = $10 }\n    at scanline 180 { ppu.mask = red }\n    at scanline 200 { ppu.mask = plain }\n    at scanline 240 { ppu.ctrl = $08 }\n}\n";
    let rom =
        match compile_source(source) {
            Ok(rom) => rom,
            // Refused is one of the two answers, but only this refusal: any other error would be a
            // fixture that stopped meaning what it says.
            Err(diagnostics) => {
                assert!(
                    diagnostics.iter().any(|d| d.message
                        == "an `irq` handler cannot run while the background is at $1000"),
                    "{diagnostics:?}"
                );
                return;
            }
        };
    for frames in 8..14 {
        let entries = *render_after_frames(
            "irq-background-1000.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs")
        .as_indices();
        for (scanline, entry) in [(180usize, 0x4fu16), (200, 0x0f)] {
            let row = &entries[scanline * FRAME_WIDTH..(scanline + 1) * FRAME_WIDTH];
            let torn = row.iter().position(|&pixel| pixel != entry);
            assert_eq!(
                torn, None,
                "frame {frames}, scanline {scanline}: column {torn:?} does not carry the value the \
                 handler stored, so the store reached the picture after dot 0"
            );
        }
    }
}

/// `vram_addr` and the raw byte pair reach the same PPU address: the demo, with
/// its two pairs written as one line each, shows the same screen.
#[test]
fn vram_addr_sets_the_address_the_raw_pair_sets() {
    let demo = backdrop_source();
    let one_line = demo
        .replace("ppu.addr = $3f              // the palette lives at $3F00 ...\n    ppu.addr = $00              // ... and $3F00 is the backdrop", "vram_addr = $3F00")
        .replace("ppu.addr = $00              // leave palette space, or the screen\n    ppu.addr = $00              // shows $3F01 instead of the backdrop", "vram_addr = $0000");
    assert!(!one_line.contains("ppu.addr"), "both pairs were replaced");

    assert_eq!(
        backdrop_colour("one-line.nes", &one_line),
        backdrop_colour("demo.nes", &demo)
    );
}

#[test]
fn a_constant_array_element_reaches_the_screen() {
    let source = |index: u8| {
        format!(
            "const SHADES: [3]u8 = [$0F, $16, $21]\nvar i: u8 = {index}\nmain {{\n    ppu.addr = $3f\n    ppu.addr = $00\n    ppu.data = SHADES[i]\n    ppu.addr = $00\n    ppu.addr = $00\n}}\n"
        )
    };
    assert_eq!(backdrop_entry("first.nes", &source(0)), 0x0f);
    assert_eq!(backdrop_entry("last.nes", &source(2)), 0x21);
}

#[test]
fn a_subtraction_takes_its_operands_in_source_order() {
    // $22 - $10 is $12, a blue; the operands the wrong way round give $EE, which the palette keeps
    // as $2E. One subtraction with a variable on the right, one with a table read there.
    let source = |expression: &str| {
        format!(
            "const T: [2]u8 = [$00, $10]\nvar a: u8 = $22\nvar b: u8 = $10\nvar j: u8 = 1\nmain {{\n    ppu.addr = $3f\n    ppu.addr = $00\n    ppu.data = {expression}\n    ppu.addr = $00\n    ppu.addr = $00\n}}\n"
        )
    };
    assert_eq!(backdrop_entry("direct.nes", &source("a - b")), 0x12);
    assert_eq!(backdrop_entry("pure.nes", &source("a - T[j]")), 0x12);
    assert_eq!(
        backdrop_entry("both.nes", &source("T[j] - (a - $20)")),
        0x0e
    );
}

#[test]
fn a_constant_u8_index_wraps_to_eight_bits_as_the_running_program_would() {
    // `PHASE + PHASE` is 400 on paper and 144 in eight bits; only entry 144 is blue.
    let table = (0..256)
        .map(|entry| if entry == 144 { "$12" } else { "$0F" })
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!(
        "const SWING: [256]u8 = [{table}]\nconst PHASE: u8 = 200\nmain {{\n    ppu.addr = $3f\n    ppu.addr = $00\n    ppu.data = SWING[PHASE + PHASE]\n    ppu.addr = $00\n    ppu.addr = $00\n}}\n"
    );
    assert_eq!(backdrop_entry("wrapped.nes", &source), 0x12);
}

#[test]
fn a_variable_stepped_by_one_holds_the_value_adding_or_taking_one_gives() {
    // Each steps a variable and then reads it back, so the read cannot reuse whatever the step
    // left in the accumulator: $11 + 1 is $12, $FF + 1 wraps to $00 and $00 - 1 to $FF.
    let source = |initial: &str, step: &str, expression: &str| {
        format!(
            "var v: u8 = {initial}\nmain {{\n    {step}\n    ppu.addr = $3f\n    ppu.addr = $00\n    ppu.data = {expression}\n    ppu.addr = $00\n    ppu.addr = $00\n}}\n"
        )
    };
    assert_eq!(
        backdrop_entry("inc.nes", &source("$11", "v += 1", "v")),
        0x12
    );
    assert_eq!(
        backdrop_entry("inc-wraps.nes", &source("$FF", "v += 1", "v + $12")),
        0x12
    );
    assert_eq!(
        backdrop_entry("dec-wraps.nes", &source("$00", "v -= 1", "v - $CD")),
        0x32
    );
}

/// A body near the top of the undrawn line's window: twenty register stores, 120 cycles, which
/// finishes on line 240 and whose epilogue runs on into vblank. The chain must still wrap and every
/// band hold its place over frames of every phase.
#[test]
fn an_irq_event_at_scanline_240_may_run_nearly_the_whole_line() {
    let stores = "        ppu.scroll = 0\n".repeat(20);
    let source = cycles_fixture("irq-colour-bars.raster").replace(
        "    at scanline 180 {\n        ppu.mask = $1e          // none again\n    }",
        &format!("    at scanline 240 {{\n{stores}        ppu.mask = $1e\n    }}"),
    );
    let rom = compile_source(&source);
    // Twenty scroll stores are 120 cycles and the mask store makes it 126: over the line.
    assert!(rom.is_err(), "twenty stores and a mask store are over 122");

    let source = source.replacen("        ppu.scroll = 0\n", "", 1);
    let rom = compile_source(&source).expect("nineteen stores and a mask store fit");
    for frames in 8..14 {
        let frame = render_after_frames(
            "irq-240-long.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        assert_eq!(
            bands(frame.as_indices()),
            vec![(0, 0x0f), (60, 0x4f), (120, 0x8f)],
            "frame {frames}"
        );
    }
}

/// Where, in dots from the top of the picture, the first pixel showing red emphasis is.
fn first_red_dot(entries: &[u16]) -> usize {
    entries
        .iter()
        .position(|&entry| entry & 0x40 != 0)
        .expect("some pixel is emphasised")
}

/// A timed frame over a rendered picture keeps its place: with rendering on, NTSC frames are a
/// dot shorter every other time, and a pass that counted three rendering-off frames slid half a
/// dot a frame along the picture. The write lands part-way along a line on purpose, so where the
/// emphasis starts is where the handler ran, to the dot.
#[test]
fn a_timed_frame_over_a_rendered_picture_does_not_drift() {
    let source = "main {\n\
                  \x20   ppu.addr = $3f\n\
                  \x20   ppu.addr = $00\n\
                  \x20   ppu.data = $0f\n\
                  \x20   ppu.addr = $00\n\
                  \x20   ppu.addr = $00\n\
                  \x20   ppu.ctrl = $00\n\
                  \x20   ppu.mask = $0a          // the background on: rendering\n\
                  }\n\
                  frame tint using timed {\n\
                  \x20   at scanline 100 { ppu.mask = $2a }   // red emphasis from where this lands\n\
                  \x20   at scanline 140 { ppu.mask = $0a }\n\
                  }\n";
    let rom = compile_source(source).expect("the program compiles");
    let render = |frames: u32| {
        let frame = render_after_frames(
            "rendered-timed.nes",
            &rom.image,
            NonZeroU32::new(frames).expect("at least one frame"),
        )
        .expect("the ROM runs");
        *frame.as_indices()
    };

    let even = first_red_dot(&render(10));
    let odd = first_red_dot(&render(11));
    assert!(
        !even.is_multiple_of(FRAME_WIDTH),
        "the write lands part-way along a line, so a drift would show"
    );
    // A rendered frame is 29780.5 cycles on average, so the handler of one frame of a pair runs
    // half a cycle, a dot and a half, from where it runs in the other: that is the grain of a loop
    // counted in cycles, and is all the two may differ by.
    assert!(
        even.abs_diff(odd) <= 2,
        "frames 10 and 11 start the emphasis {} dots apart",
        even.abs_diff(odd)
    );
    // Whatever drifts goes on drifting, so the same frame of a pair minutes later is where it was.
    for (frames, expected) in [
        (12, even),
        (16, even),
        (60, even),
        (300, even),
        (13, odd),
        (301, odd),
    ] {
        let at = first_red_dot(&render(frames));
        assert_eq!(
            at,
            expected,
            "frame {frames}: the emphasis starts at ({}, {}), not ({}, {})",
            at % FRAME_WIDTH,
            at / FRAME_WIDTH,
            expected % FRAME_WIDTH,
            expected / FRAME_WIDTH
        );
    }
}
