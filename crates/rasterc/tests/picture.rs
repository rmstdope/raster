//! A picture from `.raster` source to the screen: the tiles in CHR ROM, the rest uploaded, the
//! build summary and every diagnostic the agreed experience names, in the words it names them.

use std::{fs, io::Cursor, num::NonZeroU32};

use raster_assets::{decode_background, encode_background};
use raster_diag::{render, Diagnostic, SourceFile};
use raster_emu::render_after_frames;
use rasterc::{compile_source_with_assets, run};

#[path = "common/picture.rs"]
mod fixtures;
#[path = "common/scratch.rs"]
mod scratch;
use fixtures::{picture_png, Pictures};
use scratch::Scratch;

/// `chrrom` and `init` around a body, with `picture.png` and `font.png` available.
fn program(chrrom: &str, body: &str) -> String {
    format!(
        "asset image picture = png(\"picture.png\") {{ kind: background }}\n\
         asset image font = png(\"font.png\") {{ kind: background }}\n\
         asset image sprites = png(\"sprites.png\") {{ kind: background }}\n\
         {chrrom}\n\
         main {{\n{body}\n}}\n"
    )
}

const SHOW: &str = "    load_palette(picture.palette)\n    \
                    load_nametable(picture.nametable, picture.attributes)\n    \
                    ppu.mask = $0A\n";

fn files(picture: usize, font: usize, sprites: usize) -> Pictures {
    Pictures(vec![
        ("picture.png", picture),
        ("font.png", font),
        ("sprites.png", sprites),
    ])
}

fn rendered(source: &str, diagnostic: &Diagnostic) -> String {
    render(&SourceFile::new("demo.raster", source), diagnostic)
}

fn errors(source: &str, files: &Pictures) -> Vec<Diagnostic> {
    compile_source_with_assets(source, files).expect_err("the program is refused")
}

// ---- the ROM -------------------------------------------------------------------------------

#[test]
fn a_chrrom_bank_makes_the_tiles_real_chr_rom() {
    let source = program("chrrom bank 0 { picture.tiles }", SHOW);
    let rom = compile_source_with_assets(&source, &files(17, 1, 1)).expect("it compiles");

    // Byte 5 is the count of 8 KiB CHR ROM banks; zero would be CHR RAM.
    assert_eq!(rom.image[5], 1);
    assert_eq!(rom.image.len(), 16 + 32 * 1024 + 8 * 1024);
    let chr = &rom.image[16 + 32 * 1024..];
    let indexed = decode_background(Cursor::new(picture_png(17))).unwrap();
    let encoded = encode_background(&indexed).unwrap();
    assert_eq!(&chr[..17 * 16], encoded.chr());
    assert!(chr[17 * 16..].iter().all(|byte| *byte == 0));
}

#[test]
fn a_program_without_chrrom_is_still_chr_ram_with_no_extra_bytes() {
    let rom = compile_source_with_assets(&program("", "    ppu.mask = $00"), &files(2, 2, 2))
        .expect("it compiles");
    assert_eq!(rom.image[5], 0);
    assert_eq!(rom.image.len(), 16 + 32 * 1024);
    assert!(rom.chr_banks.is_empty());
}

#[test]
fn a_gap_in_the_bank_numbers_is_an_empty_bank() {
    let source = program(
        "chrrom bank 0 { font.tiles }\nchrrom bank 2 { sprites.tiles }",
        "    ppu.mask = $00",
    );
    let rom = compile_source_with_assets(&source, &files(2, 5, 7)).expect("it compiles");

    assert_eq!(rom.image[5], 3);
    assert_eq!(rom.image.len(), 16 + 32 * 1024 + 3 * 8 * 1024);
    assert!(
        rom.image[16 + 32 * 1024 + 8 * 1024..16 + 32 * 1024 + 16 * 1024]
            .iter()
            .all(|byte| *byte == 0)
    );
}

// ---- the build summary ---------------------------------------------------------------------

/// The summary `rasterc` prints for `source`, compiled from a directory holding its pictures.
fn summary_lines(source: &str, picture: usize, font: usize, sprites: usize) -> Vec<String> {
    let directory = Scratch::new("picture-summary");
    for (name, tiles) in [
        ("picture.png", picture),
        ("font.png", font),
        ("sprites.png", sprites),
    ] {
        fs::write(directory.path().join(name), picture_png(tiles)).unwrap();
    }
    let input = directory.path().join("demo.raster");
    fs::write(&input, source).unwrap();
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());
    run(vec![input.display().to_string()], &mut stdout, &mut stderr).unwrap_or_else(|code| {
        panic!(
            "exit {code}: {}",
            String::from_utf8_lossy(&stderr.into_inner())
        )
    });
    String::from_utf8(stdout.into_inner())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_summary_reports_one_bank() {
    let lines = summary_lines(&program("chrrom bank 0 { picture.tiles }", SHOW), 112, 1, 1);

    assert_eq!(lines[2], "      prg  32 KiB, 4 banks of 8 KiB");
    assert_eq!(lines[3], "      chr  8 KiB ROM, 1 bank of 8 KiB");
    assert_eq!(
        lines[4],
        "   bank 0  picture.tiles, 112 tiles, 1792 of 8192 bytes"
    );
    assert!(
        lines[5].starts_with("    fixed  $E000-$FFFF, "),
        "{lines:?}"
    );
}

