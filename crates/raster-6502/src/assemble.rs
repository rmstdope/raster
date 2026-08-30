use crate::{opcode::definition, AddressingMode, Instruction};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssembleError {
    UndocumentedOpcode {
        opcode: u8,
    },
    AddressingModeMismatch {
        opcode: u8,
        expected: AddressingMode,
        actual: AddressingMode,
    },
    MissingOperand {
        opcode: u8,
    },
    UnexpectedOperand {
        opcode: u8,
    },
    OperandOutOfRange {
        opcode: u8,
        operand: u16,
    },
}

pub fn assemble(instructions: &[Instruction], legal_isa: bool) -> Result<Vec<u8>, AssembleError> {
    let mut bytes = Vec::new();

    for instruction in instructions {
        let definition = definition(instruction.opcode);
        if legal_isa && !definition.official {
            return Err(AssembleError::UndocumentedOpcode {
                opcode: instruction.opcode,
            });
        }
        if instruction.mode != definition.mode {
            return Err(AssembleError::AddressingModeMismatch {
                opcode: instruction.opcode,
                expected: definition.mode,
                actual: instruction.mode,
            });
        }

        let operand_bytes = definition.mode.operand_bytes();
        match (operand_bytes, instruction.operand) {
            (0, Some(_)) => {
                return Err(AssembleError::UnexpectedOperand {
                    opcode: instruction.opcode,
                });
            }
            (0, None) => bytes.push(instruction.opcode),
            (1, None) | (2, None) => {
                return Err(AssembleError::MissingOperand {
                    opcode: instruction.opcode,
                });
            }
            (1, Some(operand)) if operand > u8::MAX.into() => {
                return Err(AssembleError::OperandOutOfRange {
                    opcode: instruction.opcode,
                    operand,
                });
            }
            (1, Some(operand)) => {
                bytes.extend([instruction.opcode, operand as u8]);
            }
            (2, Some(operand)) => {
                bytes.push(instruction.opcode);
                bytes.extend(operand.to_le_bytes());
            }
            _ => unreachable!("6502 addressing modes use at most two operand bytes"),
        }
    }

    Ok(bytes)
}
