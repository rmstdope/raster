//! Cycle analysis for Raster's timed regions.

use raster_6502::{cycles, opcode, AddressingMode, CycleContext, Instruction};

/// The budget a timed region must satisfy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CycleConstraint {
    /// The region must cost exactly this many cycles.
    Exact(u32),
    /// The region must cost at most this many cycles.
    AtMost(u32),
    /// The region carries no budget; its measured cost is reported under this label.
    Report { label: String },
}

/// A region of generated code whose cycle cost the compiler must account for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimedRegion {
    pub constraint: CycleConstraint,
    pub pad: bool,
    pub interruptible: bool,
    pub instructions: Vec<Instruction>,
}

/// What analysing a region established.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimingReport {
    pub label: Option<String>,
    pub measured_cycles: u32,
    pub padding: Vec<Instruction>,
}

/// Why a region could not be given a provable cost.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimingError {
    /// The region costs more than its budget permits.
    OverBudget {
        measured_cycles: u32,
        budget: u32,
        /// Of `measured_cycles`, the part that is OAM DMA stall. Zero when the
        /// region holds none.
        oam_dma_cycles: u32,
    },
    /// The region costs less than an exact budget requires and does not carry `pad`.
    UnderBudget { measured_cycles: u32, budget: u32 },
    /// No sequence of filler instructions costs exactly this many cycles.
    UnreachablePadding { remaining: u32 },
    /// A delay of fewer than two cycles has no instruction short enough to spend it.
    DelayTooShort { requested_cycles: u32 },
    /// A region contains an instruction that leaves the straight line, so summing its instructions
    /// is not its cost.
    ControlFlowInRegion { index: usize, opcode: u8 },
    /// An `irq` handler's body outruns the hblank the MMC3 leaves it.
    ///
    /// Distinct from [`TimingError::OverBudget`] because the window is not a budget the author
    /// wrote, the advice is different, and the numbers mean different things: a handler is over
    /// when its *body* is, and its prologue and epilogue are not charged.
    IrqHandlerOverHblank { measured_cycles: u32, budget: u32 },
}

/// `NOP`: two cycles in one byte, and the only official instruction with no effect at all.
const NOP: u8 = 0xea;
/// `STA $00`: three cycles in two bytes. It writes below the compiler's own zero-page allocations,
/// which start at `$10`, and touches no register and no flag — the official odd-cycle filler has to
/// be as invisible as `NOP`, and `BIT $00` would have clobbered N, V and Z.
const STA_ZERO_PAGE: u8 = 0x85;
/// Undocumented `NOP $00`: three cycles in two bytes, with no effect whatsoever.
const NOP_ZERO_PAGE: u8 = 0x04;
/// Undocumented `NOP $00,X`: four cycles in two bytes, with no effect whatsoever.
const NOP_ZERO_PAGE_X: u8 = 0x14;

/// `BRK`, which pushes the program counter and vectors through `$FFFE`.
const BRK: u8 = 0x00;
/// `JSR $nnnn`.
const JSR: u8 = 0x20;
/// `RTI`.
const RTI: u8 = 0x40;
/// `JMP $nnnn`.
const JMP_ABSOLUTE: u8 = 0x4c;
/// `RTS`.
const RTS: u8 = 0x60;
/// `JMP ($nnnn)`.
const JMP_INDIRECT: u8 = 0x6c;

/// Whether this instruction can put the program counter anywhere but the next instruction.
///
/// Every branch — each of which is the one [`AddressingMode::Relative`] mode — and every
/// instruction that sets the program counter outright. This is the whole of what a flat cost model
/// cannot charge, so [`analyze`] refuses a region containing one rather than summing it.
pub fn leaves_straight_line(instruction: &Instruction) -> bool {
    opcode(instruction.opcode).mode == AddressingMode::Relative
        || matches!(
            instruction.opcode,
            BRK | JSR | RTI | JMP_ABSOLUTE | RTS | JMP_INDIRECT
        )
}

fn filler(code: u8) -> Instruction {
    let definition = opcode(code);
    Instruction {
        opcode: code,
        mode: definition.mode,
        operand: (definition.bytes > 1).then_some(0),
    }
}

