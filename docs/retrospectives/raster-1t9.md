# raster-1t9 — retrospective

- **Implementer:** Rogue
- **Date:** 2026-09-01
- **PR:** #43

## A rebase auto-merged a sibling's line inside a guard I had just added, with no conflict

**What happened.** `raster-rid` merged while PR #43 was under review, and it had added two lines to
the same `if` block in `lower_expression_statement` that this bead had just wrapped in a new
condition. My change was:

```rust
if destination == Destination::Register(Register::Mmc3BankSelect)
    && self.errors.len() == errors_before          // new: Q7, suppress the warning
{
    if let Some(warning) = bank_select_warning(...) { ... }
}
```

and `raster-rid` had appended `self.selection = match value { ... }` to that same block. `git rebase
origin/main` merged the two **cleanly, with no conflict marker**, and put the selection tracking
*inside* my error guard — so a bank select whose value was refused stopped updating the selection at
all, and the following `mmc3.bank_data` write then warned against a stale selection ("nothing selects
a register before this" when something plainly did). Nothing was red: `raster-rid`'s own tests all
passed, because none of them pairs a refused read with a bank select.

Moving the line back out was not the fix either — a refused read lowers to a `Value::Constant(0)`
placeholder, so read at face value it says R0 is selected, and the bank-data warning disappears
instead of being wrong. The truthful third answer is `Unknown(InThisBody)`.

**Why.** Established. Both changes were textually additive to the same block, which is exactly the
shape git resolves without asking. The conflict was semantic — a guard's *scope* silently widened
over a sibling's statement — and a textual merge has nothing to detect there.

**Cost.** About 25 minutes, and it was caught only because one of `raster-rid`'s tests happened to go
red for an unrelated reason (below), which made me read the merged block. Had that test not existed,
this would have merged.

**Prevent by.** `implement-bead`'s *Merging* section should say that after a rebase or an
`update-branch` that touched a file the bead also changed, `git diff <pre-rebase-sha>..HEAD --
<that file>` is read before the gate — not just "resolve conflicts". A clean rebase of two additive
changes to one block is the case with no marker to notice, so re-running the gate is not enough:
the gate was green.

**Seen before.** `raster-c35` and `raster-m6z` both record a sibling merging under an open PR, but
both are about a *conflict* the rebase raised. Neither is about a clean auto-merge that changed
behaviour; that is new here.

## A sibling's test used the exact source this bead refuses, and the plan could not have known

**What happened.** The plan surveyed the corpus before it was written and named the one existing test
this bead would move (`a_compound_bank_select_cannot_be_folded_and_warns`), and that survey was
correct at the time. `raster-rid`, merged during the review, added
`a_compound_bank_select_leaves_the_selection_unknown`, which uses `mmc3.bank_select += 1` as its way
of producing a selection rasterc cannot fold — the one construct this bead refuses. It failed on the
first build after the rebase.

**Why.** Established, and not avoidable by either side: `+=` on a register was legal when
`raster-rid` was written, and `raster-rid` did not exist when this plan surveyed. The plan's list of
tests it would move is a fact about a commit, not a standing one.

**Cost.** Small — one test rewritten onto `mmc3.bank_select = 6 | 1`, which is the same unfoldable
selection, about 10 minutes. Recorded because the cost is only small when the failure is a red test;
had the sibling's fixture merely *compiled* differently rather than failing, it would have shipped.

**Prevent by.** A plan section that names "the existing tests this bead moves" should say it is true
as of the sha the plan was written against, and `implement-bead` should re-run the search after a
rebase rather than trusting the plan's list. Concretely, for this bead the search was
`grep -rn '+=' --include='*.rs' --include='*.raster'`, and re-running it after the rebase is what
would have found this without waiting for the suite.

**Seen before.** `raster-c35` — same family (a sibling invalidating a plan premise mid-PR), different
mechanism.

## `model-for`'s prescribed invocation misfires and exits 2 — tenth sighting

**What happened.** The skill's line, run verbatim as one compound command:

```bash
provider="$(.claude/cerebro/scripts/agent-cli)" || provider=""
.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer
```

exited 2 with the usage line. Run on its own, `model-for --provider claude --role reviewer` exits 0
and prints nothing — a clean miss, meaning the reviewer runs on the CLI's default.

**Why.** Not established beyond what the nine earlier files already say. The unquoted
`${var:+...}` expansion behaves differently in the compound form than when the flags are written out.

**Cost.** Two minutes. It is on the list only because it is now the tenth.

**Prevent by.** Nothing new to add — nine files have already said it. This entry exists to move the
count, which is the argument for changing the skill's prescribed line rather than tolerating it.

**Seen before.** `raster-6jr`, `raster-fl4.2`, `raster-fl4.3`, `raster-fu0`, `raster-tf5.6`,
`raster-7nl`, `raster-jv6`, `raster-yq9`, `raster-m6z`.

## `build-workload --classify` cannot run in this repository — eighth sighting

**What happened.** `build-workload --classify` exits 1 with `no rust_paths in .cerebro/project.conf -
cannot classify safely`, as the seven earlier files record. Fell back to `--workload rust`, which the
plan had already specified.

**Why.** Established and unchanged: `.cerebro/project.conf` declares no `rust_paths`.

**Cost.** A minute.

**Prevent by.** Unchanged from the seven earlier entries: either declare `rust_paths` in
`.cerebro/project.conf`, or have `implement-bead` stop prescribing the classify step for a project
that cannot answer it. Recorded to move the count.

**Seen before.** `raster-6jr`, `raster-fu0`, `raster-tf5.5`, `raster-tf5.6`, `raster-tf5.7`,
`raster-jv6`, `raster-m6z`.

## The review sub-agent spawns asynchronously, not synchronously as the skill states — fourth sighting

**What happened.** `implement-bead` says "The spawn is synchronous: wait for it inside the tool call".
The `Agent` call returned immediately with a background task id, and the review arrived as a
notification about five minutes later. I blocked in a heartbeat loop until it did, which worked, but
it is not what the skill describes.

**Why.** Established by the three earlier files: the tool is asynchronous by construction.

**Cost.** None this run — the heartbeat loop covered it. It matters because an implementer that takes
the skill at its word and does nothing after the spawn ends its turn against a running review.

**Prevent by.** The skill's *Getting the review* section should say the spawn returns immediately and
prescribe the blocking heartbeat loop from *Waiting, without ending your run* explicitly, the same
way it does for CI.

**Seen before.** `raster-fu0`, `raster-tf5.7`, `raster-jv6`.

## I committed and pushed this file to `main` directly, because a `cd` had not stuck

**What happened.** This retrospective's first version was written, committed and pushed **straight to
`main`**, unreviewed and without CI. `git push --force-with-lease` answered `4b3ec5d..fd1e9d9
main -> main`, which is how I found out.

The cause was mundane: `implement-bead` prescribes writing the state file with
`.claude/cerebro/scripts/agent-state ...`, a path relative to the repository root, so I ran
`cd /Users/henrikku/repos/raster && .claude/cerebro/scripts/agent-state Rogue ... --phase merge`.
The shell keeps its directory between calls, so every later command — the heredoc that wrote the
file, `git add -A`, `git commit`, `git push` — ran in the root checkout, on `main`, not in the
worktree.

Nothing was lost and no code reached `main`: the commit is this docs file alone, whose content was
correct and whose destination was always `main`. The branch's own work was still uncommitted in the
worktree and was committed there afterwards. `main`'s history was left alone rather than rewritten or
reverted — force-pushing a shared `main` other agents have fetched, or adding a second unreviewed
commit to undo the first, are both worse than the thing they would fix. It is disclosed in PR #43 and
in the closing message instead.

**Why.** Established. `implement-bead`'s *Telling the fleet view what you are doing* gives every
`agent-state` call as a root-relative path, and its state-file table places one of those calls
(`--phase merge`) immediately before the retrospective section — so following both in order puts a
`cd` to the repository root directly in front of the one part of the run that writes and commits a
tracked file. The skill's own *Workspace* section warns "Check `pwd` before any git command … one
`cd` into another agent's worktree leaves every later command there", which is this hazard named for
a different directory; I had read it, and still walked into it from the other side.

**Cost.** About fifteen minutes to diagnose and re-establish where each commit actually was, one
unreviewed commit permanently on `main`, and a disclosure that should not have been necessary. It
came within one command of being much worse: the same stray directory had already made me run
`git rebase origin/main` on `main` earlier in this run, which was harmless only because `main` was
up to date at that moment.

**Prevent by.** Two concrete changes, both in `implement-bead`. First, the state-file table should
give the call as `.cerebro/worktrees/<id>`-safe — either an absolute path
(`<repo>/.claude/cerebro/scripts/agent-state`) or a `git -C <repo>`-style invocation — so writing
state never requires leaving the worktree. Second, the *Retrospective* and *Merging* sections should
open with the same `pwd` check *Workspace* already prescribes, since both write to a tracked file and
both sit downstream of a state write. A guard hook that refuses a commit on `main` in this repository
would catch the whole class, and is the navigator's to decide.

**Seen before.** None found — `docs/retrospectives/` has no entry about a `cd` outliving the command
that needed it, and none about a commit reaching `main` outside a PR.
