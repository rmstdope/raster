# raster-5vk — retrospective

- **Implementer:** Storm
- **Date:** 2026-10-29
- **PR:** #75

## Two `cli` tests failed once in a full `cargo test`, and passed on every re-run

**What happened.** `a_missing_asset_file_is_reported_by_name` and
`a_missing_parent_directory_is_an_error_rather_than_a_new_tree` failed in one full run while the
new `frame_budget` tests ran in parallel; three further full runs and the single suite were green.
Before that, two of the new tests had already shared a directory and read each other's source.
**Why.** Not established for the two `cli` tests. For the new tests: `Scratch::new` names a
directory from the pid and `SystemTime` nanoseconds, which on macOS has microsecond resolution
(`...049116000`, `...049164000`), so two parallel tests can get one path.
**Cost.** About ten minutes reading wrong output, and one unexplained red.
**Prevent by.** `tests/common/scratch.rs`: add a process-wide atomic counter to the directory name.
**Seen before.** none found.

## The branch conflicted with main twice while the review ran

**What happened.** Main split `tests/common/mod.rs` into submodules and appended tests to
`crates/raster-ir/tests/lower.rs`, the same tail this bead appended to; the second rebase had
interleaved conflict regions that deleting the markers could not resolve.
**Why.** Every bead appends its tests at the end of the same file.
**Cost.** Two rebases and a re-gate.
**Prevent by.** not established.
**Seen before.** none found.
