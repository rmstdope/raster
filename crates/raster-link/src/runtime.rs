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
    /// How many of `code_len` are the reset runtime rather than the program.
    /// `code_len - runtime_len` is what the caller's `body` compiled to.
    pub runtime_len: usize,
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
    program.items.extend(interrupt_epilogue());

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
        runtime_len: mmc3_reset_runtime_bytes(),
        vectors,
    })
}

/// What the runtime appends after the program: the interrupt vector target and
/// the handler itself, which does nothing and returns.
fn interrupt_epilogue() -> Vec<FixedBankItem> {
    vec![
        FixedBankItem::Label(INTERRUPT_LABEL),
        FixedBankItem::instruction(RTI, Implied, None),
    ]
}

/// The bytes [`link_mmc3_program`] adds around every program: the reset
/// prologue at the front of the fixed bank and the interrupt handler at the
/// back. 132 today, and the same 132 for every program this compiler builds.
///
/// Measured from the item lists the linker actually emits rather than written
/// down as a constant, so it cannot drift from them. `entry` decides a
/// relocation's operand and never an instruction's width, so measuring the
/// prologue with any label gives the width the linker will use.
pub fn mmc3_reset_runtime_bytes() -> usize {
    crate::items_len(&prologue(RESET_LABEL)) + crate::items_len(&interrupt_epilogue())
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
    // code living in the fixed last bank at $E000; CHR mode 0 with R0-R5 as
    // 0, 2, 4, 5, 6, 7 gives a flat 8 KiB CHR map, pattern table 0 at PPU
    // $0000 and pattern table 1 at $1000.
    //
    // Two things here are load-bearing and neither is visible in the values.
    // In CHR mode 0 the MMC3 ignores bit 0 of R0 and R1, which address 2 KiB
    // windows in 1 KiB units: R1 = $02 is the second window, and R1 = $01
    // would silently alias onto R0's bank. And bits 6 and 7 of every bank
    // select are PRG mode and CHR A12 inversion, taking effect from whichever
    // select was written last - so every select value below must keep both
    // clear, or the map programmed is not the map the console uses.
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
