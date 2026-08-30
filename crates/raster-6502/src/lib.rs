mod assemble;
mod opcode;

pub use assemble::{assemble, AssembleError};
pub use opcode::{opcode, Opcode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressingMode {
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Relative,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    Indirect,
    IndexedIndirect,
    IndirectIndexed,
}

impl AddressingMode {
    pub(crate) const fn operand_bytes(self) -> u8 {
        match self {
            Self::Implied | Self::Accumulator => 0,
            Self::Immediate
            | Self::ZeroPage
            | Self::ZeroPageX
            | Self::ZeroPageY
            | Self::Relative
            | Self::IndexedIndirect
            | Self::IndirectIndexed => 1,
            Self::Absolute | Self::AbsoluteX | Self::AbsoluteY | Self::Indirect => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub opcode: u8,
    pub mode: AddressingMode,
    pub operand: Option<u16>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CycleContext {
    /// Instruction indices that incurred the extra cycle for an indexed read crossing a page.
    pub indexed_read_page_crossings: Vec<usize>,
    /// Instruction indices for branches whose condition evaluated to true.
    pub taken_branches: Vec<usize>,
    /// Instruction indices for taken branches whose target crosses a page boundary.
    pub branch_page_crossings: Vec<usize>,
}

pub fn cycles(instructions: &[Instruction], context: CycleContext) -> u32 {
    instructions
        .iter()
        .enumerate()
        .map(|(index, instruction)| {
            let definition = opcode::definition(instruction.opcode);
            let mut instruction_cycles = u32::from(definition.base_cycles);

            if definition.indexed_read_page_penalty
                && context.indexed_read_page_crossings.contains(&index)
            {
                instruction_cycles += 1;
            }

            if definition.mode == AddressingMode::Relative
                && context.taken_branches.contains(&index)
            {
                instruction_cycles += 1;
                if context.branch_page_crossings.contains(&index) {
                    instruction_cycles += 1;
                }
            }

            instruction_cycles
        })
        .sum()
}
