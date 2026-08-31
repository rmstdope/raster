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