/// The shortest filler sequence costing exactly `remaining` cycles.
///
/// Every count but one is reachable: a single cycle has no instruction to spend it, and asking for
/// one is an error rather than a rounding. Under `--legal-isa` the filler is `NOP` and `BIT $00`;
/// otherwise the undocumented three- and four-cycle `NOP` forms make the sequence shorter.
pub fn synthesize_padding(
    remaining: u32,
    legal_isa: bool,
) -> Result<Vec<Instruction>, TimingError> {
    if remaining == 1 {
        return Err(TimingError::UnreachablePadding { remaining });
    }
    let mut padding = Vec::new();
    let mut twos = 0;
    let mut threes = 0;
    let mut fours = 0;
    if legal_isa {
        threes = u32::from(remaining % 2 == 1);
        twos = (remaining - threes * 3) / 2;
    } else {
        fours = remaining / 4;
        match remaining % 4 {
            0 => {}
            1 => {
                fours -= 1;
                twos = 1;
                threes = 1;
            }
            2 => twos = 1,
            _ => threes = 1,
        }
    }
    padding.extend(std::iter::repeat_n(filler(NOP_ZERO_PAGE_X), fours as usize));
    padding.extend(std::iter::repeat_n(
        filler(if legal_isa {
            STA_ZERO_PAGE
        } else {
            NOP_ZERO_PAGE
        }),
        threes as usize,
    ));
    padding.extend(std::iter::repeat_n(filler(NOP), twos as usize));
    debug_assert_eq!(
        raster_6502::cycles(&padding, CycleContext::default()),
        remaining,
        "padding costs exactly what was asked of it"
    );
    Ok(padding)
}

/// Measure a region and, where it carries `pad`, synthesize the filler that makes it exact.
///
/// The cost comes from [`raster_6502::cycles`], the one instruction cost table in the compiler.
/// A flat instruction sequence cannot prove away an indexed read's page-crossing penalty or a
/// branch's taken and page-crossing penalties, so every one of them is charged at its worst case:
/// a timed region that costs less than it is charged is a region whose budget was met early, while
/// the reverse would be a ROM that misses its raster.
///
/// A sum is only a region's cost while its instructions run one after another, and nothing else
/// checks that they do. So a region holding anything [`leaves_straight_line`] names is refused
/// outright — before the [`CycleConstraint::Report`] path, which would otherwise print a measured
/// cost that is simply false.
pub fn analyze(region: &TimedRegion, legal_isa: bool) -> Result<TimingReport, TimingError> {
    if let Some((index, instruction)) = region
        .instructions
        .iter()
        .enumerate()
        .find(|(_, instruction)| leaves_straight_line(instruction))
    {
        return Err(TimingError::ControlFlowInRegion {
            index,
            opcode: instruction.opcode,
        });
    }
    let measured_cycles = worst_case_cycles(&region.instructions);
    let budget = match &region.constraint {
        CycleConstraint::Exact(budget) | CycleConstraint::AtMost(budget) => *budget,
        CycleConstraint::Report { label } => {
            return Ok(TimingReport {
                label: Some(label.clone()),
                measured_cycles,
                padding: Vec::new(),
            })
        }
    };

    if measured_cycles > budget {
        return Err(TimingError::OverBudget {
            measured_cycles,
            budget,
            oam_dma_cycles: oam_dma_stall_cycles(&region.instructions),
        });
    }

    let short_by = budget - measured_cycles;
    let padding = if short_by == 0 {
        Vec::new()
    } else if region.pad {
        synthesize_padding(short_by, legal_isa)?
    } else if matches!(region.constraint, CycleConstraint::Exact(_)) {
        return Err(TimingError::UnderBudget {
            measured_cycles,
            budget,
        });
    } else {
        Vec::new()
    };

    Ok(TimingReport {
        label: None,
        measured_cycles,
        padding,
    })
}

