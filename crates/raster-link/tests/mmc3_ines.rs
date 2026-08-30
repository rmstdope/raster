use raster_link::{
    emit_mmc3_ines, InterruptVectors, RomError, INES_HEADER_SIZE, MMC3_FIXED_BANK_SIZE,
    MMC3_FIXED_BANK_START, MMC3_PRG_ROM_SIZE,
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