#[test]
fn the_summary_reports_several_items_and_a_gap() {
    let source = program(
        "chrrom bank 0 { font.tiles, picture.tiles }\nchrrom bank 2 { sprites.tiles }",
        SHOW,
    );
    let lines = summary_lines(&source, 112, 48, 64);

    assert_eq!(lines[3], "      chr  24 KiB ROM, 3 banks of 8 KiB");
    assert_eq!(
        lines[4],
        "   bank 0  font.tiles + picture.tiles, 160 tiles, 2560 of 8192 bytes"
    );
    assert_eq!(lines[5], "   bank 1  empty");
    assert_eq!(
        lines[6],
        "   bank 2  sprites.tiles, 64 tiles, 1024 of 8192 bytes"
    );
    assert!(lines[7].starts_with("    fixed  "));
}

#[test]
fn a_bank_holding_one_tile_says_tile() {
    let lines = summary_lines(
        &program("chrrom bank 0 { font.tiles }", "    ppu.mask = $00"),
        1,
        1,
        1,
    );
    assert_eq!(lines[4], "   bank 0  font.tiles, 1 tile, 16 of 8192 bytes");
}

#[test]
fn no_chrrom_keeps_todays_summary() {
    let lines = summary_lines(&program("", "    ppu.mask = $00"), 1, 1, 1);
    assert_eq!(lines[3], "      chr  8 KiB RAM");
    assert!(lines[4].starts_with("    fixed  "));
}

// ---- the diagnostics -----------------------------------------------------------------------

#[test]
fn tiles_that_are_in_no_bank_are_an_error_with_no_rom() {
    let source = "asset image picture = png(\"picture.png\")\n\
                  main {\n\
                  \x20   load_nametable(picture.nametable, picture.attributes)\n\
                  }\n";
    let found = errors(source, &files(3, 1, 1));

    assert_eq!(found.len(), 1);
    assert_eq!(
        rendered(source, &found[0]),
        concat!(
            "error: `picture`'s tiles are not in any `chrrom` bank\n",
            " --> demo.raster:3:20\n",
            "  |\n",
            "3 |     load_nametable(picture.nametable, picture.attributes)\n",
            "  |                    ^^^^^^^^^^^^^^^^^ this nametable points at tiles that are not on the cartridge\n",
            "  |\n",
            "  = help: add `chrrom bank 0 { picture.tiles }`\n",
        )
    );
}

#[test]
fn a_bank_over_512_tiles_is_an_error_across_its_contents() {
    let source = program(
        "chrrom bank 0 { picture.tiles, font.tiles, sprites.tiles }",
        "    ppu.mask = $00",
    );
    let found = errors(&source, &files(256, 256, 64));

    assert_eq!(found.len(), 1);
    assert_eq!(
        rendered(&source, &found[0]),
        concat!(
            "error: `chrrom bank 0` is full: 576 tiles, and a bank holds 512\n",
            " --> demo.raster:4:17\n",
            "  |\n",
            "4 | chrrom bank 0 { picture.tiles, font.tiles, sprites.tiles }\n",
            "  |                 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ 9216 of 8192 bytes\n",
            "  |\n",
            "  = help: move some of these to `chrrom bank 1`\n",
        )
    );
}

#[test]
fn tiles_crossing_the_pattern_table_line_are_an_error_on_that_item() {
    let source = program(
        "chrrom bank 0 { picture.tiles, font.tiles }",
        "    ppu.mask = $00",
    );
    let found = errors(&source, &files(240, 48, 1));

    assert_eq!(found.len(), 1);
    assert_eq!(
        rendered(&source, &found[0]),
        concat!(
            "error: `font.tiles` crosses into the second pattern table\n",
            " --> demo.raster:4:32\n",
            "  |\n",
            "4 | chrrom bank 0 { picture.tiles, font.tiles }\n",
            "  |                                ^^^^^^^^^^ tiles 240–287; a picture must sit within tiles 0–255 or 256–511\n",
            "  |\n",
            "  = help: give `font.tiles` its own bank, or place it first\n",
        )
    );
}

