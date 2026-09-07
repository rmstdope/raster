//! Reading and decoding the files an `asset` item names.
//!
//! This is the one stage that needs something outside the source text, and it
//! still does not open a file: the caller supplies an [`AssetSource`], so
//! `compile_source` stays a pure function of its string and every diagnostic
//! here is testable from bytes held in memory.

use raster_assets::{
    decode_background, encode_background, BackgroundEncodeError, NesBackground, PngBackgroundError,
};
use raster_syntax::{Asset, Item, Program, Span};

/// Where an `asset` item's file comes from.
///
/// The pipeline never opens a file itself: the CLI supplies a reader rooted at
/// the directory of the source being compiled, and a test supplies bytes it
/// holds in memory. That is what keeps `compile_source` a pure function of its
/// text and keeps every asset diagnostic testable without a temporary directory.
pub trait AssetSource {
    /// The bytes of `path`, or a reason the author can act on.
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
}

/// An [`AssetSource`] that has no files, for callers compiling a string.
pub struct NoAssets;

impl AssetSource for NoAssets {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        Err(format!(
            "no file could be read for `{path}`: this program was compiled from text, \
             not from a file"
        ))
    }
}

/// Every asset the program declared, decoded and encoded, by name.
///
/// Crate-private until something reads it: nothing consumes a decoded picture in
/// this release, and exporting it would put `raster_assets::NesBackground` into
/// `rasterc`'s public API a bead before anything needs it there.
pub(crate) struct Assets {
    /// In declaration order, so a later bead lays them out predictably.
    ///
    /// Built and not yet read: encoding is what reports a picture that needs
    /// more than 256 tiles or more than four sub-palettes, so it must happen
    /// whether or not anything consumes the result. `raster-fl4.5.3` is what
    /// reads this, and lifts the allow with it.
    #[allow(dead_code)]
    pub(crate) images: Vec<(String, NesBackground)>,
}

/// Why one asset could not be built, at the span of the path that named it.
///
/// A `raster_syntax::Span` rather than a rendered diagnostic, so this module
/// stays out of the business of clamping offsets — `compile.rs` does that once,
/// for every stage, and this stage goes through the same path as the rest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AssetError {
    pub(crate) message: String,
    pub(crate) span: Span,
}

/// Read and decode every `asset` item, or report why one could not be.
///
/// Every asset is attempted, so an author with two broken pictures is told about
/// both rather than one per run.
pub fn resolve_assets(
    program: &Program,
    source: &dyn AssetSource,
) -> Result<Assets, Vec<AssetError>> {
    let mut images = Vec::new();
    let mut errors = Vec::new();
    for item in &program.items {
        let Item::Asset(asset) = &item.value else {
            continue;
        };
        match resolve_one(asset, source) {
            Ok(background) => images.push((asset.name.value.clone(), background)),
            Err(message) => errors.push(AssetError {
                message,
                span: asset.path.span,
            }),
        }
    }
    if errors.is_empty() {
        Ok(Assets { images })
    } else {
        Err(errors)
    }
}

fn resolve_one(asset: &Asset, source: &dyn AssetSource) -> Result<NesBackground, String> {
    let path = asset.path.value.as_str();
    let bytes = source
        .read(path)
        .map_err(|reason| format!("could not read `{path}`: {reason}"))?;
    let indexed =
        decode_background(bytes.as_slice()).map_err(|error| decode_message(path, error))?;
    encode_background(&indexed).map_err(|error| encode_message(path, error))
}

/// Every variant of `PngBackgroundError`, as something the author can act on.
/// Each names the file and the number that was wrong, because a picture is
/// nearly always nearly right.
fn decode_message(path: &str, error: PngBackgroundError) -> String {
    match error {
        PngBackgroundError::Decode { message } => {
            format!("`{path}` is not a readable PNG: {message}")
        }
        PngBackgroundError::InvalidDimensions { width, height } => format!(
            "`{path}` is {width} by {height} pixels; a background must be a whole number \
             of 8-pixel tiles, at most 256 by 240"
        ),
        PngBackgroundError::PartialAlpha { x, y, .. } => format!(
            "`{path}` has a partly transparent pixel at ({x}, {y}); a background pixel is \
             either opaque or fully transparent"
        ),
        PngBackgroundError::TooManyColours {
            attribute_x,
            attribute_y,
            colour_count,
        } => format!(
            "attribute cell ({attribute_x}, {attribute_y}) of `{path}` uses {colour_count} \
             colours; an attribute cell allows 4"
        ),
    }
}

/// Every variant of `BackgroundEncodeError`, likewise.
///
/// `TooManyColours` is unreachable from a picture `decode_background` accepted —
/// the decoder checks the same 16-by-16 regions the encoder derives palettes
/// from — but it is worded anyway, because the alternative to a message for a
/// variant is a panic or a shrug in front of an author.
fn encode_message(path: &str, error: BackgroundEncodeError) -> String {
    match error {
        BackgroundEncodeError::TooManyTiles { tile_count } => {
            format!("`{path}` needs {tile_count} distinct tiles; a background allows 256")
        }
        BackgroundEncodeError::TooManyPalettes { palette_count } => {
            format!("`{path}` needs {palette_count} sub-palettes; a background allows 4")
        }
        BackgroundEncodeError::TooManyColours { colour_count } => format!(
            "`{path}` has an attribute cell of {colour_count} colours; an attribute cell allows 4"
        ),
    }
}

/// The files beside a source file on disk.
///
/// Asset paths resolve relative to the source, not to the working directory:
/// that is the only rule under which `png("art/picture.png")` means the same
/// thing wherever `rasterc` is run from.
pub struct DirectoryAssets {
    root: std::path::PathBuf,
}

impl DirectoryAssets {
    /// Rooted at the directory holding `input`. A bare file name has no parent,
    /// and the working directory is then the right answer rather than a
    /// fallback: it is the directory the source is in.
    pub fn beside(input: &str) -> Self {
        Self {
            root: std::path::Path::new(input)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf(),
        }
    }
}

impl AssetSource for DirectoryAssets {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        std::fs::read(self.root.join(path)).map_err(|error| error.to_string())
    }
}
