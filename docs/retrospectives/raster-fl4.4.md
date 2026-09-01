# raster-fl4.4 — retrospective

- **Implementer:** Storm
- **Date:** 2026-08-31
- **PR:** #25

## A mid-frame `ppu.addr`/`ppu.data` write lands wherever the PPU's own address register has got to

**What happened.** The first emulator fixture for `using irq` set the backdrop from its handlers, the
way `colour-bars.raster` does under `using timed`: `ppu.addr = $3f`, `ppu.addr = $00`,
`ppu.data = <colour>`. The rendered frame was uniformly black — no bands at all — and read exactly
like a chain that never fired. It had fired: a `PHA`-to-`RTI` window measured 59 cycles under
`raster-emu`, so the handler was running and doing nothing visible.

**Why.** `using irq` requires rendering to be **on**, and a rendering PPU advances `v` — its own
address register — continuously through a scanline. The ten-odd CPU cycles between the second
`$2006` write and the `$2007` write are thirty dots, three or four coarse-X increments, so the store
lands three or four palette entries past `$3F00`. `$3F03` is not the backdrop and nothing changes on
screen. I did not measure the final address; the diagnosis is the PPU's documented behaviour plus
the fact that moving the effect to `ppu.mask` emphasis made the bands appear immediately, on the
scanlines asked for.

**Cost.** About forty minutes, most of it spent proving the chain itself was sound before suspecting
the effect.

**Prevent by.** `.cerebro/traps.md` already carries the rendering-**off** half of this — *"With
rendering off, the PPU shows the palette entry its address register points at"*. It wants the
rendering-**on** half beside it: with rendering on, a `ppu.addr`/`ppu.data` pair written mid-frame
stores at whatever `v` has reached, so a raster effect over a rendered picture has to go through
`ppu.mask` (emphasis, greyscale) or wait for hblank. That entry is the navigator's to add; every
bead after `using irq` that changes something part-way down a rendered frame meets this.

**Seen before.** None found for the rendering-on case; `.cerebro/traps.md` has the rendering-off one.

## The `$2002` vblank-poll race cost a frame in fifty, in a second lowering

**What happened.** The first `using irq` lowering armed the chain once per frame from a
`BIT $2002 / BPL` vblank poll. Rendering frames 6 to 140 showed frame 61 — and about one frame in
fifty — with no bands at all.

**Why.** Established, and already known here: a `$2002` read landing within two cycles of the dot
that sets the vblank flag returns it clear and suppresses it, so the poll misses that frame and the
schedule is never armed. `raster-fl4.3` hit the same thing in the timed lowering at one frame in
nine, and `plan_timed_frame`'s doc comment in `crates/raster-timing/src/lib.rs` says so in as many
words — I had read it, and still wrote a per-frame poll, because the MMC3 counter appeared to need
re-arming across a vblank that does not clock it.

**Cost.** One emulator experiment and a rewrite of `irq_frame`, about thirty minutes. It was caught
only because the frame sweep from the fl4.3 retrospective's *Prevent by* was already in the test.

**Prevent by.** The trap is documented against `using timed`'s own planner, where a reader looking
for it will not be. It belongs where a lowering author will meet it — `.cerebro/traps.md` — stated
as the general rule rather than as a fact about one loop: **any** per-frame `$2002` poll drops about
one frame in ten to fifty, so a repeating schedule must be built from something that does not poll.

**Seen before.** `raster-fl4.3` — same race, different lowering, one frame in nine.

## `model-for`'s documented invocation misfires under zsh, again

**What happened.** The `implement-bead` skill prescribes
`model-for ${provider:+--provider "$provider"} --role reviewer`. It exited 2 with a usage line.

**Why.** Established. `Bash` runs zsh here, and zsh does not word-split an unquoted parameter
expansion, so `${provider:+--provider "$provider"}` reaches the script as the single argument
`--provider claude` — an unknown flag. Passing `--provider claude --role reviewer` as two words
works.

**Cost.** A couple of minutes, and only that because `raster-fl4.3` had already recorded it.

**Prevent by.** The line in `.claude/cerebro/skills/implement-bead/SKILL.md` should be written so
it survives zsh — two separate expansions, or an array, or simply telling the implementer to pass
the provider as its own word. This is at least its third sighting.

**Seen before.** `raster-fl4.3` — whose section title already says *"again"*.

---

*The bead was handed back after the sections above, and came back to a second implementer when main
had moved 14 commits past the branch. What follows is that second run.*

- **Implementer:** Wolverine
- **Date:** 2026-09-01
- **PR:** #25 (heads `ef24b45`, `26a6ed2`, `eeac3c3`)

## There is no documented way to get a worktree onto a branch that already exists

**What happened.** This bead arrived with its branch already pushed and its PR already open, so the
tree it needed was one on `raster-fl4.4-mmc3-irq-frames`, not a new branch. `implement-bead`'s
*Workspace* section documents exactly one call — `prepare-worktree --path <p> --branch <name>` — and
that mode runs `git worktree add -b`, which fails outright on a branch that exists. The other mode,
with no `--branch`, is documented as "the one detached, resettable tree (Psylocke's)" and resets to
`origin/main`. Neither is the case in hand. I read the script to find that the no-`--branch` path on
a *fresh* directory only creates it detached at `origin/main` — it does not reset anything, because
there is nothing there yet — and does still run the submodule init and the project's `install`. So
the working recipe is the detached mode followed by a checkout:

    .claude/cerebro/scripts/prepare-worktree --path .cerebro/worktrees/<id>
    cd <repo>/.cerebro/worktrees/<id>
    git fetch origin <branch> && git checkout -B <branch> origin/<branch>

**Why.** `prepare-worktree`'s two modes are "a fresh branch for a new bead" and "the verifier's
resettable tree". A handed-back or reopened bead with a live branch is a third case, and it is the
case the skill's own *A reopened bead* section says to expect. Nothing refused me and nothing was
lost — the gap is that the safe recipe is only discoverable by reading 220 lines of the script,
which is the step the script exists to save.

**Cost.** About ten minutes reading `prepare-worktree` before touching the bead, and the risk that
the obvious wrong move — `--branch` with a new name, or `git worktree add` by hand without the
submodule init — is the one an implementer in a hurry makes. The five retrospectives that bought the
submodule-init step are the evidence for how that goes.

**Prevent by.** Either give `prepare-worktree` an explicit third mode (`--existing-branch <name>`, or
`--branch` accepting a branch that already exists and checking it out instead of creating it), or add
the three-line recipe above to `implement-bead`'s *A reopened bead* section, which is where an
implementer in this position is already reading. The script is the better home: it already owns the
submodule and install steps that make the difference between a tree that works and one that fails
for a reason unrelated to the bead.

**Seen before.** None found for this. Eight files mention `prepare-worktree`
(`raster-6sl.3`, `raster-6sl.4`, `raster-6sl.6`, `raster-20i.3`, `raster-20i.4`, `raster-tf5.2`,
`raster-tf5.3`, `raster-fl4.2`) and all eight are about the declared install command reaching
`rustup` with a shell operator in it, which is fixed and did not recur — a different thing.
