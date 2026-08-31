use raster_6502::{AddressingMode::Implied, Instruction};
use raster_link::{
    link_mmc3_program, FixedBankItem, InterruptVectors, Label, RelocatableProgram,
    INES_HEADER_SIZE, MMC3_FIXED_BANK_SIZE, MMC3_PRG_ROM_SIZE,
};

/// The 71-byte reset runtime every compiled program is wrapped in.
const PROLOGUE: [u8; 71] = [
    // the CPU into a known state
    0x78, 0xd8, 0xa2, 0x40, 0x8e, 0x17, 0x40, 0xa2, 0xff, 0x9a, 0xe8, 0x8e, 0x00, 0x20, 0x8e, 0x01,
    0x20, 0x8e, 0x10, 0x40, //
    // the MMC3 into a known state: a linear 32 KiB map, no mapper IRQ
    0xa9, 0x00, 0x8d, 0x00, 0xe0, // disable and acknowledge the MMC3 IRQ
    0xa9, 0x00, 0x8d, 0x00, 0xa0, // mirroring
    0xa9, 0x00, 0x8d, 0x01, 0xa0, // PRG RAM disabled, unprotected
    0xa9, 0x06, 0x8d, 0x00, 0x80, // select R6, PRG mode 0, CHR mode 0
    0xa9, 0x00, 0x8d, 0x01, 0x80, // R6 = 8K PRG bank 0 -> $8000
    0xa9, 0x07, 0x8d, 0x00, 0x80, // select R7
    0xa9, 0x01, 0x8d, 0x01, 0x80, // R7 = 8K PRG bank 1 -> $A000
    // two vblanks, then a third read to clear the $2005/$2006 latch
    0x2c, 0x02, 0x20, 0x10, 0xfb, //
    0x2c, 0x02, 0x20, 0x10, 0xfb, //
    0x2c, 0x02, 0x20, //
    // into the program
    0x4c, 0x47, 0xe0,
];

fn fixed_bank(rom: &[u8]) -> &[u8] {
    let offset = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    &rom[offset..]
}

fn one_instruction_body() -> RelocatableProgram {
    RelocatableProgram {
        items: vec![
            FixedBankItem::Label(Label(0)),
            FixedBankItem::Instruction {
                instruction: Instruction {
                    opcode: 0x60,
                    mode: Implied,
                    operand: None,
                },
                relocation: None,
            },
        ],
    }
}

#[test]
fn emits_the_reset_prologue_before_the_program() {
    let rom = link_mmc3_program(&one_instruction_body(), Label(0), true).expect("the body links");

    assert_eq!(&fixed_bank(&rom.image)[..PROLOGUE.len()], PROLOGUE);
    assert_eq!(fixed_bank(&rom.image)[PROLOGUE.len()], 0x60);
    assert_eq!(rom.code_len, 73);
    assert_eq!(
        rom.vectors,
        InterruptVectors {
            nmi: 0xe048,
            reset: 0xe000,
            irq: 0xe048,
        }
    );
}

#[test]
fn points_nmi_and_irq_at_an_rti_after_the_program() {
    let rom = link_mmc3_program(&one_instruction_body(), Label(0), true).expect("the body links");

    assert_eq!(
        &rom.image[rom.image.len() - 6..],
        [0x48, 0xe0, 0x00, 0xe0, 0x48, 0xe0]
    );
    assert_eq!(fixed_bank(&rom.image)[0x48], 0x40);
}
