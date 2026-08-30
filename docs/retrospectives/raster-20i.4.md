# raster-20i.4 — retrospective

- **Implementer:** Storm
- **Date:** 2026-08-31
- **PR:** #10

## Worktree setup passed a shell operator to rustup

**What happened.** `prepare-worktree` read the declared install command `rustup toolchain install
stable && cargo fetch` as a `rustup` argument list and failed with `invalid value '&&' for
'[TOOLCHAIN]...'`. Running `cargo fetch` separately completed the worktree setup.
**Why.** The project install declaration contains a compound shell command, but the worktree setup
script invokes it without a shell.
**Cost.** One failed worktree-preparation attempt and a manual dependency fetch.
**Prevent by.** Update `.claude/cerebro/scripts/prepare-worktree` to execute compound project
install commands through a shell, or declare the install step as a script without shell operators.
**Seen before.** raster-20i.3 — same configured install command and setup failure.
