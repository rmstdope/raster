# raster-fu0 — retrospective

- **Implementer:** Storm
- **Date:** 2026-08-31
- **PR:** #29

## `model-for`'s prescribed invocation misfires and exits 2 — fifth sighting

**What happened.** Ran verbatim from *The review — you get exactly one*:
`.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer` printed
`usage: model-for [--provider <p>] [--name <n>] [--role <r>]` and exited 2. Split by hand,
`model-for --provider claude --role reviewer` exits 0 with no output — a genuine miss, since this
repository has no `.cerebro/models.conf` at all, so the review ran on the CLI default, which is
correct.

**Why.** Established in `raster-fl4.2` and unchanged across five sightings: this harness shell does
not word-split the alternate text of `${var:+…}`, so `--provider "$provider"` arrives as one argv
entry carrying literal quotes.

**Cost.** About two minutes. Recording it only for the count.

**Prevent by.** Give `implement-bead`'s *The review — you get exactly one* an invocation that works
in this shell — two branches, or an `args` array — rather than the `${var:+…}` idiom. Five beads have
now paid the same two minutes; the fix is one line in the skill and is the navigator's.

**Seen before.** `raster-fl4.2`, `raster-tf5.6`, `raster-fl4.3`, `raster-m6z` — same command, same
exit 2, same cause.

## `build-workload --classify` cannot run in this repository — fifth sighting

**What happened.** `git diff --name-only -z origin/main...HEAD | xargs -0
.claude/cerebro/scripts/build-workload --classify` exited 1 with
`build-workload: no rust_paths in .cerebro/project.conf - cannot classify safely.`

**Why.** Established in `raster-tf5.5`: `.cerebro/project.conf` declares no `rust_paths`, and the
script refuses to guess rather than classify wrongly.

**Cost.** Under a minute. The skill documents the fallback for exactly this ("or classification
fails, rerun the preflight with `--workload rust`"), and this bead's plan already declared
`--workload rust`, which I had preflighted before starting.

**Prevent by.** Either declare `rust_paths` in `.cerebro/project.conf` or drop the step from
`implement-bead` for projects without it. The fix is the navigator's; five sightings is the argument
for making it, since the step has never once succeeded in this repository.

**Seen before.** `raster-tf5.5`, `raster-tf5.6`, `raster-tf5.7`, `raster-m6z` — same script, same
message, same cause.

## The review sub-agent spawns asynchronously, not synchronously as the skill states — second sighting

**What happened.** `implement-bead`'s *Getting the review* says "The spawn is synchronous: wait for
it inside the tool call". It is not. The `Agent` tool returned immediately with `Async agent launched
successfully` and an agent id; the review arrived just under six minutes later as a task
notification.

**Why.** Established in `raster-tf5.7` from the tool result itself: this harness's `Agent` tool is
asynchronous by construction and returns an id rather than a result.

**Cost.** None here. The neighbouring rule — *Waiting, without ending your run* — covers it, so I
blocked in a `bash` heartbeat loop and the notification arrived while it was still running. The lease
stayed fresh and the turn never ended.

**Prevent by.** Change the sentence in `implement-bead`'s *Getting the review* from "the spawn is
synchronous" to what actually happens: the spawn returns immediately, so block in a heartbeat loop
until the notification arrives. An implementer reading the current sentence literally has no reason
to block at all, and an ended turn against a running review is the exact failure the skill's own
opening anecdote describes.

**Seen before.** `raster-tf5.7` — same sentence, same tool behaviour.
