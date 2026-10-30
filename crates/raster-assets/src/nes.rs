use crate::{IndexedBackground, MAX_TILES_HIGH, MAX_TILES_WIDE, TRANSPARENT_COLOUR};

pub const NAMETABLE_WIDTH_TILES: usize = MAX_TILES_WIDE as usize;
pub const NAMETABLE_HEIGHT_TILES: usize = MAX_TILES_HIGH as usize;
pub const NAMETABLE_BYTES: usize = NAMETABLE_WIDTH_TILES * NAMETABLE_HEIGHT_TILES;
pub const ATTRIBUTE_TABLE_BYTES: usize = 64;
pub const BACKGROUND_SUBPALETTE_COUNT: usize = 4;
pub const BACKGROUND_SUBPALETTE_BYTES: usize = 4;
pub const BACKGROUND_PALETTE_BYTES: usize =
    BACKGROUND_SUBPALETTE_COUNT * BACKGROUND_SUBPALETTE_BYTES;
/// The first tile row a picture split across two pattern tables draws from the second: the top
/// half of the screen, lines 0–119, is tiles 0–255, and the bottom half tiles 256–511.
pub const SPLIT_TILE_ROW: usize = NAMETABLE_HEIGHT_TILES / 2;
/// The tiles one pattern table holds, the blank one every background starts from among them.
const PATTERN_TABLE_TILES: usize = 256;
/// The tiles at the start of [`SPLIT_TILE_ROW`] a split picture keeps in both pattern tables, at
/// the same index in each.
///
/// The PPU fetches a line's first two tiles at the end of the line before, and an MMC3 IRQ
/// counted to that line runs its handler from about dot 260: the write switching pattern tables
/// lands as those fetches happen. Measured, it lands during the first tile's fetch or the
/// second's, depending on the frame. Kept in both, they look the same whichever table they are
/// fetched from, and the third is margin. That margin assumes the CPU waits for the IRQ in the
/// three-cycle `JMP` a `using irq` frame spins in: code running beside the chain, in instructions
/// of up to seven cycles, would use it up.
pub const SPLIT_SHARED_TILES: usize = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NesBackground {
    chr: Vec<u8>,
    nametable: [u8; NAMETABLE_BYTES],
    attributes: [u8; ATTRIBUTE_TABLE_BYTES],
    palettes: [u8; BACKGROUND_PALETTE_BYTES],
    pattern_tables: usize,
}

impl NesBackground {
    /// 1, or 2 for a picture split at [`SPLIT_TILE_ROW`], whose `chr` is then both pattern
    /// tables whole and whose nametable counts the lower rows' tiles from the second.
    pub fn pattern_tables(&self) -> usize {
        self.pattern_tables
    }

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
    TooManyTiles {
        tile_count: usize,
    },
    /// A picture split across two pattern tables needs more than one holds in one half. The
    /// upper half's count includes the first tiles of the lower half, which both tables hold.
    TooManyTilesInHalf {
        lower: bool,
        tile_count: usize,
    },
    TooManyPalettes {
        palette_count: u8,
    },
    TooManyColours {
        colour_count: u8,
    },
}

pub fn encode_background(
    background: &IndexedBackground,
) -> Result<NesBackground, BackgroundEncodeError> {
    encode(background, false)
}

/// [`encode_background`] across two pattern tables: the rows above [`SPLIT_TILE_ROW`] in the
/// first and the rows below it in the second, each holding up to 256 tiles of its own.
pub fn encode_background_split(
    background: &IndexedBackground,
) -> Result<NesBackground, BackgroundEncodeError> {
    encode(background, true)
}

