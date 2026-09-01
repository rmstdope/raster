# raster-mb1 — retrospective

- **Implementer:** Rogue
- **Date:** 2026-09-01
- **PR:** #50

## Reverting a temporary edit with `mv file.bak file` left cargo running the stale build

**What happened.** Increment 1's "prove it fails" step wanted a temporary `self.emit(CLI, ...)` in
`Generator::main`. I made it with `sed -i.bak` and reverted it with
`mv crates/raster-codegen/src/lib.rs.bak crates/raster-codegen/src/lib.rs`. `git status` was clean
and `git diff --stat` showed nothing, so the revert was correct — but the very next
`cargo test -p raster-codegen` reported three failures, including two pre-existing tests I had never
touched. It reads exactly like a botched revert.

**Why.** Established. `sed -i.bak` renames the original file aside, so the `.bak` keeps the original
checkout mtime; `mv`-ing it back therefore restored a file *older* than the compiled artifact. Cargo
decides staleness by mtime, so it reused the build that still contained the injected `CLI`.
`touch crates/raster-codegen/src/lib.rs` and a rerun were green immediately.

**Cost.** About three minutes and one wasted `cargo test` run. Small here, but it presented as a red
suite with unrelated tests failing after a revert that was demonstrably correct, which is the kind of
thing that turns into a long hunt.

**Prevent by.** Reverting a temporary edit with `git checkout -- <path>`, never by moving a `.bak`
back. `git checkout` writes the file fresh, so its mtime is newer than the artifact and cargo
rebuilds. This matters specifically for the "prove each test can fail" step that `implement-bead`'s
TDD discipline requires on a regression-only bead — that step is nothing but temporary edits and
reverts, and this project builds with cargo, where the staleness check is mtime and not content.
Increment 2 used `git checkout --` and had no such problem.

**Seen before.** None found — `grep -rliE "mtime|stale build|\.bak|rebuild" docs/retrospectives/`
matched nothing before this file.
