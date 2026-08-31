//! The reset runtime every compiled program is wrapped in.
//!
//! This is the only reset sequence and the only MMC3 bank layout in the
//! repository. A program linked with [`link_mmc3_program`] enters its own code
//! with interrupts disabled, decimal mode cleared, a stack pointer, a warm PPU
//! and a chosen bank map, none of which the console supplies on its own.

use raster_6502::AddressingMode::{Absolute, Immediate, Implied, Relative};

use crate::{
    emit_mmc3_ines, link_fixed_bank, FixedBankItem, InterruptVectors, Label, LinkError,
    RelocatableProgram, RelocationKind, RomError,
};

/// Vector targets, and the runtime's own branch targets. Reserved from the top
/// of the label space so they can never collide with a label `raster-codegen`
/// allocates, which counts up from zero.
pub const RESET_LABEL: Label = Label(u32::MAX);
pub const INTERRUPT_LABEL: Label = Label(u32::MAX - 1);
const FIRST_VBLANK_LABEL: Label = Label(u32::MAX - 2);
const SECOND_VBLANK_LABEL: Label = Label(u32::MAX - 3);

const PPU_CONTROL: u16 = 0x2000;
const PPU_MASK: u16 = 0x2001;
const PPU_STATUS: u16 = 0x2002;
const APU_DMC_CONTROL: u16 = 0x4010;
const APU_FRAME_COUNTER: u16 = 0x4017;
const MMC3_BANK_SELECT: u16 = 0x8000;
const MMC3_BANK_DATA: u16 = 0x8001;
const MMC3_MIRRORING: u16 = 0xa000;
const MMC3_PRG_RAM_PROTECT: u16 = 0xa001;
const MMC3_IRQ_DISABLE: u16 = 0xe000;

const SEI: u8 = 0x78;
const CLD: u8 = 0xd8;
const LDX_IMMEDIATE: u8 = 0xa2;
const STX_ABSOLUTE: u8 = 0x8e;
const TXS: u8 = 0x9a;
const INX: u8 = 0xe8;
const LDA_IMMEDIATE: u8 = 0xa9;
const STA_ABSOLUTE: u8 = 0x8d;
const BIT_ABSOLUTE: u8 = 0x2c;
const BPL: u8 = 0x10;
const JMP_ABSOLUTE: u8 = 0x4c;
const RTI: u8 = 0x40;

/// A linked ROM, and the facts about it a caller needs to report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedRom {
    pub image: Vec<u8>,
    /// Bytes of the fixed bank actually used, runtime included.
    pub code_len: usize,
    /// The resolved `$FFFA` / `$FFFC` / `$FFFE` vectors.
    pub vectors: InterruptVectors,
}

/// Wrap `body` in the MMC3 reset runtime and emit a complete iNES image.
///
/// `entry` is the label control reaches once the runtime has finished; it must
/// be defined inside `body`. The runtime cannot simply fall through into the
/// program, because `raster-codegen` emits every function before `main`.
pub fn link_mmc3_program(
    body: &RelocatableProgram,
    entry: Label,
    legal_isa: bool,
) -> Result<LinkedRom, LinkError> {
    let mut program = RelocatableProgram {
        items: prologue(entry),
    };
    program.items.extend(body.items.iter().cloned());
    program.items.push(FixedBankItem::Label(INTERRUPT_LABEL));
    program
        .items
        .push(FixedBankItem::instruction(RTI, Implied, None));

    let (code, labels) = link_fixed_bank(&program, legal_isa)?;
    let interrupt = *labels
        .get(&INTERRUPT_LABEL)
        .expect("the runtime defines the interrupt label");
    let reset = *labels
        .get(&RESET_LABEL)
        .expect("the runtime defines the reset label");
    let vectors = InterruptVectors {
        nmi: interrupt,
        reset,
        irq: interrupt,
    };
    let code_len = code.len();
    let image = emit_mmc3_ines(&code, vectors).map_err(|error| match error {
        RomError::FixedBankTooLarge { actual, maximum } => {
            LinkError::FixedBankTooLarge { actual, maximum }
        }
        RomError::VectorOutsideFixedBank { .. } => {
            unreachable!("resolved fixed-bank labels are always valid vectors")
        }
    })?;

    Ok(LinkedRom {
        image,
        code_len,
        vectors,
    })
}

