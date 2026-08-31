use raster_6502::{
    AddressingMode::{Absolute, Implied, Relative},
    AssembleError, Instruction,
};
use raster_link::{
    link_fixed_bank, link_mmc3_ines, EntryPoints, FixedBankItem, Label, LinkError,
    RelocatableProgram, Relocation, RelocationKind, INES_HEADER_SIZE, MMC3_FIXED_BANK_SIZE,
    MMC3_PRG_ROM_SIZE,
};

fn instruction(opcode: u8, mode: raster_6502::AddressingMode) -> Instruction {
    Instruction {
        opcode,
        mode,
        operand: None,
    }
}

#[test]
fn resolves_absolute_and_relative_relocations() {
    let start = Label(1);
    let destination = Label(2);
    let program = RelocatableProgram {
        items: vec![
            FixedBankItem::Label(start),
            FixedBankItem::Instruction {
                instruction: instruction(0x20, Absolute),
                relocation: Some(Relocation {
                    kind: RelocationKind::Absolute,
                    target: destination,
                }),
            },
            FixedBankItem::Instruction {
                instruction: instruction(0xd0, Relative),
                relocation: Some(Relocation {
                    kind: RelocationKind::Relative,
                    target: destination,
                }),
            },
            FixedBankItem::Label(destination),
            FixedBankItem::Instruction {
                instruction: instruction(0xea, Implied),
                relocation: None,
            },
        ],
    };

    let (bytes, labels) = link_fixed_bank(&program, true).unwrap();
    assert_eq!(bytes, [0x20, 0x05, 0xe0, 0xd0, 0x00, 0xea]);
    assert_eq!(labels[&start], 0xe000);
    assert_eq!(labels[&destination], 0xe005);
}

#[test]
fn resolves_entry_labels_into_mmc3_vectors() {
    let entry = Label(7);
    let program = RelocatableProgram {
        items: vec![
            FixedBankItem::Label(entry),
            FixedBankItem::Instruction {
                instruction: instruction(0xea, Implied),
                relocation: None,
            },
        ],
    };
    let rom = link_mmc3_ines(
        &program,
        EntryPoints {
            nmi: entry,
            reset: entry,
            irq: entry,
        },
        true,
    )
    .unwrap();

    let fixed_bank = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    assert_eq!(rom[fixed_bank], 0xea);
    assert_eq!(&rom[rom.len() - 6..], &[0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0]);
}

#[test]
fn reports_relocation_and_assembly_errors() {
    let duplicate = Label(1);
    assert_eq!(
        link_fixed_bank(
            &RelocatableProgram {
                items: vec![
                    FixedBankItem::Label(duplicate),
                    FixedBankItem::Label(duplicate)
                ],
            },
            true,
        ),
        Err(LinkError::DuplicateLabel { label: duplicate })
    );

    let missing = Label(2);
    assert_eq!(
        link_fixed_bank(
            &RelocatableProgram {
                items: vec![FixedBankItem::Instruction {
                    instruction: instruction(0x20, Absolute),
                    relocation: Some(Relocation {
                        kind: RelocationKind::Absolute,
                        target: missing,
                    }),
                }],
            },
            true,
        ),
        Err(LinkError::UndefinedLabel { label: missing })
    );

    let source = Label(3);
    let far = Label(4);
    let mut items = vec![
        FixedBankItem::Label(source),
        FixedBankItem::Instruction {
            instruction: instruction(0xd0, Relative),
            relocation: Some(Relocation {
                kind: RelocationKind::Relative,
                target: far,
            }),
        },
    ];
    items.extend((0..128).map(|_| FixedBankItem::Instruction {
        instruction: instruction(0xea, Implied),
        relocation: None,
    }));
    items.push(FixedBankItem::Label(far));
    assert!(matches!(
        link_fixed_bank(&RelocatableProgram { items }, true),
        Err(LinkError::RelativeBranchOutOfRange { .. })
    ));

    let too_large = RelocatableProgram {
        items: (0..(MMC3_FIXED_BANK_SIZE - 5))
            .map(|_| FixedBankItem::Instruction {
                instruction: instruction(0xea, Implied),
                relocation: None,
            })
            .collect(),
    };
    assert!(matches!(
        link_fixed_bank(&too_large, true),
        Err(LinkError::FixedBankTooLarge { .. })
    ));

    let after_limit = Label(5);
    let mut items = (0..MMC3_FIXED_BANK_SIZE)
        .map(|_| FixedBankItem::Instruction {
            instruction: instruction(0xea, Implied),
            relocation: None,
        })
        .collect::<Vec<_>>();
    items.push(FixedBankItem::Label(after_limit));
    assert!(matches!(
        link_fixed_bank(&RelocatableProgram { items }, true),
        Err(LinkError::FixedBankTooLarge { .. })
    ));

    assert_eq!(
        link_fixed_bank(
            &RelocatableProgram {
                items: vec![FixedBankItem::Instruction {
                    instruction: instruction(0x02, Implied),
                    relocation: None,
                }],
            },
            true,
        ),
        Err(LinkError::Assemble(AssembleError::UndocumentedOpcode {
            opcode: 0x02
        }))
    );

    assert_eq!(
        link_fixed_bank(
            &RelocatableProgram {
                items: vec![
                    FixedBankItem::Label(missing),
                    FixedBankItem::Instruction {
                        instruction: instruction(0x20, Absolute),
                        relocation: Some(Relocation {
                            kind: RelocationKind::Relative,
                            target: missing,
                        }),
                    },
                ],
            },
            true,
        ),
        Err(LinkError::Assemble(AssembleError::AddressingModeMismatch {
            opcode: 0x20,
            expected: Relative,
            actual: Absolute,
        }))
    );
}
