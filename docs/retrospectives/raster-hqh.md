# raster-hqh — retrospective

- **Implementer:** Rogue
- **Date:** 2026-09-01
- **PR:** #45

## The plan's claims about the existing test suite were wrong in both directions

**What happened.** The plan's *Test plan* has a section headed "Existing tests that must stay green,
and are the reason to run the whole suite", naming three. Two of the tests that actually moved were
not in it, and one claim the plan did make about its own new code was false.

Not named, and both went red on the first full run:
`a_compound_assignment_to_anything_that_reads_is_untouched`
(`crates/raster-ir/tests/lower.rs`) used `ppu.data += 1` as its example of a compound assignment to a
register that reads — the exact construct this bead refuses; and `a_register_that_reads_still_compiles`
(`crates/rasterc/tests/cli.rs`) asserted `stderr == ""` for a *lone* `ppu.data` read — the exact
shape this bead warns on. Both are raster-1t9's, merged four days earlier.

False in the other direction: the plan states, of the save/restore of `ppu_data_read_has_neighbour`,
"The block test is what goes red without the restore." It does not. The review sub-agent replaced
`= outer` with `= false` and all 60 `raster-ir` tests stayed green; by inspection nothing can reach
it today, because no `lower_statement` arm lowers a nested block and then lowers an expression of its
own.

**Why.** Established for the first half: raster-1t9 introduced both fixtures, and this bead is the
one that changes what `ppu.data` means, so any test spelling `ppu.data` was a candidate the plan
could have enumerated with one grep. The plan cited raster-1t9's branch line by line for *source*
symbols and did not do the same sweep over its *tests*. Not established for the second half — the
plan reasoned about the restore rather than running the mutation, which is the same class as
`raster-m6z`'s "confidently wrong about two paths it had reasoned about but not run".

**Cost.** Small, about 15 minutes: two fixtures updated, and one comment rewritten after the review
caught the false claim. Recorded because both halves failed safe here only by luck — a red test in
the first case, an alert reviewer in the second. A plan claim that "test X pins line Y" is exactly
the kind of thing an implementer takes on trust and does not re-check.

**Prevent by.** Two concrete additions to `plan-bead`'s *Test plan* section. First: the list of
existing tests a bead moves should be built by grepping the suite for every construct the bead
changes the meaning of — here `grep -rn 'ppu\.data' --include='*.rs'`, which finds both missed tests
in one command — rather than from the tests the planner happened to read. Second: a plan must not
assert that a named existing test pins a named new line unless it has run the mutation; where it has
not, say "unpinned" so the implementer writes the test instead of believing the claim.

**Seen before.** `raster-1t9` — a sibling's test using the exact construct a bead refuses, same
family, though there the sibling merged mid-PR and the plan could not have known; here both tests
predated the plan. `raster-m6z` — a plan confidently wrong about a path it reasoned about but did not
run.

## `model-for`'s prescribed invocation misfires and exits 2 — eleventh sighting

**What happened.** The skill's line, run verbatim as one compound command:

```bash
provider="$(.claude/cerebro/scripts/agent-cli)" || provider=""
.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer
```

exited 2 with the usage line. Run on its own, `model-for --provider claude --role reviewer` exits 0
and prints nothing — a clean miss, meaning the reviewer runs on the CLI's default.

**Why.** Not established beyond what the ten earlier files say. The unquoted `${var:+...}` expansion
behaves differently in the compound form than when the flags are written out.

**Cost.** Two minutes. It is on the list only because it is now the eleventh.

**Prevent by.** Nothing new to add — ten files have already said it. This entry moves the count,
which is the argument for changing the skill's prescribed line rather than tolerating it.

**Seen before.** `raster-6jr`, `raster-fl4.2`, `raster-fl4.3`, `raster-fl4.4`, `raster-fu0`,
`raster-tf5.6`, `raster-7nl`, `raster-jv6`, `raster-yq9`, `raster-m6z`, `raster-1t9`.

## The review sub-agent spawns asynchronously, not synchronously as the skill states — fifth sighting

**What happened.** `implement-bead` says "The spawn is synchronous: wait for it inside the tool call".
The `Agent` call returned immediately with a background task id, and the review arrived as a
notification about eight and a half minutes later. I blocked in a heartbeat loop until it did, which
worked, but it is not what the skill describes.

**Why.** Established by the four earlier files: the tool is asynchronous by construction.

**Cost.** None this run — the heartbeat loop covered it, and I used the wait to watch CI on the same
head. It matters because an implementer that takes the skill at its word and does nothing after the
spawn ends its turn against a running review.

**Prevent by.** Unchanged from the four earlier entries: the skill's *Getting the review* section
should say the spawn returns immediately and prescribe the blocking heartbeat loop from *Waiting,
without ending your run* explicitly, the same way it does for CI.

**Seen before.** `raster-fu0`, `raster-tf5.7`, `raster-jv6`, `raster-1t9`.