/// Charge every penalty a flat instruction sequence cannot prove away.
///
/// Three of them: an indexed read that may cross a page, a branch that may be taken and cross
/// one, and an OAM DMA, whose stall is charged the worse of the two lengths it may take.
///
/// This is the cost the compiler holds a timed region to, so anything checking a region's cost
/// should call it rather than re-derive the same sum: a test that agrees with itself proves
/// nothing.
pub fn worst_case_cycles(instructions: &[Instruction]) -> u32 {
    let mut context = CycleContext::default();
    for (index, instruction) in instructions.iter().enumerate() {
        if opcode(instruction.opcode).mode == AddressingMode::Relative {
            context.taken_branches.push(index);
            context.branch_page_crossings.push(index);
        } else {
            context.indexed_read_page_crossings.push(index);
        }
    }
    cycles(instructions, context) + oam_dma_stall_cycles(instructions)
}

/// The port a write to which starts an OAM DMA: 256 bytes into sprite memory,
/// and the CPU halted while it happens.
pub const OAM_DMA_PORT: u16 = 0x4014;

/// The cycles one OAM DMA is charged.
///
/// The stall is 513 cycles, or 514 when the write lands on an odd CPU cycle.
/// Nothing in the compiler knows the parity — a region's cost is counted from
/// its own top and never from reset — so the worst of the two is charged, which
/// is the direction that meets a budget early rather than overrunning it.
pub const OAM_DMA_STALL_CYCLES: u32 = 514;

/// The cycles these instructions spend stalled in OAM DMAs, charged at the worst case.
///
/// A store to an absolute address is the only shape that can start one, and the address is the
/// whole test: this charges whatever writes `$4014`, not whatever the register table happens to
/// call it. `STX` and `STY` are listed beside `STA` because the 6502 has them even though codegen
/// emits only `STA` today — the rule is about the hardware, not about this month's code generator.
/// Indexed stores are not counted: their effective address is not in the instruction, so a flat
/// sequence cannot tell whether one lands on `$4014`.
/// `STY`, `STA` and `STX` absolute — the three stores whose target address is in the instruction
/// itself. All three are `op!(4, Absolute, true, false)` in `raster-6502`'s opcode table. Written
/// out rather than as the range they happen to form, because the list is three named instructions
/// and not an interval.
const ABSOLUTE_STORES: [u8; 3] = [0x8c, 0x8d, 0x8e];

pub fn oam_dma_stall_cycles(instructions: &[Instruction]) -> u32 {
    instructions
        .iter()
        .filter(|instruction| {
            ABSOLUTE_STORES.contains(&instruction.opcode)
                && instruction.operand == Some(OAM_DMA_PORT)
        })
        .count() as u32
        * OAM_DMA_STALL_CYCLES
}

/// PPU dots in one NTSC scanline — spec Appendix A.
pub const DOTS_PER_SCANLINE: u32 = 341;
/// PPU dots the console draws while the CPU spends one cycle.
pub const DOTS_PER_CPU_CYCLE: u32 = 3;
/// Scanlines from the start of vblank to the top of the next visible picture: the twenty vblank
/// lines 241-260 and the pre-render line 261.
pub const SCANLINES_FROM_VBLANK_TO_PICTURE: u32 = 21;
/// Scanlines in one NTSC frame: 240 of picture, the post-render line, twenty of vblank and the
/// pre-render line.
pub const SCANLINES_PER_FRAME: u32 = 262;
/// Frames in one pass of a timed frame loop.
///
/// One frame is 89342 dots, which is 29780 CPU cycles and two dots left over — so a loop that
/// spends one frame's worth of cycles cannot stay locked to the picture, and slides a dot a frame
/// until the bars have crossed a whole scanline. Three frames are 268026 dots, which is 89342 CPU
/// cycles exactly, so a pass of three is the shortest one a cycle-counted loop can repeat forever
/// without drifting.
///
/// **With rendering disabled**, which is every ROM this release emits. With background rendering
/// on, NTSC skips dot (339, 261) on odd frames, so frames alternate 89342 and 89341 dots and a pass
/// of three is a dot short every other time round. Spec Appendix A's 29,780.5 cycles a frame is
/// that alternation averaged; the numbers here are the rendering-off frame, which is the one the
/// compiler can currently produce. A timed frame over a rendered picture needs its own pass length
/// and its own evidence.
pub const FRAMES_PER_PASS: u32 = 3;
/// The cycles one pass of a timed frame loop spends, from its origin back to its origin.
pub const PASS_CYCLES: u32 = scanline_cycles(SCANLINES_PER_FRAME * FRAMES_PER_PASS);
/// The cycles a timed frame gives a handler that has a whole scanline to itself.
///
/// A scanline is 113.667 cycles, not 114, so a schedule of nothing but 114-cycle bodies drifts a
/// third of a cycle a line — 80 cycles, most of a scanline, over a visible picture.
/// [`plan_timed_frame`] is what spends 113 where the drift would otherwise accumulate.
pub const SCANLINE_BODY_CYCLES: u32 = 114;

