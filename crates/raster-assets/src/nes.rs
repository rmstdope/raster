use crate::{IndexedBackground, TRANSPARENT_COLOUR};

pub const NAMETABLE_WIDTH_TILES: usize = 32;
pub const NAMETABLE_HEIGHT_TILES: usize = 30;
pub const NAMETABLE_BYTES: usize = NAMETABLE_WIDTH_TILES * NAMETABLE_HEIGHT_TILES;
pub const ATTRIBUTE_TABLE_BYTES: usize = 64;
pub const BACKGROUND_SUBPALETTE_COUNT: usize = 4;
pub const BACKGROUND_SUBPALETTE_BYTES: usize = 4;
pub const BACKGROUND_PALETTE_BYTES: usize =
    BACKGROUND_SUBPALETTE_COUNT * BACKGROUND_SUBPALETTE_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NesBackground {
    chr: Vec<u8>,
    nametable: [u8; NAMETABLE_BYTES],
    attributes: [u8; ATTRIBUTE_TABLE_BYTES],
    palettes: [u8; BACKGROUND_PALETTE_BYTES],
}

impl NesBackground {
    pub fn chr(&self) -> &[u8] {
        &self.chr
    }

    pub fn nametable(&self) -> &[u8; NAMETABLE_BYTES] {
        &self.nametable
    }

    pub fn attributes(&self) -> &[u8; ATTRIBUTE_TABLE_BYTES] {
        &self.attributes
    }

    pub fn palettes(&self) -> &[u8; BACKGROUND_PALETTE_BYTES] {
        &self.palettes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackgroundEncodeError {
    TooManyTiles { tile_count: usize },
    TooManyPalettes { palette_count: u8 },
}

pub fn encode_background(
    background: &IndexedBackground,
) -> Result<NesBackground, BackgroundEncodeError> {
    let mut palette_records = Vec::new();
    let mut region_palette_ids = [[0; 16]; 16];
    for (region_y, palette_ids) in region_palette_ids.iter_mut().enumerate() {
        for (region_x, palette_id) in palette_ids.iter_mut().enumerate() {
            let record = canonical_palette(background, region_x, region_y);
            let id = if record == [TRANSPARENT_COLOUR; BACKGROUND_SUBPALETTE_BYTES] {
                0
            } else if let Some(id) = palette_records.iter().position(|known| *known == record) {
                id
            } else {
                if palette_records.len() == BACKGROUND_SUBPALETTE_COUNT {
                    return Err(BackgroundEncodeError::TooManyPalettes { palette_count: 5 });
                }
                palette_records.push(record);
                palette_records.len() - 1
            };
            *palette_id = id as u8;
        }
    }
    if palette_records.is_empty() {
        palette_records.push([TRANSPARENT_COLOUR; BACKGROUND_SUBPALETTE_BYTES]);
    }

    let mut attributes = [0; ATTRIBUTE_TABLE_BYTES];
    for (region_y, palette_ids) in region_palette_ids.iter().enumerate() {
        for (region_x, palette_id) in palette_ids.iter().enumerate() {
            let attribute_index = (region_y / 2) * 8 + region_x / 2;
            let shift = (region_y % 2) * 4 + (region_x % 2) * 2;
            attributes[attribute_index] |= *palette_id << shift;
        }
    }

    let mut nametable = [0; NAMETABLE_BYTES];
    let mut chr_tiles = vec![[0; 16]];
    for tile_y in 0..background.height_tiles() as usize {
        for tile_x in 0..background.width_tiles() as usize {
            let palette = palette_records[region_palette_ids[tile_y / 2][tile_x / 2] as usize];
            let tile = encode_tile(background, tile_x, tile_y, &palette);
            let index = if let Some(index) = chr_tiles.iter().position(|known| *known == tile) {
                index
            } else {
                if chr_tiles.len() == 256 {
                    return Err(BackgroundEncodeError::TooManyTiles { tile_count: 257 });
                }
                chr_tiles.push(tile);
                chr_tiles.len() - 1
            };
            nametable[tile_y * NAMETABLE_WIDTH_TILES + tile_x] = index as u8;
        }
    }

    let mut palettes = [TRANSPARENT_COLOUR; BACKGROUND_PALETTE_BYTES];
    for (index, palette) in palette_records.iter().enumerate() {
        palettes[index * BACKGROUND_SUBPALETTE_BYTES..(index + 1) * BACKGROUND_SUBPALETTE_BYTES]
            .copy_from_slice(palette);
    }

    Ok(NesBackground {
        chr: chr_tiles.into_iter().flatten().collect(),
        nametable,
        attributes,
        palettes,
    })
}

fn canonical_palette(background: &IndexedBackground, region_x: usize, region_y: usize) -> [u8; 4] {
    let width = background.width_tiles() as usize * 8;
    let height = background.height_tiles() as usize * 8;
    let mut present = [false; 64];
    for y in region_y * 16..(region_y + 1) * 16 {
        for x in region_x * 16..(region_x + 1) * 16 {
            let colour = if x < width && y < height {
                background.pixels()[y * width + x]
            } else {
                TRANSPARENT_COLOUR
            };
            present[colour as usize] = true;
        }
    }

    let mut palette = [TRANSPARENT_COLOUR; 4];
    let mut slot = 1;
    for (colour, is_present) in present.iter().enumerate() {
        if *is_present && colour as u8 != TRANSPARENT_COLOUR {
            palette[slot] = colour as u8;
            slot += 1;
        }
    }
    palette
}

fn encode_tile(
    background: &IndexedBackground,
    tile_x: usize,
    tile_y: usize,
    palette: &[u8; BACKGROUND_SUBPALETTE_BYTES],
) -> [u8; 16] {
    let width = background.width_tiles() as usize * 8;
    let mut encoded = [0; 16];
    for row in 0..8 {
        for column in 0..8 {
            let colour = background.pixels()[(tile_y * 8 + row) * width + tile_x * 8 + column];
            let value = palette
                .iter()
                .position(|entry| *entry == colour)
                .expect("attribute palette includes every source colour")
                as u8;
            encoded[row] |= (value & 1) << (7 - column);
            encoded[8 + row] |= (value >> 1) << (7 - column);
        }
    }
    encoded
}
