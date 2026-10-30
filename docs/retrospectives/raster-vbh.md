# raster-vbh: two bug beads from one review overlapped, and I filed a third

**What happened.** The review of #119 (raster-ra6) produced two bug beads. raster-vbh covered what a
`using irq` frame wraps with; raster-bab covered handlers that run with the background at $1000.
Neither named the other. They ran in parallel in two bugfixer sessions. My own review (#122) raised
bab's case as a finding, and, unaware bab was in flight, I filed it again as raster-zdi. bab merged
while #122 was in review. It also refused my bead's own reproduction (the 120/200 example),
which left my PR red on the merge with main, conflicting in the same function and the same
spec paragraph. I had to squash, re-resolve, re-choose the reproduction test (a lone handler at
240 is the case bab leaves open), de-duplicate the two refusals, and get a second full review.
Separately, my prepared tree was cut before #119 landed, though the bug only exists on top of it;
I rebased first.

**Why.** Nothing links sibling beads filed from one review, and neither `fix-bug` nor
`produce-bead` asks the builder to look at in-progress beads before filing a follow-up or
shaping the fix.

**Cost.** One duplicate bead (raster-zdi, now assigned elsewhere and still open), a rebase with
conflicts, a rewritten test, and a second review round: roughly forty minutes.

**Prevent by.** When a review files more than one bead against the same function, file them with
`bd dep add <a> <b> --type related` (or as one bead). In `fix-bug`, before filing a follow-up
from a review finding, run `bd list --status in_progress` and search titles for the area.

**Seen before.** Similar in shape to `raster-c35` (two beads in flight against the same
diagnostics) and `raster-fl4.1` (a needed bead in flight with no dependency). Those overlaps
were between planned beads; this one is between bug beads from one review.
