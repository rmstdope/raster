use std::io::Cursor;

use raster_assets::{convert_background, NesBackground, NAMETABLE_WIDTH_TILES};

/// The FCEUX colour of the NES entries these pictures are drawn in.
fn rgb(entry: u8) -> [u8; 3] {
    match entry {
        0x0f => [0x00, 0x00, 0x00],
        0x01 => [0x00, 0x00, 0xfc],
        0x09 => [0x00, 0x78, 0x00],
        0x11 => [0x00, 0x78, 0xf8],
        0x16 => [0xf8, 0x38, 0x00],
        0x19 => [0x00, 0xb8, 0x00],
        0x20 => [0xf8, 0xf8, 0xf8],
        0x27 => [0xfc, 0xa0, 0x44],
        0x28 => [0xf8, 0xb8, 0x00],
        0x14 => [0xd8, 0x00, 0xcc],
        _ => panic!("no colour for ${entry:02X} in this test"),
    }
}

fn png(width: u32, height: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
    assert_eq!(pixels.len(), (width * height) as usize);
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&pixels.iter().flatten().copied().collect::<Vec<_>>())
        .unwrap();
    drop(writer);
    output
}

/// A picture of NES entries as an opaque PNG, every pixel drawn `scale` by `scale`.
fn picture_png(width: u32, height: u32, entries: &[u8], scale: u32) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..height * scale {
        for x in 0..width * scale {
            let [r, g, b] = rgb(entries[((y / scale) * width + x / scale) as usize]);
            pixels.push([r, g, b, 0xff]);
        }
    }
    png(width * scale, height * scale, &pixels)
}

/// The NES entry the encoded background shows at (`x`, `y`): the tile's two bits pick a colour
/// of the palette its attribute names, and 0 is the backdrop.
fn shown(background: &NesBackground, x: usize, y: usize) -> u8 {
    let tile = background.nametable()[(y / 8) * NAMETABLE_WIDTH_TILES + x / 8] as usize;
    let bits = &background.chr()[tile * 16..tile * 16 + 16];
    let bit = 7 - x % 8;
    let value = ((bits[y % 8] >> bit) & 1) | (((bits[8 + y % 8] >> bit) & 1) << 1);
    let attribute = background.attributes()[(y / 32) * 8 + x / 32];
    let palette = (attribute >> (((y / 16) % 2) * 4 + ((x / 16) % 2) * 2)) & 3;
    if value == 0 {
        background.palettes()[0]
    } else {
        background.palettes()[usize::from(palette * 4 + value)]
    }
}

fn shown_picture(background: &NesBackground) -> Vec<u8> {
    (0..240)
        .flat_map(|y| (0..256).map(move |x| shown(background, x, y)))
        .collect()
}

/// A full-screen picture whose 16-by-16 regions each draw in black and one of `palettes`,
/// chosen by `which`, in a pattern of 4-pixel squares so that tiles repeat.
fn legal_picture(palettes: &[[u8; 3]], which: impl Fn(usize, usize) -> usize) -> Vec<u8> {
    (0..240)
        .flat_map(|y| {
            let palettes = &palettes;
            let which = &which;
            (0..256).map(move |x| {
                let palette = palettes[which(x / 16, y / 16)];
                match (x / 4 + y / 4) % 4 {
                    0 => 0x0f,
                    slot => palette[slot - 1],
                }
            })
        })
        .collect()
}

#[test]
fn a_legal_picture_drawn_twice_the_size_converts_to_the_picture_it_was_drawn_from() {
    let palettes = [
        [0x01, 0x11, 0x20],
        [0x16, 0x27, 0x28],
        [0x19, 0x09, 0x20],
        [0x16, 0x01, 0x19],
    ];
    let picture = legal_picture(&palettes, |x, y| (x + y) % 4);

    let background = convert_background(Cursor::new(picture_png(256, 240, &picture, 2))).unwrap();

    assert_eq!(shown_picture(&background), picture);
}

