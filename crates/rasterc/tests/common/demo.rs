//! The examples, shared by the `rasterc` integration tests.

use std::{fs, path::PathBuf};

/// The backdrop example (milestone 4's one-colour program), read from the repository rather than copied,
/// so a test can never drift from the example an author actually compiles.
#[allow(dead_code)]
pub fn backdrop_source() -> String {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/backdrop/backdrop.raster");
    fs::read_to_string(path).expect("the backdrop example is readable")
}

/// The MVP demo, which tests build from its path so that the PNG beside it is found.
#[allow(dead_code)]
pub fn mvp_demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp/demo.raster")
}

/// The backdrop example's path, for tests that build it through the CLI.
#[allow(dead_code)]
pub fn backdrop_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/backdrop/backdrop.raster")
}

/// The scrolled copper bar example, built from its path so that the PNG beside it is found.
#[allow(dead_code)]
pub fn copper_scroll_demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/copper-scroll/copper.raster")
}
