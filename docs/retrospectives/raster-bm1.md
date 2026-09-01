# raster-bm1 — retrospective

- **Implementer:** Storm
- **Date:** 2026-09-01
- **PR:** #51

## `git checkout <path>` to revert a mutation destroyed the whole uncommitted implementation

**What happened.** Increment 2's third test was meant to pin an ordering, and reading it I doubted it
could fail. To check, I inverted the two lines in `lower_statements`, ran the test — it failed, so the
pin was real — and reverted with `git checkout crates/raster-ir/src/lib.rs`, chained onto the same
`Bash` call. Nothing had been committed yet, so that restored the file from the index and deleted
increments 1 and 2 entirely: `grep -c LatchPair crates/raster-ir/src/lib.rs` returned 0. The work was
recoverable only because the insertion had been scripted from a heredoc still on disk in `/tmp`.

**Why.** `git checkout -- <path>` reverts to the index, not to "before my last edit". It is the right
tool for a temporary edit on top of a committed file and a work-destroying one on top of an
uncommitted file, and nothing in the command distinguishes the two cases.

**Cost.** About ten minutes to notice and re-apply, and it was luck rather than judgement that the
edit had been scripted. Had I written it with `Edit`, increments 1 and 2 would have been gone.

**Prevent by.** Commit the increment *before* mutating anything to prove a test can fail, and revert
with `git checkout` only then — the commit is what makes the index mean "my work". `implement-bead`
does not say when to commit within a bead, and the TDD skill's `RED → GREEN → REFACTOR → COMMIT`
places the commit after the refactor, which is exactly when a mutation check wants to run. A sentence
in `implement-bead`'s *Building* section — prove a test can fail only against a committed increment —
would close it.

**Seen before.** `docs/retrospectives/raster-mb1.md` records the *opposite* half of this and
prescribes `git checkout -- <path>` as the way to revert a temporary edit, because restoring a `.bak`
leaves an mtime cargo will not rebuild from. Both are right about their own case; neither says the
choice depends on whether the file is committed, and following mb1's advice literally on an
uncommitted file is what happened here. `docs/retrospectives/raster-fl4.4.md` mentions the command in
an unrelated worktree context.

## A sibling bead invalidated this plan's *out of scope* premise while the PR was open

**What happened.** The plan named `raster-xeo` as out of scope, "still open", and required that "this
bead must not refuse [`ppu.status = 0`]; a test in increment 7 pins that". `raster-xeo` (#46) merged
during the review round and now refuses that write outright. The rebase conflicted in
`crates/raster-ir/src/lib.rs` and `crates/rasterc/tests/cli.rs` — both beads appended to the same
regions — and afterwards both increment-7 tests failed, because their fixtures no longer lower at
all. Both were rewritten onto `lower_failure`, keeping their intent exactly: a compound assignment
reads `$2002` and a plain write does not, asserted on the warnings that survive a failed lowering.

**Why.** A plan's *out of scope* section states the state of the board at planning time. Nothing
re-checks it, and a bead whose scope is defined by "that other bead has not shipped yet" silently
expires when it does.

**Cost.** A conflicted seven-commit rebase, two rewritten tests, and one extra CI cycle — about
forty minutes.

**Prevent by.** Where a plan's scope depends on a *named open bead*, it should say what to do when
that bead merges first, rather than only that it is open. Cheaply: `bd show <the named bead>` before
the first increment and again before the merge.

**Seen before.** `docs/retrospectives/raster-c35.md` — same class, same phase (a sibling merged
between PR opening and merge, conflicting where both had appended to one file).
`docs/retrospectives/raster-hqh.md` — "A sibling's test used the exact source this bead refuses, and
the plan could not have known". Third sighting.

## `model-for`'s prescribed invocation misfires and exits 2 — twelfth sighting

**What happened.** `implement-bead`'s line, run verbatim, exited 2 with the usage message.
Invoked directly, `model-for --provider claude --role reviewer` exits 0 and prints nothing — a
genuine miss, since this project has no `.cerebro/models.conf` at all.

**Why.** Not established here; eleven earlier files record the same symptom from the
`${provider:+--provider "$provider"}` expansion.

**Cost.** Two minutes. The answer was unchanged either way: no key matched, so the sub-agent ran on
the CLI's default.

**Prevent by.** As eleven earlier retrospectives ask: `implement-bead` should prescribe the two-line
form (build the argument list, then call) rather than the `${var:+...}` one-liner.

**Seen before.** `raster-1t9`, `raster-6jr`, `raster-7nl`, `raster-fu0`, `raster-fl4.2`,
`raster-fl4.3`, `raster-fl4.4`, `raster-hqh`, `raster-jv6`, `raster-m6z`, `raster-tf5.6`,
`raster-yq9`.

## `build-workload --classify` cannot run in this repository — ninth sighting

**What happened.** The skill's pre-gate classification step failed: `build-workload: no rust_paths in
.cerebro/project.conf - cannot classify safely`.

**Why.** `.cerebro/project.conf` declares no `rust_paths`, and the script refuses to guess.

**Cost.** A minute. The plan named the `rust` workload anyway, and the full gate was run.

**Prevent by.** Either `.cerebro/project.conf` gains `rust_paths`, or `implement-bead` makes the step
conditional on the project declaring them. Eight earlier files ask the same.

**Seen before.** `raster-1t9`, `raster-6jr`, `raster-fu0`, `raster-jv6`, `raster-m6z`,
`raster-tf5.5`, `raster-tf5.6`, `raster-tf5.7`.

## The review sub-agent spawns asynchronously, not synchronously as the skill states — sixth sighting

**What happened.** `implement-bead` says "The spawn is synchronous: wait for it inside the tool call".
The `Agent` tool returned immediately with an agent id and a note that it runs in the background; the
result arrived as a task notification about seven minutes later.

**Why.** The tool's contract, not the skill's. Handled by blocking in a `Bash` loop that heartbeat the
bead every thirty seconds — which is what the skill's *Waiting, without ending your run* prescribes
anyway, so the outcome was correct.

**Cost.** Nothing this run, but the wrong reading of that sentence is to end the turn against the
spawn, which is the failure the skill exists to prevent.

**Prevent by.** `implement-bead`'s *Getting the review* should say the spawn returns immediately and
that the implementer blocks on the notification, rather than calling the spawn synchronous.

**Seen before.** `raster-1t9`, `raster-fu0`, `raster-hqh`, `raster-jv6`, `raster-tf5.7`.
