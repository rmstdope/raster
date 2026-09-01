# raster-hkg — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-09-01
- **PR:** #57

## The plan named a test that pinned the ordering it promised, and neither the test nor the ordering existed

**What happened.** The plan's *Order in the output* says "the handler ones follow schedule order",
the agreed page `docs/ui/raster-hkg-nmi-timed-frame.html` §7 says the same, and the plan adds
"There is a test that pins the order as described". Its *test plan* table lists eleven tests and
none of them is that test — the one nearest it,
`an_nmi_warning_prints_after_a_bank_warning_whatever_the_line_numbers`, pins only bank-vs-NMI. The
implementation quoted verbatim in the plan does not produce schedule order either: handler bodies
are lowered in source order by `lower_frame`, which then does `events.sort_by_key(|e| e.scanline)`,
and nothing re-orders the writes collected during lowering. Built exactly as written, a frame whose
`at scanline 200` handler is declared before its `at scanline 20` one warns about 200 first. The
review sub-agent found it, with a reproduction.

**Why.** Established. The plan's *Order in the output* section reasons about the frame-level warning
being appended after lowering's own warnings — which is true and is what the listed test pins — and
then states the handler half as if it followed from the same fact. It does not: appending preserves
collection order, and collection order is source order because the sort that makes schedule order
happens later, in a different function. A sentence about ordering that no test covers is a sentence
nothing checks.

**Cost.** One review round to find it and about twenty minutes to fix, plus one CI cycle. It did not
cost a bad merge, because the review caught it — but it would have shipped behaviour contradicting a
merged, word-for-word-agreed artefact.

**Prevent by.** `implement-bead`'s *When the plan is wrong* already says a helper the plan cites for
what it decides is read before it is built on. The same care is worth extending, in that section, to
a plan sentence that asserts *a test exists* for a property: check the plan's own test table for it
before believing the property is covered, and where the table has no such test, write one. That is
the cheap half. The expensive half is that this property was only checkable by running the compiler
on a two-handler program, and the plan's fixtures were all single-handler — so a plan claiming an
ordering should be read as owing a fixture where the two candidate orders actually disagree, the way
this plan's own bank-vs-NMI fixture was deliberately built to.

**Seen before.** raster-xeo — different cause (a plan written against an unmerged branch), same
shape: a confident plan sentence that the code did not support, caught downstream rather than at
planning.