/// The cycles of body a `frame ... using irq` handler has before the picture starts again.
///
/// The MMC3 asserts its interrupt on the filtered A12 rise in the sprite-fetch phase near the end
/// of the scanline *before* the one the author named — the shift [`mmc3_latch_for_delta`] and
/// [`mmc3_latch_for_first_event`] already carry. What is left of that scanline, once the CPU has
/// finished the instruction it was in, run the seven-cycle interrupt sequence and pushed the
/// accumulator, is all the room a store has before dot 0 of the next line.
///
/// **This number is measured, not derived.** The dot the rise lands on is not something this
/// compiler can read, and the arithmetic from the documented sprite-fetch window (dots 257 to 320)
/// only bounds it. It was measured by rendering handler bodies of known cost and reading the
/// column at which each store's effect appeared: a store completing at body cycle `c` lands at
/// column `3c - 27` in the latest of the six frame phases, so `c = 9` is the largest body whose
/// last cycle is certainly still inside the hblank. Three probes of different construction — a ramp
/// of visible stores, that ramp behind invisible six-cycle fillers, and the same behind five-cycle
/// ones — agreed with that model to the dot. `irq_handler_body_fits_the_hblank_it_lands_in` in
/// `crates/rasterc/tests/emulator.rs` is the executable half of the evidence, and the bead's notes
/// carry the rest.
///
/// Nine cycles is one statement: one register store is six cycles and one variable assignment is
/// five, and two of anything tears the row. That is the rule this compiler enforces, agreed with
/// the navigator once the measurement was in.
pub const IRQ_HANDLER_BODY_CYCLES: u32 = 9;

/// The CPU cycles in `scanlines` NTSC scanlines, to the nearest cycle.
///
/// 341 dots is not a whole number of CPU cycles, so this rounds; the error never exceeds half a
/// cycle however many scanlines are asked for, because the rounding is applied to the total rather
/// than accumulated line by line.
pub const fn scanline_cycles(scanlines: u32) -> u32 {
    (scanlines * DOTS_PER_SCANLINE + DOTS_PER_CPU_CYCLE / 2) / DOTS_PER_CPU_CYCLE
}

/// The cycle, counted from a pass's origin, at which `scanline` of `frame` of the pass begins.
///
/// The origin is where the frame loop's one vblank poll left the CPU, which is the start of vblank
/// give or take the poll's own granularity. The visible picture starts
/// [`SCANLINES_FROM_VBLANK_TO_PICTURE`] scanlines later, and each further frame of the pass a whole
/// frame after that.
pub const fn scanline_origin_cycles(frame: u32, scanline: u32) -> u32 {
    scanline_cycles(frame * SCANLINES_PER_FRAME + SCANLINES_FROM_VBLANK_TO_PICTURE + scanline)
}

/// Where one handler of a timed frame sits: what to spend before it, and what it must cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledHandler {
    /// Cycles to spend doing nothing before the handler starts. Zero when the previous handler
    /// runs into this one, which is what back-to-back scanlines do.
    pub delay_cycles: u32,
    /// The exact cost the handler is padded to.
    pub budget_cycles: u32,
}

/// One pass of a timed frame loop: every handler of every frame in it, and the tail that closes it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimedFramePass {
    /// The handlers in the order they run — the whole schedule, once per frame of the pass.
    pub handlers: Vec<ScheduledHandler>,
    /// Cycles between the last handler and the top of the pass.
    pub trailing_delay_cycles: u32,
}

