# raster-fl4.5.2 — retrospective

- **Implementer:** Storm
- **Date:** 2026-09-07
- **PR:** #61

## The plan asserted a property of an existing type, and the property was the opposite of true

**What happened.** The plan's *User-facing decisions* says a member's type is `ValueType::Unknown`
because "nothing in this release may add one to a `u8`, and `Unknown` is the language's existing type
for 'do not build on this', which already suppresses cascading errors". I built on that and wrote it
into a comment. `require_integer` (`crates/raster-sema/src/lib.rs:1031-1034`) lists
`ValueType::Unknown` explicitly among the types it *accepts*, and `ensure_compatible` short-circuits
on it, so `Unknown` permits arithmetic rather than forbidding it. `main { var x: u8 = 1
x = picture.tiles + 1 }` analyzes clean, and lowering then prints `this byte register is not
supported` under `picture.tiles` — an author told their picture is a byte register.

**Why.** The skill's rule is "a helper the plan cites for what it decides is read before it is built
on", and I read it as being about *named* symbols. The plan named no helper: it asserted a property
of a type. That reads like background rather than like a claim to check, which is exactly why it went
unchecked — the sentence is about `ValueType::Unknown`, and the fact that decides it lives in two
functions the plan never mentions.

**Cost.** Small in time — the review sub-agent caught it as its finding 6 and one commit fixed the
comment — but the defect it described is still in the shipped behaviour, deferred to
`raster-fl4.5.3` with a comment naming it. Had the review not caught it, the false comment would have
been the next implementer's starting point for exactly the typing decision it was wrong about.

**Prevent by.** Widening the skill's *When the plan is wrong* rule from "a helper the plan cites" to
include a plan sentence asserting what an existing **type or value** does — `X already suppresses`,
`Y is rejected by`, `Z is not allowed`. The check is the same one: open what implements it before the
increment that depends on it. As written, the rule's trigger is a symbol name, and a claim about a
type carries none.

**Seen before.** `raster-hqh` (the plan's claims about the existing suite were wrong in both
directions), `raster-hkg` (the plan named a test that did not exist, pinning an ordering that did
not either), `raster-xeo` (a plan written against an unmerged branch cited three things that had
moved). This is at least the fourth sighting of a plan stating something about existing code that
was not true, and the first where the claim was about a type's behaviour rather than about a named
symbol or test.

## A review sub-agent stalled for ten minutes and reported nothing

**What happened.** The second delta round's `reviewer` sub-agent returned
`failed — Agent stalled: no progress for 600s (stream watchdog did not recover)`, with a result
containing one sentence of preamble and no findings. The delta under review was three files,
+38/−7, entirely comments, a rename and one visibility keyword. A retry on the same head, with the
prompt told the delta's size and told not to re-verify earlier rounds exhaustively, completed in
about six minutes.

**Why.** Not established. The two rounds before it took roughly eight and nine minutes and both
completed, and the stalled one was reviewing by far the smallest delta of the three — so "the delta
was too big" is not the explanation. The retry prompt differed in scoping the work, but one success
against one failure is not evidence that the scoping is what fixed it.

**Cost.** About ten minutes of wall clock, one of the three review attempts this head is allowed,
and the retry itself — roughly sixteen minutes for a round that produced one non-blocking finding.

**Prevent by.** Nothing to prevent yet; the retry path in `implement-bead`'s *The review loop* worked
as written and the budget absorbed it. What is worth having is the count: if a later bead records a
second stall, that is two sightings of a supplier failure with no diagnosis, and the retry budget
stops being an adequate answer. Recorded so the second one is recognisable.

**Seen before.** None found — `grep -rl "stalled\|no progress for 600s\|watchdog"
docs/retrospectives/` matches nothing else.
