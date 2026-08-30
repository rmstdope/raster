# raster-6sl.3 — retrospective

- **Implementer:** Wolverine
- **Date:** 2026-08-31
- **PR:** #11

## Worktree preparation rejected the declared Rust install command

**What happened.** `prepare-worktree` created the worktree but passed the configured
`rustup toolchain install stable && cargo fetch` as a single toolchain value, which failed with
`invalid toolchain name: '&&'`.
**Why.** The setup script does not execute compound install commands from the project configuration.
**Cost.** One failed preparation step and a manual rerun of the two declared commands.
**Prevent by.** Update `.claude/cerebro/scripts/prepare-worktree` to execute the declared install
command as a shell command, or constrain `.cerebro/project.conf` install values to a single command.
**Seen before.** none found.
