# raster-m6z — retrospective

- **Implementer:** Rogue
- **Date:** 2026-08-31
- **PR:** #23

## The plan was confidently wrong about two paths it had reasoned about but not run

**What happened.** Two of the plan's claims about the existing tree were false, and both were
claims the plan had explicitly considered rather than overlooked.

The first shipped. Under *Files to change*, on `raster-sema`'s `check_function` pushing a timed
region for a function's own `cycles(...)` annotation, the plan said: "The refusal above is still
correct there and costs nothing." It was not correct there. With the refusal added,
`fn f() -> u8 cycles(20) { return 1 }` reported "`return` inside a timed block … so it belongs after
the block rather than inside one" — there is no block, nothing to move the `return` after — and,
because sema aborts before `raster-ir`, it *replaced* the accurate `function timing specifications
are not supported`, which became unreachable for any annotated function containing a `return`. Every
cycle-annotated non-void function hits it, since such a function cannot fall through. The full gate
was green on it. It was caught by the review sub-agent, not by any check.

The second would have caused a wrong hand-back if followed literally. *The test plan* said "**No
existing fixture or test should need changing.** If one does, that is a finding about the check being
too broad — stop and report it rather than editing the fixture." One did:
`indexed_reads_and_branches_are_charged_their_worst_case` reached the worst-case cost model *through*
`analyze`, with a `BNE` under a `Report` constraint — which is precisely what the plan's own decision
5 refuses. The check was not too broad; the test asserted the behaviour the bead exists to remove.

**Why.** Established for the second, not for the first. The tripwire is stated as an inference —
"an existing test needed changing" ⇒ "the check is too broad" — and the plan verified it against the
seven `tests/cycles/*.raster` fixtures, which it lists by name, but not against the unit tests. For
the first, the plan's sentence reads as reasoning about the code rather than something run: the
symptom is one command away, and the plan quotes three other symptoms it *did* compile.

**Cost.** The first: one review round, a fix commit with two tests, and one CI cycle — roughly
forty minutes, and it would have shipped a regression to an unrelated shipped diagnostic. The
second: about ten minutes deciding not to hand the bead back.

**Prevent by.** Two specific things. (a) Where a plan asserts that an *existing* diagnostic or
refusal survives its change, that assertion belongs in the plan's *Validation* section as a command
to run, not in prose — this plan already validates three symptoms that way, and a fourth line
(`fn f() -> u8 cycles(20) { return 1 }` still reports `function timing specifications are not
supported`) would have caught it before the PR opened. (b) A plan's "no existing test should need
changing" tripwire should name what it was checked against; this one was checked against fixtures
only, and an implementer reading it cannot tell that.

**Seen before.** `raster-fl4.1` §"The plan's analysis surface could not cost the constructs the plan
required of it" and `raster-tf5.5` §"The plan's machine-checkable acceptance criterion contradicted
the plan's own prescribed code" — the same shape, a plan confident about code it had read rather
than run. Third sighting.

## The skill's conflict fallback replayed seven commits where one merge sufficed

**What happened.** `gh pr view` reported `CONFLICTING DIRTY` for the head I had actually pushed
(`headRefOid` matched), so per *Merging* I went to the local fallback, `git fetch origin main &&
git rebase origin/main`. Three unrelated PRs had merged during the review. The rebase stopped with a
conflict on commit 1 of 7, then again on 3 of 7, then 4 of 7, each in a different test file, and
each was the same trivial shape — main appended tests to the end of a file and so had I. I aborted
after the fourth and ran `git merge origin/main` instead: one conflict round, six files, all the
same shape, resolved once.

**Why.** Established. A rebase replays each commit against the new base, so a conflict in a file
both sides appended to recurs in every commit of mine that touched that file — seven commits over
five files. A merge resolves each file once. Nothing about the resolution differed; only the number
of times I did it.

**Cost.** About twenty-five minutes, and four abandoned partial resolutions. One of them I got
wrong (a mis-sliced Python resolution silently dropped my side of a file) and caught only because I
grepped for the test names afterwards — a rebase makes that kind of mistake cheap to make and
expensive to notice, because the intermediate commits are not expected to hold the finished state.

**Prevent by.** `implement-bead`'s *Merging* section should say that the local fallback for a 422 or
a genuine `CONFLICTING` is `git merge origin/main`, not `git rebase origin/main`. Its own reasoning
already supports this: it recommends `update-branch` because "it merges main into the branch
server-side rather than rebasing — fine here because every PR is squash-merged, so the branch's own
history never reaches main". That argument applies unchanged to the local fallback, where the
section currently prescribes a rebase and a force-push.

**Seen before.** None found.

## `model-for`'s prescribed invocation misfires and exits 2 — fourth sighting

**What happened.** Ran verbatim from *The review — you get exactly one*:
`.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer` printed
`usage: model-for [--provider <p>] [--name <n>] [--role <r>]` and exited 2. Split by hand,
`model-for --provider claude --role reviewer` exits 0 with no output — a genuine miss, so this
repository has no `reviewer` row and the review ran on the CLI default, which is correct.

**Why.** Established in `raster-fl4.2` and unchanged: this harness shell does not word-split the
alternate text of `${var:+…}`, so `--provider "$provider"` arrives as one argv entry.

**Cost.** About two minutes. Recording it only for the count.

**Prevent by.** As `raster-fl4.2` says and `raster-fl4.3` and `raster-tf5.6` repeat: `implement-bead`
should not depend on `${var:+…}` splitting, and should say that only exit 0 with empty output means
"no key matched". Four implementers have now paid for the same two minutes and, more importantly,
have each had to work out that a usage line is not the documented miss.

**Seen before.** `raster-fl4.2`, `raster-fl4.3`, `raster-tf5.6` — same command, same cause, same
section of the skill.

## `build-workload --classify` cannot run in this repository — third sighting

**What happened.** `git diff --name-only -z origin/main...HEAD | xargs -0
.claude/cerebro/scripts/build-workload --classify` exited 1 with
`build-workload: no rust_paths in .cerebro/project.conf - cannot classify safely.`

**Why.** Established in `raster-tf5.5`: `.cerebro/project.conf` declares no `rust_paths`, and the
script refuses to guess.

**Cost.** Under a minute — the skill has a documented fallback for exactly this ("If a planned
`non-rust` workload classifies as `rust` **or classification fails**, rerun the preflight with
`--workload rust`"), and this bead's plan already declared `--workload rust`.

**Prevent by.** Either declare `rust_paths` in `.cerebro/project.conf` or drop the step from
`implement-bead` for projects without it. Recording the third sighting; the fix is the navigator's.

**Seen before.** `raster-tf5.5`, `raster-tf5.6` — same script, same message, same cause.