fn encode(
    background: &IndexedBackground,
    split: bool,
) -> Result<NesBackground, BackgroundEncodeError> {
    let mut palette_records = Vec::new();
    let mut region_palette_ids = [[0; 16]; 16];
    for (region_y, palette_ids) in region_palette_ids.iter_mut().enumerate() {
        for (region_x, palette_id) in palette_ids.iter_mut().enumerate() {
            let record = canonical_palette(background, region_x, region_y)?;
            let id = if record == [TRANSPARENT_COLOUR; BACKGROUND_SUBPALETTE_BYTES] {
                0
            } else if let Some(id) = palette_records.iter().position(|known| *known == record) {
                id
            } else {
                if palette_records.len() == BACKGROUND_SUBPALETTE_COUNT {
                    return Err(BackgroundEncodeError::TooManyPalettes {
                        palette_count: (palette_records.len() + 1) as u8,
                    });
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
    assemble(background, &palette_records, &region_palette_ids, split)
}

/// The background `background` draws with `palette_records`, region by region as
/// `region_palette_ids` says: its attributes, its tiles deduplicated in row-major order, and its
/// palettes. Every pixel's colour must be in its region's palette. Split, the rows from
/// [`SPLIT_TILE_ROW`] down are deduplicated on their own, into the second pattern table, and the
/// first [`SPLIT_SHARED_TILES`] of that row start both tables.
///
/// A picture needing more than 256 tiles in a pattern table is refused with how many it needs,
/// all of them, so an author knows how far over it is.
pub(crate) fn assemble(
    background: &IndexedBackground,
    palette_records: &[[u8; BACKGROUND_SUBPALETTE_BYTES]],
    region_palette_ids: &[[u8; 16]; 16],
    split: bool,
) -> Result<NesBackground, BackgroundEncodeError> {
    let mut attributes = [0; ATTRIBUTE_TABLE_BYTES];
    for (region_y, palette_ids) in region_palette_ids.iter().enumerate() {
        for (region_x, palette_id) in palette_ids.iter().enumerate() {
            let attribute_index = (region_y / 2) * 8 + region_x / 2;
            let shift = (region_y % 2) * 4 + (region_x % 2) * 2;
            attributes[attribute_index] |= *palette_id << shift;
        }
    }

    let mut nametable = [0; NAMETABLE_BYTES];
    let mut start = vec![[0; 16]];
    // A picture too short to reach the split has no lower half to share.
    if split && background.height_tiles() as usize > SPLIT_TILE_ROW {
        for tile_x in 0..SPLIT_SHARED_TILES.min(background.width_tiles() as usize) {
            let palette =
                palette_records[region_palette_ids[SPLIT_TILE_ROW / 2][tile_x / 2] as usize];
            let tile = encode_tile(background, tile_x, SPLIT_TILE_ROW, &palette);
            if !start.contains(&tile) {
                start.push(tile);
            }
        }
    }
    let mut tables = vec![start; if split { 2 } else { 1 }];
    for tile_y in 0..background.height_tiles() as usize {
        let chr_tiles = &mut tables[usize::from(split && tile_y >= SPLIT_TILE_ROW)];
        for tile_x in 0..background.width_tiles() as usize {
            let palette = palette_records[region_palette_ids[tile_y / 2][tile_x / 2] as usize];
            let tile = encode_tile(background, tile_x, tile_y, &palette);
            let index = if let Some(index) = chr_tiles.iter().position(|known| *known == tile) {
                index
            } else {
                chr_tiles.push(tile);
                chr_tiles.len() - 1
            };
            nametable[tile_y * NAMETABLE_WIDTH_TILES + tile_x] = index as u8;
        }
    }
    if let Some((table, tiles)) = tables
        .iter()
        .enumerate()
        .find(|(_, tiles)| tiles.len() > PATTERN_TABLE_TILES)
    {
        let tile_count = tiles.len();
        return Err(if split {
            BackgroundEncodeError::TooManyTilesInHalf {
                lower: table == 1,
                tile_count,
            }
        } else {
            BackgroundEncodeError::TooManyTiles { tile_count }
        });
    }
    // Split, the first pattern table is filled out so that the second starts at tile 256.
    if split {
        tables[0].resize(PATTERN_TABLE_TILES, [0; 16]);
        tables[1].resize(PATTERN_TABLE_TILES, [0; 16]);
    }

    let mut palettes = [TRANSPARENT_COLOUR; BACKGROUND_PALETTE_BYTES];
    for (index, palette) in palette_records.iter().enumerate() {
        palettes[index * BACKGROUND_SUBPALETTE_BYTES..(index + 1) * BACKGROUND_SUBPALETTE_BYTES]
            .copy_from_slice(palette);
    }

    Ok(NesBackground {
        chr: tables.into_iter().flatten().flatten().collect(),
        nametable,
        attributes,
        palettes,
        pattern_tables: if split { 2 } else { 1 },
    })
}

fn canonical_palette(
    background: &IndexedBackground,
    region_x: usize,
    region_y: usize,
) -> Result<[u8; 4], BackgroundEncodeError> {
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
            if slot == BACKGROUND_SUBPALETTE_BYTES {
                let colour_count = present
                    .iter()
                    .enumerate()
                    .filter(|(colour, is_present)| {
                        **is_present && *colour as u8 != TRANSPARENT_COLOUR
                    })
                    .count() as u8;
                return Err(BackgroundEncodeError::TooManyColours { colour_count });
            }
            palette[slot] = colour as u8;
            slot += 1;
        }
    }
    Ok(palette)
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
