# Raster — Language Specification

**Draft 0.1 — August 2026**
*Status: design draft. Syntax is provisional. Nothing here is built unless a
"rasterc today" column or note says so.*

---

## 1. Overview

Raster is a compiled, statically typed programming language for creating Nintendo
Entertainment System demoscene productions. A Raster project consists of source
files, asset files (PNG images, FamiStudio songs, binary blobs) and a target
declaration; the Raster compiler (`rasterc`) turns all of it into a single `.nes`
ROM.

Raster is not a general-purpose language with an NES backend bolted on. It is a
domain-specific language whose central abstraction is **time**: the number of CPU
cycles a piece of code takes, and where the PPU's electron beam is when that code
runs. Everything else in the language exists to serve that.

### 1.1 The one-sentence pitch

> Raster lets you write `cycles(114) { ... }` and have the compiler prove it.

### 1.2 Why this language exists

NES demo effects are built out of writes to hardware registers that must land at
precise positions in the video signal. A scroll split, a mid-frame palette change, a
CHR bank swap on scanline 96 — all of these are correct only if the code reaches the
store instruction at the right dot. Today this discipline is maintained entirely by
hand: the author counts cycles from a table, writes assembly, and re-counts by hand
after every edit.

Existing tools do not help with this, and the good ones actively hinder it:

- **cc65/ca65** gives you complete control but no abstraction — you are writing
  assembly, and cycle counting is a manual, error-prone, unverified activity.
- **llvm-mos** gives you C and C++ with a real optimizer, and the optimizer will
  silently change your cycle counts between builds.
- **NESFab** is the best existing high-level NES language — excellent asset pipeline,
  automatic banking, superb codegen — but its whole value proposition is aggressive
  optimization, and it has no way to express "this region costs exactly N cycles."
  Its `fence` constrains *ordering*, not *duration*.

The gap is specific and, as far as we can determine, unfilled: **no 6502 language
puts timing in the type system.** That is Raster's reason to exist. If we build
nothing else of value, a language where a raster effect either compiles or tells you
it is nine cycles over budget is worth having.

### 1.3 Design principles

1. **Timing is checked, not hoped for.** A `cycles` block that cannot be proven is a
   compile error, not a warning.
2. **The optimizer never lies about time.** Inside a timed block, the compiler may
   only apply transformations whose cycle cost it can account for exactly. Outside
   one, it optimizes freely.
3. **Effects are declarative; their lowering is the compiler's job.** The author says
   "change the backdrop on scanline 96"; the compiler decides whether that becomes an
   MMC3 IRQ, a sprite-0 hit, or a cycle-counted delay loop, and verifies the result
   fits.
4. **Assets are language constructs, not build-script output.** `png("logo.png")` is
   an expression with a type, not a Makefile rule.
5. **Assembly is a first-class citizen, not an escape hatch.** Inline assembly
   declares what it touches and what it costs, so it participates in timing analysis
   instead of defeating it.
6. **Honest about what it cannot do.** Where a construct cannot be verified, the
   language says so explicitly (`cycles(?)`) rather than pretending.

### 1.4 Use cases

**Primary:** NES demoscene productions — full demos, 4K/8K intros, and single-effect
"one-part" releases for party compos.

**Secondary and explicitly supported:**

- Effect prototyping — trying a raster idea quickly without hand-writing a frame loop.
- Teaching NES hardware timing, where the compiler's cycle report is the lesson.
- Music discs and slideshows (timeline + FamiStudio integration, no heavy effects).

**Explicitly not a goal (for now):** being a good language for writing an NES *game*.
NESFab exists and is better at that. Raster will happily compile game logic, but its
design budget goes to effects, not to entity systems or collision.

### 1.5 Target configuration

The MVP targets **MMC3 (mapper 4)** and NTSC. MMC3 is the natural demo mapper: its
scanline IRQ counter gives hardware-assisted raster splits, it has 8 KB PRG banking
plus fine-grained CHR banking for animation, and it is well emulated and cheap on
real hardware. Other mappers are planned; the `frame` construct is designed so the
same source lowers differently per mapper.

