use std::collections::{BTreeMap, BTreeSet};

use raster_6502::{
    AddressingMode::{Absolute, Immediate, Implied, Relative, ZeroPage},
    Instruction,
};
use raster_ir::{
    BinaryOperator, Comparison, Condition, CycleConstraint, Destination, Frame, Label as IrLabel,
    Main, Place, Program, Statement, UnaryOperator, Value,
};
use raster_link::{FixedBankItem, Label, RelocatableProgram, Relocation, RelocationKind};
use raster_syntax::Span;
use raster_timing::{
    analyze, plan_delay, plan_timed_frame, DelayStep, TimedRegion, TimingError, FRAMES_PER_PASS,
};

const FIRST_ZERO_PAGE_ADDRESS: u8 = 0x10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodegenOutput {
    pub program: RelocatableProgram,
    /// The label `main` was emitted at. The interrupt vectors are the runtime's
    /// to decide, so this is the one fact codegen knows about entry.
    pub main: Label,
    pub zero_page: BTreeMap<Place, u8>,
    /// The measured cost of each `cycles(?)` region, in the order the regions were generated.
    pub reports: Vec<(String, u32)>,
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
const LDY_IMMEDIATE: u8 = 0xa0;
const DEX: u8 = 0xca;
const DEY: u8 = 0x88;
const BEQ: u8 = 0xf0;
/// What `JMP absolute` costs. The jump closing a frame pass is spent inside the pass's budget.
const JMP_ABSOLUTE_CYCLES: u32 = 3;

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
    };

    for function in &program.functions {
        generator.emit_label(function.label);
        generator.statements(&function.statements, None)?;
        generator.emit(0x60, Implied, None);
    }

    generator.main(main, &program.global_initializers, program.frame.as_ref())?;
    let reports = std::mem::take(&mut generator.reports);
    Ok(CodegenOutput {
        program: generator.output,
        main: link_label(main.label),
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
    zero_page: &'a BTreeMap<Place, u8>,
    function_parameters: BTreeMap<IrLabel, Vec<Place>>,
    next_internal_label: u32,
    legal_isa: bool,
    reports: Vec<(String, u32)>,
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
            Some(frame) => self.timed_frame(frame, main.halt_label)?,
            None => self.jump(main.halt_label),
        }
        Ok(())
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
    /// `SEI` around a handler cannot mask an NMI. This release neither refuses nor detects them:
    /// a program that declares a timed frame must leave NMI off and start no DMA.
    fn timed_frame(&mut self, frame: &Frame, halt_label: IrLabel) -> Result<(), CodegenError> {
        self.statement(&Statement::SyncExact, Some(halt_label))?;
        let pass_top = self.internal_label();
        self.emit_label(pass_top);

        let scanlines: Vec<_> = frame.events.iter().map(|event| event.scanline).collect();
        let pass = plan_timed_frame(&scanlines, JMP_ABSOLUTE_CYCLES);
        let events = (0..FRAMES_PER_PASS).flat_map(|_| frame.events.iter());
        for (handler, event) in pass.handlers.iter().zip(events) {
            self.delay(handler.delay_cycles, event.span)?;
            let body = self.renumbered(&event.body);
            self.timed_region(
                &CycleConstraint::Exact(handler.budget_cycles),
                true,
                false,
                &body,
                event.span,
                Some(halt_label),
            )?;
        }
        self.delay(pass.trailing_delay_cycles, frame.span)?;
        self.jump(pass_top);
        Ok(())
    }

    /// A copy of `body` with every label it defines replaced by a fresh one.
    ///
    /// A handler is emitted once per frame of the pass, and an `every` handler once per occupied
    /// scanline on top of that. A label is the linker's, and it refuses one defined twice — so a
    /// copy that kept its labels would turn a handler with an `if` or a `while` in it into an
    /// internal compiler error for a program that is perfectly correct.
    ///
    /// Only the labels the body defines are renamed. A call's target is a function's label, defined
    /// elsewhere, and renaming it would send the call nowhere.
    fn renumbered(&mut self, body: &[Statement]) -> Vec<Statement> {
        let mut defined = BTreeSet::new();
        collect_defined_labels(body, &mut defined);
        if defined.is_empty() {
            return body.to_vec();
        }
        let mapping: BTreeMap<_, _> = defined
            .into_iter()
            .map(|label| (label, self.internal_label()))
            .collect();
        body.iter()
            .map(|statement| renumber_statement(statement, &mapping))
            .collect()
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
            } => self.timed_region(constraint, *pad, *interruptible, body, *span, halt_label)?,
            Statement::Delay { cycles, span } => {
                let plan = plan_delay(*cycles, self.legal_isa)
                    .map_err(|error| CodegenError::Timing { error, span: *span })?;
                for step in &plan {
                    self.delay_step(step);
                }
            }
            Statement::SyncExact => {
                // Spec section 6.6's read-and-branch de-jitter: spin on the PPU status register
                // until its vblank flag is set, which lands every entry on the same alignment.
                let poll = self.internal_label();
                self.emit_label(poll);
                self.emit(BIT_ABSOLUTE, Absolute, Some(PPU_STATUS));
                self.branch(BPL, poll);
            }
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
    ) -> Result<(), CodegenError> {
        let start = self.output.items.len();
        if !interruptible {
            self.emit(PHP, Implied, None);
            self.emit(SEI, Implied, None);
        }
        self.statements(body, halt_label)?;

        let mut instructions: Vec<_> = self.output.items[start..]
            .iter()
            .filter_map(|item| match item {
                FixedBankItem::Instruction { instruction, .. } => Some(*instruction),
                FixedBankItem::Label(_) => None,
                FixedBankItem::Data(_) => unreachable!("raster-codegen emits no data blocks"),
            })
            .collect();
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
        Ok(())
    }

    /// Emit one piece of a planned delay.
    ///
    /// The loops close on a relocated `JMP` rather than a backward branch, so no taken branch's
    /// page-crossing penalty has to be proven away — see [`raster_timing::DelayStep`].
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

    fn internal_label(&mut self) -> IrLabel {
        let label = IrLabel(self.next_internal_label);
        self.next_internal_label += 1;
        label
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

/// Every label a body defines, looking inside timed regions as well.
fn collect_defined_labels(body: &[Statement], into: &mut BTreeSet<IrLabel>) {
    for statement in body {
        match statement {
            Statement::Label(label) => {
                into.insert(*label);
            }
            Statement::Timed { body, .. } => collect_defined_labels(body, into),
            _ => {}
        }
    }
}

/// One statement with the labels in `mapping` replaced, and every other label left as it is.
fn renumber_statement(statement: &Statement, mapping: &BTreeMap<IrLabel, IrLabel>) -> Statement {
    let renumber = |label: &IrLabel| *mapping.get(label).unwrap_or(label);
    match statement {
        Statement::Label(label) => Statement::Label(renumber(label)),
        Statement::Branch {
            condition,
            if_false,
        } => Statement::Branch {
            condition: condition.clone(),
            if_false: renumber(if_false),
        },
        Statement::Jump { target } => Statement::Jump {
            target: renumber(target),
        },
        Statement::Timed {
            constraint,
            pad,
            interruptible,
            body,
            span,
        } => Statement::Timed {
            constraint: constraint.clone(),
            pad: *pad,
            interruptible: *interruptible,
            body: body
                .iter()
                .map(|statement| renumber_statement(statement, mapping))
                .collect(),
            span: *span,
        },
        other => other.clone(),
    }
}

/// The highest label a statement carries, looking inside timed regions as well.
fn highest_label(statement: &Statement) -> u32 {
    match statement {
        Statement::Label(label) => label.0,
        Statement::Timed { body, .. } => body.iter().map(highest_label).max().unwrap_or(0),
        _ => 0,
    }
}
