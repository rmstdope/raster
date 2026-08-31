# raster-c35 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-08-31
- **PR:** #27

## `cargo clippy --all-targets` is red on files no bead has touched

**What happened.** Running `cargo clippy --all-targets -- -D warnings` — a stronger form of the
gate than the one `.cerebro/project.conf` declares — failed with an `is_none_or` lint in
`crates/raster-codegen/tests/generate.rs:147` and `crates/rasterc/tests/emulator.rs`, neither of
which this bead touches. The declared `gate_full` is `cargo fmt --check && cargo clippy -- -D
warnings && cargo test`, without `--all-targets`, and that gate is green; so is CI. The review
sub-agent independently reached for `--all-targets` and reported the same two files.

**Why.** `cargo clippy` without `--all-targets` lints the library and binary targets only, so no
lint in `crates/*/tests/` has ever been enforced here. The two files have drifted red without
anything failing.

**Cost.** A few minutes each for the implementer and the reviewer, twice in one bead, and a moment
of believing a green change had broken an untouched crate. No CI cycles.

**Prevent by.** Either the project's `gate_full` in `.cerebro/project.conf` gains `--all-targets`
and the two lints are fixed in their own bead, or the fact that test targets are deliberately
unlinted is written into `.cerebro/traps.md` so the next agent who reaches for the stronger command
knows what it will say before running it. Both are the navigator's to choose; this run only
recorded it.

**Seen before.** None found — `docs/retrospectives/raster-fl4.2.md` mentions clippy, but as a run
that revealed uncommitted work in a worktree, not this.