fn prologue(entry: Label) -> Vec<FixedBankItem> {
    let mut items = vec![FixedBankItem::Label(RESET_LABEL)];

    // The CPU into a known state.
    items.push(FixedBankItem::instruction(SEI, Implied, None));
    items.push(FixedBankItem::instruction(CLD, Implied, None));
    items.push(FixedBankItem::instruction(
        LDX_IMMEDIATE,
        Immediate,
        Some(0x40),
    ));
    // The APU frame counter, so no frame IRQ arrives mid-program.
    items.push(FixedBankItem::instruction(
        STX_ABSOLUTE,
        Absolute,
        Some(APU_FRAME_COUNTER),
    ));
    items.push(FixedBankItem::instruction(
        LDX_IMMEDIATE,
        Immediate,
        Some(0xff),
    ));
    items.push(FixedBankItem::instruction(TXS, Implied, None)); // the stack at $01FF
    items.push(FixedBankItem::instruction(INX, Implied, None)); // X is zero from here on
    items.push(FixedBankItem::instruction(
        STX_ABSOLUTE,
        Absolute,
        Some(PPU_CONTROL),
    )); // NMI off
    items.push(FixedBankItem::instruction(
        STX_ABSOLUTE,
        Absolute,
        Some(PPU_MASK),
    )); // rendering off
    items.push(FixedBankItem::instruction(
        STX_ABSOLUTE,
        Absolute,
        Some(APU_DMC_CONTROL),
    )); // no DMC IRQ

    // The MMC3 into a known state. The console powers up with all eight bank
    // registers undefined, which an emulator forgives and hardware does not.
    // PRG mode 0 with R6 = 0 and R7 = 1 gives a linear 32 KiB map, all this
    // code living in the fixed last bank at $E000; CHR mode 0 with R0-R5
    // counting up gives a flat 8 KiB CHR map, pattern table 0 at PPU $0000
    // and pattern table 1 at $1000.
    for (value, register) in [
        (0x00, MMC3_IRQ_DISABLE),     // disable and acknowledge the MMC3 IRQ
        (0x00, MMC3_MIRRORING),       // vertical mirroring: chosen, not inherited
        (0x00, MMC3_PRG_RAM_PROTECT), // PRG RAM disabled, unprotected
        (0x00, MMC3_BANK_SELECT),     // select R0, PRG mode 0, CHR mode 0
        (0x00, MMC3_BANK_DATA),       // R0 = 2K CHR at PPU $0000
        (0x01, MMC3_BANK_SELECT),     // select R1
        (0x02, MMC3_BANK_DATA),       // R1 = 2K CHR at PPU $0800
        (0x02, MMC3_BANK_SELECT),     // select R2
        (0x04, MMC3_BANK_DATA),       // R2 = 1K CHR at PPU $1000
        (0x03, MMC3_BANK_SELECT),     // select R3
        (0x05, MMC3_BANK_DATA),       // R3 = 1K CHR at PPU $1400
        (0x04, MMC3_BANK_SELECT),     // select R4
        (0x06, MMC3_BANK_DATA),       // R4 = 1K CHR at PPU $1800
        (0x05, MMC3_BANK_SELECT),     // select R5
        (0x07, MMC3_BANK_DATA),       // R5 = 1K CHR at PPU $1C00
        (0x06, MMC3_BANK_SELECT),     // select R6
        (0x00, MMC3_BANK_DATA),       // R6 = 8K PRG bank 0 -> $8000
        (0x07, MMC3_BANK_SELECT),     // select R7
        (0x01, MMC3_BANK_DATA),       // R7 = 8K PRG bank 1 -> $A000
    ] {
        items.push(FixedBankItem::instruction(
            LDA_IMMEDIATE,
            Immediate,
            Some(value),
        ));
        items.push(FixedBankItem::instruction(
            STA_ABSOLUTE,
            Absolute,
            Some(register),
        ));
    }

    // Two vblanks, which is how long the PPU takes to warm up.
    for label in [FIRST_VBLANK_LABEL, SECOND_VBLANK_LABEL] {
        items.push(FixedBankItem::Label(label));
        items.push(FixedBankItem::instruction(
            BIT_ABSOLUTE,
            Absolute,
            Some(PPU_STATUS),
        ));
        items.push(FixedBankItem::relocated(
            BPL,
            Relative,
            RelocationKind::Relative,
            label,
        ));
    }
    // A third read clears the shared $2005/$2006 write latch, so the program's
    // first `ppu.addr` write is the high byte.
    items.push(FixedBankItem::instruction(
        BIT_ABSOLUTE,
        Absolute,
        Some(PPU_STATUS),
    ));

    items.push(FixedBankItem::relocated(
        JMP_ABSOLUTE,
        Absolute,
        RelocationKind::Absolute,
        entry,
    ));
    items
}
