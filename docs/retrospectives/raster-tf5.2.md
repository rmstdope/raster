# raster-tf5.2 — retrospective

- **Implementer:** Wolverine
- **Date:** 2026-08-30
- **PR:** #4

## Worktree setup passed a shell operator to rustup

**What happened.** `prepare-worktree` read the declared install command
`rustup toolchain install stable && cargo fetch` as a `rustup` argument list and failed with
`invalid value '&&' for '[TOOLCHAIN]...'`. Cargo subsequently fetched dependencies when the
first test ran.
**Why.** The project install declaration contains a compound shell command, but the worktree setup
script invokes it without a shell.
**Cost.** One failed worktree-preparation attempt and a second setup step, about five minutes.
**Prevent by.** Update `.claude/cerebro/scripts/prepare-worktree` to execute compound project
install commands through a shell, or declare the install step as a script without shell operators.
**Seen before.** none found.
