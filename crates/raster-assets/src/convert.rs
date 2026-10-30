//! Turning any PNG into a background, for `convert: true`.
//!
//! [`decode_background`](crate::decode_background) refuses a picture the NES cannot show as it
//! stands; this makes one it can. The picture is scaled to the screen, and four palettes of three
//! colours are fitted to it, each 16-by-16 region drawing in the backdrop and the palette that
//! loses it least. Every pixel then takes the nearest colour it is allowed. A picture that needs
//! more tiles than a pattern table holds is drawn in as many as a pattern table holds, each tile
//! in the pattern that looks most like it in its own palette.

use std::io::Read;

use crate::nes::{assemble, BACKGROUND_SUBPALETTE_COUNT, SPLIT_SHARED_TILES, SPLIT_TILE_ROW};
use crate::{
    IndexedBackground, NesBackground, FCEUX_2C02_PALETTE, MAX_TILES_HIGH, MAX_TILES_WIDE,
    TRANSPARENT_COLOUR,
};

const WIDTH: usize = (MAX_TILES_WIDE * 8) as usize;
const HEIGHT: usize = (MAX_TILES_HIGH * 8) as usize;
const REGIONS_WIDE: usize = WIDTH / 16;
const REGIONS_HIGH: usize = HEIGHT / 16;
/// How many rounds of assigning regions and refitting palettes are tried before the palettes
/// are taken as they are. They settle in a handful.
const FITTING_ROUNDS: usize = 32;
/// The distinct tiles a pattern table holds, the blank one every background starts from among
/// them.
const PATTERN_TABLE_TILES: usize = 256;
/// Like [`FITTING_ROUNDS`], for assigning tiles to patterns and refitting the patterns.
const TILE_ROUNDS: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConvertError {
    Decode { message: String },
}

/// A palette's three colours besides the backdrop.
type Palette = [u8; 3];
/// How many of a region's pixels are nearest each NES entry.
type Histogram = [u32; 64];

/// How a picture is converted: the width of the screen it is fitted into, from the left, and
/// whether it is split across two pattern tables.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Conversion {
    /// Pixels from the left edge, up to 256: the picture is fitted into that much of the screen,
    /// centred in it, and the rest is backdrop. A source program's `width` is a multiple of 16,
    /// whole attribute cells, which `raster-sema` checks.
    pub width: usize,
    /// Split at [`SPLIT_TILE_ROW`] across two pattern tables, as
    /// [`convert_background_split`] does.
    pub split: bool,
}

impl Default for Conversion {
    fn default() -> Self {
        Conversion {
            width: WIDTH,
            split: false,
        }
    }
}

pub fn convert_background(png: impl Read) -> Result<NesBackground, ConvertError> {
    convert_background_with(png, Conversion::default())
}

/// [`convert_background`] across two pattern tables, as
/// [`encode_background_split`](crate::encode_background_split) lays them out: each half of the
/// screen is drawn in up to 256 tiles of its own.
pub fn convert_background_split(png: impl Read) -> Result<NesBackground, ConvertError> {
    convert_background_with(
        png,
        Conversion {
            split: true,
            ..Conversion::default()
        },
    )
}

