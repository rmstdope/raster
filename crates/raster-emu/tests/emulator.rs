use std::num::NonZeroU32;

use raster_emu::{render_after_frames, FRAME_BYTES};

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

    assert_eq!(frame.as_rgba().len(), FRAME_BYTES);
    assert!(frame
        .as_rgba()
        .iter()
        .skip(3)
        .step_by(4)
        .all(|&alpha| alpha == 0xFF));
}

#[test]
fn rejects_an_invalid_rom_instead_of_returning_a_frame() {
    assert!(render_after_frames("invalid.nes", b"not an iNES ROM", NonZeroU32::MIN).is_err());
}
