use std::collections::BTreeMap;

use raster_6502::{assemble, AddressingMode, AssembleError, Instruction};

pub const INES_HEADER_SIZE: usize = 16;
pub const MMC3_PRG_ROM_SIZE: usize = 32 * 1024;
pub const MMC3_FIXED_BANK_SIZE: usize = 8 * 1024;
pub const MMC3_FIXED_BANK_START: u16 = 0xe000;

const INES_PRG_ROM_BANK_SIZE: usize = 16 * 1024;

mod m1;

pub use m1::m1_solid_backdrop_rom;

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
    Assemble(AssembleError),
}

pub fn link_fixed_bank(
    program: &RelocatableProgram,
    legal_isa: bool,
) -> Result<(Vec<u8>, BTreeMap<Label, u16>), LinkError> {
    let labels = measure_labels(program)?;
    let instructions = resolve_relocations(program, &labels)?;
    let bytes = assemble(&instructions, legal_isa).map_err(LinkError::Assemble)?;
    Ok((bytes, labels))
}

pub fn link_mmc3_ines(
    program: &RelocatableProgram,
    entry_points: EntryPoints,
    legal_isa: bool,
) -> Result<Vec<u8>, LinkError> {
    let (code, labels) = link_fixed_bank(program, legal_isa)?;
    let vectors = InterruptVectors {
        nmi: entry_address(&labels, entry_points.nmi)?,
        reset: entry_address(&labels, entry_points.reset)?,
        irq: entry_address(&labels, entry_points.irq)?,
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
    let maximum = MMC3_FIXED_BANK_SIZE - 6;
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

fn resolve_relocations(
    program: &RelocatableProgram,
    labels: &BTreeMap<Label, u16>,
) -> Result<Vec<Instruction>, LinkError> {
    let mut instructions = Vec::new();
    let mut offset = 0usize;
    for item in &program.items {
        let FixedBankItem::Instruction {
            instruction,
            relocation,
        } = item
        else {
            continue;
        };

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
        instructions.push(resolved);
    }
    Ok(instructions)
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
    let maximum_code_size = MMC3_FIXED_BANK_SIZE - 6;
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
