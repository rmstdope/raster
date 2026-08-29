# Raster — MVP Build Plan

**Draft 0.1 — August 2026**
*Companion to the Raster Language Specification v0.1*

---

## 1. The MVP in one sentence

`rasterc demo.raster` produces a `demo.nes` ROM that, in Mesen2 and on real hardware,
displays an image imported from a PNG file on disk with the background colour
changing at chosen vertical positions on the screen.

## 2. Acceptance criteria

The MVP is done when all of these hold:

| # | Criterion |
|---|---|
| A1 | A single `.raster` source file plus a `.png` on disk compiles to a valid iNES ROM with no external toolchain (no cc65, no ld65, no Python). |
| A2 | The ROM runs in Mesen2 without errors, warnings in the log, or open-bus reads. |
| A3 | A PNG is converted at compile time into CHR pattern data, nametable, attribute table and palette, and appears on screen pixel-identical to the source image. |
| A4 | Background colour changes at author-specified scanlines, stable, with no visible tearing or jitter between frames. |
| A5 | At least one `cycles(N)` block in the demo is verified by the compiler, and deliberately breaking its budget produces a clear compile error naming the overage. |
| A6 | The compiler prints a per-event cycle budget table for the `frame` construct. |
| A7 | The ROM runs correctly on Henrik's NES hardware (final gate, after emulator verification). |

Note the deliberate inclusion of **A5 and A6**. It would be possible to reach a
working ROM without any cycle model at all; that ROM would prove nothing about the
thesis of the project. The MVP must demonstrate the one thing that justifies building
a new language.

## 3. Scope

**In scope**

- MMC3 (mapper 4), NTSC, fixed bank layout, no PRG bank switching yet
- A language subset: `target`, `asset image png`, `chrrom`, `const`, `var`, `fn`,
  `main`, `if`, `for` with constant bounds, `while`, assignment, arithmetic on `u8`
  and `u16`, arrays, `ppu.*` / `mmc3.*` named registers, `poke`/`peek`
- `cycles(N)`, `cycles(<= N)`, `cycles(?)`, `pad`
- `wait vblank`, `wait cycles(N)`, `sync exact`
- `frame` with `at scanline`, `every N scanlines`, lowered `using irq` (MMC3) and
  `using timed`
- Inline `asm fn` with `employs` and verified `cycles`
- Diagnostics with source spans

**Out of scope for MVP** (designed for, not built)

- FamiStudio integration and the `timeline` construct
- PRG/CHR bank switching and automatic bank allocation
- `soa` layout, structs, pointers, `fx8.8`, `match`
- Sprites, OAM, DMA
- Compression (`pbz`-style), `charmap`, `bin` assets
- Mappers other than MMC3
- PAL, optimizer beyond straightforward peephole

## 4. Technical approach — the parts that need deciding now

### 4.1 Making the background actually change colour

This is worth getting right up front because it determines what the demo looks like
and how hard the timing is. There are three techniques, and they are not equivalent:

**(a) Rendering off, per-scanline `$3F00` write.** With rendering disabled the PPU
outputs the backdrop colour across the whole screen, and a write to `$3F00` takes
effect immediately. Per-scanline colour bars are then just a cycle-timed loop writing
`$2006`/`$2007`. *True* background colour, trivially safe, no glitches. But nothing
else can be on screen.

**(b) Rendering on, per-scanline `$2001` emphasis/greyscale writes.** A single `sta
$2001` per scanline, no address latch involvement, no scroll corruption, comfortably
inside hblank. Gives strong per-scanline colour shifts over a visible image. This is
the technique that lets colour changes and the PNG coexist.

**(c) Rendering on, mid-frame `$3F00` write.** The real thing: a genuine backdrop
change over a rendered image. Requires setting `$2006` to `$3F00` (which produces the
well-known glitch pixel at the current dot), writing the colour, then restoring
`$2006` to a palette address and re-establishing scroll via `$2005`/`$2006` — all
within hblank's ~28 cycles. Tight, and exactly the kind of thing Raster is for.

