use std::num::NonZeroU32;

use raster_emu::{cycles_between, render_after_frames, Window, FRAME_BYTES, FRAME_PIXELS};

fn nrom() -> Vec<u8> {
    let mut rom = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut prg = vec![0xEA; 16 * 1024];
    prg[0] = 0x4C;
    prg[1] = 0x00;
    prg[2] = 0x80;

    for offset in [0x3FFA, 0x3FFC, 0x3FFE] {
        prg[offset] = 0x00;
        prg[offset + 1] = 0x80;
    }

    rom.extend(prg);
    rom
}

#[test]
fn renders_a_deterministic_rgba_frame_from_an_in_memory_ines_rom() {
    let frame = render_after_frames(
        "loop.nes",
        &nrom(),
        NonZeroU32::new(2).expect("two is non-zero"),
    )
    .expect("the NROM should load and render");
    let repeated_frame = render_after_frames(
        "loop.nes",
        &nrom(),
        NonZeroU32::new(2).expect("two is non-zero"),
    )
    .expect("the NROM should render repeatedly");

    assert_eq!(frame.as_rgba().len(), FRAME_BYTES);
    assert!(frame
        .as_rgba()
        .iter()
        .skip(3)
        .step_by(4)
        .all(|&alpha| alpha == 0xFF));
    assert_eq!(frame, repeated_frame);
}

#[test]
fn rejects_an_invalid_rom_instead_of_returning_a_frame() {
    assert!(render_after_frames("invalid.nes", b"not an iNES ROM", NonZeroU32::MIN).is_err());
}

/// `PHP` (3 cycles), `NOP` (2) and `PLP` (4) at `$8000`, then a `JMP` onto
/// itself so the ROM never runs off the end of what it was given.
fn bracketed_nine_cycles() -> Vec<u8> {
    let mut rom = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut prg = vec![0xEA; 16 * 1024];
    prg[..6].copy_from_slice(&[0x08, 0xEA, 0x28, 0x4C, 0x03, 0x80]);

    for offset in [0x3FFA, 0x3FFC, 0x3FFE] {
        prg[offset] = 0x00;
        prg[offset + 1] = 0x80;
    }

    rom.extend(prg);
    rom
}

#[test]
fn emulator_harness_runs_a_known_rom() {
    let measured = cycles_between(
        "bracketed.nes",
        &bracketed_nine_cycles(),
        Window::new(0x08, 0x28),
    )
    .expect("the ROM runs and both markers execute");

    assert_eq!(measured, 9);
}

#[test]
fn a_marker_that_never_executes_is_reported_rather_than_waited_for() {
    let error = cycles_between(
        "bracketed.nes",
        &bracketed_nine_cycles(),
        Window::new(0x08, 0x60),
    )
    .expect_err("the ROM contains no RTS");

    assert!(
        format!("{error}").contains("$60"),
        "the error names the marker it never saw: {error}"
    );
}

#[test]
fn reports_a_palette_entry_for_every_pixel() {
    let frame = render_after_frames(
        "loop.nes",
        &nrom(),
        NonZeroU32::new(2).expect("two is non-zero"),
    )
    .expect("the NROM should load and render");
    let repeated_frame = render_after_frames(
        "loop.nes",
        &nrom(),
        NonZeroU32::new(2).expect("two is non-zero"),
    )
    .expect("the NROM should render repeatedly");

    // The ROM loops on NOPs and never writes the PPU, so every pixel is the
    // cold universal backdrop, entry `0x00`. Asserting the value rather than a
    // range also pins that no colour-emphasis bit (`0x1C0`) reaches an entry.
    for (index, &entry) in frame.as_indices().iter().enumerate() {
        assert_eq!(
            entry, 0x00,
            "pixel {index} of {FRAME_PIXELS} is not the cold backdrop entry"
        );
    }
    assert_eq!(frame.as_indices(), repeated_frame.as_indices());
}

#[test]
fn renders_one_flat_colour_per_palette_entry() {
    let frame = render_after_frames(
        "loop.nes",
        &nrom(),
        NonZeroU32::new(2).expect("two is non-zero"),
    )
    .expect("the NROM should load and render");
    let rgba = frame.as_rgba();

    // Any two pixels sharing a palette entry must share an RGBA value: the
    // RGBA view is a per-pixel palette lookup, not a filter that blends
    // neighbours. Column 0 is included, since a composite filter blacks it out.
    let mut seen: [Option<[u8; 4]>; 512] = [None; 512];
    for (index, &entry) in frame.as_indices().iter().enumerate() {
        let colour: [u8; 4] = rgba[index * 4..index * 4 + 4]
            .try_into()
            .expect("four channels");
        let slot = seen
            .get_mut(usize::from(entry))
            .unwrap_or_else(|| panic!("entry {entry:#05x} is outside the 9-bit entry space"));
        let known = slot.get_or_insert(colour);
        assert_eq!(
            *known, colour,
            "pixel {index} shows a second colour for palette entry {entry:#05x}"
        );
    }
}