/// Place every handler of a timed frame, given the scanlines its schedule is written on.
///
/// `scanlines` must be sorted and free of duplicates, which is what `raster-ir` hands over. Each
/// handler starts exactly where its scanline does, so the frame's accumulated budget is the
/// picture's own and not a sum of roundings: a handler is padded to a whole scanline body where the
/// next one is far enough away, and to the exact distance to the next where it is not.
///
/// The schedule is placed once per frame of the pass rather than once, because only a whole pass is
/// a whole number of CPU cycles. A loop that instead re-polls `$2002` each frame drifts across the
/// dot where the vblank flag is set, and a read landing in the two cycles either side of it returns
/// the flag clear and suppresses it — costing that frame its entire schedule. Measured on this
/// project before the pass existed: one frame in nine lost its bars.
///
/// `closing_cycles` is what the caller spends getting back to the top of the pass — the jump that
/// closes the loop. It comes out of the pass's own budget, because a pass that pays for its
/// schedule and then spends three cycles more is three cycles longer than the picture it is
/// tracking, which is a dot of drift a frame and a scanline of it every three seconds. Measured.
pub fn plan_timed_frame(scanlines: &[u32], closing_cycles: u32) -> TimedFramePass {
    debug_assert!(
        scanlines.windows(2).all(|pair| pair[0] < pair[1]),
        "a frame schedule is sorted and has one handler per scanline"
    );
    let mut handlers = Vec::with_capacity(scanlines.len() * FRAMES_PER_PASS as usize);
    let mut position = 0;
    for frame in 0..FRAMES_PER_PASS {
        for (index, &scanline) in scanlines.iter().enumerate() {
            let start = scanline_origin_cycles(frame, scanline);
            let next = scanlines
                .get(index + 1)
                .map(|&next| scanline_origin_cycles(frame, next))
                .or_else(|| {
                    scanlines
                        .first()
                        .filter(|_| frame + 1 < FRAMES_PER_PASS)
                        .map(|&first| scanline_origin_cycles(frame + 1, first))
                });
            let distance_to_next = next.map_or(SCANLINE_BODY_CYCLES, |next| next - start);
            // Widened to reach the next handler rather than left a cycle short of it: a gap of
            // one cycle has no instruction short enough to spend it. Consecutive scanlines are 113
            // or 114 apart and the next frame's first handler is at least 23 scanlines away, so
            // the arithmetic never produces a gap of exactly one — the bound is what keeps that
            // true of the rule rather than of the numbers that happen to reach it.
            let budget_cycles = if distance_to_next <= SCANLINE_BODY_CYCLES + 1 {
                distance_to_next
            } else {
                SCANLINE_BODY_CYCLES
            };
            handlers.push(ScheduledHandler {
                delay_cycles: start - position,
                budget_cycles,
            });
            position = start + budget_cycles;
        }
    }
    TimedFramePass {
        handlers,
        trailing_delay_cycles: PASS_CYCLES - position - closing_cycles,
    }
}

/// The most iterations one counted loop runs: `LDX #$00` decrements to 255 first.
pub const MAX_ITERATIONS: u32 = 256;

/// One piece of a synthesized delay.
///
/// A delay is not a flat instruction sequence, because its loops close on a `JMP` and only the
/// caller knows addresses. So the plan says what to emit and the caller — codegen, which owns
/// labels and relocations — builds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DelayStep {
    /// Filler to emit exactly as it is.
    Filler(Vec<Instruction>),
    /// A counted loop over X, optionally wrapped in one over Y.
    ///
    /// ```text
    ///     LDY #outer          ; only when `outer` is set
    /// outer_loop:
    ///     LDX #inner
    /// inner_loop:
    ///     DEX
    ///     BEQ inner_done      ; taken once, on the last pass
    ///     JMP inner_loop      ; three cycles, and no branch penalty to prove away
    /// inner_done:
    ///     DEY                 ; only when `outer` is set
    ///     BEQ outer_done
    ///     JMP outer_loop
    /// outer_done:
    /// ```
    ///
    /// The `JMP` back is what makes this exact. A `DEX`/`BNE` loop takes its branch on every pass
    /// but the last, and a taken branch costs an extra cycle when it crosses a page — so a
    /// thousand-cycle delay could spend twelve hundred depending on where the linker put it.
    /// Here exactly one branch is ever taken, so at worst one cycle is unaccounted for, and
    /// removing even that needs an alignment the linker cannot yet express.
    Loop { outer: Option<u32>, inner: u32 },
}