#[test]
fn the_wrong_member_for_load_palette_suggests_the_palette() {
    let source = program(
        "chrrom bank 0 { picture.tiles }",
        "    load_palette(picture.tiles)",
    );
    let found = errors(&source, &files(3, 1, 1));

    assert_eq!(found.len(), 1);
    assert_eq!(
        rendered(&source, &found[0]),
        concat!(
            "error: `load_palette` takes a palette, not tiles\n",
            " --> demo.raster:6:18\n",
            "  |\n",
            "6 |     load_palette(picture.tiles)\n",
            "  |                  ^^^^^^^^^^^^^ this is `picture`'s tiles\n",
            "  |\n",
            "  = help: did you mean `picture.palette`?\n",
        )
    );
}

#[test]
fn swapped_load_nametable_arguments_get_the_call_written_out() {
    let source = program(
        "chrrom bank 0 { picture.tiles }",
        "    load_nametable(picture.attributes, picture.nametable)",
    );
    let found = errors(&source, &files(3, 1, 1));

    assert_eq!(found.len(), 1);
    assert_eq!(
        rendered(&source, &found[0]),
        concat!(
            "error: `load_nametable` takes a nametable, then its attributes\n",
            " --> demo.raster:6:20\n",
            "  |\n",
            "6 |     load_nametable(picture.attributes, picture.nametable)\n",
            "  |                    ^^^^^^^^^^^^^^^^^^ this is `picture`'s attributes\n",
            "  |\n",
            "  = help: write `load_nametable(picture.nametable, picture.attributes)`\n",
        )
    );
}

#[test]
fn loading_a_nametable_with_rendering_on_warns_and_still_builds() {
    let source = "asset image picture = png(\"picture.png\")\n\
                  chrrom bank 0 { picture.tiles }\n\
                  main {\n\
                  \x20   ppu.mask = $0A\n\
                  \x20   load_nametable(picture.nametable, picture.attributes)\n\
                  }\n";
    let rom = compile_source_with_assets(source, &files(3, 1, 1)).expect("a warning still builds");

    assert_eq!(rom.warnings.len(), 1);
    assert_eq!(
        rendered(source, &rom.warnings[0]),
        concat!(
            "warning: `load_nametable` runs while rendering is on\n",
            " --> demo.raster:5:5\n",
            "  |\n",
            "4 |     ppu.mask = $0A\n",
            "  |     -------------- rendering turned on here\n",
            "5 |     load_nametable(picture.nametable, picture.attributes)\n",
            "  |     ^^^^^^^^^^^^^^ 1024 writes cannot finish inside one vblank; the screen will tear\n",
            "  |\n",
            "  = help: call it before `ppu.mask = $0A`, or set `ppu.mask = $00` first\n",
        )
    );
}

// ---- on screen -----------------------------------------------------------------------------

/// The decoded picture's pixels, as palette entries, and the same picture as the emulator
/// shows it after `compile` has built a ROM from `.raster` source.
fn shown(source: &str, files: &Pictures, picture: usize) -> (Vec<u16>, Vec<u16>) {
    let rom = compile_source_with_assets(source, files).expect("it compiles");
    let frame = render_after_frames(
        "picture.nes",
        &rom.image,
        NonZeroU32::new(8).expect("eight is not zero"),
    )
    .expect("the ROM renders");
    let indexed = decode_background(Cursor::new(picture_png(picture))).unwrap();
    (
        indexed
            .pixels()
            .iter()
            .map(|pixel| u16::from(*pixel))
            .collect(),
        frame.as_indices().to_vec(),
    )
}

/// The picture is 256 by 8 pixels, so it is the first rows of the 256 by 240 screen, entry for
/// entry; the rest of the nametable is the blank tile, which shows one flat colour.
fn assert_picture_on_top(expected: &[u16], actual: &[u16]) {
    let first_difference = expected.iter().zip(actual).position(|(a, b)| a != b);
    assert_eq!(first_difference, None, "first differing pixel index");
    let rest = &actual[expected.len()..];
    assert!(
        rest.iter().all(|entry| *entry == rest[0]),
        "the rest of the screen is blank"
    );
}

const LOAD_THEN_SHOW: &str = "    ppu.mask = $00\n    wait cycles(40)\n    \
    load_palette(picture.palette)\n    \
    load_nametable(picture.nametable, picture.attributes)\n    \
    ppu.ctrl = $00\n    ppu.mask = $0A\n";

#[test]
fn the_picture_appears_on_screen_tile_for_tile() {
    // 33 tiles: a full 32-wide row of distinct tiles, plus the blank one.
    let source = program("chrrom bank 0 { picture.tiles }", LOAD_THEN_SHOW);
    let (expected, actual) = shown(&source, &files(33, 1, 1), 33);
    assert_picture_on_top(&expected, &actual);
}

#[test]
fn a_picture_placed_after_another_still_shows_correctly() {
    // The font's 40 tiles come first, so the picture's nametable must be shifted by 40.
    let source = program(
        "chrrom bank 0 { font.tiles, picture.tiles }",
        LOAD_THEN_SHOW,
    );
    let (expected, actual) = shown(&source, &files(33, 40, 1), 33);
    assert_picture_on_top(&expected, &actual);
}
