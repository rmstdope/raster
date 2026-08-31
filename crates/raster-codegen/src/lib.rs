use std::collections::BTreeMap;

use raster_6502::{
    AddressingMode::{Absolute, Immediate, Implied, Relative, ZeroPage},
    Instruction,
};
use raster_ir::{
    BinaryOperator, Comparison, Condition, Destination, Label as IrLabel, Main, Place, Program,
    Statement, UnaryOperator, Value,
};
use raster_link::{
    EntryPoints, FixedBankItem, Label, RelocatableProgram, Relocation, RelocationKind,
};
use raster_syntax::Span;

const FIRST_ZERO_PAGE_ADDRESS: u8 = 0x10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodegenOutput {
    pub program: RelocatableProgram,
    pub entry_points: EntryPoints,
    pub zero_page: BTreeMap<Place, u8>,
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
}

pub fn generate(program: &Program) -> Result<CodegenOutput, CodegenError> {
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
    };

    for function in &program.functions {
        generator.emit_label(function.label);
        generator.statements(&function.statements, None)?;
        generator.emit(0x60, Implied, None);
    }

    generator.main(main, &program.global_initializers)?;
    Ok(CodegenOutput {
        program: generator.output,
        entry_points: EntryPoints {
            nmi: link_label(main.label),
            reset: link_label(main.label),
            irq: link_label(main.label),
        },
        zero_page,
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
}

impl Generator<'_> {
    fn main(&mut self, main: &Main, initializers: &[Statement]) -> Result<(), CodegenError> {
        self.emit_label(main.label);
        self.statements(initializers, Some(main.halt_label))?;
        self.statements(&main.statements, Some(main.halt_label))?;
        self.emit_label(main.halt_label);
        self.jump(main.halt_label);
        Ok(())
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
    {
        if let Statement::Label(label) = statement {
            maximum = maximum.max(label.0);
        }
    }
    maximum + 1
}
