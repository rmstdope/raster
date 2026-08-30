use raster_6502::{
    assemble, cycles, opcode, AddressingMode, AssembleError, CycleContext, Instruction, Opcode,
};

fn instruction(opcode: u8, mode: AddressingMode, operand: Option<u16>) -> Instruction {
    Instruction {
        opcode,
        mode,
        operand,
    }
}

fn mode(name: &str) -> AddressingMode {
    match name {
        "implied" => AddressingMode::Implied,
        "accumulator" => AddressingMode::Accumulator,
        "immediate" => AddressingMode::Immediate,
        "zero_page" => AddressingMode::ZeroPage,
        "zero_page_x" => AddressingMode::ZeroPageX,
        "zero_page_y" => AddressingMode::ZeroPageY,
        "relative" => AddressingMode::Relative,
        "absolute" => AddressingMode::Absolute,
        "absolute_x" => AddressingMode::AbsoluteX,
        "absolute_y" => AddressingMode::AbsoluteY,
        "indirect" => AddressingMode::Indirect,
        "indexed_indirect" => AddressingMode::IndexedIndirect,
        "indirect_indexed" => AddressingMode::IndirectIndexed,
        _ => panic!("unknown addressing mode {name}"),
    }
}

#[test]
fn all_256_opcode_entries_match_the_independent_reference() {
    let entries: Vec<_> = include_str!("fixtures/opcodes.csv")
        .lines()
        .skip(1)
        .map(|line| {
            let mut fields = line.split(',');
            let code = u8::from_str_radix(fields.next().unwrap(), 16).unwrap();
            let bytes = fields.next().unwrap().parse().unwrap();
            let base_cycles = fields.next().unwrap().parse().unwrap();
            let mode = mode(fields.next().unwrap());
            let official = fields.next().unwrap().parse().unwrap();
            assert!(
                fields.next().is_none(),
                "unexpected fixture field in {line}"
            );
            Opcode {
                code,
                bytes,
                base_cycles,
                mode,
                official,
            }
        })
        .collect();

    assert_eq!(entries.len(), 256);
    for (byte, expected) in entries.into_iter().enumerate() {
        assert_eq!(expected.code, byte as u8);
        assert_eq!(opcode(byte as u8), expected, "opcode {byte:02x}");
    }
}

#[test]
fn assembler_emits_reference_bytes_for_every_addressing_mode() {
    use AddressingMode::{
        Absolute, AbsoluteX, AbsoluteY, Accumulator, Immediate, Implied, IndexedIndirect, Indirect,
        IndirectIndexed, Relative, ZeroPage, ZeroPageX, ZeroPageY,
    };

    let instructions = [
        instruction(0x78, Implied, None),
        instruction(0x0a, Accumulator, None),
        instruction(0xa9, Immediate, Some(0xab)),
        instruction(0xa5, ZeroPage, Some(0x12)),
        instruction(0xb5, ZeroPageX, Some(0x34)),
        instruction(0xb6, ZeroPageY, Some(0x56)),
        instruction(0xd0, Relative, Some(0xfc)),
        instruction(0xad, Absolute, Some(0x1234)),
        instruction(0xbd, AbsoluteX, Some(0x5678)),
        instruction(0xb9, AbsoluteY, Some(0x9abc)),
        instruction(0x6c, Indirect, Some(0xdef0)),
        instruction(0xa1, IndexedIndirect, Some(0x98)),
        instruction(0xb1, IndirectIndexed, Some(0x76)),
    ];

    assert_eq!(
        assemble(&instructions, true),
        Ok(vec![
            0x78, 0x0a, 0xa9, 0xab, 0xa5, 0x12, 0xb5, 0x34, 0xb6, 0x56, 0xd0, 0xfc, 0xad, 0x34,
            0x12, 0xbd, 0x78, 0x56, 0xb9, 0xbc, 0x9a, 0x6c, 0xf0, 0xde, 0xa1, 0x98, 0xb1, 0x76,
        ])
    );
}

#[test]
fn assembler_validates_operands_modes_and_legal_isa() {
    use AddressingMode::{Immediate, Implied, ZeroPage};

    assert_eq!(
        assemble(&[instruction(0xa9, Immediate, None)], true),
        Err(AssembleError::MissingOperand { opcode: 0xa9 })
    );
    assert_eq!(
        assemble(&[instruction(0xea, Implied, Some(0))], true),
        Err(AssembleError::UnexpectedOperand { opcode: 0xea })
    );
    assert_eq!(
        assemble(&[instruction(0xa9, ZeroPage, Some(0))], true),
        Err(AssembleError::AddressingModeMismatch {
            opcode: 0xa9,
            expected: Immediate,
            actual: ZeroPage,
        })
    );
    assert_eq!(
        assemble(&[instruction(0xa9, Immediate, Some(0x100))], true),
        Err(AssembleError::OperandOutOfRange {
            opcode: 0xa9,
            operand: 0x100,
        })
    );
    assert_eq!(
        assemble(&[instruction(0x80, Immediate, Some(0x44))], true),
        Err(AssembleError::UndocumentedOpcode { opcode: 0x80 })
    );
    assert_eq!(
        assemble(&[instruction(0x80, Immediate, Some(0x44))], false),
        Ok(vec![0x80, 0x44])
    );
}

#[test]
fn cycles_adds_indexed_page_and_branch_penalties() {
    use AddressingMode::{AbsoluteX, Implied, Relative};

    let instructions = [
        instruction(0xbd, AbsoluteX, Some(0x12ff)),
        instruction(0x9d, AbsoluteX, Some(0x12ff)),
        instruction(0x1e, AbsoluteX, Some(0x12ff)),
        instruction(0xea, Implied, None),
        instruction(0xd0, Relative, Some(0)),
        instruction(0xd0, Relative, Some(0)),
    ];
    let context = CycleContext {
        indexed_read_page_crossings: vec![0, 1, 2, 3],
        taken_branches: vec![4],
        branch_page_crossings: vec![4, 5],
    };

    assert_eq!(cycles(&instructions, context), 25);
}
