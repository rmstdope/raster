//! A per-test scratch directory.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Tests in one binary share a process id and may start within one tick of the
/// clock, so the clock alone does not tell their directories apart.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A directory of one test's own, removed when the test ends. Tests run in
/// parallel and write ROMs, so sharing a path would make them flaky, and leaving
/// the directories behind would litter every developer machine and CI runner.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rasterc-{label}-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
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
