//! The hand-built milestone-5 ROM: an encoded background uploaded into PPU
//! memory, rendering on, and a halt loop.
//!
//! Only the body lives here. The reset sequence and the MMC3 bank layout are
//! [`crate::link_mmc3_program`]'s, as they are for milestone 1 — including the
//! CHR bank registers, so this module writes no mapper register at all.

use raster_6502::AddressingMode::{Absolute, AbsoluteX, Immediate, Implied, Relative};

use crate::{
    link_mmc3_program, FixedBankItem, Label, LinkError, LinkedRom, RelocatableProgram,
    RelocationKind,
};

/// Bytes of PPU memory each block of a background occupies.
pub const PPU_PALETTE_BYTES: usize = 16;
pub const PPU_NAMETABLE_BYTES: usize = 960;
pub const PPU_ATTRIBUTE_BYTES: usize = 64;

/// The four blocks of PPU memory a background occupies.
///
/// The array lengths are the NES's, and they are exactly the return types of
/// `raster_assets::NesBackground`'s accessors — so a caller wires the two
/// together with no conversion, and a change to either side is a compile error
/// rather than a wrong ROM. `raster-link` deliberately does not depend on
/// `raster-assets`: both are leaves, and a milestone demo is not a reason to
/// join them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackgroundData<'a> {
    /// Four four-byte subpalettes, for `$3F00`.
    pub palettes: &'a [u8; PPU_PALETTE_BYTES],
    /// Two-bitplane 8×8 tiles, for `$0000`. A multiple of 16 bytes.
    pub chr: &'a [u8],
    /// 32 by 30 tile indices, for `$2000`.
    pub nametable: &'a [u8; PPU_NAMETABLE_BYTES],
    /// 64 packed attribute bytes, for `$23C0`.
    pub attributes: &'a [u8; PPU_ATTRIBUTE_BYTES],
}

const PPU_CONTROL: u16 = 0x2000;
const PPU_MASK: u16 = 0x2001;
const PPU_STATUS: u16 = 0x2002;
const PPU_SCROLL: u16 = 0x2005;
const PPU_ADDRESS: u16 = 0x2006;
const PPU_DATA: u16 = 0x2007;

const PATTERN_TABLE: u16 = 0x0000;
const NAMETABLE: u16 = 0x2000;
const ATTRIBUTE_TABLE: u16 = 0x23c0;
const PALETTE: u16 = 0x3f00;

/// Nametable 0, background patterns at `$0000`, NMI off, `$2007` stepping by one.
const CONTROL_UPLOAD_SAFE: u16 = 0x00;
/// Show the background, including in the leftmost eight pixels.
const MASK_BACKGROUND_VISIBLE: u16 = 0x0a;

const LDA_IMMEDIATE: u8 = 0xa9;
const LDA_ABSOLUTE_X: u8 = 0xbd;
const STA_ABSOLUTE: u8 = 0x8d;
const LDX_IMMEDIATE: u8 = 0xa2;
const INX: u8 = 0xe8;
const CPX_IMMEDIATE: u8 = 0xe0;
const BNE: u8 = 0xd0;
const BIT_ABSOLUTE: u8 = 0x2c;
const JMP_ABSOLUTE: u8 = 0x4c;

/// The most bytes one copy loop can move: `CPX #n` takes a one-byte operand.
const MAX_PIECE: usize = 255;

const ENTRY: Label = Label(0);
const HALT: Label = Label(1);

/// The labels of the `index`th data piece: its bytes, and its copy loop.
///
/// Counting up from 2 cannot collide with the runtime's, which are reserved
/// from `u32::MAX` downwards.
fn piece_labels(index: usize) -> (Label, Label) {
    let index = index as u32;
    (Label(2 + 2 * index), Label(3 + 2 * index))
}