Raster emits **unofficial ("illegal") 6502 opcodes by default**, since they are
reliable on the 2A03 and save cycles in exactly the tight loops that matter. A
`--legal-isa` flag restricts output to documented opcodes for compo rules or
clone-hardware compatibility.

---

## 2. Program structure

A Raster program is a set of modules. One module declares the target and contains
`main`.

```
// demo.raster

target nes {
    mapper:  mmc3
    prg:     128K
    chr:     128K
    mirror:  horizontal
    region:  ntsc
}

import "effects/plasma.raster"

main {
    init()
    loop {
        wait vblank
        frame_body()
    }
}
```

Source files use the extension `.raster` (short form `.rst` accepted). Encoding is
UTF-8. Line comments are `//`; block comments are `/* ... */` and nest.

---

## 3. Lexical structure

| Element | Form |
|---|---|
| Identifiers | `[A-Za-z_][A-Za-z0-9_]*`, case-sensitive |
| Decimal literals | `42`, `1_000` |
| Hex literals | `0x2001`, `$2001` (both accepted; `$` is idiomatic for addresses) |
| Binary literals | `0b1000_1110` |
| Char literals | `'A'` (mapped through the active `charmap`) |
| String literals | `"text"` (mapped through the active `charmap`) |
| Boolean | `true`, `false` |

Keywords are reserved: `target import const var group fn asm main loop while for if
else match break continue return cycles wait frame at every from to using nmi irq
timeline part on asset png fam bin chrrom charmap palette bank in out employs pad
unsafe true false`.

---

## 4. Types

### 4.1 Scalar types

| Type | Meaning |
|---|---|
| `u8`, `u16`, `u24` | Unsigned integers. `u24` exists for banked pointers. |
| `i8`, `i16` | Signed integers, two's complement. |
| `bool` | One byte, `0` or `1`. |
| `fx8.8` | Signed 8.8 fixed point. `fx0.8` and `fx16.8` also provided. |
| `void` | Absence of a value. |

There is no floating point and there will not be one. There is no implicit widening;
`u8 + u8` is `u8` and wraps. Widening is explicit: `u16(a) + u16(b)`.

### 4.2 Aggregate types

```
[16]u8              // fixed array of 16 bytes
[8][4]u8            // arrays of arrays are allowed (unlike NESFab)
struct Sprite { y: u8, tile: u8, attr: u8, x: u8 }
```

Arrays are laid out contiguously. Structs are laid out in declaration order with no
padding. `struct of arrays` layout is requested explicitly:

```
soa struct Particles[64] { x: u8, y: u8, dx: i8, dy: i8 }
// lays out as four parallel 64-byte arrays — the layout effects code actually wants
```

### 4.3 Pointer types

| Type | Meaning |
|---|---|
| `^T` | 16-bit pointer to `T` in the current bank. |
| `far ^T` | 24-bit banked pointer: 16-bit address + 8-bit bank. |
| `^rom T` | Pointer into ROM (read-only, may be banked). |

### 4.4 Storage classes

```
var counter: u8                  // compiler chooses storage
var scroll_x: u8 in zp           // force zero page
var buffer: [256]u8 in bss       // main RAM
const SPEED: u8 = 3              // compile-time constant, no storage
```

Zero page is a scarce, contended resource whose allocation affects cycle counts, so
`in zp` is an explicit request. The compiler reports zero-page pressure and will
error if the program over-subscribes it rather than silently spilling — a silent
spill inside a timed block would change its cost.

### 4.5 Variable groups

Groups exist so that inline assembly can declare what it touches (see §9) and so the
compiler can reason about aliasing.

```
group raster_state {
    var line: u8 in zp
    var color: u8 in zp
    var table: [240]u8
}
```

---

## 5. Expressions and statements

Conventional, deliberately unsurprising:

```
fn clamp(v: u8, hi: u8) -> u8 {
    if v > hi { return hi }
    return v
}

fn demo() {
    var i: u8 = 0
    while i < 32 {
        buffer[i] = sine[i] + 128
        i += 1
    }

    for i in 0..32 {            // exclusive range
        buffer[i] = 0
    }

    for i in 0..32 step 4 { ... }

    match state {
        0     => intro(),
        1..3  => middle(),
        else  => outro(),
    }
}
```

