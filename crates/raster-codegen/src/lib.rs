use std::collections::{BTreeMap, BTreeSet};

use raster_6502::{
    AddressingMode::{Absolute, AbsoluteX, Immediate, Implied, Indirect, Relative, ZeroPage},
    Instruction,
};
use raster_ir::{
    BinaryOperator, Comparison, Condition, CycleConstraint, Destination, Frame, FrameEvent,
    FrameStrategy, Label as IrLabel, Main, Place, Program, Register, Statement, UnaryOperator,
    Value,
};
use raster_link::{FixedBankItem, Label, RelocatableProgram, Relocation, RelocationKind};
use raster_syntax::Span;
use raster_timing::{
    analyze, mmc3_latch_for_delta, mmc3_latch_for_first_event, mmc3_latch_for_next_frame,
    plan_delay, plan_timed_frame, DelayStep, ScheduledHandler, TimedRegion, TimingError,
    IRQ_HANDLER_BODY_CYCLES,
};

const FIRST_ZERO_PAGE_ADDRESS: u8 = 0x10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodegenOutput {
    pub program: RelocatableProgram,
    /// The label `main` was emitted at. The interrupt vectors are the runtime's
    /// to decide, so this is the one fact codegen knows about entry.
    pub main: Label,
    /// The label `$FFFE` points at, where the program has an IRQ handler of its own. A program with
    /// no `frame ... using irq` has none, and the linker leaves the vector on its bare `RTI`.
    pub irq: Option<Label>,
    pub zero_page: BTreeMap<Place, u8>,
    /// The measured cost of each `cycles(?)` region, in the order the regions were generated.
    pub reports: Vec<(String, u32)>,
    /// What each event of the program's `frame` costs against the window its strategy gives it
    /// (spec section 7.2), or nothing when the program has no frame.
    pub frame: Option<FrameBudget>,
}

/// The per-event budget table of spec section 7.2.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameBudget {
    pub name: String,
    pub strategy: FrameStrategy,
    /// Whether the source wrote `using`; the summary says `by default` when it did not.
    pub strategy_written: bool,
    /// One row per event the frame block writes, in source order. An `every` is one row.
    pub rows: Vec<EventBudget>,
}

/// One event of a frame, and what its handler costs against the window it is given.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventBudget {
    /// The event as the source names it, `scanline 60` or `scanline 96-200 every 8`.
    pub label: String,
    /// The handler's worst case: the figure the over-budget diagnostic prints for it. A timed
    /// handler's includes the save, mask and restore of the interrupt flag it is wrapped in.
    pub handler_cycles: u32,
    /// The cycles the strategy gives the handler: the hblank for `irq`, the tightest scanline
    /// budget any occurrence of the event gets for `timed`.
    pub window_cycles: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodegenError {
    MissingMain,
    ZeroPageExhausted {
        span: Span,
    },
    UnknownPlace {
        place: Place,
    },
    UnknownFunction {
        label: IrLabel,
    },
    WrongArgumentCount {
        label: IrLabel,
        expected: usize,
        actual: usize,
    },
    /// A region could not be given a provable cost, or could not be padded to its budget.
    Timing {
        error: TimingError,
        span: Span,
    },
    /// One or more of a frame's handlers is over its window. Every handler was measured, so the
    /// budget table is complete; each error is the one that handler earns on its own, in source
    /// order.
    ///
    /// `budget` is `None`, and `stopped_by` the error that ended emission, when something else
    /// went wrong after some handlers had overrun: the overruns are still told, but a table of a
    /// frame that was not fully measured is not.
    FrameHandlersOverBudget {
        errors: Vec<(TimingError, Span)>,
        budget: Option<FrameBudget>,
        stopped_by: Option<Box<CodegenError>>,
    },
}

/// `PHP`, which saves the interrupt-disable flag a timed region is about to change.
const PHP: u8 = 0x08;
/// `SEI`, which masks IRQs while a region that did not opt out is running.
const SEI: u8 = 0x78;
/// `PLP`, which puts the flag back exactly as it was.
///
/// `CLI` would *enable* interrupts rather than restore them, which is wrong three ways: a nested
/// region would unmask inside its still-running parent, a region nested in an `interruptible` one
/// would unmask inside an interrupt handler, and a program that entered its own code with
/// interrupts disabled — which is how `raster-link` starts one — would silently leave them on.
const PLP: u8 = 0x28;
/// `BIT $2002`, the PPU status read that the de-jitter poll spins on.
const BIT_ABSOLUTE: u8 = 0x2c;
/// `BPL`, taken while the vblank flag is clear.
const BPL: u8 = 0x10;
const PPU_STATUS: u16 = 0x2002;
const LDX_IMMEDIATE: u8 = 0xa2;
const LDA_ABSOLUTE_X: u8 = 0xbd;
const TAX: u8 = 0xaa;
const CLC: u8 = 0x18;
const SEC: u8 = 0x38;
const INX: u8 = 0xe8;
const CPX_IMMEDIATE: u8 = 0xe0;
const BNE: u8 = 0xd0;
const PPU_SCROLL: u16 = 0x2005;
const PPU_ADDRESS: u16 = 0x2006;
const PPU_DATA: u16 = 0x2007;
/// The most bytes one copy loop moves: `CPX #n` takes a one-byte operand.
const MAX_COPY_PIECE: usize = 255;
const LDY_IMMEDIATE: u8 = 0xa0;
const DEX: u8 = 0xca;
const DEY: u8 = 0x88;
const BEQ: u8 = 0xf0;
/// What `JMP absolute` costs. The jump closing a frame pass is spent inside the pass's budget.
const JMP_ABSOLUTE_CYCLES: u32 = 3;
/// `PHA` and `PLA`, which save the accumulator an IRQ handler is about to use.
const PHA: u8 = 0x48;
const PLA: u8 = 0x68;
/// `RTI`, which leaves an interrupt handler and restores the flags the interrupt saved — including
/// the interrupt-disable flag, so the frame loop is interruptible again on the way out.
const RTI: u8 = 0x40;
/// `CLI`, which unmasks IRQs once a frame's chain is armed.
const CLI: u8 = 0x58;
const LDA_IMMEDIATE: u8 = 0xa9;
const STA_ABSOLUTE: u8 = 0x8d;
const STA_ZERO_PAGE: u8 = 0x85;
const JMP_INDIRECT: u8 = 0x6c;
const DEC_ZERO_PAGE: u8 = 0xc6;
/// Scanlines one turn of a timed frame's loop runs: three are exactly 341 cycles, so the 114- and
/// 113-cycle scanline budgets repeat with that period and three handlers make a turn that is the
/// same every time round.
const SCANLINES_PER_TURN: usize = 3;
/// What the end of a turn spends, out of its last handler's budget, whether the loop goes round
/// again or leaves. Both ways start `JMP island` 3 and `DEC counter` 5. Going round is `BEQ exit`
/// not taken 2, three `NOP`s 6 and `JMP top` 3; leaving is the `BEQ` taken 3 (the island sits on
/// one page, so it never pays for crossing one), `LDA #turns` 2 and `STA counter` 3 to re-arm the
/// counter for the next frame, and `JMP after` 3. The `NOP`s are what make the two equal, so the
/// handler after the loop starts on its scanline like every handler in it.
const LOOP_TURN_CYCLES: u32 = 3 + 5 + 2 + 6 + 3;
const _: () = assert!(LOOP_TURN_CYCLES == 3 + 5 + 3 + 2 + 3 + 3);
const NOP: u8 = 0xea;
/// The bytes of one page, which an island is kept within.
const PAGE_BYTES: usize = 256;
/// The two RAM bytes an IRQ chain dispatches through: each handler leaves the address of its
/// successor here, and the `$FFFE` entry is a `JMP` through it.
///
/// It sits below [`FIRST_ZERO_PAGE_ADDRESS`], where the compiler's own variables start, and clear
/// of the `STA $00` the padding filler writes. A 6502 reads an indirect jump's high byte from the
/// same page as its low byte, so a vector must never straddle a page boundary; this one cannot.
const IRQ_DISPATCH_VECTOR: u8 = 0x0e;
// Said above and enforced here: lowering `FIRST_ZERO_PAGE_ADDRESS` would hand the program's first
// variable the vector's low byte and corrupt every dispatch, which is not a failure a test would
// read as its own cause. A `const` assertion makes it a red build instead.
const _: () = assert!(IRQ_DISPATCH_VECTOR + 1 < FIRST_ZERO_PAGE_ADDRESS);

