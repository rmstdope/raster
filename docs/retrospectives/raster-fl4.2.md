# raster-fl4.2 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-08-31
- **PR:** #18

## `model-for`'s prescribed invocation misfires under zsh and exits with a usage error

**What happened.** `implement-bead` gives the reviewer-model lookup verbatim as

```bash
.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer
```

Run as written it printed `usage: model-for [--provider <p>] [--name <n>] [--role <r>]` and exited
2. Re-run under `set -x` the trace reads
`model-for '--provider claude' --role reviewer` — one argument, not two.

**Why.** Established. The harness shell here is zsh (`echo $0` → `/bin/zsh`), and zsh does not
word-split the alternate text of `${var:+...}` the way bash does, so `--provider "$provider"`
arrives as a single argv entry. Splitting it by hand — `--provider claude --role reviewer` — works
and returns the correct answer.

**Cost.** Small in wall-clock, about five minutes, but the failure mode is the dangerous part
rather than the delay. `model-for`'s contract is that **a miss prints nothing**, and this prints a
usage line and exits 2. Both look like "no answer", so an implementer who does not check the exit
code concludes "no reviewer model is configured" and spawns the review on the CLI default. Here
that happened to be right — this repository has no `.cerebro/models.conf` at all, so the correct
answer was also a miss — but in a consumer that *does* declare a `reviewer` row, the review would
silently run on the wrong model and nothing would say so.

**Prevent by.** `implement-bead`'s *The review — you get exactly one* section should not use
`${provider:+…}` word-splitting. Either build the argument list in a way that does not depend on
the shell's splitting rules, or have `model-for` accept `--provider ""` as "no provider" so the
flag can always be passed unconditionally. Whichever, the same section should say to treat a
non-zero exit as an error rather than as a miss, since only exit 0 with empty output means "no key
matched".

**Seen before.** None found — `grep -rliE "model-for|zsh|quoting" docs/retrospectives/` matched
nothing.

## A `cd` to the repository root sent a file-editing script into the main checkout

**What happened.** I ran `gh pr checks` from `/Users/henrikku/repos/raster` (a `cd` inside a
compound command), and the shell kept that directory. Several calls later a `python3` heredoc that
rewrote `crates/raster-emu/src/lib.rs` by relative path therefore rewrote **the main checkout's**
copy, not my worktree's. It was harmless only by luck: the replacements targeted text that exists
only on my branch, so every one was a no-op and the file was written back byte-identical —
`git status` was clean in both trees. The clippy run in the same call gave it away by printing
`Checking raster-emu (/Users/henrikku/repos/raster/crates/raster-emu)` with no worktree path.

**Why.** Established. `implement-bead`'s trap list says "**Check `pwd` before any git command**",
and the trigger it names is a `cd` into *another agent's worktree*. Neither half covers this: the
command that did the damage was a file write, not a git command, and the directory I moved to was
my own repository root, which reads as the safe place to be. `gh pr` needs no particular directory
inside the repository, so the `cd` bought nothing.

**Cost.** About five minutes to notice, verify both trees were clean, and re-apply the six edits in
the right place. It would have cost far more had the edits matched: the main checkout is shared,
another implementer was working from it (`raster-fl4.3`), and a silent write there is not
attributable to anyone.

**Prevent by.** Widen that trap in `implement-bead` from "any git command" to **any command that
reads or writes a file by relative path**, and name the innocuous trigger: never `cd` out of the
worktree for `gh`, which works from a worktree unchanged. A stronger version is to give every
worktree-relative command an absolute path, which is what I switched to for the rest of the run.

**Seen before.** None found for this failure. Seven files
(`raster-6sl.3`, `raster-6sl.4`, `raster-6sl.6`, `raster-20i.3`, `raster-20i.4`, `raster-tf5.2`,
`raster-tf5.3`) are about worktree setup, but all seven record a different thing — the declared
install command's `&&` reaching rustup — which did **not** recur here: `.cerebro/project.conf` now
declares `install_shell`, and `prepare-worktree` runs that through `bash -euo pipefail -c`.
