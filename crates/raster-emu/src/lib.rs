use std::num::NonZeroU32;

use tetanes_core::{
    common::NesRegion,
    control_deck::{Config, ControlDeck, HeadlessMode, Result},
    memory::RamState,
};

pub const FRAME_WIDTH: usize = 256;
pub const FRAME_HEIGHT: usize = 240;
pub const FRAME_BYTES: usize = FRAME_WIDTH * FRAME_HEIGHT * 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pixels: Box<[u8; FRAME_BYTES]>,
}

impl Frame {
    pub fn as_rgba(&self) -> &[u8; FRAME_BYTES] {
        &self.pixels
    }
}

pub fn render_after_frames(rom_name: &str, rom: &[u8], frames: NonZeroU32) -> Result<Frame> {
    let mut deck = ControlDeck::with_config(Config {
        region: NesRegion::Ntsc,
        ram_state: RamState::AllZeros,
        headless_mode: HeadlessMode::NO_AUDIO,
        sram_dir: None,
        ..Default::default()
    });
    let mut reader = rom;

    deck.load_rom(rom_name, &mut reader)?;
    for _ in 0..frames.get() {
        let _ = deck.clock_frame()?;
    }

    Ok(Frame {
        pixels: Box::new(*deck.frame_buffer()),
    })
}