/// [`convert_background`] as `conversion` says.
pub fn convert_background_with(
    png: impl Read,
    conversion: Conversion,
) -> Result<NesBackground, ConvertError> {
    let split = conversion.split;
    let (width, height, rgba) = decode_rgba(png)?;
    let screen = scale(width, height, &rgba, conversion.width.clamp(1, WIDTH));
    let colours = Colours::new();

    let mut histograms = vec![[0; 64]; REGIONS_WIDE * REGIONS_HIGH];
    for (index, pixel) in screen.iter().enumerate() {
        if let Some(rgb) = pixel {
            let region = (index / WIDTH / 16) * REGIONS_WIDE + (index % WIDTH) / 16;
            histograms[region][usize::from(colours.nearest(*rgb, &colours.all))] += 1;
        }
    }
    let palettes: Vec<Palette> = colours
        .fit(&histograms)
        .iter()
        .map(|palette| colours.complete(palette))
        .collect();
    let assignment: Vec<usize> = histograms
        .iter()
        .map(|histogram| colours.best_of(histogram, &palettes))
        .collect();

    let records: Vec<[u8; 4]> = palettes
        .iter()
        .map(|palette| [TRANSPARENT_COLOUR, palette[0], palette[1], palette[2]])
        .collect();
    let record_of = |x: usize, y: usize| records[assignment[(y / 16) * REGIONS_WIDE + x / 16]];
    let tiles: Vec<Tile> = (0..MAX_TILES_HIGH as usize * MAX_TILES_WIDE as usize)
        .map(|tile| {
            let (left, top) = (
                (tile % MAX_TILES_WIDE as usize) * 8,
                (tile / MAX_TILES_WIDE as usize) * 8,
            );
            let palette = record_of(left, top);
            let mut pixels = [TRANSPARENT_COLOUR; 64];
            for (index, pixel) in pixels.iter_mut().enumerate() {
                if let Some(rgb) = screen[(top + index / 8) * WIDTH + left + index % 8] {
                    *pixel = colours.nearest(rgb, &palette);
                }
            }
            Tile { pixels, palette }
        })
        .collect();
    // Split, each half of the screen is its own pattern table, so is reduced on its own.
    let halves = if split {
        SPLIT_TILE_ROW * MAX_TILES_WIDE as usize
    } else {
        tiles.len()
    };
    let (upper, lower) = tiles.split_at(halves);
    // The first pattern table also holds the lower half's first tiles (`SPLIT_SHARED_TILES`).
    let upper_limit = if split {
        PATTERN_TABLE_TILES - SPLIT_SHARED_TILES
    } else {
        PATTERN_TABLE_TILES
    };
    let mut patterns = colours.reduce(upper, upper_limit);
    if !lower.is_empty() {
        patterns.extend(colours.reduce(lower, PATTERN_TABLE_TILES));
    }

    let mut pixels = vec![TRANSPARENT_COLOUR; WIDTH * HEIGHT];
    for (index, (tile, pattern)) in tiles.iter().zip(&patterns).enumerate() {
        let (left, top) = (
            (index % MAX_TILES_WIDE as usize) * 8,
            (index / MAX_TILES_WIDE as usize) * 8,
        );
        for (pixel, value) in pattern.iter().enumerate() {
            pixels[(top + pixel / 8) * WIDTH + left + pixel % 8] =
                tile.palette[usize::from(*value)];
        }
    }
    let background = IndexedBackground {
        width_tiles: MAX_TILES_WIDE,
        height_tiles: MAX_TILES_HIGH,
        pixels,
    };

    // The attribute table has a sixteenth row of regions below the screen, which shows nothing.
    let mut region_palette_ids = [[0; 16]; 16];
    for (region, palette) in assignment.iter().enumerate() {
        region_palette_ids[region / REGIONS_WIDE][region % REGIONS_WIDE] = *palette as u8;
    }
    Ok(assemble(&background, &records, &region_palette_ids, split)
        .expect("every pixel is in its region's palette, in at most a pattern table of tiles"))
}

/// One tile of the picture: the NES colour of each of its pixels, row by row, and the palette its
/// region draws in, backdrop first.
struct Tile {
    pixels: [u8; 64],
    palette: [u8; 4],
}

/// Which of its palette's four colours each pixel of a tile shows, row by row.
type Pattern = [u8; 64];

/// The picture's width, height and pixels as RGBA, whatever colour type the PNG holds.
fn decode_rgba(png: impl Read) -> Result<(usize, usize, Vec<[u8; 4]>), ConvertError> {
    let decode = |error: png::DecodingError| ConvertError::Decode {
        message: error.to_string(),
    };
    let mut decoder = png::Decoder::new(png);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(decode)?;
    let mut bytes = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut bytes).map_err(decode)?;
    let bytes = &bytes[..frame.buffer_size()];
    let pixels = match frame.color_type {
        png::ColorType::Rgba => bytes.as_chunks::<4>().0.to_vec(),
        png::ColorType::Rgb => bytes
            .as_chunks::<3>()
            .0
            .iter()
            .map(|[r, g, b]| [*r, *g, *b, 0xff])
            .collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[g, a]| [*g, *g, *g, *a])
            .collect(),
        png::ColorType::Grayscale => bytes.iter().map(|&g| [g, g, g, 0xff]).collect(),
        png::ColorType::Indexed => unreachable!("EXPAND turns a palette into RGB"),
    };
    Ok((frame.width as usize, frame.height as usize, pixels))
}

