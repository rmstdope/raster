# raster-vjj.1 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-09-01
- **PR:** #54

## The plan's own measurement probe cannot measure anything, and reports success while failing

**What happened.** Increment 1 of this bead was a measurement: render `irq` handler bodies of known
cost and find the widest one whose stores all land before dot 0. The plan prescribed the narrowing
probe exactly — *"a second probe of `n` five-cycle fillers followed by one `ppu.mask = $5e` ... sweep
`n` over `0..=4` ... the largest probed body size that left every row one band"*.

Run as written it reports that **every** width fits. I swept `n` over `0..=3` — bodies of 6, 11, 16
and 21 cycles against a window that turned out to be 9 — and all four rows read as a single band. Had
I taken that reading, `IRQ_HANDLER_BODY_CYCLES` would have shipped at 21 or more: a compiler that
accepts torn pictures, which is precisely what this bead exists to prevent.

**Why.** Established, and it is not subtle once seen. The probe's handler stores one fixed value into
`ppu.mask`. On the first frame that changes the mask; from the second frame on, the chain wraps and
the handler stores the value the mask already holds. A store of the value already there paints
nothing, so the row is uniform whether the store landed inside the hblank or half-way along the line.
The measurement's success condition is satisfied by the store never being observable at all.

The coarse probe in the same increment does not have this defect, because its five stores carry five
*different* emphasis values, so each one changes the picture on every frame. The plan carried both
and only noticed the property in one of them.

**Cost.** About twenty minutes, and it would have been the whole bead had the coarse probe not
disagreed. What saved it was cross-checking with a third probe of a different construction —
invisible `ppu.ctrl = $08` fillers ahead of a ramp of visible stores — and finding that all three fit
one arithmetic model (`column = 3c - 27`) to the dot. Two probes agreeing is worth more than one
probe passing.

**Prevent by.** Two things, and the second is the general one.

A plan that prescribes a measurement should state **what makes the quantity observable**, not only
what to run — here, one sentence: *a handler storing the value the mask already holds paints nothing,
so every probe body must change the picture on every frame.* `plan-bead`'s *Increments* is where that
belongs, alongside the command.

And a measurement whose failure mode is "reports success" needs a **disagreement check** before its
number is used: a second probe of different construction, and a stated model both must fit. A single
probe that passes is indistinguishable from one that cannot fail. This bead's own emulator test has
the same shape and needed the same treatment — the review found it independently (finding 3), where
`row_bands`' eight-pixel tolerance let an 11-cycle body pass an assertion billed as pinning 9.

**Seen before.** None found. The three shipping traps in `.cerebro/traps.md` and the first section of
`docs/retrospectives/raster-fl4.4.md` are all about a probe reading the *wrong* value; this is about
one that cannot read a value at all and says so as a pass.

## `model-for`'s prescribed invocation misfires and exits 2 — twelfth sighting

**What happened.** The line `implement-bead` prescribes,

```
.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer
```

exited 2 with the usage line, `agent-cli` having printed `claude`. Run without the provider it exits
0 with no output, which is a clean miss: this repository has no `.cerebro/models.conf`, so no key can
match and the sub-agent runs on the CLI's default either way.

**Why.** Not established here beyond what the eleven earlier files record.

**Cost.** Two minutes.

**Prevent by.** Unchanged from the earlier entries. Recorded to move the count, which is now twelve.

**Seen before.** `raster-1t9`, `raster-6jr`, and the seven others those two name.

## `build-workload --classify` cannot run in this repository — ninth sighting

**What happened.** `build-workload --classify` exited 1 with `no rust_paths in .cerebro/project.conf
- cannot classify safely`. Fell back to `--workload rust`, which this bead's plan had already
specified and which the preflight had already been run with.

**Why.** Established in `raster-tf5.5` and unchanged: `.cerebro/project.conf` declares no
`rust_paths`.

**Cost.** A minute.

**Prevent by.** Unchanged: either declare `rust_paths` in `.cerebro/project.conf`, or have
`implement-bead` stop prescribing the classify step for a project that cannot answer it. Recorded to
move the count, which is now nine.

**Seen before.** `raster-1t9`, `raster-6jr`, `raster-fu0`, `raster-jv6`, `raster-m6z`,
`raster-tf5.5`, `raster-tf5.6`, `raster-tf5.7`.