Operators: `+ - * / % & | ^ ~ << >> == != < <= > >= && || !`, compound assignment
forms, and `.` for member access, `[]` for indexing.

Division and general multiplication are expensive on 6502, and the compiler refuses
them inside a `cycles` block. This is intentional — see §6.4.

**rasterc today:** `*`, `/`, `%`, `<<` and `>>` are all refused inside a timed block
whatever their operands, because each lowers to a loop and a loop is not straight-line
code (§6.3). This is not caution: before the refusal existed, `v * 3` was predicted at
38 cycles and spent 69.

---

## 6. Timing — the core of the language

### 6.1 Cycle blocks

```
cycles(114) {
    ppu.scroll = scroll_x
    ppu.ctrl   = nametable
}
```

The compiler computes the exact cycle cost of the block's generated code and compares
it against the annotation. Three forms:

| Form | Meaning |
|---|---|
| `cycles(N) { }` | Must cost exactly N cycles. Error otherwise. |
| `cycles(<= N) { }` | Must cost at most N cycles. |
| `cycles(?) label { }` | No constraint; the compiler reports the measured cost under `label` in the build output. Use while developing. |

### 6.2 Automatic padding

Hitting an exact count by hand is tedious. `pad` asks the compiler to insert filler:

```
cycles(114) pad {
    ppu.mask = emphasis[line]
}
```

The compiler emits the block, measures it, and appends padding to reach exactly 114.
Padding uses `nop` and (unless `--legal-isa`) the undocumented `nop $nn` / `nop $nn,x`
forms, which cost 3 and 4 cycles in 1–2 bytes, so any count ≥ 2 is reachable with
minimal ROM cost. If the block already exceeds the target, that is an error — padding
never removes work.

**rasterc today:** as designed, except that a block exactly one cycle short of its
budget cannot be padded — a single cycle has no filler — and says so rather than
rounding.

### 6.3 Rules inside a timed block

For a cycle count to be provable, the compiler restricts what may appear. A `frame`
handler (§7) is a timed block and obeys the same rules.

**Today the restriction is one sentence: a timed block is straight-line code.**
rasterc costs a region by adding up its instructions, so anything whose generated code
can move the program counter elsewhere is refused. The designed rules below are the
relaxations a cost model that survives control flow would buy.

| Designed | rasterc today |
|---|---|
| **1. No unbounded loops.** `while` with a non-constant trip count is rejected. `for i in 0..N` with constant `N` is allowed and is always fully unrolled or compiled to a loop with a proven constant cost. | **stricter** — `for`, `while` and `loop` are all refused: a block is costed as straight-line code, so no loop is charged correctly. |
| **2. Branches must be balanced.** Both arms of an `if` must cost the same, or the block must be `pad`, in which case the compiler pads the cheaper arm. An `if` without an `else` is padded to the cost of the taken path. | **stricter** — `if` is refused outright; nothing balances arms yet. |
| **3. Page-crossing penalties are resolved, not assumed.** Indexed addressing that may cross a page boundary costs an extra cycle. The compiler either proves no crossing occurs (via alignment) or accounts for the worst case and tells you which it did. `align 256` on a variable forces the safe layout. | **stricter** — always the worst case, and it does not say which. There is no `align` in the language, and no source construct reaches the penalty yet: the code generator emits no indexed addressing mode at all. |
| **4. Calls must be timed.** A function called inside a `cycles` block must itself carry a cycle annotation. Whether cycle cost should compose across function boundaries at all, and how, is §15 question 1; this rule is the shape that question has not yet settled. | **stricter** — a call inside a block is refused, and a function carrying a cycle annotation is refused when it is lowered, so the shape this rule describes does not compile today. |
| **5. No interrupts, unless declared.** An IRQ or NMI firing inside a timed block destroys its timing. The compiler inserts `sei` around timed blocks by default; `cycles(N) interruptible { }` opts out for blocks that are themselves interrupt handlers. | **as designed** — the `php`/`sei` … `plp` bracket costs nine cycles and is charged to the region's own budget, so the annotation is the region's wall-clock cost. |
| **6. A region must be straight-line.** Generated code that can go anywhere but the next instruction — a branch, a jump, a `jsr`, an `rts`, an `rti` or a `brk` — is a compile error whatever construct produced it. | **as designed** — enforced on the emitted instructions, so a construct nobody has thought of is caught too. `return`, the arithmetic of §5, every `wait` form and `sync exact` are each refused earlier and by name, so the message points at your source rather than at an opcode. |

