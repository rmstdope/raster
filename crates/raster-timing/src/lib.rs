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
    /// A region contains an instruction that leaves the straight line, so summing its instructions
    /// is not its cost.
    ControlFlowInRegion { index: usize, opcode: u8 },
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
