# raster-3o3 — retrospective

- **Implementer:** Storm
- **Date:** 2026-09-01
- **PR:** #47 (open, handed back)

## A sibling merged between planning and implementation and falsified a user-facing decision, and only the review caught it

**What happened.** The plan's Q8 chose the wording for a bare register statement — name the fix,
``assign it to a variable: `var s: u8 = ppu.status` `` — and specified the label be built from
whatever register was named. It was written against `fd1e9d9`. raster-1t9 merged as `b55fb92` in
between, refusing reads of write-only registers. Thirteen of the sixteen named registers are
write-only, so for those the shipped label names a program that does not compile:
`var s: u8 = mmc3.irq_disable` gives ``error: `mmc3.irq_disable` cannot be read``. Every gate was
green, both suites passed, and my own test pinned the broken suggestion as correct — because it too
came from the plan. The review sub-agent found it. The bead went back to the navigator, because
what to say for a bare *write-only* statement is wording nobody has chosen.

**Why.** A plan's *user-facing decisions* section is a set of answers formed at a moment, and the
plan records the commit it was formed against — but nothing re-checks those answers against what
merged since. The plan's own *Known traps* did note the three register PRs in flight, and checked
each against §9.1's block; it did not check them against the diagnostics the bead was adding.

**Cost.** The bead handed back after a full build, a full gate, a review round and a PR. About two
hours. Nothing was wasted — findings 2, 3 and 4 are fixed and pushed, and the branch is one
decision away from merging.

**Prevent by.** `implement-bead`'s *When the plan is wrong* already says to read a helper the plan
cites before building on it. The same suspicion is worth extending to the plan's decisions: where a
plan records the commit it was planned against and `git log <that sha>..origin/main` is non-empty,
read those commit messages against the plan's *User-facing decisions*, not only against its file
list. Two lines, before the first increment. The plan named `fd1e9d9`, and `b55fb92`'s subject line
is *"reading a write-only register is refused"* — that would have been enough.

**Seen before.** raster-c35 — *A sibling bead invalidated this plan's scope premise while the PR was
open*. raster-1t9 — *A sibling's test used the exact source this bead refuses, and the plan could
not have known*. Third sighting, and the first where the casualty was an agreed user-facing message
rather than scope or a test.

## `model-for`'s prescribed invocation misfires and exits 2 — eleventh sighting

**What happened.** `implement-bead` prescribes, verbatim:

    provider="$(.claude/cerebro/scripts/agent-cli)" || provider=""
    .claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer

`agent-cli` printed `claude`; the call exited 2 with `usage: model-for ...`. Re-run by hand as
`model-for --provider claude --role reviewer` it exits 0 with a miss.

**Why.** Not established here beyond what the ten prior sightings already record.

**Cost.** A minute, and one extra command to establish the miss was a miss rather than the failure.

**Prevent by.** Ten prior files say this; an eleventh adds only the count. The fix is the
navigator's, in `implement-bead`'s *The review* snippet.

**Seen before.** raster-1t9, raster-6jr, raster-7nl, raster-fl4.2, raster-fl4.4, raster-fu0,
raster-jv6, raster-m6z, raster-tf5.6, raster-yq9.

## The review sub-agent spawns asynchronously, not synchronously as the skill states — fifth sighting

**What happened.** `implement-bead` says "The spawn is synchronous: wait for it inside the tool
call." The `Agent` call returned immediately with a background agent id, and the result arrived as a
task notification eight minutes later. I blocked in a `Bash` heartbeat loop instead, which is what
the skill's *Waiting, without ending your run* prescribes for CI.

**Why.** Not established; the tool's own contract is asynchronous.

**Cost.** None this run — the loop covered it and the lease never went stale.

**Prevent by.** The navigator's, in `implement-bead`'s *Getting the review*: say the spawn returns
immediately and that the implementer blocks on a heartbeat loop until the notification arrives.

**Seen before.** raster-1t9, raster-fu0, raster-jv6, raster-tf5.7.
