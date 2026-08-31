use raster_6502::{
    AddressingMode::{Absolute, AbsoluteX, Implied, Relative},
    AssembleError, Instruction,
};
use raster_link::{
    link_fixed_bank, link_mmc3_ines, EntryPoints, FixedBankItem, Label, LinkError,
    RelocatableProgram, Relocation, RelocationKind, INES_HEADER_SIZE, MMC3_FIXED_BANK_CODE_SIZE,
    MMC3_FIXED_BANK_SIZE, MMC3_PRG_ROM_SIZE,
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
fn rejects_entry_labels_at_the_vector_table() {
    let entry = Label(8);
    let program = RelocatableProgram {
        items: (0..(MMC3_FIXED_BANK_SIZE - 6))
            .map(|_| FixedBankItem::Instruction {
                instruction: instruction(0xea, Implied),
                relocation: None,
            })
            .chain(std::iter::once(FixedBankItem::Label(entry)))
            .collect(),
    };

    assert_eq!(
        link_mmc3_ines(
            &program,
            EntryPoints {
                nmi: entry,
                reset: entry,
                irq: entry,
            },
            true,
        ),
        Err(LinkError::EntryPointOutsideCode {
            vector: "nmi",
            address: 0xfffa,
        })
    );
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

#[test]
fn embeds_a_data_block_and_addresses_it_by_label() {
    let start = Label(1);
    let table = Label(2);
    let program = RelocatableProgram {
        items: vec![
            FixedBankItem::Label(start),
            FixedBankItem::Instruction {
                instruction: instruction(0xbd, AbsoluteX),
                relocation: Some(Relocation {
                    kind: RelocationKind::Absolute,
                    target: table,
                }),
            },
            FixedBankItem::Instruction {
                instruction: instruction(0x60, Implied),
                relocation: None,
            },
            FixedBankItem::Label(table),
            FixedBankItem::Data(vec![0xde, 0xad, 0xbe, 0xef]),
        ],
    };

    let (bytes, labels) = link_fixed_bank(&program, true).expect("the program links");

    assert_eq!(bytes, [0xbd, 0x04, 0xe0, 0x60, 0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(labels[&start], 0xe000);
    assert_eq!(labels[&table], 0xe004);
}

#[test]
fn counts_data_bytes_when_resolving_relative_branches() {
    let target = Label(1);
    let backward = RelocatableProgram {
        items: vec![
            FixedBankItem::Label(target),
            FixedBankItem::Data(vec![0x11, 0x22, 0x33]),
            FixedBankItem::Instruction {
                instruction: instruction(0xea, Implied),
                relocation: None,
            },
            FixedBankItem::Instruction {
                instruction: instruction(0xd0, Relative),
                relocation: Some(Relocation {
                    kind: RelocationKind::Relative,
                    target,
                }),
            },
        ],
    };

    let (bytes, _) = link_fixed_bank(&backward, true).expect("the backward branch links");
    assert_eq!(bytes, [0x11, 0x22, 0x33, 0xea, 0xd0, 0xfa]);

    let forward = RelocatableProgram {
        items: vec![
            FixedBankItem::Instruction {
                instruction: instruction(0xd0, Relative),
                relocation: Some(Relocation {
                    kind: RelocationKind::Relative,
                    target,
                }),
            },
            FixedBankItem::Data(vec![0x11, 0x11, 0x11]),
            FixedBankItem::Label(target),
            FixedBankItem::Instruction {
                instruction: instruction(0xea, Implied),
                relocation: None,
            },
        ],
    };

    let (bytes, _) = link_fixed_bank(&forward, true).expect("the forward branch links");
    assert_eq!(bytes, [0xd0, 0x03, 0x11, 0x11, 0x11, 0xea]);
}

#[test]
fn counts_data_against_the_fixed_bank_budget() {
    let full = RelocatableProgram {
        items: vec![FixedBankItem::Data(vec![0x00; MMC3_FIXED_BANK_CODE_SIZE])],
    };
    let (bytes, _) = link_fixed_bank(&full, true).expect("a full bank of data links");
    assert_eq!(bytes.len(), MMC3_FIXED_BANK_CODE_SIZE);

    let overfull = RelocatableProgram {
        items: vec![FixedBankItem::Data(vec![
            0x00;
            MMC3_FIXED_BANK_CODE_SIZE + 1
        ])],
    };
    assert_eq!(
        link_fixed_bank(&overfull, true),
        Err(LinkError::FixedBankTooLarge {
            actual: MMC3_FIXED_BANK_CODE_SIZE + 1,
            maximum: MMC3_FIXED_BANK_CODE_SIZE,
        })
    );
}

#[test]
fn the_refusal_counts_every_item_and_not_the_walk_to_the_first_label() {
    // The refusal fires at the first label past the limit, so everything after
    // that label used to go uncounted and `actual` was a lower bound.
    let items = vec![
        FixedBankItem::Data(vec![0x00; MMC3_FIXED_BANK_CODE_SIZE + 1]),
        FixedBankItem::Label(Label(1)),
        FixedBankItem::Data(vec![0x00; 500]),
    ];
    assert_eq!(
        link_fixed_bank(&RelocatableProgram { items }, true),
        Err(LinkError::FixedBankTooLarge {
            actual: MMC3_FIXED_BANK_CODE_SIZE + 501,
            maximum: MMC3_FIXED_BANK_CODE_SIZE,
        })
    );
}