/// `LDX #$00` sets 256 iterations, because the first `DEX` wraps it to 255.
fn iteration_operand(iterations: u32) -> u16 {
    (iterations % raster_timing::MAX_ITERATIONS) as u16
}

/// Generate code using only official opcodes.
///
/// The default is the restrictive one because `raster_6502::assemble` refuses an undocumented
/// opcode under a legal ISA, and the linker assembles under one: a caller that took the compact
/// padding by accident would see the disagreement as an internal compiler error rather than as a
/// choice it made. [`generate_with_isa`] is how a caller asks for the shorter padding on purpose.
pub fn generate(program: &Program) -> Result<CodegenOutput, CodegenError> {
    generate_with_isa(program, true)
}

/// Generate code, restricted to official opcodes when `legal_isa` is set.
pub fn generate_with_isa(
    program: &Program,
    legal_isa: bool,
) -> Result<CodegenOutput, CodegenError> {
    let zero_page = allocate_zero_page(program)?;
    let main = program.main.as_ref().ok_or(CodegenError::MissingMain)?;
    let function_parameters = program
        .functions
        .iter()
        .map(|function| (function.label, function.parameters.clone()))
        .collect();
    let mut generator = Generator {
        output: RelocatableProgram::default(),
        zero_page: &zero_page,
        function_parameters,
        next_internal_label: next_internal_label(program),
        legal_isa,
        reports: Vec::new(),
        irq: None,
        frame_rows: Vec::new(),
        frame_overruns: Vec::new(),
        data_pieces: BTreeMap::new(),
        page_aligned: BTreeSet::new(),
        next_counter: usize::from(FIRST_ZERO_PAGE_ADDRESS) + program.places.len(),
        islands: Vec::new(),
    };
    generator.split_data(program);

    for function in &program.functions {
        generator.emit_label(function.label);
        generator.statements(&function.statements, None)?;
        generator.emit(0x60, Implied, None);
    }

    if let Err(error) = generator.main(main, &program.global_initializers, program.frame.as_ref()) {
        if generator.frame_overruns.is_empty()
            || matches!(error, CodegenError::FrameHandlersOverBudget { .. })
        {
            return Err(error);
        }
        let mut errors = std::mem::take(&mut generator.frame_overruns);
        errors.sort_by_key(|(_, span)| span.start);
        return Err(CodegenError::FrameHandlersOverBudget {
            errors,
            budget: None,
            stopped_by: Some(Box::new(error)),
        });
    }
    let reports = std::mem::take(&mut generator.reports);
    generator.emit_islands();
    generator.emit_data(program);
    let irq = generator.irq.map(link_label);
    let frame = program.frame.as_ref().map(|frame| FrameBudget {
        name: frame.name.clone(),
        strategy: frame.strategy,
        strategy_written: frame.strategy_written,
        rows: generator.take_rows(),
    });
    Ok(CodegenOutput {
        frame,
        program: generator.output,
        main: link_label(main.label),
        irq,
        zero_page,
        reports,
    })
}

fn allocate_zero_page(program: &Program) -> Result<BTreeMap<Place, u8>, CodegenError> {
    let mut zero_page = BTreeMap::new();
    for (index, definition) in program.places.iter().enumerate() {
        let Some(address) = usize::from(FIRST_ZERO_PAGE_ADDRESS).checked_add(index) else {
            return Err(CodegenError::ZeroPageExhausted {
                span: definition.span,
            });
        };
        let Ok(address) = u8::try_from(address) else {
            return Err(CodegenError::ZeroPageExhausted {
                span: definition.span,
            });
        };
        zero_page.insert(definition.place, address);
    }
    Ok(zero_page)
}

struct Generator<'a> {
    output: RelocatableProgram,
    /// Each ROM data block's pieces: a copy loop moves at most 255 bytes, because `CPX #n` takes
    /// a one-byte operand, so a block is stored as consecutive pieces with a label each.
    data_pieces: BTreeMap<IrLabel, Vec<(IrLabel, usize)>>,
    /// The constant arrays, each laid out from the start of a page.
    page_aligned: BTreeSet<Label>,
    /// The zero-page byte the next timed-frame loop counts its turns in, past every place.
    next_counter: usize,
    /// Code that must sit on one page, held back to be laid out after everything else.
    islands: Vec<Vec<FixedBankItem>>,
    zero_page: &'a BTreeMap<Place, u8>,
    function_parameters: BTreeMap<IrLabel, Vec<Place>>,
    next_internal_label: u32,
    legal_isa: bool,
    reports: Vec<(String, u32)>,
    /// The entry an IRQ chain dispatches from, once one has been emitted.
    irq: Option<IrLabel>,
    /// The frame's rows so far, one per event, each remembering the span that folds its
    /// occurrences together.
    frame_rows: Vec<FrameRow>,
    /// The handlers found over their windows, which are collected rather than raised so that every
    /// handler is measured.
    frame_overruns: Vec<(TimingError, Span)>,
}

struct FrameRow {
    span: Span,
    budget: EventBudget,
}

/// How one frame of a timed pass emits its handlers: one at a time, or a run of them as a loop.
enum Segment {
    Handler(usize),
    /// `turns` turns of [`SCANLINES_PER_TURN`] handlers from `start`.
    Loop {
        start: usize,
        turns: usize,
    },
}

/// The code a loop jumps to at the end of every turn, laid out after the frame where nothing falls
/// into it.
struct Island {
    label: IrLabel,
    counter: u8,
    turns: usize,
    top: IrLabel,
    after: IrLabel,
}

