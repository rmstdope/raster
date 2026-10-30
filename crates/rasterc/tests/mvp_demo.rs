//! The MVP demo end to end: `examples/mvp/demo.raster` and the test card it imports, built the way
//! an author builds them and then run, so the picture and the bars are judged as the PPU emitted
//! them rather than as the source describes them.

use std::{
    fs,
    io::Cursor,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use raster_assets::{decode_background, encode_background};
use raster_emu::{render_after_frames, FRAME_HEIGHT, FRAME_WIDTH};
use rasterc::run;

#[path = "common/demo.rs"]
mod demo;
#[path = "common/scratch.rs"]
mod scratch;
use demo::{backdrop_path, mvp_demo_path};
use scratch::Scratch;

/// NES colours the card is made of, as `raster-assets` indexes them.
const GREY: u8 = 0x00;
const WHITE: u8 = 0x30;
const YELLOW: u8 = 0x28;
const RED: u8 = 0x16;
const GREEN: u8 = 0x1a;
const BLUE: u8 = 0x12;
const PURPLE: u8 = 0x13;
const BACKDROP: u8 = 0x0f;

fn picture_path() -> PathBuf {
    mvp_demo_path().with_file_name("art").join("picture.png")
}

fn card() -> raster_assets::IndexedBackground {
    let bytes = fs::read(picture_path()).expect("the test card is on disk");
    decode_background(Cursor::new(bytes)).expect("the test card decodes")
}

/// Builds `input` into a scratch directory; the summary, the diagnostics and the ROM.
fn build(input: &Path) -> (String, String, Vec<u8>) {
    // Tests run in parallel, so each build gets a directory of its own.
    static BUILDS: AtomicUsize = AtomicUsize::new(0);
    let directory = Scratch::new(&format!(
        "mvp-demo-{}",
        BUILDS.fetch_add(1, Ordering::Relaxed)
    ));
    let output = directory.path().join("out.nes");
    let (mut stdout, mut stderr) = (Cursor::new(Vec::new()), Cursor::new(Vec::new()));
    let result = run(
        vec![
            input.display().to_string(),
            "-o".into(),
            output.display().to_string(),
        ],
        &mut stdout,
        &mut stderr,
    );
    let stderr = String::from_utf8(stderr.into_inner()).unwrap();
    assert_eq!(result, Ok(()), "{stderr}");
    (
        String::from_utf8(stdout.into_inner()).unwrap(),
        stderr,
        fs::read(&output).expect("the ROM is written"),
    )
}

#[test]
fn the_card_fits_one_background_in_four_colour_sets() {
    let indexed = card();
    let encoded = encode_background(&indexed).expect("the card fits one background");

    assert!(
        encoded.chr().len() <= 256 * 16,
        "tiles fit one pattern table"
    );

    let mut sets: Vec<Vec<u8>> = encoded
        .palettes()
        .chunks(4)
        .map(|set| {
            let mut colours: Vec<u8> = set[1..]
                .iter()
                .copied()
                .filter(|colour| *colour != BACKDROP)
                .collect();
            colours.sort_unstable();
            colours
        })
        .collect();
    sets.sort();
    let mut expected = vec![
        vec![GREY, WHITE],
        vec![YELLOW],
        vec![RED, GREEN, WHITE],
        vec![BLUE, PURPLE, WHITE],
    ];
    for set in &mut expected {
        set.sort_unstable();
    }
    expected.sort();
    assert_eq!(
        sets, expected,
        "{{grey, white}}, {{yellow}} and the two swatch pairs"
    );

    let width = indexed.width_tiles() as usize * 8;
    for (position, colour) in indexed.pixels().iter().enumerate() {
        if *colour == YELLOW {
            assert!(
                position % width < 16,
                "yellow outside the ruler at {position}"
            );
        }
    }
}

#[test]
fn the_demo_builds_with_the_agreed_summary() {
    let (stdout, stderr, rom) = build(&mvp_demo_path());

    assert_eq!(stderr, "", "no warnings");
    assert_eq!(rom[5], 1, "the picture's tiles are CHR ROM");
    assert!(
        stdout.contains("      chr  8 KiB ROM, 1 bank of 8 KiB\n"),
        "{stdout}"
    );
    assert!(stdout.contains("   bank 0  picture.tiles, "), "{stdout}");
    let table = stdout
        .split("\n\n")
        .nth(1)
        .expect("a budget table follows the summary");
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines[0], "frame bars (using irq, mmc3)");
    assert_eq!(lines.len(), 4, "{table}");
    for (line, scanline) in lines[1..].iter().zip(["60 ", "120", "180"]) {
        assert!(
            line.starts_with(&format!("  scanline  {scanline}")) && line.ends_with(" spare"),
            "{line}"
        );
    }
}

