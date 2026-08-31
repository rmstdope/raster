//! Helpers shared by the `rasterc` integration tests. Each test file is its own
//! crate, so this module is included by each rather than linked once.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// The demo the milestone is about, read from the repository rather than copied,
/// so a test can never drift from the example an author actually compiles.
pub fn demo_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp/demo.raster");
    fs::read_to_string(path).expect("the demo example is readable")
}

/// A directory of one test's own, removed when the test ends. Tests run in
/// parallel and write ROMs, so sharing a path would make them flaky, and leaving
/// the directories behind would litter every developer machine and CI runner.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rasterc-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the clock is after the epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("the scratch directory is creatable");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
