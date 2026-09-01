# raster-7nl — retrospective

- **Implementer:** Storm
- **Date:** 2026-08-31
- **PR:** #33

## `model-for`'s prescribed invocation misfires and exits 2 — seventh sighting

**What happened.** Run verbatim from *The review — you get exactly one*:

```bash
provider="$(.claude/cerebro/scripts/agent-cli)" || provider=""
.claude/cerebro/scripts/model-for ${provider:+--provider "$provider"} --role reviewer
```

printed `usage: model-for [--provider <p>] [--name <n>] [--role <r>]` and exited 2, with `provider`
correctly set to `claude`. Split by hand, `model-for --provider claude --role reviewer` exits 0 and
prints nothing — a genuine miss, this repository having no `.cerebro/models.conf` — and the review
ran on the CLI's default model, which is the documented answer to a miss.

**Why.** Established this time, with `set -x` and a word-by-word expansion:

```
+ .claude/cerebro/scripts/model-for '--provider claude' --role reviewer
words:
  [--provider claude]
```

The tool shell here is **zsh**, which does not word-split an unquoted parameter expansion.
`${provider:+--provider "$provider"}` is therefore one argument, `--provider claude`, and
`model-for`'s `case` sends anything it does not recognise to `usage`. Under bash the same line
splits into two and works, which is why the skill's form reads as correct. `raster-fl4.2` reached
the same conclusion; this entry adds the direct expansion trace and the seventh count.

**Cost.** About three minutes here, including the trace. The time is not the point at seven
sightings: the danger is that `model-for`'s contract is *a miss prints nothing*, and this failure
also prints nothing to stdout. An implementer who does not check the exit code reads a usage error
as "no key matched" and reviews on the CLI default. On this repository that is the right answer by
accident — there is no `models.conf`. On a consumer that has one with a `reviewer` row it is silently
the wrong model, which is precisely the one-file-two-answers defect `model-for` exists to prevent.

**Prevent by.** Changing the line in `implement-bead`'s *The review — you get exactly one* to a form
that survives both shells — `set -- ; [ -n "$provider" ] && set -- --provider "$provider"` and then
`model-for "$@" --role reviewer`, or simply always passing `--provider "$provider"` and having
`model-for` treat an empty value as "resolve it yourself", which its own header already documents as
the behaviour of omitting the flag. Six prior implementers each rediscovered this in the same three
minutes; the fix is one line in the skill and belongs to the navigator.

**Seen before.** `raster-tf5.6`, `raster-m6z` (fourth), `raster-fu0` (fifth), `raster-fl4.2` (which
first named zsh), `raster-fl4.3`, `raster-6jr` (sixth).