Rule 6 is a floor rather than an ambition: rules 1–4 are the work of lifting it. These
restrictions apply *only inside* `cycles` blocks. The rest of the program is compiled
conventionally with full optimization.

### 6.4 Why the restrictions are the feature

Every one of the rules above is a case where an ordinary optimizing compiler would
make a silent choice that changes duration. Raster's position is that inside a timed
region, "I cannot prove this" must be a compile error and not a shrug. The
consequence — the one architectural commitment the whole project rests on — is that
**the backend carries a cycle-cost model through every pass**, and any optimization
that cannot report its effect on cycle count is disabled inside timed blocks.

### 6.5 Waiting

```
wait vblank                  // poll $2002 bit 7, or sync on NMI
wait cycles(1234)            // exact delay; compiler synthesizes the loop
wait scanline 96             // delay until a given scanline (see §7)
```

`wait cycles(N)` generates an optimal delay for any N ≥ 2: a nested
`dex`/`bne` loop for the bulk and padding instructions for the remainder, costing a
handful of bytes regardless of N.

**rasterc today:** every `wait` form is refused *inside* a timed block, because a
wait spends its cycles in a loop and a block is costed as straight-line code; widen
the budget and let `pad` fill it instead. Outside a region, `wait cycles(N)` is built
for any N ≥ 2; `wait vblank` and `wait scanline` are not built.

### 6.6 Frame sync and jitter

NMI entry is not cycle-exact: the CPU finishes the current instruction first, so the
handler starts with up to 6 cycles of jitter (more if a DMC DMA steals cycles).
Effects that need dot-exact alignment must de-jitter. Raster provides:

```
sync exact          // burn a variable number of cycles to a fixed alignment
```

which lowers to the standard sprite-0-hit-and-poll or read-$2002-and-branch
de-jitter sequence. `sync exact` is required before any `cycles` block that the
compiler can see writes to a PPU register during rendering; omitting it is an error
with a note explaining why.

**rasterc today:** as designed, and enforced — a timed block that writes a PPU
register with no preceding `sync exact` is refused. A `frame` handler is entered
already synchronized and needs none of its own.

---

## 7. The frame construct

A `frame` declares what happens at which vertical positions. It is the language's
signature abstraction.

```
frame main using irq {
    at scanline 0 {
        ppu.scroll = 0
    }

    at scanline 64 {
        backdrop = $0F
    }

    every 8 scanlines from 96 to 200 {
        ppu.mask = emphasis[line]
    }

    at vblank {
        upload_nametable_updates()
    }
}
```

A handler body is a timed block: every rule of §6.3 applies inside one, and its
budget is the window the schedule leaves it.

### 7.1 Lowering strategies

The `using` clause selects how the schedule is realised. If omitted, the compiler
picks based on the target mapper and the schedule's shape, and reports its choice.

| Strategy | Mechanism | Notes | rasterc today |
|---|---|---|---|
| `using irq` | MMC3 scanline counter | Preferred on MMC3. Each entry programs `$C000`/`$C001` for the delta to the next event, `$E001` to enable, and acknowledges via `$E000`/`$E001` in the handler. | **not built** |
| `using timed` | Cycle-counted delay loop from a synced frame start | Works on any mapper including NROM. Costs the whole frame's CPU time. | **as designed** — and it is what an omitted `using` clause means |
| `using sprite0` | Sprite 0 hit polling | One split only, position determined by sprite placement. | **not built** |
| `using hybrid` | Sprite 0 for the first split, IRQ chain after | Common demo idiom. | **not built** |

**rasterc today:** one `frame` per program, and events on the visible picture only —
`at vblank` is not built.

### 7.2 What the compiler verifies

For each event the compiler checks that the handler body fits its available window
and reports a per-event budget table:

```
frame main (using irq, mmc3)
  scanline   0   handler  38 cycles   window 113   ok
  scanline  64   handler  21 cycles   window  28   ok   (hblank)
  scanline  96   handler  34 cycles   window  28   OVER BY 6
```

