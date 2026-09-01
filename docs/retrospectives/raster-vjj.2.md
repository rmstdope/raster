# raster-vjj.2 — retrospective

- **Implementer:** Storm
- **Date:** 2026-09-01
- **PR:** #58

## A new public function landed under an orphaned doc block and was documented as the thing it is not

**What happened.** The plan asked for `pub const fn mask_enables_rendering` to be added "next to
`validate_mmc3_irq_frame`", and warned that its line numbers had moved and each item must be found
by name. I did find by name — I anchored the insertion on `fn known`'s own doc comment,
`/// One register's value, or the reason it does not have one this can check.`, which is immediately
above `validate_mmc3_irq_frame` in `crates/raster-timing/src/lib.rs`. The insertion was correct by
that anchor and wrong in effect: the twenty-line block beginning
`/// Check the hardware preconditions an MMC3 IRQ chain depends on.` was **already orphaned on main**,
sitting above `fn known` rather than attached to the function it describes. Inserting before that
anchor put my new item between the orphaned block and its text, so rustdoc rendered twenty lines
about 8x16 sprites and the CHR layout as `mask_enables_rendering`'s own documentation — followed by
its real doc saying "This is the same test [`validate_mmc3_irq_frame`] applies…" underneath prose
asserting it *was* that check. A brand-new public API, documented as the thing it explicitly is not.
The full gate is silent on this: `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test`
all passed. The review sub-agent caught it as its first finding.

**Why.** Established. A `///` block is attached to whatever item follows it, so inserting an item
between an orphaned block and the code below it silently reassigns that block to the new item. The
pre-existing orphaning made the region look like `fn known` had a normal doc comment above it, and
nothing in reading the anchor line reveals what sits above it.

**Cost.** One review finding and about five minutes to diagnose and fix. No extra CI cycle: the fix
travelled in the same commit as the other four review answers, and the second CI run this bead paid
for was forced by main moving under it (#57 merged mid-review), not by this.

**Prevent by.** After inserting a new item into a Rust file, read the ten lines *above* the insertion
point, not just the anchor line — `sed -n '<start>,<end>p'` over the region — and confirm the `///`
block now immediately above the new item is the new item's own. The check costs one command and is
the only thing that catches it, because rustfmt, clippy and the test suite all pass on a doc comment
attached to the wrong function. Worth adding to `implement-bead`'s guidance beside the existing
"find each item by name", which this run followed and which was not sufficient on its own.

**Seen before.** None found — `grep -rln "orphan\|doc block\|rustdoc" docs/retrospectives/` matched
nothing before this file.

## `model-for`'s prescribed invocation misfires and exits 2 — thirteenth sighting

**What happened.** `implement-bead`'s snippet, run verbatim —
`.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer` — exited 2
with the usage line, `provider` correctly set to `claude`. Run split by hand,
`model-for --provider claude --role reviewer` exits 0 (a miss: this checkout has no
`.cerebro/models.conf`, so the sub-agent runs on the CLI default).

**Why.** As established by the twelve sightings before it: the unquoted `${var:+...}` alternate value
is word-split without quote removal, so the literal double quotes reach the script as part of the
argument.

**Cost.** Two minutes and one wasted call, as every previous sighting.

**Prevent by.** Unchanged from the earlier files: fix the snippet in
`.claude/cerebro/skills/implement-bead/SKILL.md` to `--provider "${provider:-}"`, or have `model-for`
tolerate a quoted value. Recording only the count here, since the diagnosis is complete elsewhere.

**Seen before.** `raster-1t9`, `raster-6jr`, `raster-7nl`, `raster-yq9`, `raster-bm1`, `raster-m6z`,
`raster-fl4.2`, `raster-fl4.3`, `raster-fl4.4`, `raster-fu0`, `raster-tf5.6`, `raster-jv6`,
`raster-hqh`, `raster-vjj.1` — twelve numbered sightings, this being the thirteenth.

## `build-workload --classify` cannot run in this repository — eleventh sighting

**What happened.** `git diff --name-only -z origin/main...HEAD | xargs -0
.claude/cerebro/scripts/build-workload --classify`, which `implement-bead` requires immediately
before the fast gate, exited 1 with
`build-workload: no rust_paths in .cerebro/project.conf - cannot classify safely.`

**Why.** Established elsewhere: this project's `.cerebro/project.conf` declares no `rust_paths`, and
the script refuses to guess.

**Cost.** A minute. I fell back to the plan's declared `--workload rust`, which is right for a bead
touching only `crates/`.

**Prevent by.** Unchanged: either declare `rust_paths` in `.cerebro/project.conf`, or have
`implement-bead` say the step is skipped where the project declares none. Recording only the count.

**Seen before.** `raster-1t9`, `raster-6jr`, `raster-6y8`, `raster-bm1`, `raster-m6z`, `raster-fu0`,
`raster-tf5.5`, `raster-tf5.6`, `raster-tf5.7`, `raster-jv6`, `raster-vjj.1` — ten numbered
sightings, this being the eleventh.
