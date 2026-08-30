use std::io::Cursor;

use raster_assets::{decode_background, PngBackgroundError, TRANSPARENT_COLOUR};

const PALETTE: [[u8; 3]; 5] = [
    [0x7c, 0x7c, 0x7c],
    [0x00, 0x00, 0xfc],
    [0x00, 0xbc, 0x00],
    [0xfc, 0xb0, 0x60],
    [0x00, 0x00, 0x00],
];

fn png(width: u32, height: u32, pixels: Vec<[u8; 4]>) -> Vec<u8> {
    assert_eq!(pixels.len(), (width * height) as usize);

    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let bytes = pixels.into_iter().flatten().collect::<Vec<_>>();
    writer.write_image_data(&bytes).unwrap();
    drop(writer);
    output
}

fn rgb_png(width: u32, height: u32, pixels: Vec<[u8; 3]>) -> Vec<u8> {
    assert_eq!(pixels.len(), (width * height) as usize);

    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let bytes = pixels.into_iter().flatten().collect::<Vec<_>>();
    writer.write_image_data(&bytes).unwrap();
    drop(writer);
    output
}

fn image(width: u32, height: u32, colour: [u8; 4]) -> Vec<[u8; 4]> {
    vec![colour; (width * height) as usize]
}

#[test]
fn maps_opaque_pixels_to_the_nearest_fceux_entry_and_transparent_pixels_to_0f() {
    let mut pixels = image(8, 8, [0x7c, 0x7c, 0x7c, 0xff]);
    pixels[1] = [0x01, 0x02, 0xfd, 0xff];
    pixels[2] = [0xaa, 0xbb, 0xcc, 0x00];

    let background = decode_background(Cursor::new(png(8, 8, pixels))).unwrap();

    assert_eq!(background.width_tiles(), 1);
    assert_eq!(background.height_tiles(), 1);
    assert_eq!(background.pixels()[0], 0x00);
    assert_eq!(background.pixels()[1], 0x01);
    assert_eq!(background.pixels()[2], TRANSPARENT_COLOUR);
}

#[test]
fn maps_rgb_png_pixels_as_fully_opaque() {
    let background =
        decode_background(Cursor::new(rgb_png(8, 8, vec![[0x00, 0x00, 0xfc]; 64]))).unwrap();

    assert!(background.pixels().iter().all(|colour| *colour == 0x01));
}

#[test]
fn rejects_dimensions_that_are_not_nonzero_8_pixel_multiples_within_32_by_30_tiles() {
    for (width, height) in [(9, 8), (264, 8), (8, 248)] {
        let error = decode_background(Cursor::new(png(
            width,
            height,
            image(width, height, [0x7c, 0x7c, 0x7c, 0xff]),
        )))
        .unwrap_err();

        assert_eq!(
            error,
            PngBackgroundError::InvalidDimensions { width, height }
        );
    }
}

#[test]
fn reports_the_first_partially_transparent_pixel() {
    let mut pixels = image(8, 8, [0x7c, 0x7c, 0x7c, 0xff]);
    pixels[2 * 8 + 3] = [0x7c, 0x7c, 0x7c, 128];

    assert_eq!(
        decode_background(Cursor::new(png(8, 8, pixels))),
        Err(PngBackgroundError::PartialAlpha {
            x: 3,
            y: 2,
            alpha: 128,
        })
    );
}

#[test]
fn accepts_four_colours_in_an_attribute_region() {
    let mut pixels = image(16, 16, [0x7c, 0x7c, 0x7c, 0xff]);
    for (index, colour) in PALETTE[..4].iter().enumerate() {
        pixels[index] = [colour[0], colour[1], colour[2], 0xff];
    }

    assert!(decode_background(Cursor::new(png(16, 16, pixels))).is_ok());
}

#[test]
fn rejects_the_first_five_colour_attribute_region() {
    let mut pixels = image(32, 16, [0x7c, 0x7c, 0x7c, 0xff]);
    for (index, colour) in PALETTE.iter().enumerate() {
        pixels[index] = [colour[0], colour[1], colour[2], 0xff];
        pixels[16 + index] = [colour[0], colour[1], colour[2], 0xff];
    }

    assert_eq!(
        decode_background(Cursor::new(png(32, 16, pixels))),
        Err(PngBackgroundError::TooManyColours {
            attribute_x: 0,
            attribute_y: 0,
            colour_count: 5,
        })
    );
}

#[test]
fn pads_an_incomplete_edge_attribute_region_with_0f() {
    let mut pixels = image(8, 8, [0x7c, 0x7c, 0x7c, 0xff]);
    for (index, colour) in PALETTE[..3].iter().enumerate() {
        pixels[index] = [colour[0], colour[1], colour[2], 0xff];
    }

    assert!(decode_background(Cursor::new(png(8, 8, pixels))).is_ok());
}