That table is the artifact the language exists to produce.

### 7.3 MMC3 IRQ specifics the compiler handles for you

Getting an MMC3 IRQ chain right involves several non-obvious requirements that Raster
enforces at compile time:

- The counter clocks on filtered PPU A12 rising edges, so **background and sprite
  pattern tables must be in opposite halves** (`$0000` vs `$1000`) or the counter
  does not tick per scanline. The compiler checks the configured `ppu.ctrl` value and
  the CHR layout, and errors if the frame uses `using irq` without a valid A12 pattern.
- Rendering must be enabled for the counter to run.
- The latch is off-by-one relative to intuition; the compiler computes latch values
  from scanline numbers rather than making you do it.
- Handlers must acknowledge (`$E000` then `$E001`) before returning, which is emitted
  automatically.

**rasterc today:** not built. All four of these arrive with `using irq` — the A12
check, the latch computation and the automatic acknowledge are designed and
unimplemented.

---

## 8. Assets

### 8.1 Images

```
asset image logo = png("art/logo.png") {
    kind:    background        // background | sprites
    palette: auto(4)           // derive up to 4 sub-palettes from the image
    dedup:   true              // merge identical tiles
    compress: none             // none | rle | lz
}
```

Importing a PNG produces a value with named members, all resolved at compile time:

| Member | Type | Contents |
|---|---|---|
| `logo.tiles` | `[..]u8` | CHR pattern data, 16 bytes per tile |
| `logo.nametable` | `[960]u8` | Tile indices |
| `logo.attributes` | `[64]u8` | Attribute table |
| `logo.palette` | `[16]u8` | NES colour indices |
| `logo.tile_count` | `u8` | Number of unique tiles after dedup |

Compilation fails with a precise diagnostic when the image cannot be represented:
wrong dimensions, more than four colours in an 16×16 attribute cell, more than 256
unique tiles for a single pattern table, or colours outside the NES palette (with the
nearest legal colour suggested).

An explicit palette can be supplied instead of `auto`:

```
asset image logo = png("art/logo.png") {
    palette: [ $0F, $30, $21, $11,
               $0F, $30, $27, $17,
               $0F, $30, $2A, $1A,
               $0F, $30, $16, $06 ]
}
```

### 8.2 CHR ROM

```
chrrom bank 0 { logo.tiles }
chrrom bank 1 { font.tiles, sprites.tiles }
```

### 8.3 Music

```
asset music song = fam("music/demo.fms") {
    driver: famistudio
    channels: 2a03
}
```

FamiStudio's sound engine is integrated rather than reimplemented — it is mature,
well tested, and the tool demo musicians already use. Raster links its driver, calls
its init/update entry points, and — importantly — exposes its playback position to
the timeline (§10).

### 8.4 Raw data and text

```
asset blob table = bin("data/sine.bin")
charmap ascii { 'A'..'Z' => 0, '0'..'9' => 26, ' ' => 36 }
```

---

## 9. Hardware access

### 9.1 Named registers

```
ppu.ctrl   = $80
ppu.mask   = $1E
ppu.status                      // read
ppu.addr   = $3F00
ppu.data   = $21
ppu.scroll = x
oam.addr   = 0
oam.dma    = $02

apu.pulse1.volume = $BF
mmc3.bank_select  = 6
mmc3.bank_data    = 0
```

Named access exists so the compiler understands hardware side effects — write
ordering, the `$2005`/`$2006` shared latch, `$2002` clearing that latch on read. The
compiler warns when a sequence violates a known hardware requirement.

### 9.2 Raw access

```
poke $4011, 64
var v: u8 = peek($2002)
```

`poke`/`peek` are always exactly one store/load — never reordered, never elided.

### 9.3 Convenience aliases

```
backdrop = $0F                  // expands to the $3F00 write sequence
```

`backdrop` is a virtual register. Assigning to it during forced blank compiles to a
simple `$2006`/`$2007` sequence; assigning during rendering compiles to the
scroll-safe variant that restores `$2005`/`$2006` afterwards, and the compiler will
tell you that the safe version costs more and may not fit hblank.

### 9.4 The reset map is yours after reset

