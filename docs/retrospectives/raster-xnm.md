# raster-xnm: a bug bead's hardware claim was false, and it was built before anyone checked

**What happened.** The bead was filed from review of #113. It said older MMC3 revisions "do not
raise the IRQ when the counter is reloaded to 0, so a chain that asks for latch 0 stops". I took
the claim as given. I asked the navigator to choose between a warning and a refusal, and built and
pushed the warning. The cold review then quoted NESdev's MMC3 page: Sharp MMC3A and NEC parts skip
only the *automatic* reload to 0, and "a manual reload request via $C001 fulfills this condition".
raster writes `$C001` on every link. The warning was a false positive, and its help would have
steered authors off valid schedules (every `every 1 scanlines` under `using irq`).

**Why.** The fix-bug contract starts from "the bead's bug claim". Nothing asks for a hardware claim
to be checked against a primary source before it is reproduced. The reproduction test could only
pin what the compiler emits, not what an old chip does with it, so it went red for "the right
reason" against a premise nobody had checked.

**Cost.** One full implementation, a second navigator question that reversed the first answer,
a revert, and a second review round.

**Prevent by.** In `fix-bug` *The bug-fix contract* step 1: when the claimed wrong behaviour is a
fact about hardware (or any external system) that no test here can observe, cite the source for it
(e.g. the NESdev page and the sentence) before choosing the test. A claim with no source goes to
the navigator first. The same line belongs in `write-bead` for bugs filed from review.

**Seen before.** Nothing like it in `docs/retrospectives/` (grepped "premise", "nesdev",
"hardware claim").
