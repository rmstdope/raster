# raster-tf5.6 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-08-31
- **PR:** #21

## `model-for`'s prescribed invocation misfires and exits 2 — second sighting

**What happened.** `implement-bead`'s *The review — you get exactly one* still gives the lookup
verbatim as `.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer`,
and run as written it printed `usage: model-for [--provider <p>] [--name <n>] [--role <r>]` and
exited 2, exactly as `raster-fl4.2` describes. Confirmed the argv directly rather than inferring it:
`set -- ${provider:+--provider "$provider"} --role reviewer` then `printf ' %q' "$@"` gives
`--provider\ claude --role reviewer` — one argument carrying an embedded space, not two.

**Why.** Established, and unchanged from `raster-fl4.2`: this harness shell does not word-split the
alternate text of `${var:+…}`, so `--provider "$provider"` reaches the script as a single argv entry
and `model-for` rejects it as an unknown flag.

**Cost.** About three minutes. The danger is undiminished: a usage line on stderr with exit 2 looks
like `model-for`'s documented miss (which prints nothing and exits 0), so an implementer who reads
only the output concludes no reviewer model is configured. Splitting the flag by hand —
`model-for --provider claude --role reviewer` — exits 0 with no output here, so this repository's
answer really is a miss and the review ran correctly on the CLI default. In a consumer declaring a
`reviewer` row it would not be.

**Prevent by.** The prevention in `raster-fl4.2` is still the right one and is still unapplied:
`implement-bead` should not depend on `${var:+…}` splitting, and should say that only exit 0 with
empty output means "no key matched". Recording the second sighting rather than re-arguing it — the
fix is the navigator's.

**Seen before.** `raster-fl4.2` — same command, same cause, same section of the skill.

## `build-workload --classify` cannot run in this repository at all

**What happened.** `implement-bead`'s *Building* section says to classify the changed paths
immediately before the fast gate:

```bash
git diff --name-only -z | xargs -0 .claude/cerebro/scripts/build-workload --classify
```

It exits 1 with `build-workload: no rust_paths in .cerebro/project.conf - cannot classify safely.`
The key is absent from this project's `.cerebro/project.conf`, so the step cannot succeed for any
diff here, not just this one.

**Why.** Established. `.cerebro/project.conf` declares no `rust_paths`, and the script refuses to
guess rather than misclassify.

**Cost.** Under a minute, because the skill's next sentence covers the case — a failed
classification means rerunning the preflight with `--workload rust`, which the plan had already
declared and which had already passed. No wrong gate was run.

**Prevent by.** Either add `rust_paths` to `.cerebro/project.conf` so the step answers, or note in
the skill that a project without it skips classification and uses the plan's declared workload. As
written the step is a mandatory command that always fails here, which trains an implementer to read
past a red exit code in the one section where a wrong workload picks the wrong gate.

**Seen before.** None found — `grep -rl "build-workload\|rust_paths" docs/retrospectives/` matches
nothing.
