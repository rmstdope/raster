# raster-bab: two PRs green on their own bases left main red

**What happened.** While PR #121 was in review, #119 (raster-ra6) merged. It made
`ppu.ctrl = $10` before a `using irq` frame a refusal. Minutes later #120 (raster-6d4) merged, and
it added two `raster-codegen/tests/generate.rs` fixtures that open with exactly that value. Each PR
was green on the base it was tested on. Together they left main red at 247f900
(`a_handler_at_scanline_240_gets_the_undrawn_line_and_other_lines_keep_the_hblank`,
`the_window_is_chosen_per_event_and_its_edge_is_exact`). I found out only because my own rebase
went red on tests I had not touched, and `gh run list --branch main` confirmed main itself was
failing.

**Why.** `main` has no branch protection (`branches/main/protection` returns 404), so
`required_status_checks.strict` is effectively false. `produce-bead` *Merging* then lets a `BEHIND`
PR merge without re-running CI on the merged result. #120 was behind #119 when it merged, and
nothing re-tested it against #119's new refusal.

**Cost.** Two rebases instead of one, a fixture fix in a PR that was not about it, and about half
an hour during which main was red for every other session.

**Prevent by.** Protect `main` with strict required status checks, so a `BEHIND` PR must catch up
(`update-branch`) and pass CI on the merged result before `gh pr merge` succeeds. Or make
`produce-bead` *Merging* always catch up a `BEHIND` head when the base moved since CI last ran.

**Seen before.** Nothing like it in `docs/retrospectives/` (grep for "red main", "raced",
"semantic conflict").