#[test]
fn the_picture_renders_pixel_identical_to_the_source() {
    let (_, _, rom) = build(&mvp_demo_path());
    let frame = render_after_frames("demo.nes", &rom, NonZeroU32::new(8).unwrap()).unwrap();
    let source = card();

    assert_eq!(source.pixels().len(), FRAME_WIDTH * FRAME_HEIGHT);
    // The low six bits are the NES colour; the bits above are emphasis, which the bars change.
    let shown: Vec<u8> = frame
        .as_indices()
        .iter()
        .map(|entry| (*entry & 0x3f) as u8)
        .collect();
    let wrong = shown.iter().zip(source.pixels()).position(|(a, b)| a != b);
    assert_eq!(
        wrong.map(|at| (at % FRAME_WIDTH, at / FRAME_WIDTH)),
        None,
        "first pixel (x, y) that differs from the PNG"
    );
}

#[test]
fn the_tints_start_on_the_lines_the_ruler_marks() {
    let (_, _, rom) = build(&mvp_demo_path());
    // The emphasis bits above the colour, sampled down column 128: a handler runs part-way along
    // its scanline, so only a column well clear of the write dot is the same on every frame.
    let bands = |frames: u32| {
        let frame =
            render_after_frames("demo.nes", &rom, NonZeroU32::new(frames).unwrap()).unwrap();
        let mut bands: Vec<(usize, u16)> = Vec::new();
        for y in 0..FRAME_HEIGHT {
            let emphasis = frame.as_indices()[y * FRAME_WIDTH + 128] >> 6;
            if bands.last().map(|band| band.1) != Some(emphasis) {
                bands.push((y, emphasis));
            }
        }
        bands
    };

    let first = bands(8);
    // Emphasis red is bit 6 of an entry, green bit 7: `$3e` and `$5e` in PPUMASK.
    assert_eq!(first, vec![(0, 0), (60, 0b001), (120, 0b010), (180, 0)]);
    assert_eq!(
        bands(120),
        first,
        "the bars hold their lines two seconds later"
    );
}

#[test]
fn the_backdrop_example_still_builds_from_its_new_path() {
    let path = backdrop_path();
    let (_, _, rom) = build(&path);

    assert_eq!(rom, raster_link::m1_solid_backdrop_rom());
}

/// Section 13 of the specification is the language's showcase, so an author who copies it must get
/// a ROM. The program is read out of the document and built exactly as printed, beside the picture
/// its asset names.
#[test]
fn the_specifications_complete_example_builds_as_printed() {
    let spec = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/raster-language-spec.md"
    ));
    let program = spec
        .split("## 13. Complete example — the MVP demo")
        .nth(1)
        .expect("§13 is in the specification")
        .split("```")
        .nth(1)
        .expect("§13 opens with a fenced block");

    let directory = Scratch::new("spec-13");
    fs::create_dir_all(directory.path().join("art")).unwrap();
    fs::copy(picture_path(), directory.path().join("art/picture.png")).unwrap();
    let source = directory.path().join("example.raster");
    fs::write(&source, program).unwrap();

    let (_summary, diagnostics, rom) = build(&source);

    assert!(rom.starts_with(b"NES\x1a"), "a ROM, not a refusal");
    assert!(diagnostics.is_empty(), "no warnings: {diagnostics}");
}
