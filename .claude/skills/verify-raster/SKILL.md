---
name: verify-raster
description: Raster's verification skill, extending cerebro's verify-bead - Psylocke batches the pass's merged beads, writes .raster programs that exercise them, compiles them with rasterc, runs the ROMs headless in Neser with screen capture, and brings the navigator the evidence and a recommended verdict per bead; the navigator decides.
---

# Verifying raster beads with rasterc and Neser

`.cerebro/project.conf` names this skill as `verification_skill`. It **extends `verify-bead`**:
load that skill too, and keep everything it and `agents/verifier.md` say that is not replaced here —
which beads are candidates, the fresh `origin/main` worktree and its sha, the commit check,
`verification=pending`, the report on the bead before any question, the three verdicts and their
recipes, the sandwich around every question, and the end of a pass.

What this skill replaces is **how you look**. Raster is a compiler CLI: there is no server, no port
and no browser. Ignore `verify-bead`'s Chrome, `launch_targets` and port steps, and do not ask the
navigator how to run the application: this file is the answer. You look by writing `.raster`
programs, compiling them with `rasterc`, and running the ROMs headless in Neser.

What this skill adds is **batching**: one look for every candidate in the pass, not one per bead.

## The tools

```bash
root="$(.cerebro/cerebro/scripts/consumer-root --shared)"
wt="$root/.cerebro/worktrees/psylocke"            # prepare-worktree made it; its sha is the one you judge
neser_dir="$(.cerebro/cerebro/scripts/project-conf neser_dir ../neser)"
case "$neser_dir" in /*) ;; *) neser_dir="$root/$neser_dir" ;; esac
out="$root/.cerebro/scratch/verify/<short sha>"   # everything this pass writes
mkdir -p "$out"
```

- **rasterc** is the one built in the verification worktree, never one on `PATH` or from the main
  checkout: `cargo run --release --manifest-path "$wt/Cargo.toml" -p rasterc -- <in.raster> -o <out.nes>`.
- **Neser** is rebuilt once per pass, before the first capture, so captures come from the Neser on
  disk today: `cargo build --release --bin neser --manifest-path "$neser_dir/Cargo.toml"`, then
  `"$neser_dir/target/release/neser"`. A missing directory or a failed build is a **blocker**
  (`verify-bead`): ask the navigator, never substitute another emulator or an older binary.
- **Nothing is committed.** Programs, ROMs, PNGs and compiler output stay under `$out`
  (`.cerebro/scratch` is ignored); generated ROMs never enter the repository. The report names the
  paths and quotes what matters.

## Prepare the whole pass first

Select and prepare every candidate as `agents/verifier.md` says (pending, the outcome, the
acceptance, both halves of *User-facing decisions*, the `## The bug, as fixed` note, every child of
an eligible epic), **before** writing any program. Then sort each acceptance criterion into one of
two kinds:

- **Compiler-only** — a diagnostic, a refusal, a warning, the build summary, usage or exit status.
  The evidence is `rasterc`'s output, verbatim, with its exit code. No ROM, no Neser.
- **On screen** — the bead changes what a ROM does or shows: pixels, colours, which scanline a
  change lands on, start-up, banks. The evidence is a Neser capture, plus the compile output that
  built it.

A criterion that is both (a warning on a program that still builds) is checked both ways.

## Write the batch

One pass, one batch, in `$out`:

- **`batch.raster`** — a single program that exercises every *on-screen* criterion of every
  candidate together, laid out so each one is visible and attributable on its own: separate screen
  regions or scanline bands, separate frames, a comment at each part naming the bead id and the
  criterion it proves. Start from `examples/` in the worktree where that helps. If two candidates
  cannot share one program (they need different mappers, regions, or contradict each other), write
  the fewest programs that cover them: `batch-2.raster`, and so on, and say why.
- **One snippet per failing case** — `<bead id>-<n>.raster`, each the smallest program that
  triggers one diagnostic or refusal. An error stops a build, so error cases never go in the batch.
- **A bug's reproduction** — the bugfixer's *How to trigger it by hand*, as its own snippet: the
  steps that showed the bug, now showing it fixed.

Write each program from the language as the worktree's spec and examples define it. Writing a
program is not designing the bead: when the acceptance does not say what a construct should do, that
is a gap in the log, not something you settle.

## Run it

```bash
cargo run --release --manifest-path "$wt/Cargo.toml" -p rasterc -- "$out/<name>.raster" -o "$out/<name>.nes" \
  > "$out/<name>.compile.txt" 2>&1; echo "exit $?" >> "$out/<name>.compile.txt"

"$neser_dir/target/release/neser" --headless --nes-hardware nes-ntsc \
  --frames <N> --output "$out/<name>-ntsc.png" "$out/<name>.nes"
```

- **NTSC always**: it is raster's timing reference. **PAL too** (`--nes-hardware nes-pal`, to
  `<name>-pal.png`) when the program's `target` block says `region: pal` or the bead names PAL.
- **Frames**: enough for start-up to finish and the effect to settle; 120 unless the program says
  otherwise. Anything that changes over time takes `--capture-every K` so the report can show the
  sequence. Captures are deterministic: same ROM, same frames, same PNG.
- A compile that should have succeeded and did not, or a Neser exit of 1, is evidence, not a
  blocker: record it and carry on with the rest of the batch.

## Look

For each criterion, in the log `verify-bead` asks for: what you ran, what came out, and the file
that shows it.

- **Compiler output**: compare it with the acceptance's *The words, exactly* character for
  character, including the span, the `help` line and the final count line. Quote it in the log.
- **Captures**: open every PNG with the Read tool and look at it against the acceptance and the
  mockup (`docs/ui/…`, named in the acceptance). Where a criterion is a position — a colour change
  on scanline N, a picture at tile column M — measure it as well as eyeballing it: a short Python
  script over the PNG under `$out` (`python3 -I`) that prints the first row where the colour
  changes is evidence; "looks about right" for a timing claim is not.
- **What the bead did not touch**: the batch shows the rest of the program too. Notice whether it
  still behaves.

### When Neser may be the one at fault

A capture that contradicts the acceptance is either raster's fault or Neser's. Whenever the
difference is one emulation could plausibly produce — mid-scanline timing, IRQ edges, the PPU
around a register write — say so in the report, and look for evidence on raster's side: the
listing, the build summary, the bytes in the ROM. **Recommend Failed only when that evidence agrees
the fault is raster's.** Otherwise recommend no verdict on that criterion and ask the navigator
whether to check it in Mesen2.

## Recommend, for the whole batch

Write one report **per bead** onto that bead, as `verify-bead` says, first line
`Verification report at <full sha>`, before any question. Each report names the batch it was
judged in (`$out`, and which program and which part of it), and the paths of its captures and
compile output.

Then show the navigator what you saw: `open` every PNG the batch produced (macOS Preview), in one
command, before the first question.

Ask in as few calls of the question tool as the tool allows: **one question per bead**, four per
call, each carrying the bead's title, your recommended verdict marked `(Recommended)` with the
evidence in a line, and the other verdicts as options. Follow-ups you propose go in a call after
the verdicts, one question each: file or drop. The sandwich is around each call. Record only what
the navigator answered, by `verify-bead`'s recipes, one bead at a time; a family walked as one run
still uses `verifier-pass-epic-family`.

A pass whose navigator goes quiet leaves every bead in the batch pending with its report attached;
the next pass re-asks from the reports at the same sha, and looks again at a new one.
