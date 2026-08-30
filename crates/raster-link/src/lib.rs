pub const INES_HEADER_SIZE: usize = 16;
pub const MMC3_PRG_ROM_SIZE: usize = 32 * 1024;
pub const MMC3_FIXED_BANK_SIZE: usize = 8 * 1024;
pub const MMC3_FIXED_BANK_START: u16 = 0xe000;

const INES_PRG_ROM_BANK_SIZE: usize = 16 * 1024;

mod m1;

pub use m1::m1_solid_backdrop_rom;

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
