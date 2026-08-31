# raster-6sl.4 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-08-31
- **PR:** #13

## Worktree setup does not execute compound install commands

**What happened.** `prepare-worktree` created this bead's worktree but passed the declared
`rustup toolchain install stable && cargo fetch` command as one `rustup` invocation. It failed with
`invalid value '&&' for '[TOOLCHAIN]...'`; running `cargo fetch` directly completed setup.
**Why.** `prepare-worktree` does not execute compound project install commands through a shell.
**Cost.** One failed setup attempt and a manual dependency fetch.
**Prevent by.** Update `.claude/cerebro/scripts/prepare-worktree` to execute compound project
install commands, then cover the declaration in `.cerebro/project.conf` with its install test.
**Seen before.** raster-20i.3, raster-20i.4, raster-tf5.2, raster-tf5.3, raster-6sl.3, raster-6sl.6.
