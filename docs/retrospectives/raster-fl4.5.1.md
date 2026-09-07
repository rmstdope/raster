# raster-fl4.5.1 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-09-07
- **PR:** #60

## Two doc comments I wrote made false claims about other code, and one of them carried a decision

**What happened.** Twice in one bead I wrote a doc comment asserting what another symbol does,
without opening that symbol, and both assertions were false. The review caught both.

1. The constants in `crates/raster-ir/src/lib.rs` got a comment saying NTSC is a value "the codebase
   never names at all". It does: `crates/rasterc/src/report.rs:19` is
   `row("mapper", "MMC3 (4), NTSC")` — a string the author reads in every build summary — and
   `crates/raster-timing/src/lib.rs:280,287` name NTSC for the dot and scanline counts.
2. `skip_to_close_brace` in `crates/raster-syntax/src/parser.rs` got a comment saying its
   brace-skipping "is the same recovery `opaque_block` has always had". It is not: `opaque_block`
   counts depth and reports ``expected `}` to close block`` when it ends unbalanced, and mine
   counted none. I had read `opaque_block` earlier in the same pass and still wrote this from memory.

The second is the one that mattered, because the false sentence was load-bearing: I used it to
*decline* a review finding. The reviewer had offered "either track depth or say what it really
does", and I chose to document rather than fix, on the grounds that depth-tracking would change how
every block recovers. Once the claim was checked that reason evaporated — counting depth brought
`target` into line with `opaque_block` rather than changing anything else — so the next round
reversed the decision and the fix went in. An unclosed `target nes { mapper:` had been swallowing
the following `main { }` entirely.

**Why.** Both comments were written at the moment of *explaining* a change rather than at the moment
of reading the code, and by then the relevant file was in memory rather than on screen. The pull
toward this is specific: a comment justifying a decision wants a comparison to a sibling, and the
sibling is exactly the thing least likely to be re-opened, because it feels already known.

**Cost.** One extra delta round, about ten minutes of review plus the answering, and a second round
of my own attention. Cheap only because the reviewer opened `opaque_block` when I did not — had it
merged, the false sentence would have sat three functions above the code it misdescribes, and it is
the sentence a later implementer would have acted on.

**Prevent by.** `implement-bead`'s *When the plan is wrong* already ends with: "a sentence there
about what a helper or a label does is read by the reviewer and the navigator with the trust a plan
gets, so run or read the thing before writing the sentence." That sentence is scoped to *the PR
body*. Extend it to doc comments in the shipped code, which outlive the PR body and are what the
next implementer reads instead of the source: before a comment names another symbol's behaviour,
open that symbol in the same minute. And name the sharper case, because it is the one with teeth —
a claim about other code that is being used to *justify declining a review finding* is the one to
re-read before posting, since the reply stands or falls on it.

**Seen before.** `raster-hqh` — a comment's false claim caught by the review, recorded as the same
class as `raster-m6z`'s "confidently wrong about two paths it had reasoned about but not run". This
is the third sighting of reasoning-instead-of-reading, and the first where the unchecked claim
changed a decision rather than only a description.
