//! In-memory PNG fixtures for the picture tests.

/// A PNG that encodes to exactly `tiles` CHR tiles: the reserved blank tile plus `tiles - 1`
/// distinct ones. Tile `k` lights the pixels of its first row that spell `k` in binary, in the
/// one colour (NES `$30`) every test needs.
#[allow(dead_code)]
pub fn picture_png(tiles: usize) -> Vec<u8> {
    picture_png_in_row(tiles, 0)
}

/// [`picture_png`] with the lit pixels in pixel row `row` of each tile, so that two pictures of
/// the same size do not share a single tile: a test that shows one while expecting the other
/// then fails instead of passing by coincidence.
#[allow(dead_code)]
pub fn picture_png_in_row(tiles: usize, row: usize) -> Vec<u8> {
    assert!(row < 8);
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
                pixels[(tile_y * 8 + row) * width + tile_x * 8 + bit] = [0xfc, 0xfc, 0xfc, 0xff];
            }
        }
    }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("the PNG header is written");
    writer
        .write_image_data(&pixels.into_iter().flatten().collect::<Vec<_>>())
        .expect("the PNG data is written");
    drop(writer);
    bytes
}

/// The assets `picture_png` makes, by path, for `compile_source_with_assets`.
#[allow(dead_code)]
pub struct Pictures(pub Vec<(&'static str, usize)>);

impl rasterc::AssetSource for Pictures {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.0
            .iter()
            .find(|(name, _)| *name == path)
            // Only the picture is lit on its first row; everything else is lit on its fifth.
            .map(|(name, tiles)| {
                picture_png_in_row(*tiles, if *name == "picture.png" { 0 } else { 4 })
            })
            .ok_or_else(|| format!("no such file `{path}`"))
    }
}