/// `LDX #n` plus `n` passes of `DEX`/`BEQ`/`JMP`, the last of which takes the branch out.
const fn inner_loop_cycles(iterations: u32) -> u32 {
    7 * iterations
}

/// `LDY #m` plus `m` passes of [`inner_loop_cycles`] followed by `DEY`/`BEQ`/`JMP`.
const fn nested_loop_cycles(outer: u32, inner: u32) -> u32 {
    7 * outer * (inner + 1)
}

/// What a planned delay costs when it runs, assuming no branch crosses a page.
pub fn delay_cycles(steps: &[DelayStep]) -> u32 {
    steps
        .iter()
        .map(|step| match step {
            DelayStep::Filler(instructions) => cycles(instructions, CycleContext::default()),
            DelayStep::Loop {
                outer: Some(outer),
                inner,
            } => nested_loop_cycles(*outer, *inner),
            DelayStep::Loop { outer: None, inner } => inner_loop_cycles(*inner),
        })
        .sum()
}

/// A plan that spends exactly `requested_cycles` cycles doing nothing.
///
/// The bulk goes into counted loops whose iteration counts are constants, so the cost is proven
/// rather than measured, and the remainder into the same filler [`synthesize_padding`] emits. Every
/// count from two upwards is reachable in a handful of instructions; one cycle is not, because no
/// instruction is that short.
///
/// The loops clobber X, and a delay long enough to need the outer loop clobbers Y as well.
pub fn plan_delay(requested_cycles: u32, legal_isa: bool) -> Result<Vec<DelayStep>, TimingError> {
    if requested_cycles < 2 {
        return Err(TimingError::DelayTooShort { requested_cycles });
    }

    let mut steps = Vec::new();
    let mut remaining = requested_cycles;
    let per_outer_pass = nested_loop_cycles(1, MAX_ITERATIONS) - inner_loop_cycles(0);

    while remaining > inner_loop_cycles(MAX_ITERATIONS) {
        let mut outer = (remaining / per_outer_pass).min(MAX_ITERATIONS);
        // A single leftover cycle has no filler, so give one pass back to reach a reachable tail.
        if outer >= 1 && remaining - nested_loop_cycles(outer, MAX_ITERATIONS) == 1 {
            outer -= 1;
        }
        if outer == 0 {
            break;
        }
        steps.push(DelayStep::Loop {
            outer: Some(outer),
            inner: MAX_ITERATIONS,
        });
        remaining -= nested_loop_cycles(outer, MAX_ITERATIONS);
    }

    if remaining >= inner_loop_cycles(1) {
        let mut inner = (remaining / 7).min(MAX_ITERATIONS);
        if inner >= 1 && remaining - inner_loop_cycles(inner) == 1 {
            inner -= 1;
        }
        if inner >= 1 {
            steps.push(DelayStep::Loop { outer: None, inner });
            remaining -= inner_loop_cycles(inner);
        }
    }

    let filler = synthesize_padding(remaining, legal_isa)?;
    if !filler.is_empty() {
        steps.push(DelayStep::Filler(filler));
    }
    Ok(steps)
}

/// Scanlines the MMC3 counter can wait, which is one latch's worth of A12 rises plus the rise the
/// reload itself lands on. The latch is a single byte, so nothing further apart than this can be
/// asked of one link of a chain.
pub const MAX_IRQ_DELTA_SCANLINES: u16 = 256;

