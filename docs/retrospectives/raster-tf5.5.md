# raster-tf5.5 — retrospective

- **Implementer:** Wolverine
- **Date:** 2026-08-31
- **PR:** #19

## The plan's machine-checkable acceptance criterion contradicted the plan's own prescribed code

**What happened.** The plan's *Validation* section listed `grep -in "0x3f" crates/raster-emu/src/lib.rs`
finding nothing as acceptance, meaning "the crate never masks a colour out of an entry". Its *Files
to change* section separately prescribed the `as_indices` doc comment verbatim, and that comment
contains ``0x00..=0x3F`` twice. Following the plan makes the criterion fail; satisfying the criterion
means rewriting text the plan gave word for word. The review sub-agent raised the same thing as its
first finding, independently.
**Why.** A prose grep was used to check a code property. `0x3f` appears in a doc comment as
documentation of the colour space, not as a mask, and the grep cannot tell the two apart.
**Cost.** About ten minutes: a judgement call about which half of the plan governs, a deviation
paragraph in the PR body, and a review finding that had to be answered rather than fixed. No CI
cycles.
**Prevent by.** An acceptance grep should match the construct it forbids, not a substring that also
occurs in prose — here `grep -n '& *0x3[fF]'`, which is what the reviewer suggested and what actually
holds. A planner writing a *Validation* grep should run it against the code the same plan prescribes
before shipping the bead.
**Seen before.** none found.

## `build-workload --classify` cannot run in this project, and the skill makes it a required step

**What happened.** `implement-bead`'s *Building* section says to classify the changed paths
immediately before the fast gate:

    git diff --name-only -z origin/main...HEAD | xargs -0 .claude/cerebro/scripts/build-workload --classify

It exits 1 with `build-workload: no rust_paths in .cerebro/project.conf - cannot classify safely.`
The project's `.cerebro/project.conf` declares no `rust_paths`, so the step cannot succeed on any
bead in this repository, not just this one.
**Why.** Established: the script requires a `rust_paths` declaration that this project has never had.
**Cost.** A few minutes and one extra tool call to establish it was a project declaration gap rather
than something wrong with the branch. The skill's own fallback ("rerun the preflight with
`--workload rust`") covers it, so nothing was blocked.
**Prevent by.** Either declare `rust_paths` in `.cerebro/project.conf` — this is an all-Rust
workspace, so the declaration is cheap — or the skill's *Building* section should say that a project
with no `rust_paths` skips classification, the way its port block says a project with no `port_base`
has no problem to solve. Every implementer on this repository will otherwise hit the same exit 1.
**Seen before.** none found.