#[test]
fn a_picture_half_the_size_is_drawn_twice_as_large() {
    let palettes = [[0x01, 0x11, 0x20], [0x16, 0x27, 0x28]];
    let picture = legal_picture(&palettes, |x, y| (x + y) % 2);
    let half: Vec<u8> = (0..120)
        .flat_map(|y| (0..128).map(move |x| (y, x)))
        .map(|(y, x)| picture[y * 2 * 256 + x * 2])
        .collect();

    let background = convert_background(Cursor::new(picture_png(128, 120, &half, 1))).unwrap();

    let expected: Vec<u8> = (0..240)
        .flat_map(|y| (0..256).map(move |x| (y, x)))
        .map(|(y, x)| half[(y / 2) * 128 + x / 2])
        .collect();
    assert_eq!(shown_picture(&background), expected);
}

#[test]
fn a_region_of_five_colours_keeps_the_three_that_matter_most_and_moves_the_rest_to_the_nearest() {
    // Region (0, 0) holds red, orange and blue, and a little yellow, which is nearest orange.
    let mut picture = vec![0x0fu8; 256 * 240];
    for y in 0..16 {
        for x in 0..16 {
            picture[y * 256 + x] = match y {
                0..=5 => 0x16,
                6..=9 => 0x27,
                10..=12 => 0x01,
                13..=14 => 0x28,
                _ => 0x0f,
            };
        }
    }

    let background = convert_background(Cursor::new(picture_png(256, 240, &picture, 1))).unwrap();

    let mut expected = picture;
    for colour in expected.iter_mut() {
        if *colour == 0x28 {
            *colour = 0x27;
        }
    }
    assert_eq!(shown_picture(&background), expected);
}

#[test]
fn five_kinds_of_region_share_four_palettes_and_only_the_two_alike_are_merged() {
    // Regions in palettes 1 and 4 differ only in yellow against orange: those two share a
    // palette, and every other region keeps its colours exactly.
    let palettes = [
        [0x01, 0x11, 0x20],
        [0x16, 0x27, 0x20],
        [0x19, 0x09, 0x20],
        [0x14, 0x01, 0x19],
        [0x16, 0x28, 0x20],
    ];
    let picture = legal_picture(&palettes, |x, y| (x + 2 * y) % 5);

    let background = convert_background(Cursor::new(picture_png(256, 240, &picture, 1))).unwrap();

    let shown = shown_picture(&background);
    for (index, (&shown, &drawn)) in shown.iter().zip(&picture).enumerate() {
        let (x, y) = (index % 256, index / 256);
        if matches!(drawn, 0x27 | 0x28) {
            assert!(
                matches!(shown, 0x27 | 0x28),
                "({x}, {y}) is ${drawn:02X} and shows ${shown:02X}"
            );
        } else {
            assert_eq!(shown, drawn, "({x}, {y})");
        }
    }
}

#[test]
fn a_transparent_pixel_shows_the_backdrop() {
    let mut pixels = vec![[0xf8, 0x38, 0x00, 0xff]; 256 * 240];
    pixels[0] = [0x12, 0x34, 0x56, 0x00];

    let background = convert_background(Cursor::new(png(256, 240, &pixels))).unwrap();

    assert_eq!(shown(&background, 0, 0), 0x0f);
    assert_eq!(shown(&background, 1, 0), 0x16);
}

#[test]
fn a_picture_of_more_distinct_tiles_than_a_pattern_table_holds_is_drawn_in_256() {
    // Every tile of this picture is different: its rows count the tile's index in binary.
    let mut picture = vec![0x0fu8; 256 * 240];
    for tile in 0..960usize {
        let (tile_x, tile_y) = (tile % 32, tile / 32);
        for bit in 0..10 {
            if tile >> bit & 1 == 1 {
                picture[(tile_y * 8 + bit % 8) * 256 + tile_x * 8 + bit / 8] = 0x20;
            }
        }
    }

    let background = convert_background(Cursor::new(picture_png(256, 240, &picture, 1))).unwrap();

    assert!(
        background.chr().len() / 16 <= 256,
        "{} tiles",
        background.chr().len() / 16
    );
}

