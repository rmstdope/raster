//! Laying decoded pictures out in CHR ROM.
//!
//! `raster-sema` has already said that every `chrrom` entry is some declared
//! asset's `tiles`, placed once, in a bank numbered 0 to 31. What it cannot say
//! is how many tiles that is, which needs the decoded pictures — so the checks
//! that depend on a count live here, and so does the one piece of arithmetic
//! the author is promised never to do: shifting a nametable to where its tiles
//! landed.

use std::collections::BTreeMap;

use raster_assets::NesBackground;
use raster_ir::{Nametable, Picture, Pictures};
use raster_syntax::{ChrRomBank, Expression, Item, Program, Span};

use crate::assets::Assets;

/// One 8 KiB CHR ROM bank: 512 tiles of 16 bytes, the MMC3's two pattern tables.
const BANK_BYTES: usize = 8 * 1024;
/// Banks an MMC3 addresses; numbers 0 to 31.
const MAX_BANKS: u32 = 32;
const TILE_BYTES: usize = 16;
const BANK_TILES: usize = BANK_BYTES / TILE_BYTES;
/// One pattern table, which is as far as a nametable entry can reach.
const PATTERN_TABLE_TILES: usize = 256;

/// What the summary prints for one bank.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChrBankSummary {
    pub number: u32,
    /// `picture.tiles`, `font.tiles`, in the order they were placed. Empty for
    /// a bank the author skipped over.
    pub contents: Vec<String>,
    pub tiles: usize,
    pub bytes_used: usize,
}

/// Every bank, the ROM bytes behind them, and what lowering needs to point a
/// nametable at its tiles.
pub(crate) struct ChrLayout {
    pub(crate) banks: Vec<ChrBankSummary>,
    /// `banks.len() * 8192` bytes; empty when the program has no `chrrom` at all.
    pub(crate) rom: Vec<u8>,
    pub(crate) pictures: Pictures,
}

/// A layout mistake, in the shape `compile.rs` renders: it carries the label
/// and the help line the agreed diagnostics have, which `AssetError` does not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChrError {
    pub(crate) message: String,
    pub(crate) label: String,
    pub(crate) help: String,
    pub(crate) span: Span,
}