**Decision: the MVP demo does (a) and (b); (c) is the stretch goal.** The demo has
two parts — a colour-bar screen using (a) to prove cycle-timed splits, and the image
screen using (b) to prove the asset pipeline and IRQ-driven splits together. (c) is
attempted last, as the proof that the cycle model earns its keep; if it fits budget
in the compiler's table and works on hardware, that is a strong signal the whole
design is sound.

### 4.2 Split mechanism: MMC3 IRQ vs cycle-timed loop

Both are built, because they validate different things.

- **`using timed`** — sync at vblank, de-jitter, then a loop of `cycles(114) pad`
  blocks, one per scanline. Simple, needs no mapper features, and directly exercises
  the cycle model. This is the technique for part (a).
- **`using irq`** — MMC3 scanline counter. This is the real demo technique, frees the
  CPU between splits, and forces the compiler to handle the MMC3 requirements listed
  in spec §7.3. This is the technique for part (b).

Building `using timed` first is the right order: it is the shorter path to a picture
on screen and it makes the cycle model load-bearing immediately.

### 4.3 MMC3 requirements the compiler must enforce

Carried into the implementation from spec §7.3, because these are where an MMC3 IRQ
chain silently fails:

- The scanline counter clocks on filtered PPU A12 rising edges, so **background and
  sprite pattern tables must occupy opposite halves** of pattern memory (`$0000` vs
  `$1000`). With everything at `$0000` the counter simply never ticks.
- Rendering must be enabled for the counter to run at all.
- `$C000` sets the latch, `$C001` schedules a reload at the next A12 edge, `$E001`
  enables IRQs, `$E000` disables and acknowledges. Handlers must acknowledge before
  returning.
- The latch value is off by one from the naive "scanlines until next event"; the
  compiler computes it.
- `$E000-$FFFF` is fixed on MMC3, so vectors, the IRQ handler and the runtime live
  there.

### 4.4 The cycle model is the foundation, not a feature

The instruction encoding table and the cycle cost table are the same table. Build it
once, correctly, including:

- Base cycle counts for all official opcodes
- Undocumented opcodes with their counts (we accept them; `--legal-isa` filters)
- +1 for indexed reads that cross a page boundary
- +1 for taken branches, +1 more if the branch crosses a page
- Read-modify-write and dummy-read behaviour

This table is the single most correctness-critical artifact in the project, so it gets
its own test suite validated against an independent source (see §7).

## 5. Architecture

A Rust workspace. Crate boundaries chosen so the cycle model sits underneath codegen
and cannot be bypassed.

```
raster/
├── crates/
│   ├── raster-syntax/     lexer, parser, AST, source spans
│   ├── raster-sema/       name resolution, type checking, const evaluation
│   ├── raster-ir/         mid-level IR, lowering from AST
│   ├── raster-6502/       instruction encoding + cycle cost model + assembler
│   ├── raster-codegen/    IR -> 6502, register allocation, zero-page allocation
│   ├── raster-timing/     cycle analysis passes, budget checking, padding
│   ├── raster-assets/     PNG -> CHR/nametable/attributes/palette
│   ├── raster-link/       bank layout, iNES header, ROM emission
│   ├── raster-diag/       diagnostic rendering
│   └── rasterc/           CLI
├── tests/
│   ├── golden/            source -> expected ROM bytes
│   ├── emu/               run ROM headless, compare framebuffer to reference PNG
│   └── cycles/            assert measured cycle counts match compiler predictions
└── examples/
    └── mvp/               demo.raster + picture.png
```

**Proposed dependencies** — all permissive-licensed, all mature:

| Crate | Purpose |
|---|---|
| `logos` | Lexer generation (or hand-roll; the grammar is small) |
| `chumsky` or hand-written recursive descent | Parser. Recommend **hand-written** — better error recovery, and parser errors are user-facing here |
| `ariadne` or `codespan-reporting` | The diagnostic rendering in spec §14 |
| `png` (or `image`) | PNG decoding. `png` alone is lighter and sufficient |
| `clap` | CLI |
| `insta` | Snapshot testing for codegen output |
| `tetanes` (as a library) | Headless NES emulation for CI framebuffer tests — **needs verification that its library API exposes the framebuffer and can run headless**; alternatives are `nes-rs` or writing a minimal PPU-accurate test harness |