#[test]
fn tiles_alike_share_a_pattern_and_tiles_unlike_keep_theirs() {
    // 200 tiles of white noise, each unlike the others in about half its pixels, and 760 copies
    // of them with one pixel changed: 960 distinct tiles, of which the 200 are what matter.
    // Drawn in 256, every tile is its own noise or that noise with its one pixel back.
    let mut seed = 0x2545_f491u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let noise: Vec<u64> = (0..200)
        .map(|_| u64::from(next()) << 32 | u64::from(next()))
        .collect();
    let mut picture = vec![0x0fu8; 256 * 240];
    for tile in 0..960usize {
        let mut bits = noise[tile % 200];
        if tile >= 200 {
            bits ^= 1 << (tile / 200 * 13 + tile % 7);
        }
        for bit in 0..64 {
            if bits >> bit & 1 == 1 {
                picture[((tile / 32) * 8 + bit / 8) * 256 + (tile % 32) * 8 + bit % 8] = 0x20;
            }
        }
    }

    let background = convert_background(Cursor::new(picture_png(256, 240, &picture, 1))).unwrap();

    assert!(background.chr().len() / 16 <= 256);
    let shown = shown_picture(&background);
    for tile in 0..960usize {
        let differing = (0..64)
            .filter(|bit| {
                let index = ((tile / 32) * 8 + bit / 8) * 256 + (tile % 32) * 8 + bit % 8;
                shown[index] != picture[index]
            })
            .count();
        assert!(differing <= 1, "tile {tile} is {differing} pixels off");
    }
}

#[test]
fn a_picture_scaled_down_is_the_average_of_what_each_screen_pixel_covers() {
    // Twice the screen, in 2-by-2 blocks of white and black: their average, (7C, 7C, 7C), is
    // NES grey $00, a colour the picture never draws. Three times as wide as the screen and as
    // tall, it keeps its shape as a band 80 lines tall, each pixel the average of 3 by 3, and
    // two whites to a black is nearer light grey $10 than grey or white.
    let checker: Vec<[u8; 4]> = (0..480)
        .flat_map(|y| (0..512).map(move |x| (x, y)))
        .map(|(x, y)| {
            if (x + y) % 2 == 0 {
                [0xf8, 0xf8, 0xf8, 0xff]
            } else {
                [0x00, 0x00, 0x00, 0xff]
            }
        })
        .collect();
    let background = convert_background(Cursor::new(png(512, 480, &checker))).unwrap();
    assert!(shown_picture(&background)
        .iter()
        .all(|&entry| entry == 0x00));

    let wide: Vec<[u8; 4]> = (0..240)
        .flat_map(|_| 0..768)
        .map(|x| {
            if x % 3 == 2 {
                [0x00, 0x00, 0x00, 0xff]
            } else {
                [0xf8, 0xf8, 0xf8, 0xff]
            }
        })
        .collect();
    let background = convert_background(Cursor::new(png(768, 240, &wide))).unwrap();
    for (index, &entry) in shown_picture(&background).iter().enumerate() {
        let expected = if (80..160).contains(&(index / 256)) {
            0x10
        } else {
            0x0f
        };
        assert_eq!(entry, expected, "({}, {})", index % 256, index / 256);
    }
}

#[test]
fn a_square_picture_keeps_its_shape_between_black_borders() {
    let red = vec![[0xf8, 0x38, 0x00, 0xff]; 480 * 480];

    let background = convert_background(Cursor::new(png(480, 480, &red))).unwrap();

    for (index, &entry) in shown_picture(&background).iter().enumerate() {
        let expected = if (8..248).contains(&(index % 256)) {
            0x16
        } else {
            0x0f
        };
        assert_eq!(entry, expected, "({}, {})", index % 256, index / 256);
    }
}

#[test]
fn a_picture_of_224_lines_fits_unscaled_between_black_borders() {
    // The usual capture of an NES screen, without the lines a TV hides: no line is doubled.
    let palettes = [[0x01, 0x11, 0x20], [0x16, 0x27, 0x28]];
    let picture = legal_picture(&palettes, |x, y| (x + y) % 2);
    let capture = &picture[8 * 256..232 * 256];

    let background = convert_background(Cursor::new(picture_png(256, 224, capture, 1))).unwrap();

    let shown = shown_picture(&background);
    assert!(shown[..8 * 256].iter().all(|&entry| entry == 0x0f));
    assert_eq!(&shown[8 * 256..232 * 256], capture);
    assert!(shown[232 * 256..].iter().all(|&entry| entry == 0x0f));
}