impl Generator<'_> {
    /// Emit `main`, and whatever the program does once `main` has run.
    ///
    /// A program with no frame ends in the tight loop it always did. A program with one ends in its
    /// frame instead, repeated for as long as the console is on: the halt label doubles as the top
    /// of the frame loop, so a `return` out of `main` reaches the frame exactly as falling off the
    /// end of `main` does.
    fn main(
        &mut self,
        main: &Main,
        initializers: &[Statement],
        frame: Option<&Frame>,
    ) -> Result<(), CodegenError> {
        self.emit_label(main.label);
        self.statements(initializers, Some(main.halt_label))?;
        self.statements(&main.statements, Some(main.halt_label))?;
        self.emit_label(main.halt_label);
        match frame {
            Some(frame) => match frame.strategy {
                FrameStrategy::Timed => self.timed_frame(frame, main.halt_label)?,
                FrameStrategy::Irq => self.irq_frame(frame, main.halt_label)?,
            },
            None => self.jump(main.halt_label),
        }
        if !self.frame_overruns.is_empty() {
            let mut errors = std::mem::take(&mut self.frame_overruns);
            errors.sort_by_key(|(_, span)| span.start);
            let frame = frame.expect("only a frame's handlers are collected");
            return Err(CodegenError::FrameHandlersOverBudget {
                errors,
                budget: Some(FrameBudget {
                    name: frame.name.clone(),
                    strategy: frame.strategy,
                    strategy_written: frame.strategy_written,
                    rows: self.take_rows(),
                }),
                stopped_by: None,
            });
        }
        Ok(())
    }

    /// Emit a `frame ... using irq` as an MMC3 chain that wraps, armed once and never again.
    ///
    /// The program ends by waiting for vblank, pointing the dispatch vector at the first handler,
    /// programming the latch and unmasking; from there it spins, and everything the frame does
    /// happens in interrupts. Each handler programs the latch for the next, and the last programs
    /// the one that reaches round to the first handler of the next frame.
    ///
    /// **The chain wraps rather than being re-armed each frame.** Nothing clocks the counter
    /// between the last visible scanline and the next pre-render line, but the rises either side of
    /// that gap are consecutive, so one latch reaches across it — see
    /// [`mmc3_latch_for_next_frame`]. Re-arming instead means waiting on the vblank flag once a
    /// frame, and a `$2002` read landing on the dot that sets the flag suppresses it and costs that
    /// frame its whole schedule: measured here at about one frame in fifty.
    ///
    /// The one wait that remains is the arming itself, which has to happen in vblank so that the
    /// reload lands on the pre-render line's rise. A suppressed flag there costs a frame at
    /// start-up and nothing afterwards.
    ///
    /// The preconditions this depends on — rendering on, and the two pattern tables in opposite
    /// halves so A12 moves at all — are `raster-timing`'s, checked in `raster-ir` before anything
    /// reaches here.
    ///
    /// **A handler's body is held to [`IRQ_HANDLER_BODY_CYCLES`] and refused when it does not
    /// fit.** The interrupt lands in the hblank at the end of the scanline before the one the
    /// author named, and a store made once that window has closed lands part-way along a visible
    /// row — which is the one thing the schedule promises does not happen. Only the body is
    /// charged: the ten cycles of prologue before it and the thirty-eight of latch, acknowledgement
    /// and dispatch after it run once the picture has already started, where they harm nothing that
    /// can be seen. The region is `interruptible`, so no `PHP`/`SEI`/`PLP` is emitted around it —
    /// the 6502 set I on interrupt entry, and nine cycles of a window this small spent masking what
    /// is already masked would be most of it.
    fn irq_frame(&mut self, frame: &Frame, halt_label: IrLabel) -> Result<(), CodegenError> {
        let Some(first) = frame.events.first() else {
            // A schedule with no events arms nothing; the program still ends in its own loop.
            self.jump(halt_label);
            return Ok(());
        };
        let handlers: Vec<IrLabel> = frame.events.iter().map(|_| self.internal_label()).collect();

        // The arming. `SEI` covers the whole sequence: an interrupt taken part-way through it would
        // dispatch through a half-written vector.
        self.statement(&Statement::SyncExact, Some(halt_label))?;
        self.emit(SEI, Implied, None);
        self.emit(LDA_IMMEDIATE, Immediate, Some(0));
        self.emit_register(Register::Mmc3IrqDisable);
        self.set_dispatch_vector(handlers[0]);
        self.emit(
            LDA_IMMEDIATE,
            Immediate,
            Some(u16::from(mmc3_latch_for_first_event(first.scanline as u16))),
        );
        self.emit_register(Register::Mmc3IrqLatch);
        self.emit_register(Register::Mmc3IrqReload);
        self.emit_register(Register::Mmc3IrqEnable);
        self.emit(CLI, Implied, None);
        let spin = self.internal_label();
        self.emit_label(spin);
        self.jump(spin);

        // The handlers, past the spin loop: control reaches them through `$FFFE` and never by
        // falling through.
        for (index, event) in frame.events.iter().enumerate() {
            // Every scanline here is on the visible picture, which `raster-ir` bounded at 239, so
            // the counter's own 16-bit arithmetic loses nothing.
            let next = &frame.events[(index + 1) % frame.events.len()];
            let latch = if index + 1 < frame.events.len() {
                mmc3_latch_for_delta((next.scanline - event.scanline) as u16)
            } else {
                mmc3_latch_for_next_frame(event.scanline as u16, next.scanline as u16)
            };
            self.emit_label(handlers[index]);
            // Only the accumulator is saved. The loop these interrupt holds nothing in X or Y, and
            // `RTI` restores the flags the interrupt itself pushed.
            self.emit(PHA, Implied, None);
            let region = self
                .timed_region(
                    &CycleConstraint::AtMost(IRQ_HANDLER_BODY_CYCLES),
                    false, // pad: a short handler spends nothing filling the window
                    true,  // interruptible: the 6502 set I on entry, so PHP/SEI/PLP would be nine
                    // cycles of a window this small, spent masking what is already masked
                    &event.body,
                    event.span,
                    Some(halt_label),
                )
                .map_err(|error| as_irq_window_error(error, event.span));
            self.measured_handler(region, event, IRQ_HANDLER_BODY_CYCLES)?;
            self.emit(LDA_IMMEDIATE, Immediate, Some(u16::from(latch)));
            // The order hardware requires: the latch, then the reload request, then the
            // acknowledgement — which also disables — and only then the re-arm. Enabling before
            // acknowledging would leave the line asserted and the console would take this same
            // interrupt for ever.
            self.emit_register(Register::Mmc3IrqLatch);
            self.emit_register(Register::Mmc3IrqReload);
            self.emit_register(Register::Mmc3IrqDisable);
            self.emit_register(Register::Mmc3IrqEnable);
            self.set_dispatch_vector(handlers[(index + 1) % handlers.len()]);
            self.emit(PLA, Implied, None);
            self.emit(RTI, Implied, None);
        }

        let entry = self.internal_label();
        self.emit_label(entry);
        self.emit(JMP_INDIRECT, Indirect, Some(u16::from(IRQ_DISPATCH_VECTOR)));
        self.irq = Some(entry);
        Ok(())
    }

    /// Point the chain's dispatch vector at `handler`, a byte at a time.
    ///
    /// The address is the linker's to decide, so both halves of it are relocations rather than
    /// constants — which is what [`RelocationKind::LowByte`] and its high-byte twin exist for.
    fn set_dispatch_vector(&mut self, handler: IrLabel) {
        for (kind, address) in [
            (RelocationKind::LowByte, IRQ_DISPATCH_VECTOR),
            (RelocationKind::HighByte, IRQ_DISPATCH_VECTOR + 1),
        ] {
            self.relocated(LDA_IMMEDIATE, Immediate, kind, handler);
            self.emit(STA_ZERO_PAGE, ZeroPage, Some(u16::from(address)));
        }
    }

    /// Store the accumulator into a named hardware register.
    ///
    /// The address comes from `raster-ir`'s own register table, which is the one place in the
    /// compiler that says where a register lives.
    fn emit_register(&mut self, register: Register) {
        self.emit(STA_ABSOLUTE, Absolute, Some(register.address()));
    }

    /// Emit a `frame ... using timed` as a synchronized loop the console never leaves.
    ///
    /// The loop is entered once through the read-$2002-and-branch sequence of spec section 6.6,
    /// which waits for vblank and fixes the origin every handler's position is counted from. After
    /// that nothing polls: each pass spends exactly [`raster_timing::PASS_CYCLES`], so the loop
    /// stays locked to the picture for as long as the console is on.
    ///
    /// A pass is three frames, and the schedule runs in each of them. One frame is 29780 CPU cycles
    /// and two dots, so a loop of one frame cannot be exact — and a loop that re-polls `$2002` each
    /// frame instead sweeps its reads across the dot where the vblank flag is set. A read within two
    /// cycles of it returns the flag clear and suppresses it, and that frame's whole schedule is
    /// lost: measured here, one frame in nine went blank before the pass existed.
    ///
    /// Between handlers the schedule is spent in synthesized delays, and each handler is padded to
    /// the budget [`plan_timed_frame`] gives it, so a handler's position is a sum of proven costs
    /// rather than an accumulation of roundings.
    ///
    /// **Nothing may steal a cycle from the loop.** It synchronizes once and never again, so a
    /// cycle taken from it is taken from every frame after it as well. An NMI enabled through
    /// `ppu.ctrl` bit 7, an OAM DMA through `$4014`, and a DMC DMA all do exactly that, and the
    /// `SEI` around a handler cannot mask an NMI. The OAM DMA is refused: a handler is a
    /// [`CycleConstraint::Exact`] region whose budget is at most one scanline, and the 514 cycles
    /// a DMA is charged are more than four times that, so `analyze` will not admit one. NMI is
    /// detected and not refused: `raster-ir`'s `check_timed_frame_nmi` warns where `ppu.ctrl`
    /// bit 7 reaches the frame, and where a handler sets it, and the ROM is still written. The
    /// DMC DMA is neither refused nor detected, so a program that declares a timed frame must
    /// still leave NMI off and start no DMC transfer.
    fn timed_frame(&mut self, frame: &Frame, halt_label: IrLabel) -> Result<(), CodegenError> {
        let scanlines: Vec<_> = frame.events.iter().map(|event| event.scanline).collect();
        let pass = plan_timed_frame(&scanlines, JMP_ABSOLUTE_CYCLES);
        let mut frames = Vec::new();
        for handlers in pass.handlers.chunks(frame.events.len()) {
            frames.push(self.segments(&frame.events, handlers, halt_label));
        }
        // One counter for each run, shared by its loop in every frame of the pass: the copies
        // never run at once, and each leaves it re-armed. Every counter starts armed.
        let mut counters = BTreeMap::new();
        for segment in frames.iter().flatten() {
            if let Segment::Loop { start, turns } = *segment {
                if counters.contains_key(&start) {
                    continue;
                }
                let counter = self.allocate_counter(frame.span)?;
                counters.insert(start, counter);
                self.emit(LDA_IMMEDIATE, Immediate, Some(turns as u16));
                self.emit(STA_ZERO_PAGE, ZeroPage, Some(u16::from(counter)));
            }
        }

        self.statement(&Statement::SyncExact, Some(halt_label))?;
        let pass_top = self.internal_label();
        self.emit_label(pass_top);

        // A handler's body is emitted once per frame of the pass, an unrolled `every` handler's
        // once per occupied scanline as well, and a looped one once per scanline of a turn. That is
        // only safe because a handler carries no label to define twice: `analyze` refuses `if`,
        // `while` and `for` inside a timed region, and a handler is one. A body that did carry one
        // would reach the linker as a duplicate definition, and the author would see an internal
        // compiler error for a correct program.
        let mut islands = Vec::new();
        for (handlers, segments) in pass.handlers.chunks(frame.events.len()).zip(&frames) {
            for segment in segments {
                match *segment {
                    Segment::Handler(index) => {
                        self.delay(handlers[index].delay_cycles, frame.events[index].span)?;
                        self.handler(
                            &frame.events[index],
                            handlers[index].budget_cycles,
                            halt_label,
                        )?;
                    }
                    Segment::Loop { start, turns } => {
                        let counter = counters[&start];
                        self.delay(handlers[start].delay_cycles, frame.events[start].span)?;
                        let top = self.internal_label();
                        let after = self.internal_label();
                        let label = self.internal_label();
                        self.emit_label(top);
                        let turn = start..start + SCANLINES_PER_TURN;
                        for (offset, (event, scheduled)) in frame.events[turn.clone()]
                            .iter()
                            .zip(&handlers[turn])
                            .enumerate()
                        {
                            let turn_end = if offset == SCANLINES_PER_TURN - 1 {
                                LOOP_TURN_CYCLES
                            } else {
                                0
                            };
                            self.handler(event, scheduled.budget_cycles - turn_end, halt_label)?;
                        }
                        self.jump(label);
                        self.emit_label(after);
                        islands.push(Island {
                            label,
                            counter,
                            turns,
                            top,
                            after,
                        });
                    }
                }
            }
        }
        self.delay(pass.trailing_delay_cycles, frame.span)?;
        self.jump(pass_top);
        for island in &islands {
            self.loop_island(island);
        }
        Ok(())
    }

    /// One handler of a timed frame, padded to exactly `budget` cycles and measured into the table.
    fn handler(
        &mut self,
        event: &FrameEvent,
        budget: u32,
        halt_label: IrLabel,
    ) -> Result<(), CodegenError> {
        let region = self.timed_region(
            &CycleConstraint::Exact(budget),
            true,
            false,
            &event.body,
            event.span,
            Some(halt_label),
        );
        self.measured_handler(region, event, budget)
    }

    /// How one frame of the pass emits its handlers.
    ///
    /// A run of one event's handlers on consecutive scanlines — an `every 1` — becomes a loop when
    /// it is long enough for at least two turns: its handlers have no delay between them, and their
    /// budgets repeat every [`SCANLINES_PER_TURN`]. The run's last handler is always left out of
    /// the loop, because its budget reaches whatever follows the run rather than the next scanline,
    /// and so are the ones a whole turn does not cover.
    ///
    /// A loop is how the compiler saves space, never a reason to refuse a program: a run whose
    /// handler cannot also spare the [`LOOP_TURN_CYCLES`] the end of a turn takes, or would be left
    /// exactly one cycle to pad, is unrolled, and judged against its whole scanline as it always
    /// was.
    fn segments(
        &mut self,
        events: &[FrameEvent],
        handlers: &[ScheduledHandler],
        halt_label: IrLabel,
    ) -> Vec<Segment> {
        let mut segments = Vec::new();
        let mut start = 0;
        while start < events.len() {
            let run = (start + 1..events.len())
                .take_while(|&index| {
                    events[index].span == events[start].span && handlers[index].delay_cycles == 0
                })
                .count()
                + 1;
            let turns = (run - 1) / SCANLINES_PER_TURN;
            let periodic = (start..start + turns * SCANLINES_PER_TURN).all(|index| {
                let first = start + (index - start) % SCANLINES_PER_TURN;
                handlers[index].budget_cycles == handlers[first].budget_cycles
            });
            let last_of_turn = start + SCANLINES_PER_TURN - 1;
            let looped = turns >= 2
                && periodic
                && self
                    .handler_cycles(&events[start], halt_label)
                    .and_then(|cycles| {
                        handlers[last_of_turn]
                            .budget_cycles
                            .checked_sub(cycles + LOOP_TURN_CYCLES)
                    })
                    // A single spare cycle has no filler to spend it.
                    .is_some_and(|spare| spare != 1);
            let unrolled_from = if looped {
                segments.push(Segment::Loop { start, turns });
                start + turns * SCANLINES_PER_TURN
            } else {
                start
            };
            segments.extend((unrolled_from..start + run).map(Segment::Handler));
            start += run;
        }
        segments
    }

    /// What a handler's body costs with its interrupt masking, measured by emitting it and taking
    /// it back out. `None` when it cannot be measured, which the handler's own emission will say.
    fn handler_cycles(&mut self, event: &FrameEvent, halt_label: IrLabel) -> Option<u32> {
        let (items, reports) = (self.output.items.len(), self.reports.len());
        let cycles = self
            .timed_region(
                &CycleConstraint::AtMost(u32::MAX),
                false,
                false,
                &event.body,
                event.span,
                Some(halt_label),
            )
            .ok();
        self.output.items.truncate(items);
        self.reports.truncate(reports);
        cycles
    }

    /// A zero-page byte of the compiler's own, past every place the program declared.
    fn allocate_counter(&mut self, span: Span) -> Result<u8, CodegenError> {
        let counter = u8::try_from(self.next_counter)
            .map_err(|_| CodegenError::ZeroPageExhausted { span })?;
        self.next_counter += 1;
        Ok(counter)
    }

    /// The code a loop jumps to at the end of every turn, kept on one page, which is what holds
    /// its `BEQ` to exactly three cycles when taken.
    fn loop_island(&mut self, island: &Island) {
        let start = self.output.items.len();
        let exit = self.internal_label();
        self.emit_label(island.label);
        self.emit(DEC_ZERO_PAGE, ZeroPage, Some(u16::from(island.counter)));
        self.branch(BEQ, exit);
        for _ in 0..3 {
            self.emit(NOP, Implied, None);
        }
        self.jump(island.top);
        self.emit_label(exit);
        self.emit(LDA_IMMEDIATE, Immediate, Some(island.turns as u16));
        self.emit(STA_ZERO_PAGE, ZeroPage, Some(u16::from(island.counter)));
        self.jump(island.after);
        self.keep_on_one_page(start);
    }

    /// Take the code emitted since item `start` out of the stream, to be laid out after all the
    /// code where nothing falls into it, and on one page.
    fn keep_on_one_page(&mut self, start: usize) {
        let block = self.output.items.split_off(start);
        self.islands.push(block);
    }

    /// Every block that must sit on one page, after the code: a page alignment before the first,
    /// and again whenever the next would cross into another page.
    fn emit_islands(&mut self) {
        let mut used = PAGE_BYTES;
        for block in std::mem::take(&mut self.islands) {
            let bytes = raster_link::items_len(&block);
            assert!(
                bytes <= PAGE_BYTES,
                "an island of {bytes} bytes fits no page"
            );
            if used + bytes > PAGE_BYTES {
                self.output.items.push(FixedBankItem::PageAlign);
                used = 0;
            }
            used += bytes;
            self.output.items.extend(block);
        }
    }

    /// The rows in the order the frame block writes its events, which is not scanline order: the
    /// schedule is sorted for emission, and the source is read top to bottom.
    fn take_rows(&mut self) -> Vec<EventBudget> {
        let mut rows = std::mem::take(&mut self.frame_rows);
        rows.sort_by_key(|row| row.span.start);
        rows.into_iter().map(|row| row.budget).collect()
    }

    /// Record what one handler cost, and keep going if it was over its window.
    ///
    /// Only an overrun of the handler's own region is collected - one whose span is the event's -
    /// so a nested `cycles(n)` the author wrote that overruns still stops the build at once, with
    /// no table to show for a frame that was not fully measured.
    fn measured_handler(
        &mut self,
        region: Result<u32, CodegenError>,
        event: &FrameEvent,
        window: u32,
    ) -> Result<(), CodegenError> {
        let measured = match region {
            Ok(measured) => measured,
            Err(CodegenError::Timing {
                error:
                    error @ (TimingError::OverBudget {
                        measured_cycles, ..
                    }
                    | TimingError::IrqHandlerOverHblank {
                        measured_cycles, ..
                    }),
                span,
            }) if span == event.span => {
                // A timed event is emitted once per frame of the pass: it is one overrun.
                if !self.frame_overruns.iter().any(|(_, seen)| *seen == span) {
                    self.frame_overruns.push((error, span));
                }
                measured_cycles
            }
            Err(error) => return Err(error),
        };
        match self
            .frame_rows
            .iter_mut()
            .find(|row| row.span == event.span)
        {
            Some(row) => {
                row.budget.handler_cycles = row.budget.handler_cycles.max(measured);
                row.budget.window_cycles = row.budget.window_cycles.min(window);
            }
            None => self.frame_rows.push(FrameRow {
                span: event.span,
                budget: EventBudget {
                    label: event.label.clone(),
                    handler_cycles: measured,
                    window_cycles: window,
                },
            }),
        }
        Ok(())
    }

    /// Spend exactly `cycles` doing nothing. Nothing is emitted for a gap of none.
    fn delay(&mut self, cycles: u32, span: Span) -> Result<(), CodegenError> {
        if cycles == 0 {
            return Ok(());
        }
        self.statement(&Statement::Delay { cycles, span }, None)
    }

    fn statements(
        &mut self,
        statements: &[Statement],
        halt_label: Option<IrLabel>,
    ) -> Result<(), CodegenError> {
        for statement in statements {
            self.statement(statement, halt_label)?;
        }
        Ok(())
    }

    fn statement(
        &mut self,
        statement: &Statement,
        halt_label: Option<IrLabel>,
    ) -> Result<(), CodegenError> {
        match statement {
            Statement::Declare { .. } => {}
            Statement::Label(label) => self.emit_label(*label),
            Statement::Assign { destination, value } => {
                self.value(value)?;
                self.store(*destination)?;
            }
            Statement::Call {
                target,
                arguments,
                argument_temporaries,
            } => self.call(*target, arguments, argument_temporaries)?,
            Statement::Branch {
                condition,
                if_false,
            } => self.branch_if_false(condition, *if_false)?,
            Statement::Jump { target } => self.jump(*target),
            Statement::Timed {
                constraint,
                pad,
                interruptible,
                body,
                span,
            } => {
                self.timed_region(constraint, *pad, *interruptible, body, *span, halt_label)?;
            }
            Statement::Delay { cycles, span } => {
                let plan = |cycles| {
                    plan_delay(cycles, self.legal_isa)
                        .map_err(|error| CodegenError::Timing { error, span: *span })
                };
                let inline = plan(*cycles)?;
                if !inline
                    .iter()
                    .any(|step| matches!(step, DelayStep::Loop { .. }))
                {
                    for step in &inline {
                        self.delay_step(step);
                    }
                    return Ok(());
                }
                // A loop closes on a branch, which costs a cycle more when it crosses a page, and
                // where the linker puts it is not known here. So the loop runs in an island kept
                // on one page, and the jumps there and back come out of the delay.
                let looped = plan(*cycles - 2 * JMP_ABSOLUTE_CYCLES)?;
                let island = self.internal_label();
                let back = self.internal_label();
                self.jump(island);
                self.emit_label(back);
                let start = self.output.items.len();
                self.emit_label(island);
                for step in &looped {
                    self.delay_step(step);
                }
                self.jump(back);
                self.keep_on_one_page(start);
            }
            Statement::SyncExact => {
                // Spec section 6.6's read-and-branch de-jitter: spin on the PPU status register
                // until its vblank flag is set, which lands every entry on the same alignment.
                let poll = self.internal_label();
                self.emit_label(poll);
                self.emit(BIT_ABSOLUTE, Absolute, Some(PPU_STATUS));
                self.branch(BPL, poll);
            }
            Statement::PpuUpload { blocks } => self.ppu_upload(blocks)?,
            Statement::Return(value) => {
                if let Some(value) = value {
                    self.value(value)?;
                }
                if let Some(halt_label) = halt_label {
                    self.jump(halt_label);
                } else {
                    self.emit(0x60, Implied, None);
                }
            }
        }
        Ok(())
    }

    /// Emit a timed region, then hold the analysis's verdict as the region's final form.
    ///
    /// The body is generated first, measured through `raster-timing`, and the padding it returns is
    /// appended verbatim — nothing is added, removed or reordered afterwards, which is what makes
    /// the emitted bytes and the predicted cost the same artefact.
    ///
    /// The interrupt masking is charged to the budget along with the body, so `cycles(114)` means
    /// the region occupies 114 cycles from the first instruction to the last. Leaving the masking
    /// outside would make a scanline loop drift by nine cycles a line, which is the one thing a
    /// raster effect cannot survive. The padding goes before the `PLP` so that every cycle the
    /// budget pays for is spent with interrupts still masked.
    fn timed_region(
        &mut self,
        constraint: &CycleConstraint,
        pad: bool,
        interruptible: bool,
        body: &[Statement],
        span: Span,
        halt_label: Option<IrLabel>,
    ) -> Result<u32, CodegenError> {
        let start = self.output.items.len();
        if !interruptible {
            self.emit(PHP, Implied, None);
            self.emit(SEI, Implied, None);
        }
        self.statements(body, halt_label)?;

        let mut instructions: Vec<_> = raster_link::instructions(&self.output.items[start..]);
        let page_aligned_reads = self.page_aligned_reads(start);
        // The `PLP` closing the region is measured before it is emitted, so the padding that makes
        // the budget can be placed ahead of it.
        let restore = implied(PLP);
        if !interruptible {
            instructions.push(restore);
        }

        let report = analyze(
            &TimedRegion {
                constraint: constraint.clone(),
                pad,
                interruptible,
                instructions,
                page_aligned_reads,
            },
            self.legal_isa,
        )
        .map_err(|error| CodegenError::Timing { error, span })?;

        let padding = report.padding.clone();
        self.emit_all(&padding);
        if !interruptible {
            self.emit_all(&[restore]);
        }
        if let Some(label) = report.label {
            self.reports.push((label, report.measured_cycles));
        }
        Ok(report.measured_cycles)
    }

    /// The indexed reads emitted since item `start` whose base is a constant array, counted as
    /// instructions from there: a page-aligned table read with an 8-bit index never crosses a page.
    fn page_aligned_reads(&self, start: usize) -> Vec<usize> {
        self.output.items[start..]
            .iter()
            .filter_map(|item| match item {
                FixedBankItem::Instruction {
                    instruction,
                    relocation,
                } => Some((instruction, relocation)),
                _ => None,
            })
            .enumerate()
            .filter(|(_, (instruction, relocation))| {
                instruction.mode == AbsoluteX
                    && relocation
                        .as_ref()
                        .is_some_and(|relocation| self.page_aligned.contains(&relocation.target))
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Emit one piece of a planned delay.
    ///
    /// The loops close on a relocated `JMP` rather than a backward branch, so only the `BEQ` that
    /// leaves each loop is ever taken; a `Delay` keeps the whole loop on one page, so that branch
    /// never pays for crossing one — see [`raster_timing::DelayStep`].
    fn delay_step(&mut self, step: &DelayStep) {
        match step {
            DelayStep::Filler(instructions) => self.emit_all(instructions),
            DelayStep::Loop { outer, inner } => {
                let outer_loop = self.internal_label();
                let outer_done = self.internal_label();
                if let Some(outer) = outer {
                    self.emit(LDY_IMMEDIATE, Immediate, Some(iteration_operand(*outer)));
                }
                self.emit_label(outer_loop);
                self.emit(LDX_IMMEDIATE, Immediate, Some(iteration_operand(*inner)));

                let inner_loop = self.internal_label();
                let inner_done = self.internal_label();
                self.emit_label(inner_loop);
                self.emit(DEX, Implied, None);
                self.branch(BEQ, inner_done);
                self.jump(inner_loop);
                self.emit_label(inner_done);

                if outer.is_some() {
                    self.emit(DEY, Implied, None);
                    self.branch(BEQ, outer_done);
                    self.jump(outer_loop);
                }
                self.emit_label(outer_done);
            }
        }
    }

    fn emit_all(&mut self, instructions: &[Instruction]) {
        for instruction in instructions {
            self.output.items.push(FixedBankItem::Instruction {
                instruction: *instruction,
                relocation: None,
            });
        }
    }

    fn value(&mut self, value: &Value) -> Result<(), CodegenError> {
        match value {
            Value::Constant(value) => self.emit(0xa9, Immediate, Some(u16::from(*value))),
            Value::Place(place) => self.load_place(*place)?,
            Value::Register(register) => self.emit(0xad, Absolute, Some(register.address())),
            Value::Unary { operator, operand } => {
                self.value(operand)?;
                match operator {
                    UnaryOperator::Negate => {
                        self.emit(0x49, Immediate, Some(0xff));
                        self.emit(0x18, Implied, None);
                        self.emit(0x69, Immediate, Some(1));
                    }
                    UnaryOperator::Not => self.emit(0x49, Immediate, Some(0xff)),
                }
            }
            Value::Binary {
                left,
                operator,
                right,
                left_temporary,
                right_temporary,
            } => {
                if let Some((prepare, immediate, zero_page)) = direct_opcodes(*operator) {
                    match simple_operand(right) {
                        // The right is read where the operation reads it, after the left, exactly
                        // as when it went through a temporary.
                        Some(operand) => {
                            self.value(left)?;
                            if let Some(prepare) = prepare {
                                self.emit(prepare, Implied, None);
                            }
                            match operand {
                                SimpleOperand::Constant(value) => {
                                    self.emit(immediate, Immediate, Some(u16::from(value)))
                                }
                                SimpleOperand::Place(place) => self.operand(zero_page, place)?,
                            }
                            return Ok(());
                        }
                        // Nothing on the left can change what a pure right reads, so the right is
                        // worked out first and kept, and the left goes straight into the operation.
                        None if is_pure(right) && !calls(left) => {
                            self.value(right)?;
                            self.store_place(*right_temporary)?;
                            self.value(left)?;
                            if let Some(prepare) = prepare {
                                self.emit(prepare, Implied, None);
                            }
                            self.operand(zero_page, *right_temporary)?;
                            return Ok(());
                        }
                        None => {}
                    }
                }
                self.value(left)?;
                self.store_place(*left_temporary)?;
                self.value(right)?;
                self.store_place(*right_temporary)?;
                self.load_place(*left_temporary)?;
                match operator {
                    BinaryOperator::Add => {
                        self.emit(0x18, Implied, None);
                        self.operand(0x65, *right_temporary)?;
                    }
                    BinaryOperator::Subtract => {
                        self.emit(0x38, Implied, None);
                        self.operand(0xe5, *right_temporary)?;
                    }
                    BinaryOperator::Multiply => self.multiply(*left_temporary, *right_temporary)?,
                    BinaryOperator::Divide => {
                        self.divide_or_remainder(*left_temporary, *right_temporary, false)?
                    }
                    BinaryOperator::Remainder => {
                        self.divide_or_remainder(*left_temporary, *right_temporary, true)?
                    }
                    BinaryOperator::And => self.operand(0x25, *right_temporary)?,
                    BinaryOperator::Or => self.operand(0x05, *right_temporary)?,
                    BinaryOperator::Xor => self.operand(0x45, *right_temporary)?,
                    BinaryOperator::ShiftLeft => self.shift(*right_temporary, 0x0a)?,
                    BinaryOperator::ShiftRight => self.shift(*right_temporary, 0x4a)?,
                }
            }
            Value::Call {
                target,
                arguments,
                argument_temporaries,
            } => self.call(*target, arguments, argument_temporaries)?,
            Value::Element { table, index } => {
                self.value(index)?;
                self.emit(TAX, Implied, None);
                self.relocated(LDA_ABSOLUTE_X, AbsoluteX, RelocationKind::Absolute, *table);
            }
        }
        Ok(())
    }

    fn shift(&mut self, count: Place, opcode: u8) -> Result<(), CodegenError> {
        let loop_label = self.internal_label();
        let end_label = self.internal_label();
        self.emit(0xa6, ZeroPage, Some(u16::from(self.address(count)?)));
        self.emit_label(loop_label);
        self.emit(0xe0, Immediate, Some(0));
        self.branch(0xf0, end_label);
        self.emit(opcode, raster_6502::AddressingMode::Accumulator, None);
        self.emit(0xca, Implied, None);
        self.jump(loop_label);
        self.emit_label(end_label);
        Ok(())
    }

    fn multiply(&mut self, multiplicand: Place, multiplier: Place) -> Result<(), CodegenError> {
        let loop_label = self.internal_label();
        let end_label = self.internal_label();
        self.emit(0xa9, Immediate, Some(0));
        self.emit(0xa6, ZeroPage, Some(u16::from(self.address(multiplier)?)));
        self.emit_label(loop_label);
        self.emit(0xe0, Immediate, Some(0));
        self.branch(0xf0, end_label);
        self.emit(0x18, Implied, None);
        self.operand(0x65, multiplicand)?;
        self.emit(0xca, Implied, None);
        self.jump(loop_label);
        self.emit_label(end_label);
        Ok(())
    }

    fn divide_or_remainder(
        &mut self,
        dividend: Place,
        divisor: Place,
        remainder: bool,
    ) -> Result<(), CodegenError> {
        let loop_label = self.internal_label();
        let end_label = self.internal_label();
        let zero_label = self.internal_label();
        let after_label = self.internal_label();
        self.emit(0xa2, Immediate, Some(0));
        self.load_place(divisor)?;
        self.branch(0xf0, zero_label);
        self.emit_label(loop_label);
        self.load_place(dividend)?;
        self.operand(0xc5, divisor)?;
        self.branch(0x90, end_label);
        self.emit(0x38, Implied, None);
        self.operand(0xe5, divisor)?;
        self.store_place(dividend)?;
        self.emit(0xe8, Implied, None);
        self.jump(loop_label);
        self.emit_label(end_label);
        if remainder {
            self.load_place(dividend)?;
        } else {
            self.emit(0x8a, Implied, None);
        }
        self.jump(after_label);
        self.emit_label(zero_label);
        self.emit(0xa9, Immediate, Some(0));
        self.emit_label(after_label);
        Ok(())
    }

    fn call(
        &mut self,
        target: IrLabel,
        arguments: &[Value],
        argument_temporaries: &[Place],
    ) -> Result<(), CodegenError> {
        let parameters = self
            .function_parameters
            .get(&target)
            .ok_or(CodegenError::UnknownFunction { label: target })?
            .clone();
        if parameters.len() != arguments.len() {
            return Err(CodegenError::WrongArgumentCount {
                label: target,
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        for (argument, temporary) in arguments.iter().zip(argument_temporaries) {
            self.value(argument)?;
            self.store_place(*temporary)?;
        }
        for (temporary, parameter) in argument_temporaries.iter().zip(parameters) {
            self.load_place(*temporary)?;
            self.store_place(parameter)?;
        }
        self.relocated(0x20, Absolute, RelocationKind::Absolute, target);
        Ok(())
    }

    fn branch_if_false(
        &mut self,
        condition: &Condition,
        target: IrLabel,
    ) -> Result<(), CodegenError> {
        self.value(&condition.left)?;
        self.store_place(condition.left_temporary)?;
        self.value(&condition.right)?;
        self.store_place(condition.right_temporary)?;
        self.load_place(condition.left_temporary)?;
        self.operand(0xc5, condition.right_temporary)?;

        let true_label = self.internal_label();
        match condition.comparison {
            Comparison::Equal => self.branch(0xf0, true_label),
            Comparison::NotEqual => self.branch(0xd0, true_label),
            Comparison::Less => self.branch(0x90, true_label),
            Comparison::GreaterEqual => self.branch(0xb0, true_label),
            Comparison::LessEqual => {
                self.branch(0x90, true_label);
                self.branch(0xf0, true_label);
            }
            Comparison::Greater => {
                let false_label = self.internal_label();
                self.branch(0x90, false_label);
                self.branch(0xd0, true_label);
                self.emit_label(false_label);
                self.jump(target);
                self.emit_label(true_label);
                return Ok(());
            }
        }
        self.jump(target);
        self.emit_label(true_label);
        Ok(())
    }

    fn load_place(&mut self, place: Place) -> Result<(), CodegenError> {
        self.emit(0xa5, ZeroPage, Some(u16::from(self.address(place)?)));
        Ok(())
    }

    fn store_place(&mut self, place: Place) -> Result<(), CodegenError> {
        self.emit(0x85, ZeroPage, Some(u16::from(self.address(place)?)));
        Ok(())
    }

    fn operand(&mut self, opcode: u8, place: Place) -> Result<(), CodegenError> {
        self.emit(opcode, ZeroPage, Some(u16::from(self.address(place)?)));
        Ok(())
    }

    fn store(&mut self, destination: Destination) -> Result<(), CodegenError> {
        match destination {
            Destination::Place(place) => self.store_place(place),
            Destination::Register(register) => {
                self.emit(0x8d, Absolute, Some(register.address()));
                Ok(())
            }
        }
    }

    fn address(&self, place: Place) -> Result<u8, CodegenError> {
        self.zero_page
            .get(&place)
            .copied()
            .ok_or(CodegenError::UnknownPlace { place })
    }

    fn jump(&mut self, target: IrLabel) {
        self.relocated(0x4c, Absolute, RelocationKind::Absolute, target);
    }

    fn branch(&mut self, opcode: u8, target: IrLabel) {
        self.relocated(opcode, Relative, RelocationKind::Relative, target);
    }

    fn emit_label(&mut self, label: IrLabel) {
        self.output
            .items
            .push(FixedBankItem::Label(link_label(label)));
    }

    fn relocated(
        &mut self,
        opcode: u8,
        mode: raster_6502::AddressingMode,
        kind: RelocationKind,
        target: IrLabel,
    ) {
        self.output.items.push(FixedBankItem::Instruction {
            instruction: Instruction {
                opcode,
                mode,
                operand: None,
            },
            relocation: Some(Relocation {
                kind,
                target: link_label(target),
            }),
        });
    }

    fn emit(&mut self, opcode: u8, mode: raster_6502::AddressingMode, operand: Option<u16>) {
        self.output.items.push(FixedBankItem::Instruction {
            instruction: Instruction {
                opcode,
                mode,
                operand,
            },
            relocation: None,
        });
    }

    /// Give every data block's pieces labels, before any code asks for them.
    fn split_data(&mut self, program: &Program) {
        for block in &program.data {
            if block.page_aligned {
                self.page_aligned.insert(link_label(block.label));
                continue;
            }
            let pieces = block
                .bytes
                .chunks(MAX_COPY_PIECE)
                .map(|piece| (self.internal_label(), piece.len()))
                .collect();
            self.data_pieces.insert(block.label, pieces);
        }
    }

    /// The data blocks, after everything that can execute: the 6502 would run them as code if
    /// control reached them, and nothing past `main`'s halt loop does.
    ///
    /// A constant array is one block from the start of a page, addressed by its own label.
    fn emit_data(&mut self, program: &Program) {
        for block in &program.data {
            if block.page_aligned {
                self.output.items.push(FixedBankItem::PageAlign);
                self.emit_label(block.label);
                self.output
                    .items
                    .push(FixedBankItem::Data(block.bytes.clone()));
                continue;
            }
            let mut offset = 0;
            for (label, length) in self.data_pieces[&block.label].clone() {
                self.emit_label(label);
                self.output.items.push(FixedBankItem::Data(
                    block.bytes[offset..offset + length].to_vec(),
                ));
                offset += length;
            }
        }
    }

    /// `load_palette` / `load_nametable`: for each block, point `$2006` at its address and copy
    /// it through `$2007`, then put the PPU's address and scroll back at the top-left.
    ///
    /// Each block reads `$2002` first, so its `$2006` pair starts with the high byte whatever
    /// the code before it left in the shared write latch. The copy assumes `$2007` steps by one
    /// (`PPUCTRL` bit 2 clear), which the reset leaves true and the author owns afterwards.
    ///
    /// Every `$2002` read here clears the vblank flag, so a `wait vblank` straight after a load
    /// can wait for the next frame's flag; an upload costs most of a frame anyway.
    ///
    /// The closing reset matters because the last `$2006` pair also set the PPU's scroll address,
    /// which the PPU reloads at the start of every frame: left alone, the picture would start
    /// wherever the upload stopped. It also leaves the address out of palette space, which a
    /// PPU with rendering off would otherwise show as a colour.
    fn ppu_upload(&mut self, blocks: &[raster_ir::PpuBlock]) -> Result<(), CodegenError> {
        for block in blocks {
            self.latch_reset();
            self.emit(LDA_IMMEDIATE, Immediate, Some(block.address >> 8));
            self.emit(STA_ABSOLUTE, Absolute, Some(PPU_ADDRESS));
            self.emit(LDA_IMMEDIATE, Immediate, Some(block.address & 0xff));
            self.emit(STA_ABSOLUTE, Absolute, Some(PPU_ADDRESS));
            for (piece, length) in self.data_pieces[&block.data].clone() {
                let body = self.internal_label();
                self.emit(LDX_IMMEDIATE, Immediate, Some(0));
                self.emit_label(body);
                self.relocated(LDA_ABSOLUTE_X, AbsoluteX, RelocationKind::Absolute, piece);
                self.emit(STA_ABSOLUTE, Absolute, Some(PPU_DATA));
                self.emit(INX, Implied, None);
                self.emit(CPX_IMMEDIATE, Immediate, Some(length as u16));
                self.branch(BNE, body);
            }
        }
        self.latch_reset();
        self.emit(LDA_IMMEDIATE, Immediate, Some(0));
        for register in [PPU_SCROLL, PPU_SCROLL, PPU_ADDRESS, PPU_ADDRESS] {
            self.emit(STA_ABSOLUTE, Absolute, Some(register));
        }
        Ok(())
    }

    /// A `$2002` read, which clears the `$2005`/`$2006` write latch.
    fn latch_reset(&mut self) {
        self.emit(BIT_ABSOLUTE, Absolute, Some(PPU_STATUS));
    }

    fn internal_label(&mut self) -> IrLabel {
        let label = IrLabel(self.next_internal_label);
        self.next_internal_label += 1;
        label
    }
}

/// A right operand an instruction can take as it stands: an immediate byte or a zero-page place.
#[derive(Clone, Copy)]
enum SimpleOperand {
    Constant(u8),
    Place(Place),
}

fn simple_operand(value: &Value) -> Option<SimpleOperand> {
    match value {
        Value::Constant(value) => Some(SimpleOperand::Constant(*value)),
        Value::Place(place) => Some(SimpleOperand::Place(*place)),
        _ => None,
    }
}

/// For an operation that is one instruction on the accumulator: the flag it needs set first, and
/// its immediate and zero-page opcodes. The loops — multiply, divide, remainder and the shifts —
/// have none.
fn direct_opcodes(operator: BinaryOperator) -> Option<(Option<u8>, u8, u8)> {
    match operator {
        BinaryOperator::Add => Some((Some(CLC), 0x69, 0x65)),
        BinaryOperator::Subtract => Some((Some(SEC), 0xe9, 0xe5)),
        BinaryOperator::And => Some((None, 0x29, 0x25)),
        BinaryOperator::Or => Some((None, 0x09, 0x05)),
        BinaryOperator::Xor => Some((None, 0x49, 0x45)),
        BinaryOperator::Multiply
        | BinaryOperator::Divide
        | BinaryOperator::Remainder
        | BinaryOperator::ShiftLeft
        | BinaryOperator::ShiftRight => None,
    }
}

/// Whether working a value out reads nothing but memory: no register, whose read can change the
/// hardware, and no call, which can change memory.
fn is_pure(value: &Value) -> bool {
    match value {
        Value::Constant(_) | Value::Place(_) => true,
        Value::Register(_) | Value::Call { .. } => false,
        Value::Unary { operand, .. } => is_pure(operand),
        Value::Binary { left, right, .. } => is_pure(left) && is_pure(right),
        Value::Element { index, .. } => is_pure(index),
    }
}

/// Whether working a value out calls a function, which may change any place.
fn calls(value: &Value) -> bool {
    match value {
        Value::Constant(_) | Value::Place(_) | Value::Register(_) => false,
        Value::Call { .. } => true,
        Value::Unary { operand, .. } => calls(operand),
        Value::Binary { left, right, .. } => calls(left) || calls(right),
        Value::Element { index, .. } => calls(index),
    }
}

/// The budget refusal an `irq` handler earns, which is not the one a `cycles` block earns.
///
/// [`Generator::timed_region`] reports an overrun as [`TimingError::OverBudget`], whose diagnostic
/// talks about a block and the budget its author wrote. A handler's window is the hblank the MMC3
/// leaves it and its advice is different, so the error is retyped here — rather than by teaching
/// `analyze` a second kind of region, which would put a fact about one lowering inside the analyser
/// every lowering shares.
///
/// **`handler` is what keeps the retype honest, and it is not a tidy-up.** `timed_region` emits the
/// body before it analyses it, so an author's own `cycles(n) { }` nested inside the handler raises
/// its overrun out through this same `Result`. Retyped indiscriminately, that block would be told
/// the hblank leaves `n` — the author's own budget, not the window — under a caret on `cycles(n) {`
/// rather than on the event. Only the failure carrying the handler's own span is this lowering's to
/// rename; every other one is already saying something true and travels on untouched.
fn as_irq_window_error(error: CodegenError, handler: Span) -> CodegenError {
    match error {
        CodegenError::Timing {
            error:
                TimingError::OverBudget {
                    measured_cycles,
                    budget,
                    // Dropped rather than carried across: `IrqHandlerOverHblank`
                    // is a different error with its own advice, and the OAM DMA
                    // ratio note belongs to the over-budget message this one
                    // replaces. A handler holding a DMA is refused either way.
                    oam_dma_cycles: _,
                },
            span,
        } if span == handler => CodegenError::Timing {
            error: TimingError::IrqHandlerOverHblank {
                measured_cycles,
                budget,
            },
            span,
        },
        other => other,
    }
}

fn implied(opcode: u8) -> Instruction {
    Instruction {
        opcode,
        mode: Implied,
        operand: None,
    }
}

fn link_label(label: IrLabel) -> Label {
    Label(label.0)
}

fn next_internal_label(program: &Program) -> u32 {
    let mut maximum = program
        .functions
        .iter()
        .map(|function| function.label.0)
        .chain(
            program
                .main
                .iter()
                .flat_map(|main| [main.label.0, main.halt_label.0]),
        )
        .max()
        .unwrap_or(0);
    maximum = program
        .data
        .iter()
        .map(|block| block.label.0)
        .fold(maximum, u32::max);
    for statement in program
        .global_initializers
        .iter()
        .chain(
            program
                .functions
                .iter()
                .flat_map(|function| function.statements.iter()),
        )
        .chain(program.main.iter().flat_map(|main| main.statements.iter()))
        .chain(
            program
                .frame
                .iter()
                .flat_map(|frame| frame.events.iter().flat_map(|event| event.body.iter())),
        )
    {
        maximum = maximum.max(highest_label(statement));
    }
    maximum + 1
}

/// The highest label a statement carries, looking inside timed regions as well.
fn highest_label(statement: &Statement) -> u32 {
    match statement {
        Statement::Label(label) => label.0,
        Statement::Timed { body, .. } => body.iter().map(highest_label).max().unwrap_or(0),
        _ => 0,
    }
}