/// The picture scaled to fit the left `area` pixels of the screen with its shape kept, centred
/// in them, and the backdrop around it: each screen pixel the average of the source pixels it
/// covers, or the nearest one when the picture is smaller. A pixel mostly transparent is `None`,
/// the backdrop.
fn scale(width: usize, height: usize, rgba: &[[u8; 4]], area: usize) -> Vec<Option<[u8; 3]>> {
    let (fit_width, fit_height) = if width * HEIGHT >= height * area {
        (area, ((height * area + width / 2) / width).clamp(1, HEIGHT))
    } else {
        (
            ((width * HEIGHT + height / 2) / height).clamp(1, area),
            HEIGHT,
        )
    };
    let (left_border, top_border) = ((area - fit_width) / 2, (HEIGHT - fit_height) / 2);
    let span = |index: usize, source: usize, screen: usize| {
        let start = index * source / screen;
        (start, ((index + 1) * source / screen).max(start + 1))
    };
    let mut screen = vec![None; WIDTH * HEIGHT];
    for y in 0..fit_height {
        let (top, bottom) = span(y, height, fit_height);
        for x in 0..fit_width {
            let (left, right) = span(x, width, fit_width);
            let (mut sum, mut opaque) = ([0u32; 3], 0u32);
            for row in top..bottom {
                for pixel in &rgba[row * width + left..row * width + right] {
                    if pixel[3] >= 0x80 {
                        for (total, channel) in sum.iter_mut().zip(pixel) {
                            *total += u32::from(*channel);
                        }
                        opaque += 1;
                    }
                }
            }
            let covered = ((bottom - top) * (right - left)) as u32;
            screen[(top_border + y) * WIDTH + left_border + x] = (opaque * 2 >= covered
                && opaque > 0)
                .then(|| sum.map(|total| ((total + opaque / 2) / opaque) as u8));
        }
    }
    screen
}

/// The NES colours a converted picture may use, and how far apart they look.
struct Colours {
    /// Every entry with a colour of its own: the blacks are all `$0F`, the backdrop.
    all: Vec<u8>,
    /// The ones a palette may hold besides the backdrop: every colour but black.
    paintable: Vec<u8>,
    /// How unlike each pair of entries looks.
    distance: Vec<[u32; 64]>,
    /// The square root of `distance`, which tile reduction sums: summing squares, a pattern
    /// shared by tiles that disagree on a pixel would take a colour between theirs, which none
    /// of them shows, where summing these it takes the colour most of them show.
    difference: Vec<[u32; 64]>,
}

impl Colours {
    fn new() -> Self {
        let mut all = vec![TRANSPARENT_COLOUR];
        for entry in 0..64u8 {
            let rgb = FCEUX_2C02_PALETTE[usize::from(entry)];
            if !all
                .iter()
                .any(|&known| FCEUX_2C02_PALETTE[usize::from(known)] == rgb)
            {
                all.push(entry);
            }
        }
        let paintable = all[1..].to_vec();
        let distance: Vec<[u32; 64]> = (0..64)
            .map(|from| {
                let mut row = [0; 64];
                for (to, cell) in row.iter_mut().enumerate() {
                    *cell = perceived_distance(FCEUX_2C02_PALETTE[from], FCEUX_2C02_PALETTE[to]);
                }
                row
            })
            .collect();
        let difference = distance
            .iter()
            .map(|row: &[u32; 64]| row.map(|squared| f64::from(squared).sqrt().round() as u32))
            .collect();
        Colours {
            all,
            paintable,
            distance,
            difference,
        }
    }

    /// The entry of `choices` nearest `rgb`, the first of them on a tie.
    fn nearest(&self, rgb: [u8; 3], choices: &[u8]) -> u8 {
        *choices
            .iter()
            .min_by_key(|&&entry| perceived_distance(rgb, FCEUX_2C02_PALETTE[usize::from(entry)]))
            .expect("there is always a colour to choose")
    }

