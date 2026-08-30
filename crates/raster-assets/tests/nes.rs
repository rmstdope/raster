use std::io::Cursor;

use raster_assets::{
    decode_background, encode_background, BackgroundEncodeError, BACKGROUND_PALETTE_BYTES,
    NAMETABLE_BYTES, TRANSPARENT_COLOUR,
};

const COLOURS: [[u8; 4]; 6] = [
    [0, 0, 0, 0],
    [0x00, 0x00, 0xfc, 0xff],
    [0x00, 0x00, 0xbc, 0xff],
    [0x44, 0x28, 0xbc, 0xff],
    [0x94, 0x00, 0x84, 0xff],
    [0xa8, 0x00, 0x20, 0xff],
];

fn png(width: u32, height: u32, pixels: Vec<[u8; 4]>) -> Vec<u8> {
    assert_eq!(pixels.len(), (width * height) as usize);

    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&pixels.into_iter().flatten().collect::<Vec<_>>())
        .unwrap();
    drop(writer);
    output
}

fn encode(width: u32, height: u32, pixels: Vec<[u8; 4]>) -> raster_assets::NesBackground {
    let background = decode_background(Cursor::new(png(width, height, pixels))).unwrap();
    encode_background(&background).unwrap()
}

#[test]
fn encodes_a_two_bit_tile_into_nes_bitplanes_and_a_32_by_30_nametable() {
    let mut pixels = Vec::new();
    for _ in 0..8 {
        pixels.extend([
            COLOURS[0], COLOURS[1], COLOURS[2], COLOURS[3], COLOURS[0], COLOURS[0], COLOURS[0],
            COLOURS[0],
        ]);
    }

    let background = encode(8, 8, pixels);

    assert_eq!(&background.chr()[..16], &[0; 16]);
    assert_eq!(
        &background.chr()[16..32],
        &[0b0101_0000; 8]
            .into_iter()
            .chain([0b0011_0000; 8])
            .collect::<Vec<_>>()
    );
    assert_eq!(background.nametable()[0], 1);
    assert!(background.nametable()[1..].iter().all(|entry| *entry == 0));
    assert_eq!(background.nametable().len(), NAMETABLE_BYTES);
    assert_eq!(background.attributes()[0], 0);
    assert_eq!(background.palettes()[..4], [TRANSPARENT_COLOUR, 1, 2, 3]);
}

#[test]
fn deduplicates_identical_encoded_tiles_in_row_major_order() {
    let mut pixels = Vec::new();
    for _ in 0..8 {
        pixels.extend([COLOURS[1]; 16]);
    }

    let background = encode(16, 8, pixels);

    assert_eq!(background.nametable()[..2], [1, 1]);
    assert_eq!(background.chr().len(), 32);
}

#[test]
fn packs_attribute_quadrants_with_their_palette_ids() {
    let mut pixels = vec![COLOURS[0]; 32 * 32];
    for region_y in 0..2 {
        for region_x in 0..2 {
            let colour = COLOURS[region_y * 2 + region_x + 1];
            for y in region_y * 16..(region_y + 1) * 16 {
                for x in region_x * 16..(region_x + 1) * 16 {
                    pixels[y * 32 + x] = colour;
                }
            }
        }
    }

    let background = encode(32, 32, pixels);

    assert_eq!(
        background.palettes(),
        &[
            TRANSPARENT_COLOUR,
            1,
            TRANSPARENT_COLOUR,
            TRANSPARENT_COLOUR,
            TRANSPARENT_COLOUR,
            2,
            TRANSPARENT_COLOUR,
            TRANSPARENT_COLOUR,
            TRANSPARENT_COLOUR,
            3,
            TRANSPARENT_COLOUR,
            TRANSPARENT_COLOUR,
            TRANSPARENT_COLOUR,
            4,
            TRANSPARENT_COLOUR,
            TRANSPARENT_COLOUR,
        ]
    );
    assert_eq!(background.palettes().len(), BACKGROUND_PALETTE_BYTES);
    assert_eq!(background.attributes()[0], 0b11_10_01_00);
}

#[test]
fn rejects_the_257th_unique_chr_tile() {
    let mut pixels = vec![COLOURS[0]; 256 * 64];
    for tile in 0..256 {
        for bit in 0..16 {
            if (tile + 1) & (1 << bit) != 0 {
                let x = (tile % 32) * 8 + bit % 8;
                let y = (tile / 32) * 8 + bit / 8;
                pixels[y * 256 + x] = COLOURS[1];
            }
        }
    }

    let background = decode_background(Cursor::new(png(256, 64, pixels))).unwrap();

    assert_eq!(
        encode_background(&background),
        Err(BackgroundEncodeError::TooManyTiles { tile_count: 257 })
    );
}

#[test]
fn rejects_the_fifth_distinct_background_subpalette() {
    let mut pixels = vec![COLOURS[0]; 80 * 16];
    for region_x in 0..5 {
        for y in 0..16 {
            for x in region_x * 16..(region_x + 1) * 16 {
                pixels[y * 80 + x] = COLOURS[region_x + 1];
            }
        }
    }

    let background = decode_background(Cursor::new(png(80, 16, pixels))).unwrap();

    assert_eq!(
        encode_background(&background),
        Err(BackgroundEncodeError::TooManyPalettes { palette_count: 5 })
    );
}

#[test]
fn rejects_an_attribute_region_with_four_nontransparent_colours() {
    let mut pixels = vec![COLOURS[1]; 16 * 16];
    for (index, colour) in COLOURS[2..5].iter().enumerate() {
        pixels[index] = *colour;
    }
    let background = decode_background(Cursor::new(png(16, 16, pixels))).unwrap();

    assert_eq!(
        encode_background(&background),
        Err(BackgroundEncodeError::TooManyColours { colour_count: 4 })
    );
}
