use raster_link::{
    emit_mmc3_ines, emit_mmc3_ines_with_chr, link_mmc3_program, link_mmc3_program_with_chr,
    InterruptVectors, RomError, CHR_ROM_BANK_SIZE, INES_HEADER_SIZE, MAX_CHR_ROM_BANKS,
    MMC3_FIXED_BANK_SIZE, MMC3_FIXED_BANK_START, MMC3_PRG_ROM_SIZE,
};

#[test]
fn emits_mapper_four_ines_header_and_fixed_bank() {
    let code = [0xa9, 0x00, 0x8d, 0x00, 0x20];
    let rom = emit_mmc3_ines(
        &code,
        InterruptVectors {
            nmi: 0xe000,
            reset: 0xe000,
            irq: 0xe000,
        },
    )
    .unwrap();

    assert_eq!(
        &rom[..INES_HEADER_SIZE],
        &[b'N', b'E', b'S', 0x1a, 2, 0, 0x40, 0, 1, 0, 0, 0, 0, 0, 0, 0,]
    );
    assert_eq!(rom.len(), INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE);

    let fixed_bank_offset = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    assert_eq!(rom[fixed_bank_offset - 1], 0xff);
    assert_eq!(
        &rom[fixed_bank_offset..fixed_bank_offset + code.len()],
        code
    );
}

#[test]
fn writes_little_endian_vectors_at_cpu_vector_offsets() {
    let rom = emit_mmc3_ines(
        &[],
        InterruptVectors {
            nmi: 0xe123,
            reset: 0xe456,
            irq: 0xe789,
        },
    )
    .unwrap();

    let cpu_vector_offset = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE
        + (0xfffa - MMC3_FIXED_BANK_START) as usize;
    assert_eq!(
        &rom[cpu_vector_offset..],
        &[0x23, 0xe1, 0x56, 0xe4, 0x89, 0xe7]
    );
}

#[test]
fn rejects_code_that_reaches_the_vector_table() {
    let error = emit_mmc3_ines(
        &[0; MMC3_FIXED_BANK_SIZE - 5],
        InterruptVectors {
            nmi: 0xe000,
            reset: 0xe000,
            irq: 0xe000,
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        RomError::FixedBankTooLarge {
            actual: MMC3_FIXED_BANK_SIZE - 5,
            maximum: MMC3_FIXED_BANK_SIZE - 6,
        }
    );
}

#[test]
fn rejects_vectors_outside_the_fixed_bank() {
    let vectors = InterruptVectors {
        nmi: 0xdfff,
        reset: 0xdffe,
        irq: 0xdffd,
    };

    assert_eq!(
        emit_mmc3_ines(&[], vectors),
        Err(RomError::VectorOutsideFixedBank {
            vector: "nmi",
            address: 0xdfff,
        })
    );
    assert_eq!(
        emit_mmc3_ines(
            &[],
            InterruptVectors {
                nmi: 0xe000,
                ..vectors
            },
        ),
        Err(RomError::VectorOutsideFixedBank {
            vector: "reset",
            address: 0xdffe,
        })
    );
    assert_eq!(
        emit_mmc3_ines(
            &[],
            InterruptVectors {
                nmi: 0xe000,
                reset: 0xe000,
                ..vectors
            },
        ),
        Err(RomError::VectorOutsideFixedBank {
            vector: "irq",
            address: 0xdffd,
        })
    );
}

const VECTORS: InterruptVectors = InterruptVectors {
    nmi: 0xe000,
    reset: 0xe000,
    irq: 0xe000,
};

#[test]
fn chr_rom_sets_the_header_and_follows_the_prg() {
    let mut chr = vec![0u8; 3 * CHR_ROM_BANK_SIZE];
    chr[0] = 0xaa;
    chr[3 * CHR_ROM_BANK_SIZE - 1] = 0xbb;
    let rom = emit_mmc3_ines_with_chr(&[], VECTORS, &chr).unwrap();

    // Byte 5 counts 8 KiB CHR ROM banks; zero would mean CHR RAM.
    assert_eq!(rom[5], 3);
    assert_eq!(rom.len(), INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE + chr.len());
    assert_eq!(&rom[INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE..], chr.as_slice());
    // The rest of the header is the CHR-RAM header's.
    assert_eq!(&rom[..5], &[b'N', b'E', b'S', 0x1a, 2]);
    assert_eq!(
        &rom[6..INES_HEADER_SIZE],
        &[0x40, 0, 1, 0, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn no_chr_rom_is_byte_identical_to_the_chr_ram_image() {
    assert_eq!(
        emit_mmc3_ines_with_chr(&[0xea], VECTORS, &[]).unwrap(),
        emit_mmc3_ines(&[0xea], VECTORS).unwrap()
    );
}

#[test]
fn chr_rom_that_is_not_whole_banks_or_too_many_is_refused() {
    assert_eq!(
        emit_mmc3_ines_with_chr(&[], VECTORS, &[0; 100]),
        Err(RomError::ChrRomNotWholeBanks { length: 100 })
    );
    let too_many = vec![0u8; (MAX_CHR_ROM_BANKS + 1) * CHR_ROM_BANK_SIZE];
    assert_eq!(
        emit_mmc3_ines_with_chr(&[], VECTORS, &too_many),
        Err(RomError::TooManyChrRomBanks {
            banks: MAX_CHR_ROM_BANKS + 1
        })
    );
}

#[test]
fn a_linked_program_carries_the_chr_rom_it_is_given() {
    use raster_link::{FixedBankItem, Label, RelocatableProgram};
    let body = RelocatableProgram {
        items: vec![FixedBankItem::Label(Label(0))],
    };
    let chr = vec![0x5a; CHR_ROM_BANK_SIZE];
    let with = link_mmc3_program_with_chr(&body, Label(0), None, true, &chr).unwrap();
    let without = link_mmc3_program(&body, Label(0), None, true).unwrap();

    assert_eq!(with.image[5], 1);
    assert_eq!(without.image[5], 0);
    assert_eq!(
        &with.image[INES_HEADER_SIZE..INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE],
        &without.image[INES_HEADER_SIZE..]
    );
    assert_eq!(with.code_len, without.code_len);
}
