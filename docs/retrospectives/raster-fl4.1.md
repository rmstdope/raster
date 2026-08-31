# raster-fl4.1 — retrospective

- **Implementer:** Storm
- **Date:** 2026-08-31
- **PR:** #17

## The plan's analysis surface could not cost the constructs the plan required of it

**What happened.** The plan specified the public surface of `raster-timing` as a literal code block,
with `TimedRegion { …, instructions: Vec<raster_6502::Instruction> }` — a flat list — and
`analyze(&TimedRegion, bool)`. A flat list can only be costed by summing it. But the plan's own
increments require costing `for` over a constant range (increment 4), padded conditionals
(increment 4), and `wait cycles` inside a region — and `raster-ir` lowers every one of those to a
loop or a branch that codegen emits. Summing charges a loop a single pass. Built as specified,
`cycles(76) { for i in 0..10 { total = total + 1 } }` compiled clean, asserted 76 cycles, and spent
697. The same held for `*`, `/`, `%`, `<<`, `>>` and `sync exact`, each mistimed by tens to hundreds
of cycles, silently, in the one guarantee the project exists to make.

**Why.** Established. The plan named the analysis surface without naming which constructs it can
cost, and nothing checked that list against what `raster-codegen` actually emits. I did not catch it
either: increments 1–3 pass their tests because a flat sum is exactly right for straight-line code,
and every fixture I reached for was straight-line. The full gate is green on the broken version.

**Cost.** A second round of work about the size of the first: eight findings to answer, two more
navigator questions, a redesign of the delay's loop form, a change to what a budget means, and every
fixture in the bead re-derived. Roughly two hours and one extra CI cycle. It was found only because
the navigator asked for a review when Copilot was unavailable — the review reproduced each case
against the built compiler rather than reading the diff.

**Prevent by.** When a plan specifies a cost model, an analysis surface, or any structure that must
account for generated code, its *Files to change* section should name the constructs the surface can
account for and the constructs it cannot, checked against what the lowering crate actually emits —
`grep 'internal_label\|self.branch\|self.jump' crates/raster-codegen/src/lib.rs` lists every place
codegen emits control flow, and each one is a construct a flat cost model gets wrong. Concretely
here: the plan should have said that a timed region is straight-line only, or specified the path
analysis, rather than leaving the gap for the implementer to discover from a review.

**Seen before.** raster-6sl.5 — "The plan's data structure could not express the output the plan
agreed", the same class of defect: a plan specifying both a structure and what it must produce,
without running one through the other.

## Increment 6 needed a bead that was in flight, and no dependency said so

**What happened.** `raster-fl4.1` declares one blocking dependency, on `raster-6sl.4`. Its
increment 6 is a CLI integration test, but when I claimed the bead `rasterc` had no compile pipeline
at all — it depended only on `raster-diag` and printed "compilation is not available yet". The
pipeline is `raster-6sl.5`, which Cyclops was building at that moment. I had to stop and ask the
navigator whether to build a second pipeline, test at the crate seam, or hand the bead back. It
merged mid-run, so the answer was to merge `main` into the branch — which conflicted in
`raster-ir`, and then required updating four of `raster-6sl.5`'s own fixtures whose "not supported
yet" expectations this bead had just made false.

**Why.** Established. The bead had already been handed back once, by an earlier session of mine, for
seams that did not exist; it was returned to the queue when `raster-6sl.4` landed, but the
dependency added covered only that bead and not the CLI pipeline increment 6 actually needs.

**Cost.** One blocked question to the navigator, one merge with two conflicts, four downstream
fixture updates, and roughly forty minutes.

**Prevent by.** When a hand-back names missing seams and the bead is later unblocked, the dependency
added should cover every increment, not the one that prompted the hand-back — reading each
increment's named test and asking which crate must exist for it to run. Here, increment 6's test
lives in `crates/rasterc/tests/`, so the bead needed `raster-6sl.5` as a second blocking dependency.

**Seen before.** none found.

## A multi-edit script that failed part-way discarded the edits that had succeeded

**What happened.** I applied several source edits per `python3` heredoc, each guarded by
`assert old in text`, writing the file once at the end. One assertion failed — `rustfmt` had
rewrapped the line I was matching — so the script raised and wrote nothing, silently losing five
edits I believed were applied. The next test run failed with the *old* diagnostic messages, which
read like a logic bug rather than an edit that never happened, and I spent a cycle looking in the
wrong place.

**Why.** Established. `p.write_text` ran after every replacement rather than after each one, so a
late failure discarded early successes.

**Cost.** About fifteen minutes and two confused test runs.

**Prevent by.** Write the file after each successful edit rather than once at the end, or apply the
edits as a list and report which ones matched — so a failure says exactly what did and did not
happen. Matching against `rustfmt`-formatted source is the specific hazard: run `cargo fmt` before
composing a match, or match on a shorter anchor that wrapping cannot move.

**Seen before.** none found.