## 6. Milestones

Each milestone has a concrete, demonstrable output. Estimates assume part-time work.

### M0 — Scaffolding *(~2 days)*
Workspace, CI, `rasterc --version`, diagnostic plumbing with a fake error to prove
spans render correctly.

**Done when:** `cargo test` is green in CI and a deliberately malformed source file
produces a nicely formatted error with a caret under the right character.

### M1 — Hand-built ROM *(~3 days)*
No language yet. A Rust function that emits, byte by byte, a valid MMC3 iNES ROM
containing hand-assembled 6502 that initialises the PPU and sets a solid backdrop
colour.

**Purpose:** de-risk the entire hardware end before writing a compiler. Every
subsequent milestone assumes ROM emission works.

**Done when:** the ROM shows a solid blue screen in Mesen2. This is also the first
thing to try on real hardware — flash it early, because a header or init mistake is
much cheaper to find now.

*Details to get right here:* 16-byte iNES header (`4E 45 53 1A`, PRG size in 16 KB
units, CHR size in 8 KB units, mapper 4 split across flags 6 and 7), vectors at
`$FFFA`/`$FFFC`/`$FFFE` in the fixed bank, the standard reset sequence (disable
interrupts and decimal mode, set stack, wait two vblanks before touching the PPU),
palette write at `$3F00`.

### M2 — The 6502 core *(~1 week)*
`raster-6502`: every opcode with its encoding, addressing mode, byte length and cycle
cost, plus page-crossing and branch penalties. An assembler that takes a list of
instructions and produces bytes, and a cycle counter that takes the same list and
produces a count.

**Done when:** an exhaustive test asserts encoding and cycle cost for all 256 opcodes
against an independent reference, and M1's hand-assembled ROM is regenerated through
this crate byte-identically.

### M3 — Front end *(~1.5 weeks)*
Lexer, parser, AST for the MVP subset. Name resolution, type checking, constant
folding.

**Done when:** the MVP demo source parses and type-checks (it will not yet compile to
anything), and a suite of deliberately broken sources produce the intended errors.

### M4 — Codegen *(~2 weeks)*
IR, lowering, a straightforward 6502 code generator, zero-page allocation. No
optimizer beyond peephole. Emits a working ROM.

**Done when:** a `.raster` file that sets a solid backdrop colour and loops compiles
and runs — the same result as M1, but from source.

### M5 — Asset pipeline *(~1 week)*
PNG import: decode, quantise to the NES palette, tile the image into 8×8 cells,
deduplicate, build the nametable, derive up to four sub-palettes and build the
attribute table, emit CHR.

Diagnostics matter here as much as the conversion: "cell (12, 7) uses 5 colours,
attribute cells allow 4" with the offending pixels named is the difference between a
usable tool and a frustrating one.

**Done when:** the PNG appears on screen in Mesen2 pixel-identical to the source, and
a CI test compares the emulator framebuffer against the source image automatically.

**This satisfies A3, and is the point at which the project is visibly real.**

### M6 — Cycle blocks *(~1.5 weeks)*
`cycles(N)`, `cycles(<= N)`, `cycles(?)`, `pad`, `wait cycles(N)`, and the
restrictions of spec §6.3. Padding synthesis. The budget report.

**Done when:** a `cycles(114) pad` block measurably takes exactly 114 cycles when
measured in the emulator's cycle counter, and an over-budget block fails to compile
with the error format from spec §14.

**This satisfies A5 and is the real proof of concept.**

### M7 — The frame construct *(~1.5 weeks)*
`frame`, `at scanline`, `every N scanlines`, lowering to `using timed` first, then
`using irq` with full MMC3 handling and the A12 validation. The budget table (A6).

