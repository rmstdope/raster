# raster-6jr — retrospective

- **Implementer:** Storm
- **Date:** 2026-08-31
- **PR:** #30

## `model-for`'s prescribed invocation misfires and exits 2 — sixth sighting

**What happened.** Run verbatim from *The review — you get exactly one*:

```bash
provider="$(.claude/cerebro/scripts/agent-cli)" || provider=""
.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer
```

printed `usage: model-for [--provider <p>] [--name <n>] [--role <r>]` and exited 2, with
`provider` correctly set to `claude`. Split by hand, `model-for --provider claude --role reviewer`
exits 0 and prints nothing — a genuine miss, since this repository has no `.cerebro/models.conf` —
and the review then ran on the CLI's default model, which is the documented answer to a miss.

**Why.** Not established beyond what `raster-tf5.6` recorded: the `${provider:+--provider "$provider"}`
expansion does not reach the script as two arguments the way the skill's line assumes.

**Cost.** About two minutes, and it is self-diagnosing — the usage line names the flags, so splitting
the call by hand is the obvious next move. The risk is not the time: an implementer who reads exit 2
as "no answer" rather than "the call was malformed" spawns its reviewer without ever having asked the
file, which on a project that *does* have a `models.conf` is silently the wrong model.

**Prevent by.** Replacing that line in `implement-bead`'s *The review* section with one that does not
depend on the conditional expansion — two branches, or an unconditional `--provider "$provider"` when
`agent-cli` always answers. Sixth sighting; the fix is the navigator's, and the step has never once
worked as written in this repository.

**Seen before.** `raster-tf5.6`, `raster-fl4.2`, `raster-fl4.3`, `raster-m6z`, `raster-fu0` — same
command, same exit 2, same cause.

## `build-workload --classify` cannot run in this repository — sixth sighting

**What happened.** `git diff --name-only -z origin/main...HEAD | xargs -0
.claude/cerebro/scripts/build-workload --classify` exited 1 with
`build-workload: no rust_paths in .cerebro/project.conf - cannot classify safely.`

**Why.** Established in `raster-tf5.5`: `.cerebro/project.conf` declares no `rust_paths`, and the
script refuses to guess rather than classify wrongly.

**Cost.** Under a minute. The skill documents the fallback ("or classification fails, rerun the
preflight with `--workload rust`"), and this bead's plan declared `--workload rust`, which I had
preflighted before starting.

**Prevent by.** Either declare `rust_paths` in `.cerebro/project.conf` or drop the step from
`implement-bead` for projects without it. Sixth sighting, and the step has never succeeded here.

**Seen before.** `raster-tf5.5`, `raster-tf5.6`, `raster-tf5.7`, `raster-m6z`, `raster-fu0` — same
script, same message, same cause.

## The plan's user-facing decision was wider than the code section that implemented it

**What happened.** The plan's *User-facing decisions* Q3 chose, with the navigator, that a failed
build prints its warnings and counts both severities. Its *Files to change* section wired that into
one of the three failure arms in `crates/rasterc/src/compile.rs` — the `lower` arm — and said nothing
about the codegen and link arms, which discard `ir.warnings`. I built what the files section
specified, and the gap was reachable: a source with `mmc3.bank_select = $80` and enough `var`s to
exhaust the zero page printed `error: could not compile <path> (1 error)` with no warning at all. The
review sub-agent found it as its first finding, with a reproduction.

**Why.** Established. The two sections of the plan disagreed, and I read the files section as the
specification and the decisions section as background. Nothing prompted me to check the second
against the first.

**Cost.** One extra commit, two extra tests and one CI cycle — about fifteen minutes. It cost nothing
worse only because the review caught it; unreviewed it would have shipped as a warning the author who
most needed it never saw.

**Prevent by.** `implement-bead`'s *When the plan is wrong* already says a helper the plan cites for
what it decides is read before it is built on. The same care is worth stating for the plan's own two
halves: before the first increment, read *User-facing decisions* against *Files to change* and check
every decision has a named site. A decision with no site in the files section is the plan being
incomplete rather than the decision being background — hand it back or implement it and record the
deviation, but do not build the narrower half silently.

**Seen before.** `raster-6sl.5`, twice in one file — "A safety rule the plan specified did not
prevent the thing it named" and "The plan's data structure could not express the output the plan
agreed". Both are the same family: the plan's *User-facing decisions* promised something its
specified implementation could not deliver, and the implementer built the specification. Third
sighting of the family, first of this exact shape (a decision with no site at all in *Files to
change*).