/// `LDA #value` / `STA register`.
fn store_immediate(items: &mut Vec<FixedBankItem>, value: u16, register: u16) {
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

/// Copy `length` bytes from `data` into `$2007`, which auto-increments.
fn copy_loop(items: &mut Vec<FixedBankItem>, data: Label, body: Label, length: usize) {
    items.push(FixedBankItem::instruction(
        LDX_IMMEDIATE,
        Immediate,
        Some(0x00),
    ));
    items.push(FixedBankItem::Label(body));
    items.push(FixedBankItem::relocated(
        LDA_ABSOLUTE_X,
        AbsoluteX,
        RelocationKind::Absolute,
        data,
    ));
    items.push(FixedBankItem::instruction(
        STA_ABSOLUTE,
        Absolute,
        Some(PPU_DATA),
    ));
    items.push(FixedBankItem::instruction(INX, Implied, None));
    items.push(FixedBankItem::instruction(
        CPX_IMMEDIATE,
        Immediate,
        Some(length as u16),
    ));
    items.push(FixedBankItem::relocated(
        BNE,
        Relative,
        RelocationKind::Relative,
        body,
    ));
}

fn body(background: BackgroundData<'_>) -> RelocatableProgram {
    // The nametable and the attribute table are contiguous in PPU memory, but
    // each block sets its own $2006 so that none of them depends on another's
    // length.
    let blocks: [(u16, &[u8]); 4] = [
        (PALETTE, background.palettes.as_slice()),
        (PATTERN_TABLE, background.chr),
        (NAMETABLE, background.nametable.as_slice()),
        (ATTRIBUTE_TABLE, background.attributes.as_slice()),
    ];

    let mut items = vec![FixedBankItem::Label(ENTRY)];
    let mut pieces = Vec::new();
    for (address, bytes) in blocks {
        // Clear the shared $2005/$2006 write latch, so the first byte below is
        // the high one whatever the previous block left behind.
        items.push(FixedBankItem::instruction(
            BIT_ABSOLUTE,
            Absolute,
            Some(PPU_STATUS),
        ));
        store_immediate(&mut items, address >> 8, PPU_ADDRESS);
        store_immediate(&mut items, address & 0xff, PPU_ADDRESS);
        // $2007 auto-increments, so $2006 is written once per block however
        // many pieces the block is split into.
        for piece in bytes.chunks(MAX_PIECE) {
            let (data, loop_body) = piece_labels(pieces.len());
            copy_loop(&mut items, data, loop_body, piece.len());
            pieces.push((data, piece));
        }
    }

    // The last $2006 pair left the PPU's `t` register pointing into the block
    // just uploaded, and the PPU reloads `t` into `v` at every pre-render
    // scanline. Without this the frame starts wherever the upload stopped.
    items.push(FixedBankItem::instruction(
        BIT_ABSOLUTE,
        Absolute,
        Some(PPU_STATUS),
    ));
    items.push(FixedBankItem::instruction(
        LDA_IMMEDIATE,
        Immediate,
        Some(0x00),
    ));
    for register in [PPU_SCROLL, PPU_SCROLL, PPU_ADDRESS, PPU_ADDRESS] {
        items.push(FixedBankItem::instruction(
            STA_ABSOLUTE,
            Absolute,
            Some(register),
        ));
    }

    store_immediate(&mut items, CONTROL_UPLOAD_SAFE, PPU_CONTROL);
    store_immediate(&mut items, MASK_BACKGROUND_VISIBLE, PPU_MASK);

    items.push(FixedBankItem::Label(HALT));
    items.push(FixedBankItem::relocated(
        JMP_ABSOLUTE,
        Absolute,
        RelocationKind::Absolute,
        HALT,
    ));

    // Data last, and only after the halt loop: nothing stops the 6502 executing
    // a data block it can fall into.
    for (data, piece) in pieces {
        items.push(FixedBankItem::Label(data));
        items.push(FixedBankItem::Data(piece.to_vec()));
    }

    RelocatableProgram { items }
}

/// Emit the milestone-5 background ROM.
///
/// The program uploads `background` into PPU memory with rendering disabled,
/// puts the scroll back to the top-left, enables background rendering including
/// the leftmost eight pixels, and halts.
///
/// The error is [`LinkError::FixedBankTooLarge`], for a `chr` slice longer than
/// a real background can hold; every 8 KiB CHR page fits with room to spare.
pub fn m5_background_rom(background: BackgroundData<'_>) -> Result<LinkedRom, LinkError> {
    link_mmc3_program(&body(background), ENTRY, true)
}
