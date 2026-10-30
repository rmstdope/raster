//! Turning any PNG into a background, for `convert: true`.
//!
//! [`decode_background`](crate::decode_background) refuses a picture the NES cannot show as it
//! stands; this makes one it can. The picture is scaled to the screen, and four palettes of three
//! colours are fitted to it, each 16-by-16 region drawing in the backdrop and the palette that
//! loses it least. Every pixel then takes the nearest colour it is allowed.

use std::io::Read;

use crate::nes::{assemble, BackgroundEncodeError, BACKGROUND_SUBPALETTE_COUNT};
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConvertError {
    Decode { message: String },
    TooManyTiles { tile_count: usize },
}

/// A palette's three colours besides the backdrop.
type Palette = [u8; 3];
/// How many of a region's pixels are nearest each NES entry.
type Histogram = [u32; 64];

pub fn convert_background(png: impl Read) -> Result<NesBackground, ConvertError> {
    let (width, height, rgba) = decode_rgba(png)?;
    let screen = scale(width, height, &rgba);
    let colours = Colours::new();

    let mut histograms = vec![[0; 64]; REGIONS_WIDE * REGIONS_HIGH];
    for (index, pixel) in screen.iter().enumerate() {
        if let Some(rgb) = pixel {
            let region = (index / WIDTH / 16) * REGIONS_WIDE + (index % WIDTH) / 16;
            histograms[region][usize::from(colours.nearest(*rgb, &colours.all))] += 1;
        }
    }
    let palettes = colours.fit(&histograms);
    let assignment: Vec<usize> = histograms
        .iter()
        .map(|histogram| colours.best_of(histogram, &palettes))
        .collect();

    let pixels = screen
        .iter()
        .enumerate()
        .map(|(index, pixel)| match pixel {
            None => TRANSPARENT_COLOUR,
            Some(rgb) => {
                let region = (index / WIDTH / 16) * REGIONS_WIDE + (index % WIDTH) / 16;
                let palette = palettes[assignment[region]];
                colours.nearest(
                    *rgb,
                    &[TRANSPARENT_COLOUR, palette[0], palette[1], palette[2]],
                )
            }
        })
        .collect();
    let background = IndexedBackground {
        width_tiles: MAX_TILES_WIDE,
        height_tiles: MAX_TILES_HIGH,
        pixels,
    };

    let records: Vec<[u8; 4]> = palettes
        .iter()
        .map(|palette| [TRANSPARENT_COLOUR, palette[0], palette[1], palette[2]])
        .collect();
    // The attribute table has a sixteenth row of regions below the screen, which shows nothing.
    let mut region_palette_ids = [[0; 16]; 16];
    for (region, palette) in assignment.iter().enumerate() {
        region_palette_ids[region / REGIONS_WIDE][region % REGIONS_WIDE] = *palette as u8;
    }
    assemble(&background, &records, &region_palette_ids).map_err(|error| match error {
        BackgroundEncodeError::TooManyTiles { tile_count } => {
            ConvertError::TooManyTiles { tile_count }
        }
        other => unreachable!("every region draws in a palette of its own: {other:?}"),
    })
}

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

/// The picture scaled to the screen: each screen pixel the average of the source pixels it
/// covers, or the nearest one when the picture is smaller. A pixel mostly transparent is `None`,
/// the backdrop.
fn scale(width: usize, height: usize, rgba: &[[u8; 4]]) -> Vec<Option<[u8; 3]>> {
    let span = |index: usize, source: usize, screen: usize| {
        let start = index * source / screen;
        (start, ((index + 1) * source / screen).max(start + 1))
    };
    let mut screen = Vec::with_capacity(WIDTH * HEIGHT);
    for y in 0..HEIGHT {
        let (top, bottom) = span(y, height, HEIGHT);
        for x in 0..WIDTH {
            let (left, right) = span(x, width, WIDTH);
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
            screen.push(
                (opaque * 2 >= covered && opaque > 0)
                    .then(|| sum.map(|total| ((total + opaque / 2) / opaque) as u8)),
            );
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
        let distance = (0..64)
            .map(|from| {
                let mut row = [0; 64];
                for (to, cell) in row.iter_mut().enumerate() {
                    *cell = perceived_distance(FCEUX_2C02_PALETTE[from], FCEUX_2C02_PALETTE[to]);
                }
                row
            })
            .collect();
        Colours {
            all,
            paintable,
            distance,
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
    /// The first draws the whole picture best; each next one draws best the region the palettes
    /// so far draw worst. Then, until nothing changes, every region takes the palette that draws
    /// it best and every palette is refitted to the regions that took it.
    fn fit(&self, histograms: &[Histogram]) -> Vec<Palette> {
        let total = histograms.iter().fold([0; 64], |mut total, histogram| {
            for (sum, count) in total.iter_mut().zip(histogram) {
                *sum += count;
            }
            total
        });
        let mut palettes = vec![self.best_palette(&total)];
        while palettes.len() < BACKGROUND_SUBPALETTE_COUNT {
            let (worst, cost) = histograms
                .iter()
                .enumerate()
                .map(|(index, histogram)| {
                    let best = self.best_of(histogram, &palettes);
                    (index, self.cost(histogram, &palettes[best]))
                })
                .max_by_key(|(index, cost)| (*cost, std::cmp::Reverse(*index)))
                .expect("a screen has regions");
            if cost == 0 {
                break;
            }
            palettes.push(self.best_palette(&histograms[worst]));
        }

        for _ in 0..FITTING_ROUNDS {
            let mut members = vec![[0; 64]; palettes.len()];
            for histogram in histograms {
                let sum = &mut members[self.best_of(histogram, &palettes)];
                for (sum, count) in sum.iter_mut().zip(histogram) {
                    *sum += count;
                }
            }
            let refitted: Vec<Palette> = members
                .iter()
                .zip(&palettes)
                .map(|(members, palette)| {
                    if members.iter().all(|count| *count == 0) {
                        *palette
                    } else {
                        self.best_palette(members)
                    }
                })
                .collect();
            if refitted == palettes {
                break;
            }
            palettes = refitted;
        }
        palettes
    }
}

/// How unlike two colours look: RGB distance, weighted as the eye weighs it ("redmean").
fn perceived_distance(left: [u8; 3], right: [u8; 3]) -> u32 {
    let mean_red = (i32::from(left[0]) + i32::from(right[0])) / 2;
    let [dr, dg, db] =
        [0, 1, 2].map(|channel| i32::from(left[channel]) - i32::from(right[channel]));
    ((((512 + mean_red) * dr * dr) >> 8) + 4 * dg * dg + (((767 - mean_red) * db * db) >> 8)) as u32
}
