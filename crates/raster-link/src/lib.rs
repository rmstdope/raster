use std::collections::BTreeMap;

use raster_6502::{assemble, AddressingMode, AssembleError, Instruction};

pub const INES_HEADER_SIZE: usize = 16;
pub const MMC3_PRG_ROM_SIZE: usize = 32 * 1024;
pub const MMC3_FIXED_BANK_SIZE: usize = 8 * 1024;
pub const MMC3_FIXED_BANK_START: u16 = 0xe000;

const INES_PRG_ROM_BANK_SIZE: usize = 16 * 1024;
pub const MMC3_FIXED_BANK_CODE_SIZE: usize = MMC3_FIXED_BANK_SIZE - 6;

mod m1;
mod m5;
mod runtime;

pub use m1::m1_solid_backdrop_rom;
pub use m5::{
    m5_background_rom, BackgroundData, PPU_ATTRIBUTE_BYTES, PPU_NAMETABLE_BYTES, PPU_PALETTE_BYTES,
};
pub use runtime::{
    link_mmc3_program, mmc3_reset_runtime_bytes, LinkedRom, INTERRUPT_LABEL, RESET_LABEL,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Label(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelocationKind {
    Absolute,
    Relative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Relocation {
    pub kind: RelocationKind,
    pub target: Label,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixedBankItem {
    Label(Label),
    Instruction {
        instruction: Instruction,
        relocation: Option<Relocation>,
    },
    /// Literal bytes, laid out where they sit in the item list.
    ///
    /// A `Label` immediately before a block addresses its first byte, so code
    /// reaches it with an ordinary absolute relocation. The bytes count against
    /// the fixed bank's budget like instructions do.
    ///
    /// Nothing checks that the block is unreachable. The 6502 will happily
    /// execute it, so the caller places data where control does not fall
    /// through — after a halt loop, an `RTS` or a `JMP`.
    Data(Vec<u8>),
}

impl FixedBankItem {
    /// One instruction, resolved at assembly time.
    pub(crate) fn instruction(opcode: u8, mode: AddressingMode, operand: Option<u16>) -> Self {
        Self::Instruction {
            instruction: Instruction {
                opcode,
                mode,
                operand,
            },
            relocation: None,
        }
    }

    /// One instruction whose operand the linker fills in from `target`.
    pub(crate) fn relocated(
        opcode: u8,
        mode: AddressingMode,
        kind: RelocationKind,
        target: Label,
    ) -> Self {
        Self::Instruction {
            instruction: Instruction {
                opcode,
                mode,
                operand: None,
            },
            relocation: Some(Relocation { kind, target }),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelocatableProgram {
    pub items: Vec<FixedBankItem>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntryPoints {
    pub nmi: Label,
    pub reset: Label,
    pub irq: Label,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkError {
    DuplicateLabel { label: Label },
    UndefinedLabel { label: Label },
    RelativeBranchOutOfRange { from: u16, target: u16 },
    FixedBankTooLarge { actual: usize, maximum: usize },
    EntryPointOutsideCode { vector: &'static str, address: u16 },
    Assemble(AssembleError),
}

pub fn link_fixed_bank(
    program: &RelocatableProgram,
    legal_isa: bool,
) -> Result<(Vec<u8>, BTreeMap<Label, u16>), LinkError> {
    let labels = measure_labels(program)?;
    let bytes = emit_fixed_bank(program, &labels, legal_isa)?;
    Ok((bytes, labels))
}

/// Link a program that already contains its own reset sequence.
///
/// **Prefer [`link_mmc3_program`]**, which supplies the shared reset runtime and
/// the MMC3 bank layout. A ROM linked here enters `entry_points.reset` with
/// interrupts enabled, no stack pointer, a cold PPU and undefined mapper
/// registers — which an emulator forgives and hardware does not.
pub fn link_mmc3_ines(
    program: &RelocatableProgram,
    entry_points: EntryPoints,
    legal_isa: bool,
) -> Result<Vec<u8>, LinkError> {
    let (code, labels) = link_fixed_bank(program, legal_isa)?;
    let vectors = InterruptVectors {
        nmi: entry_point_address(&labels, "nmi", entry_points.nmi)?,
        reset: entry_point_address(&labels, "reset", entry_points.reset)?,
        irq: entry_point_address(&labels, "irq", entry_points.irq)?,
    };
    emit_mmc3_ines(&code, vectors).map_err(|error| match error {
        RomError::FixedBankTooLarge { actual, maximum } => {
            LinkError::FixedBankTooLarge { actual, maximum }
        }
        RomError::VectorOutsideFixedBank { .. } => {
            unreachable!("resolved fixed-bank labels are always valid vectors")
        }
    })
}

fn measure_labels(program: &RelocatableProgram) -> Result<BTreeMap<Label, u16>, LinkError> {
    let mut labels = BTreeMap::new();
    let mut offset = 0usize;
    let maximum = MMC3_FIXED_BANK_CODE_SIZE;
    for item in &program.items {
        match item {
            FixedBankItem::Label(label) => {
                if offset > maximum {
                    return Err(LinkError::FixedBankTooLarge {
                        actual: offset,
                        maximum,
                    });
                }
                let address_offset =
                    u16::try_from(offset).map_err(|_| LinkError::FixedBankTooLarge {
                        actual: offset,
                        maximum,
                    })?;
                let address = MMC3_FIXED_BANK_START.checked_add(address_offset).ok_or(
                    LinkError::FixedBankTooLarge {
                        actual: offset,
                        maximum,
                    },
                )?;
                if labels.insert(*label, address).is_some() {
                    return Err(LinkError::DuplicateLabel { label: *label });
                }
            }
            FixedBankItem::Instruction { instruction, .. } => {
                offset += instruction_bytes(instruction.mode);
            }
            FixedBankItem::Data(data) => offset += data.len(),
        }
    }

    if offset > maximum {
        return Err(LinkError::FixedBankTooLarge {
            actual: offset,
            maximum,
        });
    }
    Ok(labels)
}

/// Resolve every relocation and lay the program out as the bytes of the fixed bank.
///
/// The bytes are built as the walk goes rather than assembled from one flat
/// instruction list at the end, because a data block has no `Instruction` to
/// put in such a list.
fn emit_fixed_bank(
    program: &RelocatableProgram,
    labels: &BTreeMap<Label, u16>,
    legal_isa: bool,
) -> Result<Vec<u8>, LinkError> {
    let mut bytes = Vec::new();
    let mut offset = 0usize;
    for item in &program.items {
        match item {
            FixedBankItem::Label(_) => {}
            FixedBankItem::Data(data) => {
                offset += data.len();
                bytes.extend_from_slice(data);
            }
            FixedBankItem::Instruction {
                instruction,
                relocation,
            } => {
                let mut resolved = *instruction;
                if let Some(relocation) = relocation {
                    let target = entry_address(labels, relocation.target)?;
                    match relocation.kind {
                        RelocationKind::Absolute => {
                            if !matches!(
                                resolved.mode,
                                AddressingMode::Absolute
                                    | AddressingMode::AbsoluteX
                                    | AddressingMode::AbsoluteY
                                    | AddressingMode::Indirect
                            ) {
                                return Err(incompatible_relocation(
                                    resolved.opcode,
                                    AddressingMode::Absolute,
                                    resolved.mode,
                                ));
                            }
                            resolved.operand = Some(target);
                        }
                        RelocationKind::Relative => {
                            if resolved.mode != AddressingMode::Relative {
                                return Err(incompatible_relocation(
                                    resolved.opcode,
                                    AddressingMode::Relative,
                                    resolved.mode,
                                ));
                            }
                            let from = MMC3_FIXED_BANK_START + offset as u16;
                            let following = from + instruction_bytes(resolved.mode) as u16;
                            let displacement = i32::from(target) - i32::from(following);
                            if !(i32::from(i8::MIN)..=i32::from(i8::MAX)).contains(&displacement) {
                                return Err(LinkError::RelativeBranchOutOfRange { from, target });
                            }
                            resolved.operand = Some(displacement as i8 as u8 as u16);
                        }
                    }
                }
                offset += instruction_bytes(resolved.mode);
                bytes.extend(
                    assemble(std::slice::from_ref(&resolved), legal_isa)
                        .map_err(LinkError::Assemble)?,
                );
            }
        }
    }
    Ok(bytes)
}

fn incompatible_relocation(
    opcode: u8,
    expected: AddressingMode,
    actual: AddressingMode,
) -> LinkError {
    LinkError::Assemble(AssembleError::AddressingModeMismatch {
        opcode,
        expected,
        actual,
    })
}

/// How many bytes an item list lays down. A `Label` occupies none; everything
/// else is sized by its addressing mode or its own length, exactly as
/// `measure_labels` and `emit_fixed_bank` size it.
pub(crate) fn items_len(items: &[FixedBankItem]) -> usize {
    items
        .iter()
        .map(|item| match item {
            FixedBankItem::Label(_) => 0,
            FixedBankItem::Instruction { instruction, .. } => instruction_bytes(instruction.mode),
            FixedBankItem::Data(data) => data.len(),
        })
        .sum()
}

const fn instruction_bytes(mode: AddressingMode) -> usize {
    match mode {
        AddressingMode::Implied | AddressingMode::Accumulator => 1,
        AddressingMode::Immediate
        | AddressingMode::ZeroPage
        | AddressingMode::ZeroPageX
        | AddressingMode::ZeroPageY
        | AddressingMode::Relative
        | AddressingMode::IndexedIndirect
        | AddressingMode::IndirectIndexed => 2,
        AddressingMode::Absolute
        | AddressingMode::AbsoluteX
        | AddressingMode::AbsoluteY
        | AddressingMode::Indirect => 3,
    }
}

fn entry_address(labels: &BTreeMap<Label, u16>, label: Label) -> Result<u16, LinkError> {
    labels
        .get(&label)
        .copied()
        .ok_or(LinkError::UndefinedLabel { label })
}

fn entry_point_address(
    labels: &BTreeMap<Label, u16>,
    vector: &'static str,
    label: Label,
) -> Result<u16, LinkError> {
    let address = entry_address(labels, label)?;
    let maximum = MMC3_FIXED_BANK_START + MMC3_FIXED_BANK_CODE_SIZE as u16 - 1;
    if !(MMC3_FIXED_BANK_START..=maximum).contains(&address) {
        return Err(LinkError::EntryPointOutsideCode { vector, address });
    }
    Ok(address)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterruptVectors {
    pub nmi: u16,
    pub reset: u16,
    pub irq: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RomError {
    FixedBankTooLarge { actual: usize, maximum: usize },
    VectorOutsideFixedBank { vector: &'static str, address: u16 },
}

pub fn emit_mmc3_ines(
    fixed_bank_code: &[u8],
    vectors: InterruptVectors,
) -> Result<Vec<u8>, RomError> {
    let maximum_code_size = MMC3_FIXED_BANK_CODE_SIZE;
    if fixed_bank_code.len() > maximum_code_size {
        return Err(RomError::FixedBankTooLarge {
            actual: fixed_bank_code.len(),
            maximum: maximum_code_size,
        });
    }

    for (vector, address) in [
        ("nmi", vectors.nmi),
        ("reset", vectors.reset),
        ("irq", vectors.irq),
    ] {
        if !(MMC3_FIXED_BANK_START..=u16::MAX).contains(&address) {
            return Err(RomError::VectorOutsideFixedBank { vector, address });
        }
    }

    let mut image = vec![0xff; INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE];
    // Byte 6 is `0x40`: mapper 4's low nibble, with the mirroring bit clear. The
    // bit is not the authority — MMC3 controls mirroring through $A000, which the
    // reset runtime programs — so do not read this as the ROM's mirroring.
    let prg_rom_banks = (MMC3_PRG_ROM_SIZE / INES_PRG_ROM_BANK_SIZE) as u8;
    image[..INES_HEADER_SIZE].copy_from_slice(&[
        b'N',
        b'E',
        b'S',
        0x1a,
        prg_rom_banks,
        0,
        0x40,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]);

    let fixed_bank_offset = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    image[fixed_bank_offset..fixed_bank_offset + fixed_bank_code.len()]
        .copy_from_slice(fixed_bank_code);
    let [nmi_low, nmi_high] = vectors.nmi.to_le_bytes();
    let [reset_low, reset_high] = vectors.reset.to_le_bytes();
    let [irq_low, irq_high] = vectors.irq.to_le_bytes();
    image[INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - 6..]
        .copy_from_slice(&[nmi_low, nmi_high, reset_low, reset_high, irq_low, irq_high]);

    Ok(image)
}
