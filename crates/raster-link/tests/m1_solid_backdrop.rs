use raster_link::{
    m1_solid_backdrop_rom, INES_HEADER_SIZE, MMC3_FIXED_BANK_SIZE, MMC3_FIXED_BANK_START,
    MMC3_PRG_ROM_SIZE,
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

#[test]
fn uses_the_fixed_bank_reset_and_interrupt_vectors() {
    let rom = m1_solid_backdrop_rom();

    assert_eq!(vector(&rom, 0xfffa), 0xe063);
    assert_eq!(vector(&rom, 0xfffc), 0xe000);
    assert_eq!(vector(&rom, 0xfffe), 0xe063);
    assert_eq!(fixed_bank(&rom)[0x63], 0x40);
}

#[test]
fn initializes_before_writing_the_blue_universal_backdrop() {
    let rom = m1_solid_backdrop_rom();
    let program = fixed_bank(&rom);

    let setup = [
        0x78, 0xd8, 0xa2, 0x40, 0x8e, 0x17, 0x40, 0xa2, 0xff, 0x9a, 0xe8, 0x8e, 0x00, 0x20, 0x8e,
        0x01, 0x20, 0x8e, 0x10, 0x40,
    ];
    let mmc3 = [
        0xa9, 0x00, 0x8d, 0x00, 0xe0, 0xa9, 0x00, 0x8d, 0x00, 0xa0, 0xa9, 0x00, 0x8d, 0x01, 0xa0,
        0xa9, 0x06, 0x8d, 0x00, 0x80, 0xa9, 0x00, 0x8d, 0x01, 0x80, 0xa9, 0x07, 0x8d, 0x00, 0x80,
        0xa9, 0x01, 0x8d, 0x01, 0x80,
    ];
    let poll = [0x2c, 0x02, 0x20, 0x10, 0xfb];
    let latch_reset_and_entry = [0x2c, 0x02, 0x20, 0x4c, 0x47, 0xe0];
    let palette_write = [
        0xa9, 0x3f, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, 0xa9, 0x12, 0x8d, 0x07, 0x20,
    ];

    let mut offset = 0;
    for expected in [
        &setup[..],
        &mmc3[..],
        &poll[..],
        &poll[..],
        &latch_reset_and_entry[..],
        &palette_write[..],
    ] {
        assert_eq!(&program[offset..offset + expected.len()], expected);
        offset += expected.len();
    }

    // Nothing writes the PPU address or data registers before the PPU is warm.
    // The MMC3 block between the setup and the polls writes only $8000, $A000,
    // $A001 and $E000.
    let polls_end = setup.len() + mmc3.len() + poll.len() * 2;
    assert!(!program[..polls_end]
        .windows(3)
        .any(|instruction| instruction == [0x8d, 0x06, 0x20] || instruction == [0x8d, 0x07, 0x20]));
}

#[test]
fn hand_built_mmc3_rom_is_byte_identical_after_assembler_migration() {
    let mut expected = vec![0xff; INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE];
    expected[..INES_HEADER_SIZE].copy_from_slice(&[
        b'N', b'E', b'S', 0x1a, 2, 0, 0x40, 0, 1, 0, 0, 0, 0, 0, 0, 0,
    ]);

    let fixed_bank_offset = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    // The 71-byte shared runtime, then M1's own body: the palette write, the two
    // writes that leave palette space, a halt loop that jumps to itself at $E060,
    // and the interrupt stub the vectors point at, at $E063.
    let reset_program = [
        0x78, 0xd8, 0xa2, 0x40, 0x8e, 0x17, 0x40, 0xa2, 0xff, 0x9a, 0xe8, 0x8e, 0x00, 0x20, 0x8e,
        0x01, 0x20, 0x8e, 0x10, 0x40, 0xa9, 0x00, 0x8d, 0x00, 0xe0, 0xa9, 0x00, 0x8d, 0x00, 0xa0,
        0xa9, 0x00, 0x8d, 0x01, 0xa0, 0xa9, 0x06, 0x8d, 0x00, 0x80, 0xa9, 0x00, 0x8d, 0x01, 0x80,
        0xa9, 0x07, 0x8d, 0x00, 0x80, 0xa9, 0x01, 0x8d, 0x01, 0x80, 0x2c, 0x02, 0x20, 0x10, 0xfb,
        0x2c, 0x02, 0x20, 0x10, 0xfb, 0x2c, 0x02, 0x20, 0x4c, 0x47, 0xe0, 0xa9, 0x3f, 0x8d, 0x06,
        0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, 0xa9, 0x12, 0x8d, 0x07, 0x20, 0xa9, 0x00, 0x8d, 0x06,
        0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, 0x4c, 0x60, 0xe0, 0x40,
    ];
    expected[fixed_bank_offset..fixed_bank_offset + reset_program.len()]
        .copy_from_slice(&reset_program);
    expected[INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - 6..]
        .copy_from_slice(&[0x63, 0xe0, 0x00, 0xe0, 0x63, 0xe0]);

    assert_eq!(m1_solid_backdrop_rom(), expected);
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
