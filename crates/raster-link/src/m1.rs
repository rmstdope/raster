use crate::{emit_mmc3_ines, InterruptVectors, MMC3_FIXED_BANK_START};
use raster_6502::{
    assemble,
    AddressingMode::{Absolute, Immediate, Implied, Relative},
    Instruction,
};

const PPU_CONTROL: u16 = 0x2000;
const PPU_MASK: u16 = 0x2001;
const PPU_STATUS: u16 = 0x2002;
const PPU_ADDRESS: u16 = 0x2006;
const PPU_DATA: u16 = 0x2007;
const APU_DMC_CONTROL: u16 = 0x4010;
const APU_FRAME_COUNTER: u16 = 0x4017;

const FIRST_VBLANK_POLL_OFFSET: usize = 20;
const SECOND_VBLANK_POLL_OFFSET: usize = FIRST_VBLANK_POLL_OFFSET + 5;
const IDLE_LOOP_OFFSET: usize = SECOND_VBLANK_POLL_OFFSET + 5 + 18;
const RTI_OFFSET: usize = IDLE_LOOP_OFFSET + 3;

const fn relative_branch(from_next_offset: usize, target_offset: usize) -> u8 {
    let displacement = target_offset as isize - from_next_offset as isize;
    assert!(displacement >= i8::MIN as isize && displacement <= i8::MAX as isize);
    displacement as i8 as u8
}

const fn fixed_bank_address(offset: usize) -> u16 {
    assert!(offset <= u16::MAX as usize - MMC3_FIXED_BANK_START as usize);
    MMC3_FIXED_BANK_START + offset as u16
}

const RESET_PROGRAM: &[Instruction] = &[
    Instruction {
        opcode: 0x78,
        mode: Implied,
        operand: None,
    },
    Instruction {
        opcode: 0xd8,
        mode: Implied,
        operand: None,
    },
    Instruction {
        opcode: 0xa2,
        mode: Immediate,
        operand: Some(0x40),
    },
    Instruction {
        opcode: 0x8e,
        mode: Absolute,
        operand: Some(APU_FRAME_COUNTER),
    },
    Instruction {
        opcode: 0xa2,
        mode: Immediate,
        operand: Some(0xff),
    },
    Instruction {
        opcode: 0x9a,
        mode: Implied,
        operand: None,
    },
    Instruction {
        opcode: 0xe8,
        mode: Implied,
        operand: None,
    },
    Instruction {
        opcode: 0x8e,
        mode: Absolute,
        operand: Some(PPU_CONTROL),
    },
    Instruction {
        opcode: 0x8e,
        mode: Absolute,
        operand: Some(PPU_MASK),
    },
    Instruction {
        opcode: 0x8e,
        mode: Absolute,
        operand: Some(APU_DMC_CONTROL),
    },
    Instruction {
        opcode: 0x2c,
        mode: Absolute,
        operand: Some(PPU_STATUS),
    },
    Instruction {
        opcode: 0x10,
        mode: Relative,
        operand: Some(
            relative_branch(FIRST_VBLANK_POLL_OFFSET + 5, FIRST_VBLANK_POLL_OFFSET) as u16,
        ),
    },
    Instruction {
        opcode: 0x2c,
        mode: Absolute,
        operand: Some(PPU_STATUS),
    },
    Instruction {
        opcode: 0x10,
        mode: Relative,
        operand: Some(
            relative_branch(SECOND_VBLANK_POLL_OFFSET + 5, SECOND_VBLANK_POLL_OFFSET) as u16,
        ),
    },
    Instruction {
        opcode: 0x2c,
        mode: Absolute,
        operand: Some(PPU_STATUS),
    },
    Instruction {
        opcode: 0xa9,
        mode: Immediate,
        operand: Some(0x3f),
    },
    Instruction {
        opcode: 0x8d,
        mode: Absolute,
        operand: Some(PPU_ADDRESS),
    },
    Instruction {
        opcode: 0xa9,
        mode: Immediate,
        operand: Some(0x00),
    },
    Instruction {
        opcode: 0x8d,
        mode: Absolute,
        operand: Some(PPU_ADDRESS),
    },
    Instruction {
        opcode: 0xa9,
        mode: Immediate,
        operand: Some(0x12),
    },
    Instruction {
        opcode: 0x8d,
        mode: Absolute,
        operand: Some(PPU_DATA),
    },
    Instruction {
        opcode: 0x4c,
        mode: Absolute,
        operand: Some(fixed_bank_address(IDLE_LOOP_OFFSET)),
    },
    Instruction {
        opcode: 0x40,
        mode: Implied,
        operand: None,
    },
];

pub fn m1_solid_backdrop_rom() -> Vec<u8> {
    let reset_program =
        assemble(RESET_PROGRAM, true).expect("the fixed M1 reset program must be legal 6502");
    emit_mmc3_ines(
        &reset_program,
        InterruptVectors {
            nmi: fixed_bank_address(RTI_OFFSET),
            reset: fixed_bank_address(0),
            irq: fixed_bank_address(RTI_OFFSET),
        },
    )
    .expect("the fixed M1 reset program must fit its bank")
}
