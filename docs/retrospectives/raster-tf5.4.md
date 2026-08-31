# raster-tf5.4 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-08-31
- **PR:** #26

## The merge-state check hung for six minutes on a network call for a value already on disk

**What happened.** `implement-bead`'s *Merging* section opens its merge-state poll with
`want="$(git ls-remote --heads origin "$(git rev-parse --abbrev-ref HEAD)" | cut -f1)"`. Run
immediately after a successful `git push`, that call did not return: the whole `Bash` call was
killed at its 5-minute timeout, and `git` then reported
`fatal: unable to access 'https://github.com/rmstdope/raster.git/': Failed to connect to
github.com port 443 after 392921 ms: Timeout was reached`. The `push` seconds earlier and the
`gh pr view` calls seconds later both worked, so this was a transient failure of that one
connection, not an outage.

**Why.** Not established beyond "the connection hung". What made it cost a whole tool call rather
than a retry is that `ls-remote` has no timeout of its own and `git`'s default connect timeout is
longer than the call it sits in.

**Cost.** One aborted `Bash` call and about five minutes, with the bead claimed and the PR green
the whole time. No CI cycle.

**Prevent by.** `implement-bead`'s *Merging* section should read the tip from
`git rev-parse HEAD` rather than `git ls-remote`. The value wanted is "the branch tip you just
pushed", which is exactly local `HEAD` after a successful push — the same sha, with no network call
and nothing to hang. The `headRefOid` comparison that follows is unchanged and still does its job.
Keeping `ls-remote` is only worth it if the check must also detect that the push did not land, which
the push's own exit status already covers.

**Seen before.** none found.
