use crate::{emit_mmc3_ines, InterruptVectors, MMC3_FIXED_BANK_START};

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

const RESET_PROGRAM: &[u8] = &[
    0x78, // SEI
    0xd8, // CLD
    0xa2,
    0x40, // LDX #$40
    0x8e,
    APU_FRAME_COUNTER as u8,
    (APU_FRAME_COUNTER >> 8) as u8, // STX $4017
    0xa2,
    0xff, // LDX #$FF
    0x9a, // TXS
    0xe8, // INX
    0x8e,
    PPU_CONTROL as u8,
    (PPU_CONTROL >> 8) as u8, // STX $2000
    0x8e,
    PPU_MASK as u8,
    (PPU_MASK >> 8) as u8, // STX $2001
    0x8e,
    APU_DMC_CONTROL as u8,
    (APU_DMC_CONTROL >> 8) as u8, // STX $4010
    0x2c,
    PPU_STATUS as u8,
    (PPU_STATUS >> 8) as u8, // BIT $2002
    0x10,
    relative_branch(FIRST_VBLANK_POLL_OFFSET + 5, FIRST_VBLANK_POLL_OFFSET), // BPL first poll
    0x2c,
    PPU_STATUS as u8,
    (PPU_STATUS >> 8) as u8, // BIT $2002
    0x10,
    relative_branch(SECOND_VBLANK_POLL_OFFSET + 5, SECOND_VBLANK_POLL_OFFSET), // BPL second poll
    0x2c,
    PPU_STATUS as u8,
    (PPU_STATUS >> 8) as u8, // BIT $2002 resets the address latch
    0xa9,
    0x3f, // LDA #$3F
    0x8d,
    PPU_ADDRESS as u8,
    (PPU_ADDRESS >> 8) as u8, // STA $2006
    0xa9,
    0x00, // LDA #$00
    0x8d,
    PPU_ADDRESS as u8,
    (PPU_ADDRESS >> 8) as u8, // STA $2006
    0xa9,
    0x12, // LDA #$12
    0x8d,
    PPU_DATA as u8,
    (PPU_DATA >> 8) as u8, // STA $2007
    0x4c,
    fixed_bank_address(IDLE_LOOP_OFFSET) as u8,
    (fixed_bank_address(IDLE_LOOP_OFFSET) >> 8) as u8, // JMP to itself
    0x40,                                              // RTI
];

pub fn m1_solid_backdrop_rom() -> Vec<u8> {
    emit_mmc3_ines(
        RESET_PROGRAM,
        InterruptVectors {
            nmi: fixed_bank_address(RTI_OFFSET),
            reset: fixed_bank_address(0),
            irq: fixed_bank_address(RTI_OFFSET),
        },
    )
    .expect("the fixed M1 reset program must fit its bank")
}