Reset programs all eight MMC3 bank registers: R0-R5 give a flat 8 KiB CHR map
with pattern table 0 at PPU $0000, and R6/R7 give a linear 32 KiB PRG map with
your code in the fixed bank at $E000.

Nothing preserves that afterwards. Repointing a window with `mmc3.bank_select`
and `mmc3.bank_data` is what those registers are for, and repointing a CHR
window compiles silently. Two things do not. Bits 6 and 7 of a bank select are
PRG mode and CHR A12 inversion, and they reinterpret every bank register at
once, from whichever select was written last — so rasterc warns when it can see
one of them set, and warns when it cannot see the value at all. And R6 and R7
are the two 8 KiB PRG windows, so a `mmc3.bank_data` write that lands on either
one retires the linear map — rasterc warns there too, and says so more quietly
when it cannot tell which register a write lands on. A bank data write with no
select before it lands on R7, because R7 is what reset selected last.

Nothing runs between two of `main`'s statements. Reset masks IRQs with `SEI`
before your program starts, and the `CLI` that arms an IRQ chain is emitted
after `main`'s last statement — a `timed` frame arms no interrupt at all. NMI
is the one interrupt the flag cannot mask, and `$FFFA` points at an `RTI` in
the runtime for every program this release builds, so enabling NMI through
`ppu.ctrl` runs none of your code. A `mmc3.bank_select` and the `mmc3.bank_data`
write that follows it are therefore always consecutive.

Handlers are a different matter. They run in schedule order and each leaves its
selection behind for the next, so rasterc treats every handler as starting with
no register it can name.

**rasterc today:** as designed.

### 9.5 Which registers read and which accept writes

Sixteen registers have names. Three of them can be read, and fifteen of
them can be written; `ppu.status` is the only one that can be read and not
written.

| Register | Address | A read of it | A write of it |
|---|---|---|---|
| `ppu.ctrl` | `$2000` | refused | **allowed** |
| `ppu.mask` | `$2001` | refused | **allowed** |
| `ppu.status` | `$2002` | **allowed** | refused |
| `ppu.oam_addr` | `$2003` | refused | **allowed** |
| `ppu.oam_data` | `$2004` | **allowed** | **allowed** |
| `ppu.scroll` | `$2005` | refused | **allowed** |
| `ppu.addr` | `$2006` | refused | **allowed** |
| `ppu.data` | `$2007` | **allowed, and buffered** | **allowed** |
| `mmc3.bank_select` | `$8000` | refused | **allowed** |
| `mmc3.bank_data` | `$8001` | refused | **allowed** |
| `mmc3.mirroring` | `$A000` | refused | **allowed** |
| `mmc3.ram_protect` | `$A001` | refused | **allowed** |
| `mmc3.irq_latch` | `$C000` | refused | **allowed** |
| `mmc3.irq_reload` | `$C001` | refused | **allowed** |
| `mmc3.irq_disable` | `$E000` | refused | **allowed** |
| `mmc3.irq_enable` | `$E001` | refused | **allowed** |

The thirteen that cannot be read are write-only ports. A read of one does
not return the last value written: a PPU port returns whatever was last on
the PPU's data bus, and an MMC3 port returns a byte of your own program,
from the PRG bank the mapper has at that address. There is no value that
makes such a read correct, so the compiler refuses it rather than warning
about it. `ppu.status` is the mirror of that: `$2002` is a status port the
CPU can only read, the PPU ignores a store to it completely, and a write is
refused for the same reason.

That includes the read or write a compound assignment makes for you.
`ppu.mask += $18` reads `$2001` before it writes, and `ppu.status += 1`
writes `$2002` after it reads; both are refused. Keep what you wrote in a
variable of your own and write the whole value.

`ppu.data` reads back, but not the byte you just addressed. A $2007
read hands you the byte the previous read fetched and loads the byte
at the current address for the next one, so a single read gives you
the wrong byte and rasterc warns about it. Read it twice in a row —
discard the first, keep the second — and rasterc is silent. Palette
addresses, $3F00 to $3FFF, are not buffered and read back at once.

A compound assignment is refused outright: `ppu.data += 1` reads
$2007 for you, and there is no way to prime a read that happens
inside a statement.

---

## 10. Timeline

