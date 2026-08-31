use std::{fmt, num::NonZeroU32};

use tetanes_core::{
    common::NesRegion,
    control_deck::{Config, ControlDeck, Error, HeadlessMode, Result as EmuResult},
    memory::RamState,
};

pub const FRAME_WIDTH: usize = 256;
pub const FRAME_HEIGHT: usize = 240;
pub const FRAME_BYTES: usize = FRAME_WIDTH * FRAME_HEIGHT * 4;

/// How many instructions a measurement will execute before it gives up looking for a marker.
///
/// The reset runtime alone spends two vblanks — some tens of thousands of instructions — before
/// a program's own code runs, so the limit has to be generous. It exists only so that a marker
/// which never executes fails with a diagnosis instead of hanging the suite.
pub const MEASUREMENT_INSTRUCTION_LIMIT: u32 = 2_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pixels: Box<[u8; FRAME_BYTES]>,
}

impl Frame {
    pub fn as_rgba(&self) -> &[u8; FRAME_BYTES] {
        &self.pixels
    }
}

/// A headless NTSC console with `rom` loaded and reset, the one place emulator configuration
/// is decided so that a rendered frame and a measured region come from the same machine.
fn load(rom_name: &str, rom: &[u8]) -> EmuResult<ControlDeck> {
    let mut deck = ControlDeck::with_config(Config {
        region: NesRegion::Ntsc,
        ram_state: RamState::AllZeros,
        headless_mode: HeadlessMode::NO_AUDIO,
        sram_dir: None,
        ..Default::default()
    });
    let mut reader = rom;

    deck.load_rom(rom_name, &mut reader)?;
    Ok(deck)
}

pub fn render_after_frames(rom_name: &str, rom: &[u8], frames: NonZeroU32) -> EmuResult<Frame> {
    let mut deck = load(rom_name, rom)?;
    for _ in 0..frames.get() {
        let _ = deck.clock_frame()?;
    }

    Ok(Frame {
        pixels: Box::new(*deck.frame_buffer()),
    })
}

/// The pair of opcodes bracketing the region a measurement is about.
///
/// A marker is an opcode rather than an address, so nothing here has to know where a compiler put
/// its code. The window opens on the first execution of `start` and closes when the first `end`
/// executed after it has finished, so the count covers both markers and everything between them.
///
/// `Window::new(0x08, 0x28)` — `PHP` to `PLP` — is the bracket the Raster compiler puts around
/// every timed region, and therefore the window whose cost its timing analysis predicted.
///
/// `start` and `end` may be the same opcode, and then the window is that one instruction: the
/// search for `end` begins with the instruction that opened the window rather than after it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Window {
    pub start: u8,
    pub end: u8,
}

impl Window {
    #[must_use]
    pub const fn new(start: u8, end: u8) -> Self {
        Self { start, end }
    }
}

/// Why a ROM could not be measured.
///
/// Loading and running are separate variants because they send a reader to different places: a
/// load failure is about the image, and a failure two hundred thousand instructions in is about
/// what the program did.
#[derive(Debug)]
pub enum MeasureError {
    /// The emulator would not accept the image as a ROM.
    Load(Error),
    /// The CPU stopped part-way through — an opcode the emulator refuses, or a corrupted CPU.
    Run { error: Error, instructions: u32 },
    /// The marker never executed within [`MEASUREMENT_INSTRUCTION_LIMIT`] instructions.
    MarkerNotReached { opcode: u8, instructions: u32 },
}

impl fmt::Display for MeasureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => write!(formatter, "the emulator refused the ROM: {error}"),
            Self::Run {
                error,
                instructions,
            } => write!(
                formatter,
                "the CPU stopped after {instructions} instructions: {error}"
            ),
            Self::MarkerNotReached {
                opcode,
                instructions,
            } => write!(
                formatter,
                "the marker ${opcode:02X} did not execute within {instructions} instructions"
            ),
        }
    }
}

impl std::error::Error for MeasureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Load(error) | Self::Run { error, .. } => Some(error),
            Self::MarkerNotReached { .. } => None,
        }
    }
}

/// The CPU cycles a ROM spends running `window`, measured by executing it.
///
/// This is the runtime half of Raster's timing claim: the compiler predicts a region's cost from
/// an instruction table, and this counts what a 6502 actually spends on the same region. Only the
/// marked window is measured, so reset, the vblank wait and emulator startup are all outside it.
///
/// Everything *inside* the window is counted, an interrupt serviced there included. Nothing here
/// masks interrupts: a ROM whose measurement must be free of them has to arrange that itself, as
/// Raster's reset runtime and its timed regions both do.
pub fn cycles_between(rom_name: &str, rom: &[u8], window: Window) -> Result<u32, MeasureError> {
    let mut deck = load(rom_name, rom).map_err(MeasureError::Load)?;
    let mut instructions = 0;

    while next_opcode(&deck) != window.start {
        step(&mut deck, &mut instructions, window.start)?;
    }
    let opened_at = deck.bus().cpu.cycle;

    loop {
        let opcode = next_opcode(&deck);
        step(&mut deck, &mut instructions, window.end)?;
        if opcode == window.end {
            break;
        }
    }

    Ok(deck.bus().cpu.cycle - opened_at)
}

/// The opcode the CPU is about to execute.
fn next_opcode(deck: &ControlDeck) -> u8 {
    let bus = deck.bus();
    bus.peek(bus.cpu.pc)
}

/// One instruction, giving up once the marker being looked for is plainly never coming.
fn step(
    deck: &mut ControlDeck,
    instructions: &mut u32,
    looking_for: u8,
) -> Result<(), MeasureError> {
    if *instructions >= MEASUREMENT_INSTRUCTION_LIMIT {
        return Err(MeasureError::MarkerNotReached {
            opcode: looking_for,
            instructions: *instructions,
        });
    }
    *instructions += 1;
    deck.clock_instr().map_err(|error| MeasureError::Run {
        error,
        instructions: *instructions,
    })
}
