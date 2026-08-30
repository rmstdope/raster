use std::io::Read;

pub const TILE_WIDTH: u32 = 8;
pub const TILE_HEIGHT: u32 = 8;
pub const MAX_TILES_WIDE: u32 = 32;
pub const MAX_TILES_HIGH: u32 = 30;
pub const TRANSPARENT_COLOUR: u8 = 0x0f;

/// FCEUX's 2C02 RGB palette, indexed by NES palette entry.
const FCEUX_2C02_PALETTE: [[u8; 3]; 64] = [
    [0x7c, 0x7c, 0x7c],
    [0x00, 0x00, 0xfc],
    [0x00, 0x00, 0xbc],
    [0x44, 0x28, 0xbc],
    [0x94, 0x00, 0x84],
    [0xa8, 0x00, 0x20],
    [0xa8, 0x10, 0x00],
    [0x88, 0x14, 0x00],
    [0x50, 0x30, 0x00],
    [0x00, 0x78, 0x00],
    [0x00, 0x68, 0x00],
    [0x00, 0x58, 0x00],
    [0x00, 0x40, 0x58],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xbc, 0xbc, 0xbc],
    [0x00, 0x78, 0xf8],
    [0x00, 0x58, 0xf8],
    [0x68, 0x44, 0xfc],
    [0xd8, 0x00, 0xcc],
    [0xe4, 0x00, 0x58],
    [0xf8, 0x38, 0x00],
    [0xe4, 0x5c, 0x10],
    [0xac, 0x7c, 0x00],
    [0x00, 0xb8, 0x00],
    [0x00, 0xa8, 0x00],
    [0x00, 0xa8, 0x44],
    [0x00, 0x88, 0x88],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xf8, 0xf8, 0xf8],
    [0x3c, 0xbc, 0xfc],
    [0x68, 0x88, 0xfc],
    [0x98, 0x78, 0xf8],
    [0xf8, 0x78, 0xf8],
    [0xf8, 0x58, 0x98],
    [0xf8, 0x78, 0x58],
    [0xfc, 0xa0, 0x44],
    [0xf8, 0xb8, 0x00],
    [0xb8, 0xf8, 0x18],
    [0x58, 0xd8, 0x54],
    [0x58, 0xf8, 0x98],
    [0x00, 0xe8, 0xd8],
    [0x78, 0x78, 0x78],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xfc, 0xfc, 0xfc],
    [0xa4, 0xe4, 0xfc],
    [0xb8, 0xb8, 0xf8],
    [0xd8, 0xb8, 0xf8],
    [0xf8, 0xb8, 0xf8],
    [0xf8, 0xa4, 0xc0],
    [0xf0, 0xd0, 0xb0],
    [0xfc, 0xe0, 0xa8],
    [0xf8, 0xd8, 0x78],
    [0xd8, 0xf8, 0x78],
    [0xb8, 0xf8, 0xb8],
    [0xb8, 0xf8, 0xd8],
    [0x00, 0xfc, 0xfc],
    [0xf8, 0xd8, 0xf8],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexedBackground {
    width_tiles: u32,
    height_tiles: u32,
    pixels: Vec<u8>,
}

impl IndexedBackground {
    pub fn width_tiles(&self) -> u32 {
        self.width_tiles
    }

    pub fn height_tiles(&self) -> u32 {
        self.height_tiles
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PngBackgroundError {
    Decode {
        message: String,
    },
    InvalidDimensions {
        width: u32,
        height: u32,
    },
    PartialAlpha {
        x: u32,
        y: u32,
        alpha: u8,
    },
    TooManyColours {
        attribute_x: u32,
        attribute_y: u32,
        colour_count: u8,
    },
}

pub fn decode_background(png: impl Read) -> Result<IndexedBackground, PngBackgroundError> {
    let mut decoder = png::Decoder::new(png);
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().map_err(decode_error)?;
    let mut bytes = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut bytes).map_err(decode_error)?;
    let width = frame.width;
    let height = frame.height;

    if width == 0
        || height == 0
        || width % TILE_WIDTH != 0
        || height % TILE_HEIGHT != 0
        || width > MAX_TILES_WIDE * TILE_WIDTH
        || height > MAX_TILES_HIGH * TILE_HEIGHT
    {
        return Err(PngBackgroundError::InvalidDimensions { width, height });
    }

    let mut pixels = Vec::with_capacity((width * height) as usize);
    let (rgba_pixels, remainder) = bytes[..frame.buffer_size()].as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    for (index, rgba) in rgba_pixels.iter().enumerate() {
        let x = index as u32 % width;
        let y = index as u32 / width;
        let colour = match rgba[3] {
            0 => TRANSPARENT_COLOUR,
            255 => nearest_palette_entry([rgba[0], rgba[1], rgba[2]]),
            alpha => return Err(PngBackgroundError::PartialAlpha { x, y, alpha }),
        };
        pixels.push(colour);
    }

    validate_attribute_regions(width, height, &pixels)?;

    Ok(IndexedBackground {
        width_tiles: width / TILE_WIDTH,
        height_tiles: height / TILE_HEIGHT,
        pixels,
    })
}

fn decode_error(error: png::DecodingError) -> PngBackgroundError {
    PngBackgroundError::Decode {
        message: error.to_string(),
    }
}

fn nearest_palette_entry(rgb: [u8; 3]) -> u8 {
    FCEUX_2C02_PALETTE
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| squared_distance(rgb, **candidate))
        .map(|(index, _)| index as u8)
        .expect("the FCEUX palette is non-empty")
}

fn squared_distance(left: [u8; 3], right: [u8; 3]) -> u32 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| {
            let difference = i32::from(left) - i32::from(right);
            (difference * difference) as u32
        })
        .sum()
}

fn validate_attribute_regions(
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), PngBackgroundError> {
    for attribute_y in 0..height.div_ceil(16) {
        for attribute_x in 0..width.div_ceil(16) {
            let mut present = [false; 64];
            for y in attribute_y * 16..(attribute_y + 1) * 16 {
                for x in attribute_x * 16..(attribute_x + 1) * 16 {
                    let colour = if x < width && y < height {
                        pixels[(y * width + x) as usize]
                    } else {
                        TRANSPARENT_COLOUR
                    };
                    present[colour as usize] = true;
                }
            }
            let colour_count = present.into_iter().filter(|present| *present).count() as u8;
            if colour_count > 4 {
                return Err(PngBackgroundError::TooManyColours {
                    attribute_x,
                    attribute_y,
                    colour_count,
                });
            }
        }
    }
    Ok(())
}