    /// What drawing `histogram` in the backdrop and `palette` loses.
    fn cost(&self, histogram: &Histogram, palette: &Palette) -> u64 {
        histogram
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0)
            .map(|(entry, &count)| {
                let row = &self.distance[entry];
                let nearest = palette
                    .iter()
                    .map(|&colour| row[usize::from(colour)])
                    .fold(row[usize::from(TRANSPARENT_COLOUR)], u32::min);
                u64::from(count) * u64::from(nearest)
            })
            .sum()
    }

    /// The palette of `palettes` that draws `histogram` best.
    fn best_of(&self, histogram: &Histogram, palettes: &[Palette]) -> usize {
        (0..palettes.len())
            .min_by_key(|&index| self.cost(histogram, &palettes[index]))
            .expect("there is always a palette")
    }

    /// The three colours that draw `histogram` best: chosen one at a time, then each swapped for
    /// any better one until none is.
    fn best_palette(&self, histogram: &Histogram) -> Palette {
        let mut palette = [TRANSPARENT_COLOUR; 3];
        for slot in 0..3 {
            palette[slot] = self.best_for_slot(histogram, &palette, slot);
        }
        loop {
            let before = palette;
            for slot in 0..3 {
                palette[slot] = self.best_for_slot(histogram, &palette, slot);
            }
            if palette == before {
                return palette;
            }
        }
    }

    fn best_for_slot(&self, histogram: &Histogram, palette: &Palette, slot: usize) -> u8 {
        let mut best = (self.cost(histogram, palette), palette[slot]);
        for &colour in &self.paintable {
            if palette.contains(&colour) {
                continue;
            }
            let mut trial = *palette;
            trial[slot] = colour;
            let cost = self.cost(histogram, &trial);
            if cost < best.0 {
                best = (cost, colour);
            }
        }
        best.1
    }

    /// Four palettes for the regions `histograms` describe.
    ///
    /// The first draws the whole picture best; each next one is [`fresh`](Self::fresh_palette).
    /// Then, until nothing changes, every region takes the palette that draws it best and every
    /// palette is refitted to the regions that took it.
    fn fit(&self, histograms: &[Histogram]) -> Vec<Palette> {
        let total = histograms.iter().fold([0; 64], |mut total, histogram| {
            for (sum, count) in total.iter_mut().zip(histogram) {
                *sum += count;
            }
            total
        });
        let mut palettes = vec![self.best_palette(&total)];
        while palettes.len() < BACKGROUND_SUBPALETTE_COUNT {
            let Some(palette) = self.fresh_palette(histograms, &palettes) else {
                break;
            };
            palettes.push(palette);
        }

        for _ in 0..FITTING_ROUNDS {
            let mut members = vec![[0; 64]; palettes.len()];
            for histogram in histograms {
                let sum = &mut members[self.best_of(histogram, &palettes)];
                for (sum, count) in sum.iter_mut().zip(histogram) {
                    *sum += count;
                }
            }
            let mut refitted: Vec<Palette> = Vec::with_capacity(palettes.len());
            for (members, palette) in members.iter().zip(&palettes) {
                let palette = if members.iter().all(|count| *count == 0) {
                    *palette
                } else {
                    self.best_palette(members)
                };
                // Two palettes refitted alike would draw the same cells: the copy is fitted
                // afresh instead, to cells the others draw badly.
                let palette = if refitted.contains(&palette) {
                    self.fresh_palette(histograms, &refitted).unwrap_or(palette)
                } else {
                    palette
                };
                refitted.push(palette);
            }
            if refitted == palettes {
                break;
            }
            palettes = refitted;
        }
        palettes
    }

    /// The pattern each of `tiles` is drawn in, among at most `limit` distinct ones.
    ///
    /// A picture whose tiles fit is drawn exactly. Otherwise the blank pattern is kept, as the
    /// tile every background starts from, and each next one is the tile drawn worst by the
    /// patterns so far. Then, until nothing changes, every tile takes the pattern that looks most
    /// like it in its own palette, and every pattern but the blank one is refitted, pixel by
    /// pixel, to the tiles that took it.
    fn reduce(&self, tiles: &[Tile], limit: usize) -> Vec<Pattern> {
        let exact: Vec<Pattern> = tiles
            .iter()
            .map(|tile| {
                tile.pixels.map(|colour| {
                    tile.palette
                        .iter()
                        .position(|entry| *entry == colour)
                        .expect("every pixel is in its tile's palette") as u8
                })
            })
            .collect();
        let mut distinct: Vec<Pattern> = vec![[0; 64]];
        for pattern in &exact {
            if !distinct.contains(pattern) {
                distinct.push(*pattern);
            }
        }
        if distinct.len() <= limit {
            return exact;
        }

        let mut patterns: Vec<Pattern> = vec![[0; 64]];
        let mut worst: Vec<u64> = tiles
            .iter()
            .map(|tile| self.drawn(tile, &patterns[0]))
            .collect();
        while patterns.len() < limit {
            let (tile, cost) = worst
                .iter()
                .enumerate()
                .max_by_key(|(index, cost)| (**cost, std::cmp::Reverse(*index)))
                .expect("a screen has tiles");
            if *cost == 0 {
                break;
            }
            let pattern = exact[tile];
            for (cost, tile) in worst.iter_mut().zip(tiles) {
                *cost = (*cost).min(self.drawn(tile, &pattern));
            }
            patterns.push(pattern);
        }

        let mut assignment = vec![usize::MAX; tiles.len()];
        for _ in 0..TILE_ROUNDS {
            let assigned: Vec<usize> = tiles
                .iter()
                .map(|tile| {
                    (0..patterns.len())
                        .min_by_key(|&index| self.drawn(tile, &patterns[index]))
                        .expect("there is always the blank pattern")
                })
                .collect();
            if assigned == assignment {
                break;
            }
            assignment = assigned;
            for (index, pattern) in patterns.iter_mut().enumerate().skip(1) {
                let members: Vec<&Tile> = tiles
                    .iter()
                    .zip(&assignment)
                    .filter(|(_, assigned)| **assigned == index)
                    .map(|(tile, _)| tile)
                    .collect();
                if members.is_empty() {
                    continue;
                }
                for (pixel, value) in pattern.iter_mut().enumerate() {
                    *value = (0..4u8)
                        .min_by_key(|&value| {
                            members
                                .iter()
                                .map(|tile| {
                                    let shown = tile.palette[usize::from(value)];
                                    u64::from(
                                        self.difference[usize::from(tile.pixels[pixel])]
                                            [usize::from(shown)],
                                    )
                                })
                                .sum::<u64>()
                        })
                        .expect("a pixel has four values");
                }
            }
        }
        // Refitting can make two patterns alike, which only lowers the count.
        assignment.iter().map(|&index| patterns[index]).collect()
    }

    /// How unlike `tile` looks drawn in `pattern` in its own palette.
    fn drawn(&self, tile: &Tile, pattern: &Pattern) -> u64 {
        tile.pixels
            .iter()
            .zip(pattern)
            .map(|(&colour, &value)| {
                u64::from(
                    self.difference[usize::from(colour)]
                        [usize::from(tile.palette[usize::from(value)])],
                )
            })
            .sum()
    }

    /// `palette` with every slot left at the backdrop given a colour of its own.
    ///
    /// A palette fitted to a cell of fewer than three colours keeps the backdrop in the slots it
    /// had no use for, and then three of a pattern's four values draw black: the encoder, which
    /// works a tile's values out again from its colours, would give one pattern several tiles,
    /// and a picture reduced to 256 could need more. A colour nobody asked for costs nothing.
    fn complete(&self, palette: &Palette) -> Palette {
        let mut complete = *palette;
        for slot in 0..3 {
            if complete[slot] == TRANSPARENT_COLOUR {
                complete[slot] = *self
                    .paintable
                    .iter()
                    .find(|colour| !complete.contains(colour))
                    .expect("there are more colours than a palette holds");
            }
        }
        complete
    }

    /// A palette unlike any of `palettes`: the one that draws best the cell they draw worst,
    /// or the next worst when that one would be a copy. `None` when every cell is drawn
    /// perfectly or none would give a new palette.
    fn fresh_palette(&self, histograms: &[Histogram], palettes: &[Palette]) -> Option<Palette> {
        let mut worst: Vec<(u64, usize)> = histograms
            .iter()
            .enumerate()
            .map(|(index, histogram)| {
                let best = self.best_of(histogram, palettes);
                (self.cost(histogram, &palettes[best]), index)
            })
            .filter(|(cost, _)| *cost > 0)
            .collect();
        worst.sort_by_key(|&(cost, index)| (std::cmp::Reverse(cost), index));
        worst
            .into_iter()
            .map(|(_, index)| self.best_palette(&histograms[index]))
            .find(|palette| !palettes.contains(palette))
    }
}

/// How unlike two colours look: RGB distance, weighted as the eye weighs it ("redmean").
fn perceived_distance(left: [u8; 3], right: [u8; 3]) -> u32 {
    let mean_red = (i32::from(left[0]) + i32::from(right[0])) / 2;
    let [dr, dg, db] =
        [0, 1, 2].map(|channel| i32::from(left[channel]) - i32::from(right[channel]));
    ((((512 + mean_red) * dr * dr) >> 8) + 4 * dg * dg + (((767 - mean_red) * db * db) >> 8)) as u32
}
