# raster-rid — retrospective

- **Implementer:** Storm
- **Date:** 2026-09-01
- **PR:** #41

## A site the plan wired had no row in the plan's test plan, and shipped unguarded

**What happened.** The plan's *Where each site is wired* table has ten rows, one of which is the
`SyntaxStatement::Loop` arm ("Treat it exactly as `lower_while`"). Its *The test plan* table has
twenty-one rows, with a `while` row, a `for` row and no `loop` row. Building the plan exactly as
written therefore produced three correct lines in `crates/raster-ir/src/lib.rs` that no test
exercised: every `loop` fixture in the repository has an empty body — five of them, across
`lower.rs`, `cli.rs` and `compile.rs` — and on an empty body the pre-scan is `false` and the join
takes its `self == other` branch, so the entry selection survives whether the arm is there or not.
The full gate was green on it, twice. The review sub-agent found it by deleting the three lines and
observing that all 39 other tests still passed; I reproduced that before fixing it.

**Why.** Established. The plan derives its increments from *Where each site is wired* and its tests
from *The test plan*, and nothing checks the two against each other. The `loop` row is the one site
whose fixtures are all degenerate, so it is exactly the row where the missing test does not announce
itself.

**Cost.** Small, because the review caught it: one test, one commit that was already being made to
answer other findings, no extra CI cycle. The cost worth naming is the counterfactual — had the
review not probed by deletion, three lines of live analysis would have merged with no guard, and the
next change to that arm would have regressed to silence with a green gate.

**Prevent by.** Extending the cross-check this plan already carries in its *Known traps* — "every
string in *Every string, verbatim* should map to an arm of `bank_data_warning` and every table row in
*Where each site is wired* to a line number" — by one axis: **every row in *Where each site is wired*
should also map to a row in *The test plan*.** Concretely, in `plan-bead`'s guidance for a plan whose
change is spread across several call sites, say that the wiring table and the test table are checked
against each other before the plan is filed, and that a site with no test row is either given one or
explicitly written down as unobservable. The check is mechanical and takes a minute; this run is what
it would have caught.

**Seen before.** `docs/retrospectives/raster-m6z.md` — a different failure of the same shape, where a
plan's *The test plan* asserted something its own decisions contradicted, and the two sections had
not been read against each other. `docs/retrospectives/raster-6jr.md` is what put the two-axis version
of this check into raster-rid's plan in the first place.