/// The `$C000` latch that places the next MMC3 IRQ `scanlines` scanlines after the one being
/// handled.
///
/// The counter does not reload where the handler writes: `$C001` only sets the reload flag, and the
/// flag is honoured on the *next* filtered A12 rise — one scanline later. From there the counter
/// decrements once a scanline and the IRQ is asserted on the rise where it reaches zero, so a latch
/// of `n` fires `n + 1` scanlines after the IRQ that programmed it. That is the off-by-one spec
/// section 7.3 says the compiler carries for the author.
///
/// `scanlines` is a real distance between two events, so it is at least one and at most
/// [`MAX_IRQ_DELTA_SCANLINES`]. Nothing in this crate enforces that — what bounds it is a crate
/// away, in `raster-ir`: an event's scanline is refused unless it is a visible one (0 to 239), and
/// two events on the same scanline are refused as well, so no delta a chain can ask for exceeds
/// 241. The `debug_assert!` below is the guard for a caller that stops being true of, and the
/// `clamp` keeps a release build from producing a latch out of thin air.
pub fn mmc3_latch_for_delta(scanlines: u16) -> u8 {
    debug_assert!(
        (1..=MAX_IRQ_DELTA_SCANLINES).contains(&scanlines),
        "a chained MMC3 IRQ is 1 to {MAX_IRQ_DELTA_SCANLINES} scanlines away, not {scanlines}"
    );
    (scanlines.clamp(1, MAX_IRQ_DELTA_SCANLINES) - 1) as u8
}

/// Scanlines the MMC3 counter sees in one frame: the 240 visible ones and the pre-render line.
///
/// The counter clocks on filtered A12 rises, which happen once per scanline the PPU renders — so
/// the post-render line and the twenty of vblank are not among them, and the counter's year is 241
/// rises long rather than 262. This is what lets one chain reach from a frame's last event to the
/// next frame's first without anything re-arming it in between.
pub const IRQ_SCANLINES_PER_FRAME: u16 = 241;

/// The `$C000` latch that carries a chain from a frame's last event to the next frame's first.
///
/// Nothing clocks the counter between the last visible scanline and the next pre-render line, so
/// the gap is not the 22 scanlines the picture spends there: it is one rise, the pre-render line's.
/// Counted that way the wrap is [`IRQ_SCANLINES_PER_FRAME`] minus the distance the schedule already
/// covered, which is at most 241 and always fits a latch.
///
/// A chain that wraps needs no per-frame re-arming, and that is the point rather than an economy:
/// re-arming means waiting on the vblank flag in `$2002`, and a read landing within two cycles of
/// the dot that sets the flag returns it clear and suppresses it. That costs the frame its whole
/// schedule — measured here at about one frame in fifty before the chain wrapped.
pub fn mmc3_latch_for_next_frame(last_scanline: u16, first_scanline: u16) -> u8 {
    debug_assert!(
        first_scanline <= last_scanline,
        "a schedule's first event is not after its last"
    );
    mmc3_latch_for_delta(IRQ_SCANLINES_PER_FRAME - last_scanline + first_scanline)
}

/// The `$C000` latch that places a frame's first MMC3 IRQ on `scanline`, armed in vblank.
///
/// Armed in vblank there is no A12 rise left in the frame, so the reload flag is honoured on the
/// first rise of the *next* frame — the pre-render line's. That is one rise earlier than the reload
/// a handler's own `$C001` write gets, and one scanline earlier than the visible picture: counting
/// from the pre-render line, an IRQ on scanline `s` is `s + 1` rises away and so needs a latch of
/// `s`.
///
/// The IRQ is asserted towards the end of the scanline it is counted to, because A12 rises during
/// that scanline's sprite fetches — so the handler counted to scanline `s - 1` is the one that runs
/// at the top of scanline `s`. Both of these functions already carry that shift: they are given the
/// picture scanline the author wrote and return the latch that runs its handler there.
pub fn mmc3_latch_for_first_event(scanline: u16) -> u8 {
    debug_assert!(
        scanline < MAX_IRQ_DELTA_SCANLINES,
        "a frame's first IRQ is at most {MAX_IRQ_DELTA_SCANLINES} scanlines from the pre-render \
         line, not {scanline}"
    );
    scanline.min(MAX_IRQ_DELTA_SCANLINES - 1) as u8
}

/// `ppu.ctrl` bit 3: the half of pattern memory sprites are fetched from.
const PPU_CTRL_SPRITE_PATTERN_HALF: u8 = 0b0000_1000;
/// `ppu.ctrl` bit 4: the half of pattern memory the background is fetched from.
const PPU_CTRL_BACKGROUND_PATTERN_HALF: u8 = 0b0001_0000;
/// `ppu.ctrl` bit 5: 8x16 sprites, which take their half from the tile index instead.
const PPU_CTRL_TALL_SPRITES: u8 = 0b0010_0000;
/// `ppu.mask` bits 3 and 4: show the background, and show the sprites. Either one is rendering.
const PPU_MASK_RENDERING: u8 = 0b0001_1000;

