# raster

## The project

raster is a compiler CLI for NES software developers building timing-critical NES ROMs. A change is
working when the toolchain correctly builds their ROMs while preserving the timing guarantees its
source constructs express.

Raster runs locally from source on macOS, Linux, and Windows. Maintainers create downloadable GitHub
releases by setting tags; generated ROMs and intermediate assets stay outside the source repository.
Already-installed Raster has no runtime service dependency.

Developers write a `.raster` file, run `rasterc`, and test the resulting ROM in an emulator. No input
prints usage and exits nonzero; invalid source produces concise, actionable source-spanned diagnostics.
The terminal interface is keyboard-first, has no authentication or persistent state, communicates no
meaning by colour alone, and feels fast, streamlined, and tech.

## Producer review

Nothing merges red. Before delivery, a producer obtains and addresses one independent, full review of the complete diff and bead. If its changes are substantial enough to make another review useful, the producer chooses the right follow-up scope and obtains it; minor, self-contained answers need not create a review loop. Unresolved findings, a red or missing check, or a reviewer that cannot produce a usable result go to a person. Whether the navigator also takes part in the plan, the review or the merge is `navigator_gates` in `.cerebro/project.conf`; absent means none. This review satisfies the Four eye Principle below.

## Work tracking

Planned work is tracked in beads (`bd`), prefix `rq`; `skills/beads-workflow` carries the commands. GitHub issues are the inbox for outside requests and bug reports only. Every bead is created unranked and ranked later with the navigator; a bead is planned in one session and implemented in another.

The board syncs through the Dolt remote on `origin`, not through git. A fresh clone runs `bd bootstrap` once (it refuses if a database already exists, so do not run `bd list` first); after that it is `bd dolt pull` and `bd dolt push`.

## Skills Usage

Always select the appropriate skill for a specific task. Be sure to ALWAYS explicitly write in the chat what skills that are currently being used. Always follow the instructions in the skills to the letter.

## Development practices

### Small Increments

The application shall ALWAYS be developed in very small, manageable increments that can be delivered independently. Each increment should add a specific feature or improvement to the application. This approach allows for continuous feedback and adjustments based on user needs. The code base should ALWAYS have a great safety net of tests to ensure that new changes do not break existing functionality.

### Test-driven Development (TDD)

In the development process, the application should be developed using Test-driven Development (TDD) principles. This means that you should write tests before writing the actual implementation code. This should be the case also when fixing bugs. First write a test that reproduces the bug, then fix the bug and verify that the test passes along with all existing tests.
However, when trying to pinpoint a bug, you are free to add any traces, try fixes or anything else without having to write tests for that immediately. But once the issue has been pinpointed, either update existing tests or add a new test that triggers the error before applying the fix. This ensures no unnecessary modifications are done and helps to prevent regressions in the future.

### Collaboration

Always seek clarification from the navigator when requirements are ambiguous. Use the question UI/tool with predefined answers when possible, and a free-text option when necessary.

### Design

Always prefer simple design solutions. Avoid over-engineering. If unsure, ask the navigator for clarification. The design should be easy to change if need be.

## Where the project declares its facts

- `.cerebro/project.conf` declares the project, paths, gates, provider, and verification policy.
- `.cerebro/roster.conf` optionally declares the fleet; when absent, Cerebro uses its built-in fleet.
- `.cerebro/traps.md` records project-specific traps once the project has paid for them.

The runtime directories `.cerebro/worktrees`, `.cerebro/state`, and `.cerebro/scratch` are ignored;
the declarations above are tracked.
