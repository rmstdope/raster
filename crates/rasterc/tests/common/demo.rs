//! The demo example, shared by every `rasterc` integration test.

use std::{fs, path::PathBuf};

/// The demo the milestone is about, read from the repository rather than copied,
/// so a test can never drift from the example an author actually compiles.
pub fn demo_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp/demo.raster");
    fs::read_to_string(path).expect("the demo example is readable")
}
