# raster-tf5.7 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-08-31
- **PR:** #24

## `build-workload --classify` cannot run in this repository — third sighting

**What happened.** `implement-bead`'s *Building* section makes classification a required step
immediately before the fast gate:

    git diff --name-only -z origin/main...HEAD | xargs -0 .claude/cerebro/scripts/build-workload --classify

It printed `build-workload: no rust_paths in .cerebro/project.conf - cannot classify safely.` and
exited non-zero, exactly as it did for `raster-tf5.5` and `raster-tf5.6`. The key is still absent
from `.cerebro/project.conf`, so the step still cannot succeed for any diff in this repository.

**Why.** Established, and unchanged from the two earlier findings: the project declares no
`rust_paths` and the script refuses to guess rather than misclassify.

**Cost.** Under a minute — the skill's own fallback ("rerun the preflight with `--workload rust`")
covers it and the plan had already declared that workload. No wrong gate was run, and nothing was
blocked. The cost is not the minute; it is that three consecutive implementers have now been trained
to read past a red exit code in the one section where a wrong workload picks the wrong gate.

**Prevent by.** Two implementers have already proposed the same two fixes — declare `rust_paths` in
`.cerebro/project.conf` (this is an all-Rust workspace, so the declaration is cheap), or say in
`implement-bead`'s *Building* section that a project without `rust_paths` skips classification and
uses the plan's declared workload, the way its port-block paragraph already says a project with no
`port_base` has no problem to solve. Neither has been done. This third sighting is the evidence that
it is worth doing rather than tolerating; both are the navigator's to make, not an implementer's.

**Seen before.** `raster-tf5.5` and `raster-tf5.6` — same command, same message, same fallback, same
two proposed preventions.

## The review sub-agent spawns asynchronously, not synchronously as the skill states

**What happened.** `implement-bead`'s *Getting the review* section says "The spawn is synchronous:
wait for it inside the tool call". It is not. The `Agent` tool returned immediately with
`Async agent launched successfully` and `The agent is working in the background. You will be
notified automatically when it completes`; the review arrived four and a half minutes later as a
task notification. There is no tool call to wait inside.

**Why.** Established from the tool result itself: this harness's `Agent` tool is asynchronous by
construction and returns an agent id rather than a result.

**Cost.** None here, because the same section's neighbouring rule — *Waiting, without ending your
run* — covers it: I blocked in a `bash` heartbeat loop and the notification arrived after it
returned, so the lease stayed fresh and the turn never ended. The risk is what makes it worth
writing down. An implementer reading "the spawn is synchronous" literally has no reason to block at
all, and would end its turn against a running reviewer with the bead claimed and the PR open — which
is precisely the failure that section exists to prevent, arrived at by following the sentence rather
than by ignoring it.

**Prevent by.** `implement-bead`'s *Getting the review* should say that the spawn returns
immediately and that the implementer must block on it the way it blocks on CI — a heartbeat loop
per *Waiting, without ending your run* — rather than describing it as synchronous. One sentence,
in the section that currently says the opposite.

**Seen before.** None found — `grep -rl "sub-agent\|subagent\|synchronous" docs/retrospectives/`
matches only `raster-tf5.5`, on an unrelated point.
