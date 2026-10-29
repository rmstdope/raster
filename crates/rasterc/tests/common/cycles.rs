//! The `tests/cycles` fixtures, shared by the tests that compile them.

use std::{fs, path::PathBuf};

/// A `tests/cycles` fixture, read from the repository so a test can never drift from the source an
/// author actually compiles.
pub fn cycles_fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cycles")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()))
}