pub(crate) fn layout(program: &Program, assets: &Assets) -> Result<ChrLayout, Vec<ChrError>> {
    let declared: Vec<(u32, &ChrRomBank)> = program
        .items
        .iter()
        .filter_map(|item| match &item.value {
            Item::ChrRom(bank) => Some((bank_number(bank), bank)),
            _ => None,
        })
        .collect();
    let images: BTreeMap<&str, &NesBackground> = assets
        .images
        .iter()
        .map(|(name, image)| (name.as_str(), image))
        .collect();

    let bank_count = declared
        .iter()
        .map(|(number, _)| *number as usize + 1)
        .max()
        .unwrap_or(0);
    let mut banks: Vec<ChrBankSummary> = (0..bank_count)
        .map(|number| ChrBankSummary {
            number: number as u32,
            contents: Vec::new(),
            tiles: 0,
            bytes_used: 0,
        })
        .collect();
    let mut rom = vec![0u8; bank_count * BANK_BYTES];
    // Each placed picture's bank, and the tile in that bank its tiles start at.
    let mut placed: BTreeMap<String, (u32, usize)> = BTreeMap::new();
    let mut errors = Vec::new();

    for (number, bank) in &declared {
        let slot = *number as usize;
        let mut tiles_so_far = 0usize;
        let mut crossings = Vec::new();
        for entry in &bank.entries {
            let Some(name) = tiles_of(&entry.value) else {
                continue;
            };
            let Some(image) = images.get(name) else {
                continue;
            };
            let chr = image.chr();
            let tiles = chr.len() / TILE_BYTES;
            if tiles_so_far / PATTERN_TABLE_TILES
                != (tiles_so_far + tiles - 1) / PATTERN_TABLE_TILES
            {
                crossings.push(ChrError {
                    message: format!("`{name}.tiles` crosses into the second pattern table"),
                    label: format!(
                        "tiles {}–{}; a picture must sit within tiles 0–255 or 256–511",
                        tiles_so_far,
                        tiles_so_far + tiles - 1
                    ),
                    help: format!("give `{name}.tiles` its own bank, or place it first"),
                    span: entry.span,
                });
            }
            let at = slot * BANK_BYTES + tiles_so_far * TILE_BYTES;
            if at + chr.len() <= (slot + 1) * BANK_BYTES {
                rom[at..at + chr.len()].copy_from_slice(chr);
            }
            placed.insert(name.to_owned(), (*number, tiles_so_far));
            banks[slot].contents.push(format!("{name}.tiles"));
            tiles_so_far += tiles;
        }
        banks[slot].tiles = tiles_so_far;
        banks[slot].bytes_used = tiles_so_far * TILE_BYTES;

        if tiles_so_far > BANK_TILES {
            // The crossing is a symptom of the bank being too big, so only the
            // cause is reported.
            errors.push(ChrError {
                message: format!(
                    "`chrrom bank {number}` is full: {tiles_so_far} tiles, and a bank holds {BANK_TILES}"
                ),
                label: format!(
                    "{} of {BANK_BYTES} bytes",
                    tiles_so_far * TILE_BYTES
                ),
                help: if *number + 1 < MAX_BANKS {
                    format!("move some of these to `chrrom bank {}`", number + 1)
                } else {
                    "move some of these to another bank".to_owned()
                },
                span: bank.contents,
            });
        } else {
            errors.extend(crossings);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let by_name = assets
        .images
        .iter()
        .map(|(name, image)| {
            let tiles = image.chr().len() / TILE_BYTES;
            let nametable = match placed.get(name) {
                None => Nametable::Unplaced,
                // Until bank switching exists, the PPU sees bank 0's first pattern table only.
                Some(&(bank, first)) if bank != 0 || first >= PATTERN_TABLE_TILES => {
                    Nametable::Unmapped {
                        bank,
                        first,
                        last: first + tiles.max(1) - 1,
                    }
                }
                Some(&(_, first)) => Nametable::Shown(
                    image
                        .nametable()
                        .iter()
                        // `first + tiles <= 256` was checked above, so this cannot wrap.
                        .map(|entry| entry + first as u8)
                        .collect(),
                ),
            };
            (
                name.clone(),
                Picture {
                    palette: image.palettes().to_vec(),
                    attributes: image.attributes().to_vec(),
                    nametable,
                },
            )
        })
        .collect();
    let next_free_bank = (0u32..)
        .find(|number| !declared.iter().any(|(declared, _)| declared == number))
        .unwrap_or(0)
        .min(MAX_BANKS);

    Ok(ChrLayout {
        banks,
        rom,
        pictures: Pictures {
            by_name,
            next_free_bank,
        },
    })
}

/// `n` for an entry `n.tiles`. `raster-sema` guarantees the shape.
fn tiles_of(entry: &Expression) -> Option<&str> {
    let Expression::Member { base, member } = entry else {
        return None;
    };
    match &base.value {
        Expression::Name(name) if member.value == "tiles" => Some(name.value.as_str()),
        _ => None,
    }
}

/// `raster-sema` has checked the spelling and the range.
fn bank_number(bank: &ChrRomBank) -> u32 {
    let text = &bank.number.value;
    match text.strip_prefix('$') {
        Some(hexadecimal) => u32::from_str_radix(hexadecimal, 16),
        None => text.parse(),
    }
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raster_assets::{decode_background, encode_background};
    use raster_syntax::parse;

    /// A picture that encodes to exactly `tiles` CHR tiles: the reserved blank
    /// tile plus `tiles - 1` distinct ones. Tile `k` lights the pixels of row 0
    /// that spell `k` in binary, so no two differ by less than a pixel.
    fn picture(tiles: usize) -> NesBackground {
        assert!((1..=256).contains(&tiles));
        let distinct = tiles - 1;
        let columns = 32;
        let rows = distinct.div_ceil(columns).max(1);
        let (width, height) = (columns * 8, rows * 8);
        let mut pixels = vec![[0u8, 0, 0, 0]; width * height];
        for k in 1..=distinct {
            let (tile_x, tile_y) = ((k - 1) % columns, (k - 1) / columns);
            for bit in 0..8 {
                if k >> bit & 1 == 1 {
                    pixels[(tile_y * 8) * width + tile_x * 8 + bit] = [0xfc, 0xfc, 0xfc, 0xff];
                }
            }
        }
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&pixels.into_iter().flatten().collect::<Vec<_>>())
            .unwrap();
        drop(writer);
        let indexed = decode_background(std::io::Cursor::new(bytes)).expect("decodes");
        let image = encode_background(&indexed).expect("encodes");
        assert_eq!(image.chr().len(), tiles * TILE_BYTES);
        image
    }

    fn run(source: &str, sizes: &[(&str, usize)]) -> Result<ChrLayout, Vec<ChrError>> {
        let program = parse(source).expect("the source parses");
        let assets = Assets {
            images: sizes
                .iter()
                .map(|(name, tiles)| ((*name).to_owned(), picture(*tiles)))
                .collect(),
        };
        layout(&program, &assets)
    }

    fn text<'a>(source: &'a str, error: &ChrError) -> &'a str {
        &source[error.span.start as usize..error.span.end as usize]
    }

    #[test]
    fn no_chrrom_means_no_banks_and_no_rom() {
        let layout = run("main { }", &[("picture", 4)]).unwrap();
        assert!(layout.banks.is_empty());
        assert!(layout.rom.is_empty());
        assert_eq!(
            layout.pictures.by_name["picture"].nametable,
            Nametable::Unplaced
        );
        assert_eq!(layout.pictures.next_free_bank, 0);
    }

    #[test]
    fn one_picture_fills_the_start_of_bank_zero_unshifted() {
        let layout = run("chrrom bank 0 { picture.tiles }", &[("picture", 112)]).unwrap();

        assert_eq!(
            layout.banks,
            [ChrBankSummary {
                number: 0,
                contents: vec!["picture.tiles".into()],
                tiles: 112,
                bytes_used: 1792,
            }]
        );
        assert_eq!(layout.rom.len(), 8192);
        let image = picture(112);
        assert_eq!(&layout.rom[..1792], image.chr());
        assert!(layout.rom[1792..].iter().all(|byte| *byte == 0));
        assert_eq!(
            layout.pictures.by_name["picture"].nametable,
            Nametable::Shown(image.nametable().to_vec())
        );
        assert_eq!(layout.pictures.next_free_bank, 1);
    }

    #[test]
    fn a_gap_is_an_empty_bank_and_numbers_mean_what_they_say() {
        let layout = run(
            "chrrom bank 2 { sprites.tiles }\nchrrom bank 0 { font.tiles }",
            &[("font", 10), ("sprites", 64)],
        )
        .unwrap();

        let numbers: Vec<_> = layout
            .banks
            .iter()
            .map(|bank| (bank.number, bank.contents.clone(), bank.tiles))
            .collect();
        assert_eq!(
            numbers,
            [
                (0, vec!["font.tiles".to_owned()], 10),
                (1, vec![], 0),
                (2, vec!["sprites.tiles".to_owned()], 64),
            ]
        );
        assert_eq!(layout.rom.len(), 3 * 8192);
        assert_eq!(&layout.rom[2 * 8192..2 * 8192 + 1024], picture(64).chr());
        assert_eq!(layout.pictures.next_free_bank, 1);
    }

    #[test]
    fn a_picture_placed_after_another_has_its_nametable_shifted_to_its_tiles() {
        let layout = run(
            "chrrom bank 0 { font.tiles, picture.tiles }",
            &[("font", 48), ("picture", 20)],
        )
        .unwrap();

        assert_eq!(layout.banks[0].contents, ["font.tiles", "picture.tiles"]);
        assert_eq!(layout.banks[0].tiles, 68);
        assert_eq!(&layout.rom[48 * 16..68 * 16], picture(20).chr());
        let shifted: Vec<u8> = picture(20).nametable().iter().map(|t| t + 48).collect();
        assert_eq!(
            layout.pictures.by_name["picture"].nametable,
            Nametable::Shown(shifted)
        );
        // The first item did not move.
        assert_eq!(
            layout.pictures.by_name["font"].nametable,
            Nametable::Shown(picture(48).nametable().to_vec())
        );
    }

    #[test]
    fn a_picture_in_the_second_pattern_table_is_unmapped() {
        let layout = run(
            "chrrom bank 0 { a.tiles, b.tiles }",
            &[("a", 256), ("b", 30)],
        )
        .unwrap();

        // b starts at tile 256 of the bank: on the cartridge, and not where the PPU looks.
        assert_eq!(
            layout.pictures.by_name["b"].nametable,
            Nametable::Unmapped {
                bank: 0,
                first: 256,
                last: 285
            }
        );
    }

    #[test]
    fn a_picture_in_a_bank_other_than_zero_is_unmapped() {
        let layout = run(
            "chrrom bank 0 { a.tiles }\nchrrom bank 1 { b.tiles }",
            &[("a", 5), ("b", 30)],
        )
        .unwrap();

        assert_eq!(
            layout.pictures.by_name["b"].nametable,
            Nametable::Unmapped {
                bank: 1,
                first: 0,
                last: 29
            }
        );
    }

    #[test]
    fn a_picture_that_ends_exactly_on_tile_255_is_fine_and_one_more_tile_crosses() {
        let source = "chrrom bank 0 { a.tiles, b.tiles }";
        assert!(run(source, &[("a", 200), ("b", 56)]).is_ok());

        let errors = run(source, &[("a", 200), ("b", 57)]).err().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "`b.tiles` crosses into the second pattern table"
        );
        assert_eq!(
            errors[0].label,
            "tiles 200–256; a picture must sit within tiles 0–255 or 256–511"
        );
        assert_eq!(
            errors[0].help,
            "give `b.tiles` its own bank, or place it first"
        );
        assert_eq!(text(source, &errors[0]), "b.tiles");
    }

    #[test]
    fn a_bank_of_exactly_512_tiles_fits_and_513_is_full() {
        let source = "chrrom bank 0 { a.tiles, b.tiles }";
        assert!(run(source, &[("a", 256), ("b", 256)]).is_ok());

        // The first picture ends the first table and the second crosses
        // nothing, so the only fault is the size.
        let source = "chrrom bank 3 { picture.tiles, font.tiles, sprites.tiles }";
        let errors = run(source, &[("picture", 256), ("font", 256), ("sprites", 64)])
            .err()
            .unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(
            errors[0].message,
            "`chrrom bank 3` is full: 576 tiles, and a bank holds 512"
        );
        assert_eq!(errors[0].label, "9216 of 8192 bytes");
        assert_eq!(errors[0].help, "move some of these to `chrrom bank 4`");

        assert_eq!(
            text(source, &errors[0]),
            "picture.tiles, font.tiles, sprites.tiles"
        );
    }

    #[test]
    fn the_last_bank_has_no_next_bank_to_suggest() {
        let source = "chrrom bank 31 { a.tiles, b.tiles, c.tiles }";
        let errors = run(source, &[("a", 256), ("b", 256), ("c", 1)])
            .err()
            .unwrap();
        assert_eq!(errors[0].help, "move some of these to another bank");
    }

    #[test]
    fn every_bank_with_a_fault_is_reported() {
        let errors = run(
            "chrrom bank 0 { a.tiles, b.tiles }\nchrrom bank 1 { c.tiles, d.tiles }",
            &[("a", 200), ("b", 100), ("c", 200), ("d", 100)],
        )
        .err()
        .unwrap();
        assert_eq!(errors.len(), 2);
    }
}
