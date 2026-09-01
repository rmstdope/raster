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

## A sibling bead invalidated this plan's scope premise while the PR was open

**What happened.** The plan scoped `crates/rasterc/` out entirely, on the stated premise that "its
five timing diagnostics and its one backstop test already say 'timed block' throughout". Nine
commits merged to main between this PR opening and its merge, one of them `raster-fu0` (#29), which
added two new *user-facing note* strings to `crates/rasterc/src/compile.rs` — `TIMED_REGION_COST`
and `SUPPORTED_SUBSET` — both saying "timed region", and eleven new assertions in
`crates/raster-sema/tests/analyze.rs` and `crates/rasterc/tests/compile.rs` on the old wording. The
rebase conflicted in `analyze.rs`, where both beads had appended a test at the end of the file, and
the conflict boundary cut the sibling's function before its closing brace, so the first build after
resolving failed with `this file contains an unclosed delimiter` rather than with anything about
the merge.

**Why.** Two beads were in flight against the same twelve diagnostics at once. `raster-c35` renames
the noun in them; `raster-fu0` attaches a note to them. Neither plan named the other, and nothing in
the board expressed that they touch the same strings.

**Cost.** A rebase with one conflict, a scope judgement that had to be made in the implementer
rather than in the plan (the two notes are printed directly beneath the renamed messages, so
leaving them would have shipped both words in one screenful), one extra CI cycle, and roughly
40 minutes.

**Prevent by.** When two planned beads edit the same user-facing strings, the second plan should
name the first as a dependency in `bd` so they cannot be claimed concurrently — here `raster-c35`
and `raster-fu0` both listed the same twelve `self.error(` sites and neither declared a dependency.
Failing that, a plan's "out of scope because X is already true" premise should be re-checked against
`origin/main` at rebase time rather than trusted from the plan; this run only found the two notes
because the rebase forced a re-read of `crates/rasterc/`.

**Seen before.** None found.