Where `frame` handles vertical position within one frame, `timeline` handles
progression across the whole production, driven by music position.

```
timeline demo {
    part intro   from row   0 to row 128
    part plasma  from row 128 to row 384 { fade in over 16 rows }
    part tunnel  from row 384 to row 640
    part greets  from row 640 to end

    on beat        { flash() }
    on row 512     { trigger_shake() }
}
```

`row` refers to the FamiStudio song's row counter, so effect scheduling is bound to
the music rather than to a manually maintained frame count. Each `part` names a
function; the compiler generates the dispatch and the transition logic.

---

## 11. Inline assembly

```
asm fn upload_column(src: ^rom u8) -> void
    employs(vram_state)
    cycles(<= 1400)
{
    ldy #0
.loop:
    lda (src), y
    sta $2007
    iny
    cpy #30
    bne .loop
    rts
}
```

Rules:

- `employs(...)` names the variable groups the block reads or writes, so the
  optimizer can reason about it instead of treating it as an opaque barrier. This
  idea is taken directly from NESFab, which gets it right.
- A `cycles` annotation on an `asm fn` is **verified**, not trusted — the compiler
  counts the instructions itself. An `asm fn` without an annotation may not be called
  from inside a timed block.
- Labels beginning with `.` are local to the block.
- Parameters and locals are referable by name; the compiler substitutes the allocated
  address.
- `unsafe` before the block disables all checking, for the cases where you genuinely
  know better. It is deliberately ugly.

---

## 12. Banking

Banking is automatic by default — the compiler assigns functions and data to banks
and inserts switches — but unlike NESFab, the automation can be scoped out where it
would interfere with an effect:

```
bank fixed {                    // this code lives in the fixed bank, never switched
    fn irq_handler() { ... }
}

bank manual chr_anim {          // compiler does not touch CHR banking in here
    cycles(<= 28) {
        mmc3.bank_select = 0
        mmc3.bank_data   = frame_tile[i]
    }
}
```

On MMC3 the `$E000-$FFFF` window is fixed and holds the interrupt vectors and core
runtime; `bank fixed` places code there. CHR bank registers used by a `frame` are
reserved from automatic allocation so the compiler cannot clobber an animation
schedule.

---

## 13. Complete example — the MVP demo

This is the program the MVP must compile — not one that compiles today. It parses, and
a test in `raster-syntax` reads it out of this document and keeps it parsing;
everything past the parser is still ahead of the compiler, including arrays, indexed
reads, a call inside a handler and `using irq`. For a program that compiles now, see
`examples/mvp/demo.raster`.

```
target nes {
    mapper: mmc3
    prg:    128K
    chr:    128K
    mirror: horizontal
    region: ntsc
}

asset image picture = png("art/picture.png") {
    kind:    background
    palette: auto(4)
    dedup:   true
}

chrrom bank 0 { picture.tiles }

group raster {
    var bars: [240]u8 in bss
}

fn init() {
    ppu.ctrl = $00
    ppu.mask = $00
    wait vblank
    wait vblank

    load_palette(picture.palette)
    load_nametable(picture.nametable, picture.attributes)

    for i in 0..240 {
        raster.bars[i] = sine_table[i] & $E0
    }

    ppu.ctrl = $88          // NMI on, BG pattern table at $0000
    ppu.mask = $1E          // rendering on
}

frame main using irq {
    every 1 scanlines from 0 to 239 {
        cycles(<= 28) pad {
            ppu.mask = $1E | raster.bars[line]     // emphasis bits per scanline
        }
    }
}

main {
    init()
    loop {
        wait vblank
        sync exact
    }
}
```

See the companion MVP plan document.

---

## 14. Diagnostics

Error messages are part of the language design, not an afterthought. The intended
shape:

```
error: timed block exceeds its budget
  --> demo.raster:41:9
   |
41 |         cycles(<= 28) pad {
   |         ^^^^^^^^^^^^^^^^^ block costs 34 cycles, budget is 28
   |
   = note: hblank on NTSC is 85 PPU dots = 28 CPU cycles
   = note: the indexed load `raster.bars[line]` crosses a page boundary (+1 cycle);
           adding `align 256` to `raster.bars` would remove it
   = note: 5 cycles are the scroll restore required because rendering is enabled
```

