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
    OverBudget { measured_cycles: u32, budget: u32 },
    /// The region costs less than an exact budget requires and does not carry `pad`.
    UnderBudget { measured_cycles: u32, budget: u32 },
    /// No sequence of filler instructions costs exactly this many cycles.
    UnreachablePadding { remaining: u32 },
    /// A delay of fewer than two cycles has no instruction short enough to spend it.
    DelayTooShort { requested_cycles: u32 },
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
pub fn analyze(region: &TimedRegion, legal_isa: bool) -> Result<TimingReport, TimingError> {
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
    cycles(instructions, context)
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