**Done when:** colour bars are stable on screen with no jitter across frames, via
both lowering strategies.

### M8 — Integration and hardware *(~1 week)*
The full MVP demo from spec §13. Real-hardware testing. Documentation of what
actually shipped versus what the spec claims.

**Done when:** A1–A7 all hold.

**Rough total: 10–11 weeks part-time.** M4 and the emulator test harness are the most
likely to overrun.

## 7. Verification strategy

Timing correctness cannot be eyeballed, so testing is a first-class part of the
toolchain rather than an afterthought.

**Three layers:**

1. **Unit — the cycle table.** Every opcode's encoding and cost checked against an
   independent reference. If this table is wrong, everything above it is wrong in a
   way that is nearly impossible to debug from a screenshot.

2. **Predicted vs. actual cycles.** The critical test. Compile a `cycles(N)` block,
   run the ROM in a cycle-accurate emulator, read the CPU cycle counter before and
   after, assert it equals N. Mesen2's Lua scripting API can do this interactively;
   for CI a Rust emulator used as a library is preferable so tests run headless with
   no GUI dependency. **Verify early which Rust NES emulator crate exposes both a
   cycle counter and the framebuffer** — this choice gates the CI story, and if none
   is suitable, the fallback is driving Mesen2 via Lua from a test script.

3. **Visual regression.** Run the ROM headless for N frames, dump the framebuffer,
   compare against a committed reference PNG. Catches asset-pipeline and rendering
   regressions that cycle counts cannot.

**Emulator choice:** Mesen2 for interactive debugging and as the authority when
emulators disagree — its debugger, event viewer and PPU viewer are the best available
for exactly this class of bug. Cross-check surprising results in a second emulator
before concluding the ROM is wrong.

**Hardware:** flash and test on real hardware at M1, M5 and M8 at minimum. Emulators
are good but the MMC3 IRQ counter's A12 filtering behaviour is precisely the kind of
detail where an emulator can be forgiving and hardware is not.

## 8. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Cycle model subtly wrong (page crossings, dummy reads) | Everything downstream is wrong and it looks like a hardware bug | Layer-1 and layer-2 tests from M2 onward; never let untested cycle costs into codegen |
| DMC DMA steals cycles and breaks exact timing | Effects glitch unpredictably on hardware but not obviously in emulation | Not an MVP problem (no audio yet), but document it; do not design as if it does not exist |
| MMC3 IRQ chain works in emulator, fails on hardware | Late, demoralising discovery | Hardware-test at M1 and M5, not just at the end |
| Codegen quality is poor enough to be unusable | Language works but nobody wants to use it | Accept it for the MVP. Non-timed code being merely adequate is an explicitly acceptable trade; timed code is what must be right |
| Scope creep into a general-purpose language | Never ships | The out-of-scope list in §3 is a commitment, not a wish list |
| Rust emulator crate does not fit the CI need | Test story weakens | Identify and validate the crate during M0, not M6 |

## 9. First concrete steps

1. Create the workspace and CI (M0).
2. **Before writing any compiler code, hand-build the M1 ROM and flash it to real
   hardware.** Confirm the header, the MMC3 init, and the palette write are right on
   the actual machine. Everything else rests on this.
3. Build the opcode/cycle table and its exhaustive test (M2).
4. Investigate the headless-emulator options and pick one, so the M6 test can be
   written the day M6 starts.

## 10. Sources and references

- [NESdev Wiki: MMC3](https://www.nesdev.org/wiki/MMC3) — register semantics, A12 filtering, banking modes
- [NESdev Wiki: Programming MMC3](https://www.nesdev.org/wiki/Programming_MMC3)
- [NESdev Wiki: Cycle counting](https://www.nesdev.org/wiki/Cycle_counting)
- [NESdev Wiki: Tools](https://www.nesdev.org/wiki/Tools)
- [NESFab documentation](https://pubby.games/nesfab/doc.html) — prior art
- [FamiStudio Sound Engine](https://famistudio.org/doc/soundengine/) — post-MVP integration target