The compiler should always say *what it cost*, *what the budget was*, and *where the
cycles went*.

A diagnostic is an `error`, which fails the build, or a `warning`, which does
not. A warning names something that has stopped being true rather than
something that is forbidden: the build still produces a ROM, and the summary
counts the warnings it printed.

---

## 15. Open design questions

1. **Does `cycles` compose across function boundaries** in a way that avoids
   annotating every leaf function? Some form of inferred cost propagation is probably
   needed, with annotations only at the points the author cares about.
2. **How is DMC DMA cycle theft modelled?** The DMC channel steals 4 cycles at
   unpredictable times, which can break any exact timing. Options: forbid DMC inside
   timed blocks, budget worst-case, or provide a `dmc_safe` mode.
3. **What is the story for PAL?** Different scanline counts, different cycle ratio
   (3.2 CPU cycles per dot rather than 3). Probably a target flag that changes all
   budget constants, with `region: dual` a later ambition.
4. **How much of the language is available outside timed blocks?** The temptation is
   to grow a full general-purpose language; the discipline is to stay small.
5. **Should the timeline be an interpreter or generated code?** An interpreted
   bytecode timeline is smaller and easier to author; generated code is faster.

---

## 16. Grammar sketch

*Informal EBNF for the constructs defined above. The MVP parser now exists and is the
authority where the two differ. Two differences are known: a cycle bound is any
compile-time constant expression rather than a bare `number`, and `interruptible` is a
contextual identifier rather than a reserved keyword — §3's keyword list is correct in
omitting it.*

```
program     = { item } ;
item        = target | import | const | var | group | fn | asmfn
            | frame | timeline | asset | chrrom | charmap | bank | main ;

target      = "target" "nes" "{" { key ":" value } "}" ;
asset       = "asset" kind ident "=" source "(" string ")" [ "{" { key ":" value } "}" ] ;
fn          = "fn" ident "(" [ params ] ")" [ "->" type ] { modifier } block ;
asmfn       = [ "unsafe" ] "asm" "fn" ident "(" [ params ] ")" [ "->" type ]
              { "employs" "(" idents ")" | cyclespec } asmblock ;
modifier    = cyclespec | "in" ident ;
cyclespec   = "cycles" "(" ( expr | "<=" expr | "?" ) ")" [ "pad" ] [ "interruptible" ] ;
frame       = "frame" ident [ "using" strategy ] "{" { event } "}" ;
event       = "at" ( "scanline" expr | "vblank" ) block
            | "every" expr "scanlines" "from" expr "to" expr block ;
timeline    = "timeline" ident "{" { part | on } "}" ;
part        = "part" ident "from" pos "to" pos [ block ] ;
on          = "on" ( "beat" | "row" expr ) block ;
stmt        = let | assign | if | while | for | match | loop | wait | sync
            | "break" | "continue" | "return" [ expr ] | cyclesblock | expr ;
cyclesblock = cyclespec [ ident ] block ;
```

---

## Appendix A — NTSC timing constants

| Quantity | Value |
|---|---|
| CPU frequency | 1.789773 MHz |
| PPU dots per CPU cycle | 3 |
| Dots per scanline | 341 |
| CPU cycles per scanline | 113.667 |
| Visible scanlines | 0–239 |
| Post-render / vblank / pre-render | 240 / 241–260 / 261 |
| Vblank duration | ~2,273 CPU cycles |
| Hblank (dots 256–340) | 85 dots ≈ 28 CPU cycles |
| Frame length | 29,780.5 CPU cycles |

## Appendix B — References

- [NESdev Wiki: Cycle counting](https://www.nesdev.org/wiki/Cycle_counting)
- [NESdev Wiki: PPU rendering](https://www.nesdev.org/wiki/PPU_rendering)
- [NESdev Wiki: MMC3](https://www.nesdev.org/wiki/MMC3)
- [NESdev Wiki: Programming MMC3](https://www.nesdev.org/wiki/Programming_MMC3)
- [NESFab documentation](https://pubby.games/nesfab/doc.html) — prior art for assets,
  banking, and the `employs` idea
- [FamiStudio Sound Engine](https://famistudio.org/doc/soundengine/)
