use std::{io::Cursor, num::NonZeroU32};

use raster_assets::{
    decode_background, encode_background, IndexedBackground, NesBackground, TRANSPARENT_COLOUR,
};
use raster_emu::{render_after_frames, FRAME_PIXELS, FRAME_WIDTH};
use raster_link::{
    m5_background_rom, BackgroundData, LinkError, LinkedRom, INES_HEADER_SIZE,
    MMC3_FIXED_BANK_CODE_SIZE, MMC3_FIXED_BANK_SIZE, MMC3_FIXED_BANK_START, MMC3_PRG_ROM_SIZE,
};

const JMP_ABSOLUTE: u8 = 0x4c;
const RTI: u8 = 0x40;

const WIDTH: u32 = 256;
const HEIGHT: u32 = 240;

/// Four three-colour sets, each ascending, each an exact FCEUX entry.
///
/// Ascending because `canonical_palette` fills a record by scanning entries
/// `0..64` in order, so a record is always sorted; writing them sorted here
/// keeps `PALETTES[id][value]` the same mapping the encoder will produce.
const PALETTES: [[u8; 4]; 4] = [
    [TRANSPARENT_COLOUR, 0x01, 0x11, 0x21],
    [TRANSPARENT_COLOUR, 0x06, 0x16, 0x26],
    [TRANSPARENT_COLOUR, 0x09, 0x19, 0x29],
    [TRANSPARENT_COLOUR, 0x00, 0x10, 0x30],
];

/// The FCEUX RGB of the twelve entries above. The only RGB in this file, and it
/// is fixture input: nothing here is ever compared against a rendered pixel.
fn rgb(entry: u8) -> [u8; 3] {
    match entry {
        0x00 => [0x7c, 0x7c, 0x7c],
        0x01 => [0x00, 0x00, 0xfc],
        0x06 => [0xa8, 0x10, 0x00],
        0x09 => [0x00, 0x78, 0x00],
        0x10 => [0xbc, 0xbc, 0xbc],
        0x11 => [0x00, 0x78, 0xf8],
        0x16 => [0xf8, 0x38, 0x00],
        0x19 => [0x00, 0xb8, 0x00],
        0x21 => [0x3c, 0xbc, 0xfc],
        0x26 => [0xf8, 0x78, 0x58],
        0x29 => [0xb8, 0xf8, 0x18],
        0x30 => [0xfc, 0xfc, 0xfc],
        other => panic!("the fixture has no RGB for entry {other:#04x}"),
    }
}

/// The subpalette slot of pixel (`row`, `column`) inside tile pattern `p`.
///
/// Column 0 and row 0 are always non-zero, so the leftmost screen column and the
/// top row are real colours and PPUMASK's left-edge bit is under test. The
/// slot-0 pixel is what makes the universal background colour appear on screen.
fn value(p: u32, row: u32, column: u32) -> u8 {
    if column == 0 {
        3
    } else if row == 0 {
        2
    } else if row == 2 + p / 4 && column == 2 + p % 4 {
        0
    } else {
        1
    }
}

fn fixture_png() -> Vec<u8> {
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (tile_x, tile_y) = (x / 8, y / 8);
            let p = tile_x % 4 + 4 * (tile_y % 4);
            let palette = PALETTES[((tile_x / 2 + tile_y / 2) % 4) as usize];
            let rgba = match value(p, y % 8, x % 8) {
                0 => [0, 0, 0, 0],
                slot => {
                    let [r, g, b] = rgb(palette[slot as usize]);
                    [r, g, b, 0xff]
                }
            };
            pixels.push(rgba);
        }
    }

    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("the PNG header is written");
    writer
        .write_image_data(&pixels.into_iter().flatten().collect::<Vec<_>>())
        .expect("the PNG image data is written");
    drop(writer);
    output
}

#[test]
fn the_fixture_encodes_to_four_subpalettes_and_seventeen_chr_tiles() {
    let indexed = decode_background(Cursor::new(fixture_png())).expect("the fixture decodes");
    let encoded = encode_background(&indexed).expect("the fixture encodes");

    assert_eq!(indexed.width_tiles(), 32);
    assert_eq!(indexed.height_tiles(), 30);
    assert_eq!(encoded.chr().len(), 272); // 17 tiles
    assert_eq!(
        encoded.palettes(),
        &[
            0x0f, 0x01, 0x11, 0x21, //
            0x0f, 0x06, 0x16, 0x26, //
            0x0f, 0x09, 0x19, 0x29, //
            0x0f, 0x00, 0x10, 0x30,
        ],
    );
    assert_eq!(
        &encoded.attributes()[..8],
        [0x94, 0x3e, 0x94, 0x3e, 0x94, 0x3e, 0x94, 0x3e]
    );
    // tile 0 is reserved blank and the fixture never uses it
    assert!(encoded
        .nametable()
        .iter()
        .all(|tile| (1..=16).contains(tile)));
}

fn fixed_bank(rom: &[u8]) -> &[u8] {
    let offset = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    &rom[offset..]
}

fn linked_fixture() -> (IndexedBackground, NesBackground, LinkedRom) {
    let indexed = decode_background(Cursor::new(fixture_png())).expect("the fixture decodes");
    let encoded = encode_background(&indexed).expect("the fixture encodes");
    let rom = m5_background_rom(BackgroundData {
        palettes: encoded.palettes(),
        chr: encoded.chr(),
        nametable: encoded.nametable(),
        attributes: encoded.attributes(),
    })
    .expect("the M5 program links and fits its bank");
    (indexed, encoded, rom)
}

