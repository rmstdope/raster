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
    // Opaque black is entry $0D, not the backdrop, so tile 0 is not the blank tile
    // every background starts from, and that is 513, every one of them counted. Two colours everywhere keeps every attribute cell and the
    // sub-palette budget well inside their limits, so it is the tile count that is
    // reached.
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
        "`art/picture.png` needs 513 distinct tiles; a background allows 256"
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

/// A picture nothing loads is a declaration: it is read and checked, and costs the ROM nothing.
#[test]
fn a_valid_picture_that_nothing_loads_compiles() {
    let bytes = png_bytes(16, 16, |x, y| {
        if (x / 8 + y / 8) % 2 == 0 {
            [0xff, 0, 0, 0xff]
        } else {
            BLACK
        }
    });
    let rom = compile_source_with_assets(&source(), &Fixture(PATH, bytes)).expect("it compiles");
    assert_eq!(rom.image[5], 0, "no `chrrom`, so still CHR RAM");
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

/// The program of [`source`], asking for the picture to be converted.
fn converting_source() -> String {
    format!(
        "asset image picture = png(\"{PATH}\") {{ kind: background  convert: true }}\nmain {{ }}"
    )
}

#[test]
fn a_picture_too_large_for_the_screen_compiles_when_converted() {
    // Twice the screen, in five colours to a region: refused as it stands, shown once converted.
    let picture = png_bytes(512, 480, |x, y| match (x / 8 + y / 8) % 5 {
        0 => BLACK,
        1 => [0xf8, 0x38, 0x00, 0xff],
        2 => [0xfc, 0xa0, 0x44, 0xff],
        3 => [0x00, 0x00, 0xfc, 0xff],
        _ => [0xf8, 0xf8, 0xf8, 0xff],
    });

    let found = compile_source_with_assets(&source(), &Fixture(PATH, picture.clone()))
        .expect_err("refused as it stands");
    assert!(
        found[0].message.contains("is 512 by 480 pixels"),
        "got {found:?}"
    );

    compile_source_with_assets(&converting_source(), &Fixture(PATH, picture))
        .expect("converted, it compiles");
}

#[test]
fn a_converted_picture_of_too_many_tiles_says_how_many_it_needs() {
    // Every tile different: its first rows count the tile's index in binary.
    let picture = png_bytes(256, 240, |x, y| {
        let tile = (y / 8) * 32 + x / 8;
        let bit = (x % 8) * 8 + y % 8;
        if bit < 10 && tile >> bit & 1 == 1 {
            [0xf8, 0xf8, 0xf8, 0xff]
        } else {
            BLACK
        }
    });
    let source = converting_source();

    let found =
        compile_source_with_assets(&source, &Fixture(PATH, picture)).expect_err("too many tiles");

    assert_eq!(found.len(), 1, "got {found:?}");
    assert_eq!(
        found[0].message,
        "`art/picture.png` needs 960 distinct tiles; a background allows 256"
    );
    assert_points_at_the_path(&source, &found[0]);
}

#[test]
fn a_jpeg_named_png_is_called_a_jpeg() {
    let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F'].to_vec();
    for source in [source(), converting_source()] {
        let found = compile_source_with_assets(&source, &Fixture(PATH, jpeg.clone()))
            .expect_err("a JPEG is not a PNG");
        assert_eq!(
            found[0].message, "`art/picture.png` is a JPEG, not a PNG; save it as a PNG",
            "for {source}"
        );
    }
}
