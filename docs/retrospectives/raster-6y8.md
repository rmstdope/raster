# raster-6y8 — retrospective

- **Implementer:** Wolverine
- **Date:** 2026-09-01
- **PR:** #55

## A `cd` to the repository root put `git stash pop` into the main checkout, on another agent's stash

**What happened.** To find out whether a `clippy --all-targets` lint was pre-existing, I ran one
compound command beginning `cd /Users/henrikku/repos/raster && git stash && cargo clippy … ; git
stash pop`. That path is the **shared main checkout**, not my worktree. `git stash` correctly
reported "No local changes to save" — so it stashed nothing — and `git stash pop` therefore popped
the entry that was already on the stack: `stash@{0}: WIP on (no branch): 35b7da4 feat(raster-3o3)`,
in-flight work belonging to another agent. It conflicted (`CONFLICT (content): Merge conflict in
crates/rasterc/tests/cli.rs`), which is the only reason I noticed. `git reset --hard HEAD` in main
restored it; the pop had kept the stash entry, so `git stash list` still showed it unchanged and
nothing was lost.

**Why.** Established, and it is two mistakes compounding. The `cd` is the one `raster-fl4.2`
recorded — moving to the repository root for a read-only-looking command and having the shell keep
it. The new half is that `git stash` and `git stash pop` are **not symmetric** when there is nothing
to stash: the first is a no-op that reports as such, the second is an unconditional pop of whatever
somebody else left. I wrote them as a matched pair and they were not one. `implement-bead` warns
against `cd`-ing into *another agent's worktree*; the main checkout is neither mine nor obviously
another agent's, and it is where every agent's stash stack lives.

**Cost.** About four minutes to notice, diagnose and restore, and the check it was meant to perform
was answered by the same run anyway. The exposure was much larger than the cost: had the pop applied
cleanly, raster-3o3's uncommitted work would have been silently merged into main's working tree and
removed from the stash, attributable to nobody.

**Prevent by.** Two specific changes. First, in `implement-bead`'s trap list, widen the `pwd` trap
as `raster-fl4.2` already asked, and add that **`git stash` is never the way to test a question
about main** — `git -C <path> stash list` is read-only, and "is this lint pre-existing?" is answered
by `git worktree add` a throwaway tree, or by reading the CI workflow, which is what actually
answered it here. Second, never write `cd <repo> && …` at all: every command in this run that
needed the main checkout could have used `git -C <repo>`, which cannot leak into the next call.

**Seen before.** `raster-fl4.2` — same `cd`-to-repository-root trigger, but the damaging command
there was a relative-path file write and the damage was a no-op. This is the first time the shared
checkout's *git state* was altered, and the first time another agent's work was at risk.

## The plan decided two user-facing behaviours whose interaction made its own increments unbuildable

**What happened.** The plan's Q3 warns on any `cycles(N)` holding an OAM DMA; its Q4 adds a second
note to the over-budget error when the block holds one. `cycles(114) pad { ppu.oam_dma = $02 }`
earns both, so rasterc printed `this block spends 113 or 114` immediately above `block costs 529
cycles, budget is 114`. The plan's own increment 5 quotes that stderr character for character with
no warning in it, so increments 5 and 6 as written could not both pass — implementing the second
turned the first red.

**Why.** Established. Both questions were settled with the navigator, in isolation, over three
rounds, and each answer is right on its own. Nothing in the plan or in the interview examined the
one input that reaches both, and `docs/ui/raster-6y8-oam-dma.html` renders each message in its own
section, which is exactly the shape that hides an overlap.

**Cost.** A navigator round-trip mid-build, and a mechanism the plan does not contain
(`LowerWarning::assumes_budget_met` plus its withdrawal in `compile.rs`) designed and reviewed
inside an implementation session. The review then found the withdrawal covered one of the three
timing errors that leave a block unpadded, and — writing the test for that — that it was withdrawing
*other* blocks' warnings too. Two defects that exist only because this decision was made here.

**Prevent by.** In `plan-bead`, where a plan settles more than one user-facing decision about the
same construct, require one increment or one paragraph naming **the input that triggers both** and
what it prints. Concretely for a diagnostics tool: a plan that adds a warning and an error to the
same syntactic form must quote the output of a program that earns both, or say that none can.

**Seen before.** `raster-jv6` names `raster-tf5.5` as a plan contradicting itself, but that was one
statement disagreeing with another. This is two individually-correct decisions with an unexamined
intersection — none found for that.

## `build-workload --classify` cannot run in this repository — tenth sighting

**What happened.** `git diff --cached --name-only -z | xargs -0 .claude/cerebro/scripts/build-workload
--classify` printed `build-workload: no rust_paths in .cerebro/project.conf - cannot classify
safely.` The step is mandatory in `implement-bead`'s *Building* section, immediately before the fast
gate.

**Why.** Established and unchanged since `raster-tf5.5`: `.cerebro/project.conf` declares no
`rust_paths`. The skill has the fallback — rerun the preflight with `--workload rust` and use the
private-target gate — and the plan had already declared this a `rust` workload, so nothing was
blocked.

**Cost.** Under a minute, as in the nine sightings before it.

**Seen before.** `raster-tf5.7`, `raster-m6z`, `raster-fu0`, `raster-6jr`, `raster-jv6`,
`raster-1t9`, `raster-bm1` and others — ten sightings now, with the same one-line fix outstanding
throughout: declare `rust_paths` in `.cerebro/project.conf`, or drop the step.
