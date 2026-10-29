use raster_link::{
    m1_solid_backdrop_rom, mmc3_reset_prologue_bytes, INES_HEADER_SIZE, MMC3_FIXED_BANK_SIZE,
    MMC3_FIXED_BANK_START, MMC3_PRG_ROM_SIZE,
};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn fixed_bank(rom: &[u8]) -> &[u8] {
    let offset = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    &rom[offset..]
}

fn vector(rom: &[u8], address: u16) -> u16 {
    let offset = (address - MMC3_FIXED_BANK_START) as usize;
    u16::from_le_bytes(fixed_bank(rom)[offset..offset + 2].try_into().unwrap())
}

/// Where M1's own code begins: right after the shared runtime's prologue.
fn entry() -> u16 {
    MMC3_FIXED_BANK_START + u16::try_from(mmc3_reset_prologue_bytes()).unwrap()
}

#[test]
fn uses_the_fixed_bank_reset_and_interrupt_vectors() {
    let rom = m1_solid_backdrop_rom();
    let handler = vector(&rom, 0xfffa);

    assert_eq!(vector(&rom, 0xfffc), 0xe000);
    assert_eq!(vector(&rom, 0xfffe), handler);
    // The handler is the runtime's lone RTI, the last byte M1 emits, after
    // the prologue and M1's 28-byte body.
    assert_eq!(handler, entry() + 28);
    assert_eq!(
        fixed_bank(&rom)[usize::from(handler - MMC3_FIXED_BANK_START)],
        0x40
    );
}

#[test]
fn initializes_before_writing_the_blue_universal_backdrop() {
    let rom = m1_solid_backdrop_rom();
    let program = fixed_bank(&rom);
    let prologue_len = mmc3_reset_prologue_bytes();
    let [entry_low, entry_high] = entry().to_le_bytes();

    // The prologue's tail is where the console is made ready: two vblank polls,
    // a third read that clears the write latch, and the jump to M1's body. The
    // bytes before it are `tests/runtime.rs`'s golden prologue to pin.
    let poll = [0x2c, 0x02, 0x20, 0x10, 0xfb];
    let latch_reset_and_entry = [0x2c, 0x02, 0x20, 0x4c, entry_low, entry_high];
    let palette_write = [
        0xa9, 0x3f, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, 0xa9, 0x12, 0x8d, 0x07, 0x20,
    ];

    let tail_len = poll.len() * 2 + latch_reset_and_entry.len();
    let mut offset = prologue_len - tail_len;
    for expected in [
        &poll[..],
        &poll[..],
        &latch_reset_and_entry[..],
        &palette_write[..],
    ] {
        assert_eq!(&program[offset..offset + expected.len()], expected);
        offset += expected.len();
    }

    // Nothing writes the PPU address or data registers before the PPU is warm.
    let polls_end = prologue_len - latch_reset_and_entry.len();
    assert!(!program[..polls_end]
        .windows(3)
        .any(|instruction| instruction == [0x8d, 0x06, 0x20] || instruction == [0x8d, 0x07, 0x20]));
}

#[test]
fn hand_built_mmc3_rom_is_byte_identical_after_assembler_migration() {
    let actual = m1_solid_backdrop_rom();
    let mut expected = vec![0xff; INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE];
    expected[..INES_HEADER_SIZE].copy_from_slice(&[
        b'N', b'E', b'S', 0x1a, 2, 0, 0x40, 0, 1, 0, 0, 0, 0, 0, 0, 0,
    ]);

    let fixed_bank_offset = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    let prologue_len = mmc3_reset_prologue_bytes();
    // The shared runtime's prologue is taken from the ROM itself: its bytes are
    // pinned once, in `raster-link/tests/runtime.rs`. What this test owns is
    // M1's body: the palette write, the two writes that leave palette space, a
    // halt loop that jumps to itself, and the interrupt stub the vectors point at.
    expected[fixed_bank_offset..fixed_bank_offset + prologue_len]
        .copy_from_slice(&actual[fixed_bank_offset..fixed_bank_offset + prologue_len]);
    let [loop_low, loop_high] = (entry() + 25).to_le_bytes();
    let body = [
        0xa9, 0x3f, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, 0xa9, 0x12, 0x8d, 0x07, 0x20,
        0xa9, 0x00, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, 0x4c, loop_low, loop_high,
        0x40,
    ];
    let body_start = fixed_bank_offset + prologue_len;
    expected[body_start..body_start + body.len()].copy_from_slice(&body);
    let [handler_low, handler_high] = (entry() + 28).to_le_bytes();
    expected[INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - 6..].copy_from_slice(&[
        handler_low,
        handler_high,
        0x00,
        0xe0,
        handler_low,
        handler_high,
    ]);

    assert_eq!(actual, expected);
}

#[test]
fn binary_writes_the_rom_and_requires_an_output_path() {
    let executable = env!("CARGO_BIN_EXE_m1_solid_backdrop");
    let path = std::env::temp_dir().join(format!(
        "m1-solid-backdrop-{}-{}.nes",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let output = Command::new(executable).arg(&path).output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(&path).unwrap(), m1_solid_backdrop_rom());
    fs::remove_file(&path).unwrap();

    let output = Command::new(executable).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"Usage: m1_solid_backdrop <OUTPUT.nes>\n");
}
