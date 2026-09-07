//! Every way a picture can be wrong, as a diagnostic under the path that named it.
//!
//! The fixtures are built in memory rather than committed: a test's input is then
//! readable in the test that uses it, and this repository stays free of binary
//! fixtures. `AssetSource` is what makes that possible — the pipeline never opens
//! a file itself.

use raster_diag::Severity;
use rasterc::{compile_source_with_assets, AssetSource};

/// One file, by the name the source asks for.
struct Fixture(&'static str, Vec<u8>);

impl AssetSource for Fixture {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        if path == self.0 {
            Ok(self.1.clone())
        } else {
            Err(format!("no such file `{path}`"))
        }
    }
}

const PATH: &str = "art/picture.png";

/// A program that is valid but for its picture, so the asset stage is the one that
/// runs: stages stop at the first with errors.
fn source() -> String {
    format!("asset image picture = png(\"{PATH}\") {{ kind: background }}\nmain {{ }}")
}

/// The single diagnostic a fixture produces, with the source it was compiled from.
fn only_diagnostic(fixture: Fixture) -> (String, raster_diag::Diagnostic) {
    let source = source();
    let found =
        compile_source_with_assets(&source, &fixture).expect_err("the fixture does not compile");
    assert_eq!(found.len(), 1, "expected one diagnostic, got {found:?}");
    assert_eq!(found[0].severity, Severity::Error);
    (source, found[0].clone())
}

/// The caret sits under the quoted path, which is the thing the author must change.
fn assert_points_at_the_path(source: &str, diagnostic: &raster_diag::Diagnostic) {
    let span = diagnostic.span.expect("an asset diagnostic has a span");
    assert_eq!(&source[span.start..span.end], format!("\"{PATH}\""));
}

/// An RGBA PNG of `width` by `height`, coloured by `pixel`.
fn png_bytes(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&pixel(x, y));
        }
    }
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("the PNG header is written");
    writer
        .write_image_data(&pixels)
        .expect("the PNG image data is written");
    drop(writer);
    output
}

const BLACK: [u8; 4] = [0, 0, 0, 0xff];

#[test]
fn an_unreadable_asset_file_names_the_path() {
    let (source, diagnostic) = only_diagnostic(Fixture("art/other.png", Vec::new()));
    assert_eq!(
        diagnostic.message,
        "could not read `art/picture.png`: no such file `art/picture.png`"
    );
    assert_points_at_the_path(&source, &diagnostic);
}

#[test]
fn a_file_that_is_not_a_png_says_so() {
    let (source, diagnostic) = only_diagnostic(Fixture(PATH, b"not a png".to_vec()));
    assert!(
        diagnostic
            .message
            .starts_with("`art/picture.png` is not a readable PNG: "),
        "got {}",
        diagnostic.message
    );
    assert_points_at_the_path(&source, &diagnostic);
}

#[test]
fn an_image_that_is_not_whole_tiles_says_what_it_is() {
    let (source, diagnostic) = only_diagnostic(Fixture(PATH, png_bytes(12, 8, |_, _| BLACK)));
    assert_eq!(
        diagnostic.message,
        "`art/picture.png` is 12 by 8 pixels; a background must be a whole number \
         of 8-pixel tiles, at most 256 by 240"
    );
    assert_points_at_the_path(&source, &diagnostic);
}

#[test]
fn a_partly_transparent_pixel_is_located() {
    let bytes = png_bytes(8, 8, |x, y| {
        if (x, y) == (3, 5) {
            [0xff, 0, 0, 128]
        } else {
            BLACK
        }
    });
    let (source, diagnostic) = only_diagnostic(Fixture(PATH, bytes));
    assert_eq!(
        diagnostic.message,
        "`art/picture.png` has a partly transparent pixel at (3, 5); a background \
         pixel is either opaque or fully transparent"
    );
    assert_points_at_the_path(&source, &diagnostic);
}

