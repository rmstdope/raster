# raster-6sl.5 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-08-31
- **PR:** #16

## A safety rule the plan specified did not prevent the thing it named

**What happened.** The plan's *User-facing decisions* §5 added a rule under the heading
"**Decided by me, not asked**": if the resolved output path equals the input path, refuse before
compiling, "because `rasterc demo.nes` ... destroys the author's source; it is a safety rule, not a
preference". It specified the test as *byte-identical* path strings. I implemented exactly that.
`rasterc src.raster -o ./src.raster` then exited 0, printed the success summary, and left
`src.raster` as a 32784-byte iNES image. So did `-o SRC.raster` on macOS's case-insensitive default
filesystem. The review caught it; I did not.

**Why.** Established. I read the plan's *mechanism* ("byte-identical") as the requirement and did not
re-derive it from the plan's own stated *purpose*, one sentence earlier, in a paragraph that says
outright it is a safety rule. A string comparison cannot decide file identity, and the failure is
reachable by a typo.

**Cost.** No wall-clock at the time — it was written and merged straight past me — and one review
round plus one CI cycle to fix. The real cost was avoided rather than paid: had the review not run,
a released `rasterc` would silently destroy source files.

**Prevent by.** `implement-bead`'s *When the plan is wrong* already says a helper the plan cites for
what it decides must be read before it is built on, because a plan can be confidently wrong about
what a symbol accepts. The same care is not stated for a plan's own *predicate*. Suggest extending
that section: where a plan states a rule's purpose and then specifies the test for it, check the test
against the purpose before implementing, and treat a gap as a detail to decide rather than a
specification to follow. Concretely here: file identity is `fs::canonicalize` plus, on unix,
`dev`/`ino` — never a string comparison.

**Seen before.** None found.

## The plan reported a check it had run, and the check was wrong

**What happened.** The plan's `raster-ir` section says of the thirteen reworded refusal messages:
"No test asserts any of these strings — checked: `crates/raster-ir/tests/lower.rs` and
`crates/raster-sema/tests/analyze.rs` contain no match for `not supported`. They count errors and
match on structure." `lower.rs::rejects_all_accepted_forms_not_supported_by_initial_codegen` matches
those messages by *keyword substring* — `"assembly"`, `"indexing"`, `"timing"` — and the agreed new
wording drops all three (`` `asm` ``, `arrays`, `` `cycles` ``).

**Why.** Established. The plan grepped for the phrase `not supported`; the test greps for single
words out of the middle of each message. A grep for the wrong string returns a confident negative.

**Cost.** Around fifteen minutes: one probe run to find valid `frame`/`cycles`/`wait` syntax, then
reading the fixture to work out which expectations moved.

**Prevent by.** Cheap and mechanical: before changing a user-visible string, grep for a distinctive
*word* of the old string across the test tree, not for the whole phrase — `grep -rn "assembly"
crates/*/tests/` would have found this. Worth a line in `plan-bead` where it tells a planner to
verify claims about existing tests, and in `implement-bead`'s *When the plan is wrong* as a claim to
re-check rather than inherit.

**Seen before.** None found.

## The plan's data structure could not express the output the plan agreed

**What happened.** The plan specified `raster_diag::Diagnostic` with a required `pub span: Span`,
adding only a `notes` field. Its own *User-facing decisions* §8 then gives four failure texts that
render as a header and notes with **no location line** — "this program has no `main` block", "the
program does not fit the MMC3 fixed bank", "a branch is too far for the 6502". A required `Span`
cannot produce that: every diagnostic draws a `-->` line and a caret.

**Why.** Established. The two sections were written against each other without one being run.
Resolving it needed `docs/ui/raster-6sl.5-rasterc-cli.html`, which the plan describes as "merged from
PR #15" — PR #15 is still open, so the page is not on main and had to be read with `gh pr diff 15`.

**Cost.** About twenty minutes, and one extra increment (`Diagnostic::span` became `Option<Span>`
with a `without_span` constructor and a render branch).

**Prevent by.** Where a plan specifies both a data structure and the exact user-visible text it must
produce, the planner should render one example of each text through the proposed structure before
filing. And a plan should not describe a docs PR as merged without checking: `gh pr view <n> --json
state` is one command, and "merged from PR #15" sent me looking for a file that was not there.

**Seen before.** None found.
