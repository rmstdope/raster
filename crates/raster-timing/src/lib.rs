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
/// `BIT $00`: three cycles in two bytes. It reads the zero page below the compiler's own
/// allocations and touches only the flags, which is why it is the official odd-cycle filler.
const BIT_ZERO_PAGE: u8 = 0x24;
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
            BIT_ZERO_PAGE
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
fn worst_case_cycles(instructions: &[Instruction]) -> u32 {
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

/// The most iterations a single `DEX`/`BNE` loop runs: `LDX #$00` decrements to 255 first.
const MAX_ITERATIONS: u32 = 256;
const LDX_IMMEDIATE: u8 = 0xa2;
const LDY_IMMEDIATE: u8 = 0xa0;
const DEX: u8 = 0xca;
const DEY: u8 = 0x88;
const BNE: u8 = 0xd0;
/// Back three bytes from the end of the `BNE`, to the `DEX` that opens the loop body.
const INNER_BRANCH_OFFSET: u16 = 0xfd;
/// Back eight bytes from the end of the outer `BNE`, to the `LDX` that reloads the inner count.
const OUTER_BRANCH_OFFSET: u16 = 0xf8;

/// `LDX #n` plus `n` passes of `DEX`/`BNE`, the last of which does not take the branch.
const fn inner_loop_cycles(iterations: u32) -> u32 {
    5 * iterations + 1
}

/// `LDY #m` plus `m` passes of [`inner_loop_cycles`] followed by `DEY`/`BNE`.
const fn nested_loop_cycles(outer: u32, inner: u32) -> u32 {
    outer * (5 * inner + 6) + 1
}

fn immediate(opcode: u8, iterations: u32) -> Instruction {
    debug_assert!(
        (1..=MAX_ITERATIONS).contains(&iterations),
        "an immediate iteration count fits a byte, with zero standing for 256"
    );
    Instruction {
        opcode,
        mode: AddressingMode::Immediate,
        operand: Some((iterations % MAX_ITERATIONS) as u16),
    }
}

fn implied(opcode: u8) -> Instruction {
    Instruction {
        opcode,
        mode: AddressingMode::Implied,
        operand: None,
    }
}

fn branch(offset: u16) -> Instruction {
    Instruction {
        opcode: BNE,
        mode: AddressingMode::Relative,
        operand: Some(offset),
    }
}

fn inner_loop(iterations: u32) -> Vec<Instruction> {
    vec![
        immediate(LDX_IMMEDIATE, iterations),
        implied(DEX),
        branch(INNER_BRANCH_OFFSET),
    ]
}

fn nested_loop(outer: u32, inner: u32) -> Vec<Instruction> {
    let mut instructions = vec![immediate(LDY_IMMEDIATE, outer)];
    instructions.extend(inner_loop(inner));
    instructions.push(implied(DEY));
    instructions.push(branch(OUTER_BRANCH_OFFSET));
    instructions
}

/// The instructions that spend exactly `requested_cycles` cycles doing nothing.
///
/// The bulk goes into nested `DEX`/`BNE` loops whose iteration counts are constants, so the cost is
/// proven rather than measured, and the remainder into the same filler [`synthesize_padding`]
/// emits. Every count from two upwards is reachable in a handful of instructions; one cycle is not,
/// because no instruction is that short.
///
/// The loops clobber X, and a delay long enough to need the outer loop clobbers Y as well.
///
/// The cost assumes each `BNE` reaches its target without crossing a page boundary, which is what
/// makes it exact: the sequence is five or eight bytes and is emitted contiguously, so the caller
/// must not split it across a page.
pub fn synthesize_delay(
    requested_cycles: u32,
    legal_isa: bool,
) -> Result<Vec<Instruction>, TimingError> {
    if requested_cycles < 2 {
        return Err(TimingError::DelayTooShort { requested_cycles });
    }

    let mut instructions = Vec::new();
    let mut remaining = requested_cycles;
    let per_outer_pass = nested_loop_cycles(1, MAX_ITERATIONS) - 1;

    while remaining > inner_loop_cycles(MAX_ITERATIONS) {
        let mut outer = ((remaining - 1) / per_outer_pass).min(MAX_ITERATIONS);
        // A single leftover cycle has no filler, so give one pass back to reach a reachable tail.
        if outer >= 1 && remaining - nested_loop_cycles(outer, MAX_ITERATIONS) == 1 {
            outer -= 1;
        }
        if outer == 0 {
            break;
        }
        instructions.extend(nested_loop(outer, MAX_ITERATIONS));
        remaining -= nested_loop_cycles(outer, MAX_ITERATIONS);
    }

    if remaining >= inner_loop_cycles(1) {
        let mut inner = ((remaining - 1) / 5).min(MAX_ITERATIONS);
        if inner >= 1 && remaining - inner_loop_cycles(inner) == 1 {
            inner -= 1;
        }
        if inner >= 1 {
            instructions.extend(inner_loop(inner));
            remaining -= inner_loop_cycles(inner);
        }
    }

    instructions.extend(synthesize_padding(remaining, legal_isa)?);
    Ok(instructions)
}