/// What a PPU register holds where the compiler can prove it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterState {
    /// Every store to the register before the frame runs was a constant, and this was the last.
    Known(u8),
    /// The last store was not a constant, so nothing here can be decided from it.
    Unproven,
    /// The register is written on some paths through the program and not others, so there is no
    /// single value to check — whichever arm the author wrote first.
    Conditional,
}

/// The PPU configuration a `frame ... using irq` inherits from the program that set it up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PpuConfiguration {
    pub ctrl: RegisterState,
    pub mask: RegisterState,
}

/// Why an MMC3 IRQ schedule cannot be built from this program.
///
/// Each of these is a ROM that assembles, runs, and silently never takes an interrupt — the class
/// of failure spec section 7.3 asks the compiler to catch instead of the author.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mmc3IrqError {
    /// Rendering is off, so the PPU fetches nothing and A12 never moves.
    RenderingDisabled { mask: u8 },
    /// 8x16 sprites are selected, and their half of pattern memory is not in `ppu.ctrl` to check.
    TallSpritesUncheckable { ctrl: u8 },
    /// Background and sprite patterns are in the same half of pattern memory, so a scanline's
    /// fetches never raise A12 and the counter never clocks.
    PatternTablesShareHalf { ctrl: u8 },
    /// The program's last store to this register was not a constant, so the check cannot be made.
    UnprovenConfiguration { register: &'static str },
    /// The register is written under a branch, so the program has no one configuration to check.
    ConditionalConfiguration { register: &'static str },
}

/// Check the hardware preconditions an MMC3 IRQ chain depends on.
///
/// Rendering has to be on — either half of it. The counter clocks on filtered A12 rises, and A12
/// only rises where a scanline fetches from both halves of pattern memory; a rendering PPU runs its
/// sprite pattern fetches at dots 257 to 320 whether or not sprites are composited, so a
/// background-only split clocks the counter exactly as a full one does. What is fatal is rendering
/// nothing at all, or configuring both tables into the same half.
///
/// 8x16 sprites are refused rather than judged: in that mode the hardware ignores the sprite-half
/// bit and takes the half from bit 0 of each tile index, which is not in a register this can read.
///
/// `ppu.ctrl` is the whole of what decides the two halves *today*, and that is a fact about the
/// reset runtime rather than about the MMC3: it programs CHR mode 0 with R0-R5 as 0, 2, 4, 5, 6, 7,
/// a flat 8 KiB map with no A12 inversion, and nothing else can change it while PRG/CHR bank
/// switching is unsupported. Spec section 7.3 asks for the CHR layout to be checked too; when a
/// program can choose its own, this function needs it passed in and the sentence above stops being
/// true on its own.
/// One register's value, or the reason it does not have one this can check.
fn known(state: RegisterState, register: &'static str) -> Result<u8, Mmc3IrqError> {
    match state {
        RegisterState::Known(value) => Ok(value),
        RegisterState::Unproven => Err(Mmc3IrqError::UnprovenConfiguration { register }),
        RegisterState::Conditional => Err(Mmc3IrqError::ConditionalConfiguration { register }),
    }
}

pub fn validate_mmc3_irq_frame(ppu: &PpuConfiguration) -> Result<(), Mmc3IrqError> {
    let ctrl = known(ppu.ctrl, "ppu.ctrl")?;
    let mask = known(ppu.mask, "ppu.mask")?;
    if mask & PPU_MASK_RENDERING == 0 {
        return Err(Mmc3IrqError::RenderingDisabled { mask });
    }
    if ctrl & PPU_CTRL_TALL_SPRITES != 0 {
        return Err(Mmc3IrqError::TallSpritesUncheckable { ctrl });
    }
    let background_half = ctrl & PPU_CTRL_BACKGROUND_PATTERN_HALF != 0;
    let sprite_half = ctrl & PPU_CTRL_SPRITE_PATTERN_HALF != 0;
    if background_half == sprite_half {
        return Err(Mmc3IrqError::PatternTablesShareHalf { ctrl });
    }
    Ok(())
}
