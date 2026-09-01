//! The hand-built milestone-1 ROM: a solid backdrop colour and a halt loop.
//!
//! Only the body lives here. The reset sequence and the MMC3 bank layout are
//! [`crate::link_mmc3_program`]'s, so there is one reset sequence in the
//! repository rather than two, and the labels below are the ones
//! `raster-codegen` allocates for a program whose only item is `main` — which is
//! what makes the compiled demo byte-identical to this ROM.

use crate::{link_mmc3_program, FixedBankItem, Label, RelocatableProgram, RelocationKind};
use raster_6502::AddressingMode::{Absolute, Immediate};

const PPU_ADDRESS: u16 = 0x2006;
const PPU_DATA: u16 = 0x2007;

const LDA_IMMEDIATE: u8 = 0xa9;
const STA_ABSOLUTE: u8 = 0x8d;
const JMP_ABSOLUTE: u8 = 0x4c;

const ENTRY: Label = Label(0);
const HALT: Label = Label(1);

/// NES colour $12, a mid blue.
const BACKDROP: u16 = 0x12;

fn body() -> RelocatableProgram {
    let mut items = vec![FixedBankItem::Label(ENTRY)];
    for (value, register) in [
        (0x3f, PPU_ADDRESS), // the palette lives at $3F00 ...
        (0x00, PPU_ADDRESS), // ... and $3F00 is the universal backdrop
        (BACKDROP, PPU_DATA),
        // Leave palette space. With rendering disabled the PPU emits the palette
        // entry its address register points at, and the $2007 write above left
        // that at $3F01 — zero on a cold console, which is grey.
        (0x00, PPU_ADDRESS),
        (0x00, PPU_ADDRESS),
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
    items.push(FixedBankItem::Label(HALT));
    items.push(FixedBankItem::relocated(
        JMP_ABSOLUTE,
        Absolute,
        RelocationKind::Absolute,
        HALT,
    ));
    RelocatableProgram { items }
}

pub fn m1_solid_backdrop_rom() -> Vec<u8> {
    link_mmc3_program(&body(), ENTRY, None, true)
        .expect("the fixed M1 program must link and fit its bank")
        .image
}
