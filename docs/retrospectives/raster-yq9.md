# raster-yq9 — retrospective

- **Implementer:** Cyclops
- **Date:** 2026-09-01
- **PR:** #40

## `model-for`'s prescribed invocation misfires and exits 2 — ninth sighting

**What happened.** `implement-bead`'s *The review* section prescribes, verbatim:

```bash
provider="$(.claude/cerebro/scripts/agent-cli)" || provider=""
.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer
```

With `provider` correctly set to `claude`, this printed
`usage: model-for [--provider <p>] [--name <n>] [--role <r>]` and exited 2. Split by hand,
`model-for --provider claude --role reviewer` exits 0 with no output — a genuine miss, so the review
ran on the CLI's default model, which is the documented correct outcome.

**Why.** Established this run, with `set -x` and a `printf '[%s]\n'` over the same expansion. The
`Bash` tool's shell here is **zsh** (`$0` and `ps -p $$ -o comm=` both print `/bin/zsh`), and zsh
does not word-split an unquoted parameter expansion. So `${provider:+--provider "$provider"}`
produces **one** argument, `--provider claude`, where bash would produce two. Traced:

```
+ .claude/cerebro/scripts/model-for '--provider claude' --role reviewer
[--provider claude]
[--role]
[reviewer]
```

The line is correct bash and wrong zsh; nothing about `model-for` is at fault.

**Cost.** Two or three minutes and one wasted call, the same as the eight sightings before it.
No CI cycles. The real cost is not this run's: it is that eight implementers have now each paid it
and written it down, and the two failure modes it invites are both silent. `model-for`'s contract is
*a miss prints nothing* at exit 0, so exit 2 with a `usage:` line is easy to read as "no key
matched" and move on; and the obvious workaround — dropping `--provider` — is precisely the
one-file-two-answers defect the flag exists to prevent, since a consumer declaring
`agent_cli copilot` may carry a `reviewer@copilot` row that the bare key would silently miss.

**Prevent by.** The fix is one line in `.claude/cerebro/skills/implement-bead/SKILL.md`, in *The
review*, and it needs no new mechanism — replace the expansion with a form that is correct in both
shells:

```bash
provider="$(.claude/cerebro/scripts/agent-cli)" || provider=""
if [ -n "$provider" ]; then
  .claude/cerebro/scripts/model-for --provider "$provider" --role reviewer
else
  .claude/cerebro/scripts/model-for --role reviewer
fi
```

Nine implementers recording the same two-minute fix is now considerably more expensive than the
fix. This is the navigator's to apply; this run only recorded it.

**Seen before.** `docs/retrospectives/raster-tf5.6.md` (second), `raster-m6z.md` (fourth),
`raster-fu0.md` (fifth), `raster-6jr.md` (sixth), `raster-7nl.md` (seventh), `raster-jv6.md`
(eighth). The first is not labelled with an ordinal and I did not locate it. `raster-7nl.md` already
names the "a miss prints nothing" confusion as the danger; this run adds the established cause —
zsh, not bash — which none of the earlier files states.

## `cargo clippy --all-targets` is red on files no bead has touched — second sighting

**What happened.** Reaching for `cargo clippy --all-targets -- -D warnings` as a stronger form of
the declared gate failed with `error: could not compile raster-codegen (test "generate")` on an
`is_none_or` lint at `crates/raster-codegen/tests/generate.rs:147`, and separately on
`crates/rasterc/tests/emulator.rs` with two dead-code errors. This bead's diff touches neither
crate. I stashed the working tree and reran to confirm it is red at `3714c86` on `main` and not
caused by the change. The declared `gate_full` — `cargo fmt --check && cargo clippy -- -D warnings
&& cargo test` — is green, and so is CI. The review sub-agent independently reached for
`--all-targets` and reported the same two files, exactly as it did on `raster-c35`.

**Why.** `cargo clippy` without `--all-targets` lints library and binary targets only, so nothing
under `crates/*/tests/` has ever been linted here and those files have drifted red unobserved.
Established in `raster-c35` and re-confirmed this run.

**Cost.** A few minutes for the implementer plus a stash-and-rerun to prove it pre-existing, and a
few more for the reviewer, who paid it again independently. No CI cycles. A moment of believing a
green documentation change had broken an untouched crate.

**Prevent by.** `raster-c35` already named the two options and neither has been taken: either
`.cerebro/project.conf`'s `gate_full` gains `--all-targets` and the lints are fixed in their own
bead, or `.cerebro/traps.md` records that test targets are deliberately unlinted, so the next agent
reaching for the stronger command knows what it will say before running it. The second is the
cheaper of the two and would have saved four people-minutes this run alone. Both are the
navigator's to choose.

**Seen before.** `docs/retrospectives/raster-c35.md`, one bead ago — same two files, same lint,
same independent rediscovery by the review sub-agent.
