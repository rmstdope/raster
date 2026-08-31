# raster-6sl.5 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-08-31
- **PR:** #14

## The documented review-wait loop reported a review that did not exist

**What happened.** The `implement-bead` wait loop
`until gh api .../reviews --jq 'length' | grep -qv '^0$'; do ...; done` exited printing
`REVIEW ARRIVED` while `gh api repos/rmstdope/raster/pulls/14/reviews` returned `[]`. The
run log shows `gh` had emitted `error connecting to api.github.com` on that iteration; the
error text is a line that is not `0`, so `grep -qv '^0$'` matched it and ended the wait.
**Why.** Established. The loop's condition treats any `gh` output other than the literal
`0` as evidence of a review, so a transient network failure is indistinguishable from a
review arriving. An implementer that trusted it would go on to merge believing the Four
Eye Principle was satisfied.
**Cost.** No wrong merge here — the state was re-checked before acting — but about ten
minutes of re-polling and re-querying to establish that no review existed.
**Prevent by.** Make the condition require a number, not merely "not zero", in
`.claude/skills/implement-bead/SKILL.md`'s *The review — you get exactly one*: e.g.
`count="$(gh api ... --jq 'length')" && case "$count" in ''|*[!0-9]*) false;; 0) false;;
*) true;; esac`, so a `gh` failure keeps waiting instead of ending the wait.
**Seen before.** none found.

## No Copilot review has arrived on this repository since PR #9

**What happened.** `request-review 14` exited 0, and no review arrived in about fifty
minutes. `gh pr view 14 --json reviewRequests` reads `[]` and the issue timeline records
no `review_requested` event at all. The same holds for every recent pull request:
`copilot-pull-request-reviewer[bot]` reviewed #7, #8 and #9, and #10, #11, #12 and #13
have zero reviews — all four were merged anyway, #13 two and a half minutes after it
opened.
**Why.** Not established. The request is accepted and then leaves no trace; whether
Copilot review is disabled on the repository, rate-limited, or failing silently was not
determined from an implementer's vantage point.
**Cost.** This bead: about fifty minutes of review waiting and a hand-back with the PR
left open and green. Across the fleet: four pull requests merged with no second pair of
eyes, which the Four Eye Principle does not permit.
**Prevent by.** The navigator should check whether Copilot code review is still enabled
for `rmstdope/raster` (Settings → Copilot → Code review), and, separately, decide whether
`.claude/skills/implement-bead/SKILL.md` should make an implementer verify a review
actually exists before merging rather than only before replying — #10 through #13 suggest
the twenty-minute escalation was not taken when it should have been.
**Seen before.** none found.

## `build-workload --classify` cannot classify anything in this project

**What happened.** The gate step
`git diff --name-only -z origin/main...HEAD | xargs -0 .claude/cerebro/scripts/build-workload --classify`
exited 1 with `build-workload: no rust_paths in .cerebro/project.conf - cannot classify
safely.`
**Why.** Established. `.cerebro/project.conf` declares `app_paths` but no `rust_paths`,
which the script requires.
**Cost.** A minute, and a fallback to `--workload rust` as the skill's failure branch
directs. It will cost every implementer on this project the same minute until the
declaration exists.
**Prevent by.** Add a `rust_paths` line to `.cerebro/project.conf` — the whole workspace
is Rust, so `^(crates/|Cargo\.(toml|lock)$)` matches what `app_paths` already declares.
That is a change to a tracked declaration outside this bead, so it is the navigator's.
**Seen before.** none found.
