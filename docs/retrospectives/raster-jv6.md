# raster-jv6 — retrospective

- **Implementer:** Storm
- **Date:** 2026-09-01
- **PR:** #38

## A plan that verified everything it prescribed still missed a stale figure one crate over

**What happened.** The plan for this bead was unusually well verified: it had compiled its own
prescribed code, run the whole suite against a probe tree, and measured every figure it quoted. Its
*Known traps* even named this exact rot — "the one figure quoted in prose is in `m5.rs`, not in a
test" — and gave the command that found it, `grep -rn "8386" crates/`. Increment 2 corrected that
figure and a new assertion pinned it.

A second stale figure survived, and the review sub-agent found it. `crates/rasterc/src/compile.rs`
carried a doc comment saying "1700 `ppu.mask = 1` statements compile to 8631 bytes, of which 445 are
over the bank" above a test that constructs the `LinkError` directly with those numbers. After the
fix that program compiles to 8635 / 449. Nothing went red — the test never runs the compiler, so it
stayed green while its stated justification was false — and the plan had listed
`crates/rasterc/src/compile.rs` under *Files to change* as an explicit **no change**.

**Why.** Established. The trap's grep searched for `8386`, the figure the planner already knew about,
rather than for the figures the change *moves*. 8631 and 445 are moved by this change and appear
nowhere in the plan's greps — even though the plan's own context table states the move
(`8631 bytes, 445 to go | 8635, 449 to go`), so the numbers were in hand. A grep for a known-bad
constant finds only what you already knew; the change's own before/after table is the list worth
grepping for.

**Cost.** One review finding, one extra commit, one CI cycle — about fifteen minutes. Cheap only
because the review caught it; uncaught it would have left a comment in the tree confidently stating
figures the compiler contradicts, of the same kind this bead exists to remove.

**Prevent by.** When a plan's *Known traps* names a quoted-figure risk, the grep it prescribes should
be over **every figure the change moves**, taken from the plan's own before/after table, not over the
one occurrence the planner happened to find. Concretely, for this bead:
`grep -rn -e 8631 -e 445 -e 8386 crates/`. Worth saying in `plan-bead`'s guidance on traps, since the
planner is the one holding the before/after table.

**Seen before.** None found — `raster-m6z` records a plan wrong about paths it had not run, and
`raster-tf5.5` a plan contradicting itself, but neither is a verified plan whose own sweep was too
narrow.

## `model-for`'s prescribed invocation misfires and exits 2 — eighth sighting

**What happened.** Exactly as recorded seven times before. Run verbatim from *The review — you get
exactly one*, with `provider` correctly set to `claude`, the line printed
`usage: model-for [--provider <p>] [--name <n>] [--role <r>]` and exited 2. Split by hand,
`model-for --provider claude --role reviewer` exits 0 and prints nothing — a genuine miss, this
repository having no `.cerebro/models.conf` — so the review ran on the CLI's default model.

**Why.** Established again, by expanding the arguments directly rather than by inference:

```
$ p=claude; set -- ${p:+--provider "$p"} --role reviewer
argc=3
[--provider claude]
[--role]
[reviewer]
```

The tool shell is zsh, which does not word-split an unquoted parameter expansion, so
`${provider:+--provider "$provider"}` arrives as the single argument `--provider claude` and
`model-for`'s `case` sends it to `usage`. Under bash it splits into two and works.

**Cost.** About two minutes, and it is the count that matters rather than the minutes.

**Prevent by.** The skill's line needs a form that survives both shells — the simplest being two
lines rather than one:
`args=(); [ -n "$provider" ] && args=(--provider "$provider")`, then
`model-for "${args[@]}" --role reviewer`. Eight implementers have now re-derived this; it is one line
in `implement-bead`.

**Seen before.** `raster-fl4.2`, `raster-fl4.3`, `raster-tf5.6`, `raster-m6z`, `raster-fu0`,
`raster-6jr`, `raster-7nl`.

## `build-workload --classify` cannot run in this repository — seventh sighting

**What happened.** The skill makes classification a required step immediately before the fast gate.
It printed `build-workload: no rust_paths in .cerebro/project.conf - cannot classify safely.` and
failed closed, as designed.

**Why.** `.cerebro/project.conf` declares no `rust_paths`, and the script will not guess. The plan
had already anticipated this and declared the workload `rust`, so nothing was blocked.

**Cost.** Seconds. Recorded only to keep the count honest.

**Prevent by.** Either add `rust_paths` to `.cerebro/project.conf`, or have `implement-bead` say that
a project declaring none makes this step a no-op whose failure is expected. Seven sightings of a step
that has never once produced an answer here is an argument for one or the other.

**Seen before.** `raster-tf5.5`, `raster-tf5.6`, `raster-tf5.7`, `raster-m6z`, `raster-fu0`,
`raster-6jr`.

## The review sub-agent spawns asynchronously, not synchronously as the skill states — third sighting

**What happened.** *Getting the review* says "The spawn is synchronous: wait for it inside the tool
call". The `Agent` tool returned immediately with an agent id and the note that it runs in the
background, and the result arrived five minutes later as a task notification.

**Why.** The tool's own contract — it launches in the background and notifies on completion. The
skill's sentence describes behaviour the tool does not have.

**Cost.** None here; the instruction to block was satisfied by a heartbeating `until`-style loop in
`Bash`, which is what the skill's *Waiting, without ending your run* prescribes anyway. The hazard is
an implementer that reads "synchronous", ends its turn expecting the result inline, and strands the
bead — which is precisely the failure that section exists to prevent.

**Prevent by.** `implement-bead`'s *Getting the review* should say the spawn returns immediately and
that the wait is a heartbeating `Bash` loop like every other wait in the skill, rather than calling it
synchronous.

**Seen before.** `raster-tf5.7`, `raster-fu0`.
