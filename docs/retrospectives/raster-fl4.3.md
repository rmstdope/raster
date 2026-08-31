# raster-fl4.3 — retrospective

- **Implementer:** Storm
- **Date:** 2026-08-31
- **PR:** #20

## Comparing consecutive frames proved stability that was not there

**What happened.** The bead's fourth increment is named
`timed_colour_bars_are_identical_across_consecutive_frames`, and the plan's validation says to
inspect "Mesen2 output for stable bars over repeated frames". Written literally — render frames 8
through 13 and compare — it passed on two separate ROMs that were both visibly broken. The first
lost one frame in nine entirely: the loop re-polled `$2002` every frame, and a read within two
cycles of the vblank flag being set returns it clear, so that frame's whole schedule never ran.
The second slid the bars down the screen by a scanline every three seconds, because the `JMP`
closing the loop cost three cycles a pass that nothing was paying for. Neither is visible in six
consecutive frames: the first shows five good frames out of six, and the second moves the picture
by one dot a frame. Both were found only because I rendered frames 2 to 140 out of curiosity, saw
`frames with no bars: [24, 33, 42, 51, ...]`, and then measured the drift rate over a hundred
frames rather than over five.

**Why.** Established, and it is arithmetic rather than luck. A defect that costs a fraction of a
dot per frame needs *n* frames to move one dot; a comparison over five frames can only see a defect
costing at least a fifth of a dot per frame. The interesting timing defects here are all small and
accumulating — that is what "locked to the picture" means — so the interval a stability test uses is
the whole of its sensitivity, and "consecutive" chooses the least sensitive one available.

**Cost.** About two hours: two diagnosis cycles, each ending in a redesign — the three-frame pass,
and then the closing jump's accounting — plus the probe runs to characterise each. Nothing reached
CI wrong, because the probing happened before the first push.

**Prevent by.** A timing plan's *Validation* section should name the **distance** an emulator
comparison spans, not just "repeated frames": for a construct claiming to stay locked to the
picture, a frame far enough away that a one-cycle-per-pass error would have moved the picture by a
scanline. Concretely, for anything at NTSC frame rate that is a few hundred frames, and it costs
about a second of test time. The test that shipped compares frame 8 with frame 300 for that reason,
and says so in a comment; `raster-fl4.4` will write the same kind of test for the IRQ strategy and
should start from the distance rather than from the word "consecutive".

**Seen before.** None found.

## The plan did not say what a compiler-imposed budget may be wrapped around

**What happened.** The lowering wraps each frame handler's body in a cycle budget the author never
wrote — "one 114-cycle scanline body per scheduled event", in the plan's words. Nothing in the plan
said whether the wrapped body is therefore subject to the restrictions of spec section 6.3, which
every author-written `cycles` block obeys, and I did not ask. I shipped it unrestricted. The review
then found that a handler containing an `if` reaches the linker as a duplicate label definition and
the author sees `internal compiler error: DuplicateLabel`, and I answered it by renumbering labels
per copy. That was the wrong fix: rebasing brought `raster-fl4.2`, whose new rule refuses
`wait cycles` inside a timed region, and made it obvious that a handler *is* a timed region — at
which point `at scanline 60 { wait cycles(200) }`, which compiled clean into a 114-cycle body, was
revealed as a ROM whose schedule was simply wrong. The right fix made the review's finding
unreachable and deleted the renumbering along with it.

**Why.** Established. The plan described the lowering as a *placement* — where each handler runs —
and said nothing about it being a *judgement* on the handler's contents. Both are true, and only
the first was written down, so only the first got built.

**Cost.** One review round and one extra commit, plus the correction posted to the PR after I had
already answered the finding a different way. Perhaps an hour.

**Prevent by.** When a plan has the compiler impose a constraint on code the author wrote — a
budget, a window, an alignment — its increments should name what the constrained code may contain,
or the implementer should treat the omission as a question worth asking before the first increment
rather than after the review. The tell here was available on day one: `Statement::Timed` is the same
IR node an author's `cycles` block lowers to, and `raster-sema` already had a list of what may
appear inside one.

**Seen before.** None found.

## `model-for`'s prescribed invocation misfired under zsh again

**What happened.** `implement-bead` gives the reviewer-model lookup verbatim as
`.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer`. Run as
written it printed the usage line and exited 2. Split by hand —
`model-for --provider claude --role reviewer` — it exited 0 and printed nothing, which is the
correct answer for a repository with no `.cerebro/models.conf`.

**Why.** Established by the earlier sighting, and confirmed here: zsh does not word-split the
alternate text of `${var:+...}`, so the flag and its value arrive as one argument.

**Cost.** Two minutes, because the earlier retrospective had already named it. The hazard is
unchanged and is the reason to record the second sighting rather than shrug: a usage error and a
miss both look like "no model configured", and in a consumer that does declare a `reviewer` row the
review would run on the wrong model with nothing saying so.

**Seen before.** `raster-fl4.2` — same script, same line of the skill, same shell.
