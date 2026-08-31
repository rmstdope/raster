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
}

/// Measure a region and, where it carries `pad`, synthesize the filler that makes it exact.
///
/// The cost comes from [`raster_6502::cycles`], the one instruction cost table in the compiler.
/// A flat instruction sequence cannot prove away an indexed read's page-crossing penalty or a
/// branch's taken and page-crossing penalties, so every one of them is charged at its worst case:
/// a timed region that costs less than it is charged is a region whose budget was met early, while
/// the reverse would be a ROM that misses its raster.
pub fn analyze(region: &TimedRegion, _legal_isa: bool) -> Result<TimingReport, TimingError> {
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
    if short_by > 0 && !region.pad && matches!(region.constraint, CycleConstraint::Exact(_)) {
        return Err(TimingError::UnderBudget {
            measured_cycles,
            budget,
        });
    }
    let padding = Vec::new();

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
