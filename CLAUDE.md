# raster

## The project

raster is a high-level programming language for writing timing-critical NES ROMs. Its users are
NES developers; a change is working when the language toolchain correctly builds their ROMs while
preserving the timing guarantees its source constructs express.

## Four Eye Principle

Nothing merges unreviewed and nothing merges red.

For a change built by an agent, GitHub Copilot's automatic review counts as the second pair of eyes
when it has reviewed the pull request's head commit, every comment it left is answered by a change
or a posted reply, and every check is green.

Everything else needs the navigator.

## Work tracking

Planned work is tracked in beads (`bd`), not in GitHub issues. GitHub issues are the inbox for
external requests and bug reports only. Every bead is created unranked at P4 and ranked later with
the navigator; a bead is planned in one session and implemented in another.

The board syncs through the Dolt remote rather than through git. A fresh clone runs `bd bootstrap`;
after that use `bd dolt pull` and `bd dolt push`.

## Development practices

- Work is delivered in small increments that stand on their own.
- Code is written test-first.
- Prefer the simple design; say so when you decline a more general one.

## Where the project declares its facts

- `.cerebro/project.conf` declares the project, paths, gates, provider, and verification policy.
- `.cerebro/roster.conf` optionally declares the fleet; when absent, Cerebro uses its built-in fleet.
- `.cerebro/traps.md` records project-specific traps once the project has paid for them.

The runtime directories `.cerebro/worktrees`, `.cerebro/state`, and `.cerebro/scratch` are ignored;
the declarations above are tracked.
