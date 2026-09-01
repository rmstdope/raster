# raster-xeo — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-09-01
- **PR:** #46

## A plan written against an unmerged branch cited three things that had moved by the time it was built

**What happened.** `raster-xeo`'s plan was written while `raster-1t9` was still an open PR, and it
says so honestly. Three of its citations were nevertheless wrong or unreachable when I picked the
bead up, in three different ways:

1. **A verbatim replacement block that would have deleted merged code.** *Files to change* item 3
   says to replace the assignment branch of `lower_expression_statement` "from
   `let Some(destination) = self.lower_destination(left) else {` down to and including the
   `output.push(Statement::Assign { destination, value });`" with a quoted snippet. That snippet was
   written against `raster-1t9`'s pre-rebase branch. What actually merged (`b55fb92`) has machinery
   the snippet does not contain: the `errors_before` / `refused` count, the `self.selection` /
   `BankSelection` update, and the `bank_data_warning` call. Following the instruction literally
   deletes all three.
2. **A PR number that was not the one that merged.** The plan says `raster-1t9` "is open as PR #36"
   and to wait for #36. #36 is still open and stale; the bead actually merged as **#43**.
   `gh pr view 36` says `OPEN`, which reads as "your dependency has not landed" when it had.
3. **A fact that a later merge falsified.** *Increments* item 4 states "`= 0` is silent for every one
   of the sixteen", and I copied that sentence into a test comment. It is false:
   `mmc3.bank_data = 0` warns, because `raster-rid` merged as #41 *after* this plan was written. The
   review sub-agent caught it.

**Why.** Established for all three. A plan is a document with a timestamp, and this project plans a
bead in one session and implements it in another by design — so between the two, siblings merge,
branches rebase, and the PR a plan names may not be the PR that lands. Item 1 is the sharpest case:
prose citing a symbol survives a rebase, a quoted code block does not.

**Cost.** About fifteen minutes of reading, and one review round to catch item 3 — small, because
the skill's *When the plan is wrong* rule ("a helper the plan cites for what it decides is read
before it is built on") sent me to the real code before the first increment. The cost if that rule
had not been followed is two silently deleted warnings, both of which have merged tests, so CI would
have caught them — but only after a wasted cycle and a confusing red.

**Prevent by.** Two specific things, both in `plan-bead`:

- **Never quote a replacement code block spanning code the plan does not own**, when the base it is
  quoted from is unmerged. Name the insertion point by symbol and say what must be preserved, the
  way the same plan correctly does everywhere else ("Every citation of `raster-1t9`'s code below is
  by symbol name for exactly that reason" — it made the rule and then broke it for one block).
- **Cite a dependency by bead id, never by PR number.** `git log origin/main --grep "(<id>):" -F`
  answers "has it merged?" correctly regardless of which PR carried it; a PR number does not, and a
  stale open PR under the same branch name reads as the opposite of the truth.

A third, smaller: this plan also cites `docs/ui/raster-xeo-read-only-register-wording.html` as the
verbatim source for shipped spec prose, and that page is in **open PR #42** — so the spec text in
this PR was copied from `origin/raster-xeo-mockup` rather than from `main`. That worked, but a plan
whose *Validation* asks a human to open a file that is not on `main` should say which branch it is
on.

**Seen before.** `docs/retrospectives/raster-1t9.md`, second section — "A sibling's test used the
exact source this bead refuses, and the plan could not have known". Same root (a plan predating its
siblings' merges), different symptom: there the plan could not have known, here it could have, by
citing ids rather than PR numbers and symbols rather than blocks.