#[test]
fn a_screen_pixel_mostly_transparent_is_the_backdrop_and_mostly_opaque_is_not() {
    // In 2-by-2 blocks: a block with one transparent pixel of four is red, and one with three
    // is the backdrop.
    let red = [0xf8, 0x38, 0x00, 0xff];
    let clear = [0x00, 0x00, 0x00, 0x00];
    let pixels: Vec<[u8; 4]> = (0..480)
        .flat_map(|y| (0..512).map(move |x| (x, y)))
        .map(|(x, y)| {
            let transparent_in_block = if (x / 2) % 2 == 0 { 1 } else { 3 };
            if (y % 2) * 2 + x % 2 < transparent_in_block {
                clear
            } else {
                red
            }
        })
        .collect();

    let background = convert_background(Cursor::new(png(512, 480, &pixels))).unwrap();

    for x in 0..256 {
        let expected = if x % 2 == 0 { 0x16 } else { 0x0f };
        assert_eq!(shown(&background, x, 0), expected, "x = {x}");
    }
}

/// A 256-by-240 PNG of one colour, written in `colour_type` at `depth` from the bytes of one
/// pixel, with `palette` for an indexed one.
fn solid_png(
    colour_type: png::ColorType,
    depth: png::BitDepth,
    pixel: &[u8],
    palette: Option<&[u8]>,
) -> Vec<u8> {
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, 256, 240);
    encoder.set_color(colour_type);
    encoder.set_depth(depth);
    if let Some(palette) = palette {
        encoder.set_palette(palette.to_vec());
    }
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&pixel.repeat(256 * 240)).unwrap();
    drop(writer);
    output
}

#[test]
fn every_png_colour_type_decodes_to_the_colour_it_holds() {
    use png::{BitDepth::*, ColorType::*};
    for (name, picture) in [
        ("rgb", solid_png(Rgb, Eight, &[0xf8, 0x38, 0x00], None)),
        (
            "rgb16",
            solid_png(Rgb, Sixteen, &[0xf8, 0, 0x38, 0, 0x00, 0], None),
        ),
        (
            "rgba16",
            solid_png(Rgba, Sixteen, &[0xf8, 0, 0x38, 0, 0, 0, 0xff, 0xff], None),
        ),
        (
            "indexed",
            solid_png(Indexed, Eight, &[1], Some(&[0, 0, 0, 0xf8, 0x38, 0x00])),
        ),
    ] {
        let background = convert_background(Cursor::new(picture)).unwrap();
        assert_eq!(shown(&background, 100, 100), 0x16, "{name}");
    }
    for (name, picture) in [
        ("grey", solid_png(Grayscale, Eight, &[0x7c], None)),
        ("grey16", solid_png(Grayscale, Sixteen, &[0x7c, 0], None)),
        (
            "grey alpha",
            solid_png(GrayscaleAlpha, Eight, &[0x7c, 0xff], None),
        ),
    ] {
        let background = convert_background(Cursor::new(picture)).unwrap();
        assert_eq!(shown(&background, 100, 100), 0x00, "{name}");
    }
}

#[test]
fn no_two_palettes_are_the_same() {
    // A red-to-blue ramp repeated across 10000 pixels and one row: every screen pixel averages
    // about 39 of them, so the cells differ only a little, and a palette fitted to the cell drawn
    // worst can come out the same as one already chosen. A copy is a palette wasted.
    let pixels: Vec<[u8; 4]> = (0..10000)
        .map(|x| [(x % 256) as u8, 0, 255 - (x % 256) as u8, 0xff])
        .collect();

    let background = convert_background(Cursor::new(png(10000, 1, &pixels))).unwrap();

    let palettes: Vec<&[u8]> = background.palettes().chunks(4).collect();
    for first in 0..4 {
        for second in first + 1..4 {
            assert_ne!(
                palettes[first], palettes[second],
                "palettes {first} and {second}: {palettes:02x?}"
            );
        }
    }
}