/// Five distinct colours in one 16-by-16 attribute region, which the decoder
/// refuses before the encoder ever sees the image.
#[test]
fn an_attribute_cell_over_four_colours_is_located() {
    const COLOURS: [[u8; 4]; 5] = [
        [0, 0, 0, 0xff],
        [0xff, 0, 0, 0xff],
        [0, 0xff, 0, 0xff],
        [0, 0, 0xff, 0xff],
        [0xff, 0xff, 0xff, 0xff],
    ];
    let bytes = png_bytes(16, 16, |x, y| {
        if y < 5 && x == 0 {
            COLOURS[y as usize]
        } else {
            BLACK
        }
    });
    let (source, diagnostic) = only_diagnostic(Fixture(PATH, bytes));
    assert_eq!(
        diagnostic.message,
        "attribute cell (0, 0) of `art/picture.png` uses 5 colours; an attribute cell allows 4"
    );
    assert_points_at_the_path(&source, &diagnostic);
}

/// More than 256 distinct 8-by-8 tiles, each attribute region staying inside four
/// colours so it is the tile budget that is reached and not the palette one.
#[test]
fn too_many_distinct_tiles_says_the_count() {
    // 32 by 30 tiles is 960, and each tile draws its own index in binary — eight
    // bits along its top row and a ninth below — so the first 512 are distinct.
    // Two colours everywhere keeps every attribute cell and the sub-palette
    // budget well inside their limits, so it is the tile count that is reached.
    let bytes = png_bytes(256, 240, |x, y| {
        let index = (y / 8) * 32 + (x / 8);
        let (column, row) = (x % 8, y % 8);
        let lit = (row == 0 && (index >> column) & 1 == 1)
            || (row == 1 && column == 0 && (index >> 8) & 1 == 1);
        if lit {
            [0xff, 0xff, 0xff, 0xff]
        } else {
            BLACK
        }
    });
    let (source, diagnostic) = only_diagnostic(Fixture(PATH, bytes));
    assert_eq!(
        diagnostic.message,
        "`art/picture.png` needs 257 distinct tiles; a background allows 256"
    );
    assert_points_at_the_path(&source, &diagnostic);
}

/// Five distinct sub-palettes across the picture, each attribute region inside the
/// four-colour limit, so it is the encoder's palette budget that is reached.
#[test]
fn too_many_subpalettes_says_the_count() {
    const PALETTES: [[u8; 4]; 5] = [
        [0xff, 0, 0, 0xff],
        [0, 0xff, 0, 0xff],
        [0, 0, 0xff, 0xff],
        [0xff, 0xff, 0, 0xff],
        [0xff, 0xff, 0xff, 0xff],
    ];
    let bytes = png_bytes(80, 16, |x, y| {
        let region = (x / 16) as usize;
        if y < 8 {
            PALETTES[region]
        } else {
            BLACK
        }
    });
    let (source, diagnostic) = only_diagnostic(Fixture(PATH, bytes));
    assert_eq!(
        diagnostic.message,
        "`art/picture.png` needs 5 sub-palettes; a background allows 4"
    );
    assert_points_at_the_path(&source, &diagnostic);
}

/// The end of this bead: the file was read, decoded and encoded without complaint,
/// and the only thing left is that no byte of it is placed yet.
#[test]
fn a_valid_picture_is_read_and_then_refused_for_not_reaching_the_rom() {
    let bytes = png_bytes(16, 16, |x, y| {
        if (x / 8 + y / 8) % 2 == 0 {
            [0xff, 0, 0, 0xff]
        } else {
            BLACK
        }
    });
    let (_, diagnostic) = only_diagnostic(Fixture(PATH, bytes));
    assert_eq!(diagnostic.message, "an `asset` does not reach the ROM yet");
}

/// `compile_source` compiles from text and has no files, so an asset in a string
/// says that rather than reporting a filesystem error the author cannot act on.
#[test]
fn a_program_compiled_from_text_has_no_files_to_read() {
    let source = source();
    let found = rasterc::compile_source(&source).expect_err("there is no file to read");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "could not read `art/picture.png`: no file could be read for `art/picture.png`: \
         this program was compiled from text, not from a file"
    );
}

/// Every asset is attempted, so an author with two broken pictures is told about
/// both rather than one per run.
#[test]
fn two_broken_pictures_are_both_reported() {
    struct NoFiles;
    impl AssetSource for NoFiles {
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            Err(format!("no such file `{path}`"))
        }
    }

    let source = "asset image one = png(\"a.png\")\nasset image two = png(\"b.png\")\nmain { }";
    let found = compile_source_with_assets(source, &NoFiles).expect_err("neither file exists");

    let messages: Vec<_> = found.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "could not read `a.png`: no such file `a.png`",
            "could not read `b.png`: no such file `b.png`",
        ]
    );
}