/// The offset of `block` inside `bank`, as one contiguous run.
fn one_run(bank: &[u8], block: &[u8], what: &str) -> usize {
    let first = bank
        .windows(block.len())
        .position(|window| window == block)
        .unwrap_or_else(|| panic!("the fixed bank carries the {what} as one contiguous run"));
    let next = bank[first + 1..]
        .windows(block.len())
        .position(|window| window == block);
    assert_eq!(next, None, "the {what} appears in the fixed bank once");
    first
}

#[test]
fn ships_the_encoded_background_in_the_fixed_bank() {
    let (_, encoded, rom) = linked_fixture();

    assert_eq!(
        &rom.image[..INES_HEADER_SIZE],
        // mapper 4, two 16 KiB PRG banks, zero CHR-ROM banks: CHR RAM
        &[b'N', b'E', b'S', 0x1a, 2, 0, 0x40, 0, 1, 0, 0, 0, 0, 0, 0, 0],
    );

    let bank = fixed_bank(&rom.image);
    // The halt loop is the only JMP that targets its own address.
    let halt = bank
        .windows(3)
        .enumerate()
        .position(|(offset, window)| {
            window[0] == JMP_ABSOLUTE
                && u16::from_le_bytes([window[1], window[2]])
                    == MMC3_FIXED_BANK_START + offset as u16
        })
        .expect("the body ends in a JMP halt loop");

    let palettes = one_run(bank, encoded.palettes(), "palettes");
    let chr = one_run(bank, encoded.chr(), "CHR");
    let nametable = one_run(bank, encoded.nametable(), "nametable");
    let attributes = one_run(bank, encoded.attributes(), "attributes");

    assert!(
        halt < palettes && palettes < chr && chr < nametable && nametable < attributes,
        "the four blocks follow the halt loop in order: \
         halt {halt}, palettes {palettes}, CHR {chr}, nametable {nametable}, \
         attributes {attributes}",
    );
}

#[test]
fn renders_the_source_png_entry_for_entry() {
    let (indexed, _, rom) = linked_fixture();

    // The reset runtime waits two vblanks and the upload costs most of another
    // frame, so the fourth is the first complete one. The program then halts
    // with rendering on, so every frame from there is identical.
    let frame = render_after_frames(
        "m5-background.nes",
        &rom.image,
        NonZeroU32::new(8).expect("eight is not zero"),
    )
    .expect("the ROM renders");

    let rendered = frame.as_indices();
    let differing: Vec<usize> = indexed
        .pixels()
        .iter()
        .zip(rendered.iter())
        .enumerate()
        .filter(|(_, (source, shown))| u16::from(**source) != **shown)
        .map(|(index, _)| index)
        .collect();
    if let Some(&index) = differing.first() {
        panic!(
            "{} of {FRAME_PIXELS} pixels differ; first at ({}, {}): \
             source entry {:#04x}, rendered {:#05x}",
            differing.len(),
            index % FRAME_WIDTH,
            index / FRAME_WIDTH,
            indexed.pixels()[index],
            rendered[index],
        );
    }
}

#[test]
fn keeps_the_program_and_its_data_clear_of_the_vector_table() {
    let (_, _, rom) = linked_fixture();

    assert!(
        rom.code_len <= MMC3_FIXED_BANK_CODE_SIZE,
        "the program and its data fit the fixed bank: {} of {MMC3_FIXED_BANK_CODE_SIZE} bytes",
        rom.code_len,
    );
    // The runtime's prologue is first in the bank.
    assert_eq!(rom.vectors.reset, MMC3_FIXED_BANK_START);
    // NMI and IRQ share the runtime's RTI, which sits after the data blocks.
    assert_eq!(rom.vectors.nmi, rom.vectors.irq);
    let bank = fixed_bank(&rom.image);
    let interrupt = usize::from(rom.vectors.irq - MMC3_FIXED_BANK_START);
    assert_eq!(bank[interrupt], RTI);
    assert!(
        interrupt < MMC3_FIXED_BANK_CODE_SIZE,
        "the interrupt handler is clear of the vector table: offset {interrupt}",
    );
}

#[test]
fn refuses_a_background_whose_data_would_reach_the_vector_table() {
    // The only input that can exhaust the bank, and the only path to the
    // function's error. 6516 CHR bytes is the largest that links on this
    // layout; a real background never exceeds 4096, because encode_background
    // caps at 256 distinct tiles.
    let palettes = [0u8; 16];
    let nametable = [0u8; 960];
    let attributes = [0u8; 64];
    let background = |chr: &[u8]| {
        m5_background_rom(BackgroundData {
            palettes: &palettes,
            chr,
            nametable: &nametable,
            attributes: &attributes,
        })
    };

    assert!(background(&vec![0u8; 6516]).is_ok());
    assert!(matches!(
        background(&vec![0u8; 6517]),
        Err(LinkError::FixedBankTooLarge { .. })
    ));

    // The figure `m5_background_rom`'s doc comment quotes. It is the whole
    // program, not the walk to the first label past the limit. If this number
    // ever moves, move the one in that comment (`m5.rs`, `m5_background_rom`)
    // with it - this assertion is the only thing holding the two together.
    assert_eq!(
        background(&vec![0u8; 8192]),
        Err(LinkError::FixedBankTooLarge {
            actual: 9953,
            maximum: MMC3_FIXED_BANK_CODE_SIZE,
        })
    );
}
