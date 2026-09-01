use raster_6502::AddressingMode;
use raster_codegen::{generate, CodegenError};
use raster_ir::{
    lower, Comparison, Condition, CycleConstraint, Destination, Function as IrFunction,
    Label as IrLabel, Main, Place, PlaceDefinition, PlaceKind, Program, Statement, Value,
};
use raster_link::{
    link_mmc3_program, FixedBankItem, Label, RelocationKind, INES_HEADER_SIZE,
    MMC3_FIXED_BANK_SIZE, MMC3_PRG_ROM_SIZE,
};
use raster_sema::analyze;
use raster_syntax::{parse, Span};

fn generate_source(source: &str) -> raster_codegen::CodegenOutput {
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    generate(&lower(&typed).expect("fixture should lower")).expect("fixture should generate")
}

#[test]
fn generates_relocatable_calls_control_flow_arithmetic_and_register_stores() {
    let output = generate_source(
        r#"
            fn increment(value: u8) -> u8 {
                if value < 3 { return value + 1 }
                return value
            }
            main {
                var value: u8 = increment(1)
                while value != 0 {
                    ppu.mask = value + 1
                    mmc3.irq_latch = value
                    value = (value * 3) / 3 - 1
                }
            }
        "#,
    );
    let instructions: Vec<_> = output
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            FixedBankItem::Instruction { instruction, .. } => Some(*instruction),
            FixedBankItem::Label(_) => None,
            FixedBankItem::Data(_) => unreachable!("raster-codegen emits no data blocks"),
        })
        .collect();

    assert!(instructions
        .iter()
        .any(|instruction| instruction.opcode == 0x20));
    assert!(instructions
        .iter()
        .any(|instruction| instruction.opcode == 0x65));
    assert!(instructions
        .iter()
        .any(|instruction| instruction.opcode == 0xe5));
    assert!(instructions
        .iter()
        .any(|instruction| instruction.opcode == 0xa6));
    assert!(instructions.iter().any(|instruction| {
        instruction.opcode == 0x8d
            && instruction.mode == AddressingMode::Absolute
            && instruction.operand == Some(0x2001)
    }));
    assert!(output.program.items.iter().any(|item| matches!(
        item,
        FixedBankItem::Instruction {
            relocation: Some(relocation),
            ..
        } if relocation.kind == RelocationKind::Absolute
    )));
    assert!(output.program.items.iter().any(|item| matches!(
        item,
        FixedBankItem::Instruction {
            relocation: Some(relocation),
            ..
        } if relocation.kind == RelocationKind::Relative
    )));
}

#[test]
fn stages_outer_call_arguments_after_nested_call_evaluation() {
    let source = r#"
        fn f(first: u8, second: u8) {}
        fn h() -> u8 { return 2 }
        main { f(1, h()) }
    "#;
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    let program = lower(&typed).expect("fixture should lower");
    let output = generate(&program).expect("fixture should generate");
    let f = program
        .functions
        .iter()
        .find(|function| function.name == "f")
        .expect("fixture should define f");
    let h = program
        .functions
        .iter()
        .find(|function| function.name == "h")
        .expect("fixture should define h");
    let parameter_addresses: Vec<_> = f
        .parameters
        .iter()
        .map(|parameter| output.zero_page[parameter])
        .collect();

    let h_call = instruction_position(&output.program.items, 0x20, Some(Label(h.label.0)));
    let f_call = instruction_position(&output.program.items, 0x20, Some(Label(f.label.0)));
    for address in parameter_addresses {
        let staged = output
            .program
            .items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    FixedBankItem::Instruction { instruction, .. }
                        if instruction.opcode == 0x85
                            && instruction.mode == AddressingMode::ZeroPage
                            && instruction.operand == Some(u16::from(address))
                )
            })
            .expect("outer parameter should be staged");
        assert!(
            h_call < staged && staged < f_call,
            "outer parameters must be staged only after h() evaluates"
        );
    }
}

fn instruction_position(
    items: &[FixedBankItem],
    opcode: u8,
    relocation_target: Option<Label>,
) -> usize {
    items
        .iter()
        .position(|item| {
            matches!(
                item,
                FixedBankItem::Instruction {
                    instruction,
                    relocation,
                } if instruction.opcode == opcode
                    && relocation_target.map_or(true, |target| {
                        relocation.as_ref().is_some_and(|relocation| relocation.target == target)
                    })
            )
        })
        .expect("expected instruction")
}

#[test]
fn allocates_zero_page_deterministically_reserving_hardware_bytes() {
    let output = generate_source(
        r#"
            var global: u8 in zp
            fn copy(parameter: u8) { var local: u8 = parameter }
            main { var value: u8 = 1; for index in 0..2 { value = value + 1 } }
        "#,
    );
    let addresses: Vec<_> = output.zero_page.values().copied().collect();
    assert_eq!(addresses.first(), Some(&0x10));
    assert_eq!(
        addresses,
        (0x10..=0x10 + addresses.len() as u8 - 1).collect::<Vec<_>>()
    );

    let declarations = (0..241)
        .map(|index| format!("var value{index}: u8"))
        .collect::<Vec<_>>()
        .join("\n");
    let syntax = parse(&format!("{declarations}\nmain {{}}\n")).unwrap();
    let typed = analyze(&syntax).unwrap();
    assert!(matches!(
        generate(&lower(&typed).unwrap()),
        Err(CodegenError::ZeroPageExhausted { .. })
    ));
}

#[test]
fn generated_main_links_to_an_executable_register_store() {
    let output = generate_source("main { ppu.mask = 1 }");
    let rom = link_mmc3_program(&output.program, output.main, None, true).unwrap();
    let fixed_bank = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    // The fixed bank now opens with the runtime's reset sequence, and the
    // program's own first bytes follow the 131-byte prologue.
    assert_eq!(
        &rom.image[fixed_bank..fixed_bank + 4],
        &[0x78, 0xd8, 0xa2, 0x40]
    );
    assert_eq!(
        &rom.image[fixed_bank + 131..fixed_bank + 136],
        &[0xa9, 1, 0x8d, 1, 0x20]
    );
    // Reset enters the runtime, not the main label.
    assert_eq!(
        &rom.image[rom.image.len() - 4..rom.image.len() - 2],
        &[0x00, 0xe0]
    );
}

fn instructions_of(items: &[FixedBankItem]) -> Vec<raster_6502::Instruction> {
    items
        .iter()
        .filter_map(|item| match item {
            FixedBankItem::Instruction { instruction, .. } => Some(*instruction),
            FixedBankItem::Label(_) => None,
            FixedBankItem::Data(_) => unreachable!("raster-codegen emits no data blocks"),
        })
        .collect()
}

/// `PHP` 3 + `SEI` 2 + `PLP` 4: the interrupt masking, which the budget pays for so that a region
/// occupies exactly the cycles its annotation names.
const MASKING: u32 = 9;

#[test]
fn timed_region_emits_analyzed_bytes_without_post_analysis_changes() {
    let output = generate_source(
        r#"
            main {
                sync exact
                cycles(20) pad {
                    ppu.mask = 1
                }
            }
        "#,
    );
    let instructions = instructions_of(&output.program.items);

    let php = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0x08)
        .expect("a timed region saves the interrupt flag on the way in");
    let plp = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0x28)
        .expect("a timed region restores it on the way out");
    assert_eq!(instructions[php + 1].opcode, 0x78, "`SEI` masks interrupts");

    // The region is exactly what `raster-timing` analysed: the masking, `LDA #1`, `STA $2001`, the
    // padding it returned, and the `PLP`. Nothing is added, removed or reordered afterwards.
    let region = &instructions[php..=plp];
    let body = vec![
        raster_6502::Instruction {
            opcode: 0x08,
            mode: AddressingMode::Implied,
            operand: None,
        },
        raster_6502::Instruction {
            opcode: 0x78,
            mode: AddressingMode::Implied,
            operand: None,
        },
        raster_6502::Instruction {
            opcode: 0xa9,
            mode: AddressingMode::Immediate,
            operand: Some(1),
        },
        raster_6502::Instruction {
            opcode: 0x8d,
            mode: AddressingMode::Absolute,
            operand: Some(0x2001),
        },
    ];
    let restore = raster_6502::Instruction {
        opcode: 0x28,
        mode: AddressingMode::Implied,
        operand: None,
    };
    let mut measured = body.clone();
    measured.push(restore);

    let mut expected = body;
    expected.extend(
        raster_timing::analyze(
            &raster_timing::TimedRegion {
                constraint: raster_timing::CycleConstraint::Exact(20),
                pad: true,
                interruptible: false,
                instructions: measured,
            },
            true,
        )
        .expect("the fixture is within its budget")
        .padding,
    );
    expected.push(restore);

    assert_eq!(region, expected.as_slice());
    assert_eq!(raster_timing::worst_case_cycles(region), 20);
}

#[test]
fn an_interruptible_region_keeps_interrupts_enabled() {
    let output = generate_source("main { cycles(10) pad interruptible { var value: u8 = 1 } }");
    let instructions = instructions_of(&output.program.items);

    assert!(!instructions
        .iter()
        .any(|instruction| matches!(instruction.opcode, 0x08 | 0x78 | 0x28)));
}

#[test]
fn a_nested_region_restores_the_interrupt_flag_rather_than_enabling_interrupts() {
    let output = generate_source("main { cycles(<= 40) { cycles(<= 20) { var value: u8 = 1 } } }");
    let instructions = instructions_of(&output.program.items);

    // `CLI` here would unmask interrupts for the rest of the outer region, which had asked for
    // exactly the opposite. `PLP` puts back whatever `PHP` saved.
    assert!(!instructions
        .iter()
        .any(|instruction| instruction.opcode == 0x58));
    for opcode in [0x08, 0x28] {
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.opcode == opcode)
                .count(),
            2
        );
    }
}

#[test]
fn a_region_over_its_budget_is_reported_with_its_cost_and_span() {
    let source = "main { sync exact\n cycles(2) { ppu.mask = 1 } }";
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    let error = generate(&lower(&typed).expect("fixture should lower"))
        .expect_err("an over-budget region is rejected");

    let CodegenError::Timing { error, span } = error else {
        panic!("an over-budget region is a timing error, got {error:?}")
    };
    assert_eq!(
        error,
        raster_timing::TimingError::OverBudget {
            measured_cycles: 6 + MASKING,
            budget: 2
        }
    );
    // The span underlines the header alone, as spec section 14 shows.
    assert_eq!(&source[span.start as usize..span.end as usize], "cycles(2)");
}

#[test]
fn a_report_region_carries_its_measured_cost_out_under_its_label() {
    let output = generate_source("main { sync exact\n cycles(?) hblank { ppu.mask = 1 } }");
    assert_eq!(output.reports, vec![("hblank".to_owned(), 6 + MASKING)]);
}

#[test]
fn wait_cycles_emits_the_planned_delay_as_a_counted_loop() {
    let output = generate_source("main { wait cycles(40) }");
    let instructions = instructions_of(&output.program.items);

    let plan = raster_timing::plan_delay(40, true).expect("a 40 cycle delay");
    assert_eq!(raster_timing::delay_cycles(&plan), 40);

    // `LDX #n`, `DEX`, `BEQ out`, `JMP back`: the loop closes on a jump, so no taken branch's
    // page-crossing penalty has to be proven away.
    let ldx = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0xa2)
        .expect("the delay loads its iteration count");
    assert_eq!(instructions[ldx + 1].opcode, 0xca, "`DEX`");
    assert_eq!(instructions[ldx + 2].opcode, 0xf0, "`BEQ` out of the loop");
    assert_eq!(instructions[ldx + 3].opcode, 0x4c, "`JMP` back to the top");
}

#[test]
fn sync_exact_lowers_to_the_documented_de_jitter_poll() {
    let output = generate_source("main { sync exact }");
    let instructions = instructions_of(&output.program.items);

    // `BIT $2002` then `BPL` back to it: the read-and-branch de-jitter of spec section 6.6.
    let bit = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0x2c && instruction.operand == Some(0x2002))
        .expect("`sync exact` polls $2002");
    assert_eq!(instructions[bit + 1].opcode, 0x10);
}

#[test]
fn a_return_inside_a_timed_block_is_refused_by_the_analyser() {
    // `raster-sema` refuses this from source, which is the friendly message — but the analyser is
    // the backstop for constructs nobody listed, so it is exercised from the IR that reaches
    // codegen rather than from source sema would have stopped first.
    let result = generate(&program_whose_timed_block_contains(Statement::Return(None)));

    assert!(
        matches!(
            result,
            Err(CodegenError::Timing {
                error: raster_timing::TimingError::ControlFlowInRegion { opcode: 0x4c, .. },
                ..
            })
        ),
        "expected a control-flow refusal, got {result:?}"
    );
}

/// `PHP` opens a region the compiler masked interrupts around, and `PLP` closes it.
const PHP: u8 = 0x08;
const PLP: u8 = 0x28;
/// `BIT $2002`, which the de-jitter poll spins on.
const BIT_ABSOLUTE: u8 = 0x2c;
/// `STA`, absolute — how every `ppu.*` write reaches its register.
const STA_ABSOLUTE: u8 = 0x8d;

fn instructions(source: &str) -> Vec<raster_6502::Instruction> {
    generate_source(source)
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            FixedBankItem::Instruction { instruction, .. } => Some(*instruction),
            FixedBankItem::Label(_) | FixedBankItem::Data(_) => None,
        })
        .collect()
}

/// The cost of each timed region in the emitted stream, in order. The fixtures here nest none, so
/// every `PHP` is closed by the next `PLP`.
fn timed_region_costs(instructions: &[raster_6502::Instruction]) -> Vec<u32> {
    let mut costs = Vec::new();
    let mut start = None;
    for (index, instruction) in instructions.iter().enumerate() {
        match instruction.opcode {
            PHP => start = Some(index),
            PLP => {
                if let Some(start) = start.take() {
                    costs.push(raster_timing::worst_case_cycles(
                        &instructions[start..=index],
                    ));
                }
            }
            _ => {}
        }
    }
    costs
}

#[test]
fn timed_lowering_repeats_a_114_cycle_scanline_body() {
    let instructions = instructions(
        r#"
            main { ppu.mask = 0 }
            frame bars using timed {
                every 4 scanlines from 100 to 108 { ppu.addr = $3f }
            }
        "#,
    );

    // One body per occurrence — 100, 104 and 108 — each padded to a whole scanline, and the
    // schedule repeated in each of the three frames one pass of the loop covers.
    assert_eq!(
        timed_region_costs(&instructions),
        vec![114; 3 * raster_timing::FRAMES_PER_PASS as usize]
    );
}

#[test]
fn a_nested_timed_block_is_not_control_flow() {
    generate_source("main { cycles(40) pad { cycles(20) pad { var v: u8 = 1 } } }");
}

/// The places the corpus below mentions. Every one needs a `PlaceDefinition` or `allocate_zero_page`
/// returns `UnknownPlace`, which is a failure that looks nothing like the one under test.
const P: Place = Place(0);
const Q: Place = Place(1);
const R: Place = Place(2);
/// An empty parameterless function, so the `Call` sample has a target `Generator::call` knows.
const HELPER: IrLabel = IrLabel(80);
const MAIN: IrLabel = IrLabel(81);
const HALT: IrLabel = IrLabel(82);

fn place_definition(place: Place) -> PlaceDefinition {
    PlaceDefinition {
        place,
        kind: PlaceKind::Local,
        span: Span::new(0, 0),
        explicit_zero_page: false,
    }
}

/// A whole IR program whose `main` is one timed block containing `statement`, and nothing else.
///
/// The budget is deliberately wide: a straight-line sample must not fail for `OverBudget`, or the
/// test would stop measuring what it means to.
fn program_whose_timed_block_contains(statement: Statement) -> Program {
    Program {
        places: vec![
            place_definition(P),
            place_definition(Q),
            place_definition(R),
        ],
        functions: vec![IrFunction {
            name: "helper".to_owned(),
            label: HELPER,
            parameters: Vec::new(),
            statements: Vec::new(),
            span: Span::new(0, 0),
        }],
        main: Some(Main {
            label: MAIN,
            halt_label: HALT,
            statements: vec![Statement::Timed {
                constraint: CycleConstraint::AtMost(10_000),
                pad: false,
                interruptible: false,
                body: vec![statement],
                span: Span::new(0, 0),
            }],
            span: Span::new(0, 0),
        }),
        ..Default::default()
    }
}

/// Whether codegen lowers this statement to something that leaves the straight line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InsideATimedBlock {
    /// Costable by summing instructions, so `analyze` accepts it.
    Straight,
    /// Emits a branch, a jump, a `JSR` or an `RTS`, so `analyze` must refuse it.
    Refused,
}

/// What a timed block does with one sample of every `raster_ir::Statement` variant.
///
/// The `match` is exhaustive on purpose: adding a variant to `raster_ir::Statement` breaks this
/// function, and whoever adds it must say whether a timed block can cost it. That is the check
/// nothing performed when `return` was added, and `main { cycles(20) pad { return } }` compiled to
/// a block charged 20 cycles that spends 8.
///
/// It guards the `Statement` axis and only that one. Codegen emits loops from `Value::Binary` too,
/// for `Multiply`, `Divide`, `Remainder`, `ShiftLeft` and `ShiftRight`, and a new `BinaryOperator`
/// that lowered to a loop would not break this test. `analyze`'s refusal is what catches that, and
/// is the durable half of the guarantee; this is the part that makes an author stop and answer.
fn expectation(statement: &Statement) -> InsideATimedBlock {
    match statement {
        Statement::Declare { .. } => InsideATimedBlock::Straight,
        Statement::Label(_) => InsideATimedBlock::Straight,
        Statement::Assign { .. } => InsideATimedBlock::Straight,
        Statement::Call { .. } => InsideATimedBlock::Refused,
        Statement::Branch { .. } => InsideATimedBlock::Refused,
        Statement::Jump { .. } => InsideATimedBlock::Refused,
        Statement::Return(_) => InsideATimedBlock::Refused,
        Statement::Timed { .. } => InsideATimedBlock::Straight,
        Statement::Delay { .. } => InsideATimedBlock::Refused,
        Statement::SyncExact => InsideATimedBlock::Refused,
    }
}

#[test]
fn every_ir_statement_is_classified_for_a_timed_block() {
    let corpus = vec![
        Statement::Declare { place: P },
        Statement::Label(IrLabel(90)),
        Statement::Assign {
            destination: Destination::Place(P),
            // A `Constant`, never a `Binary` whose operator is `Multiply`, `Divide`, `Remainder`,
            // `ShiftLeft` or `ShiftRight` — each of those lowers to a loop, so the sample would be
            // `Refused` for a reason that is about the value and not about `Assign`.
            value: Value::Constant(1),
        },
        Statement::Call {
            target: HELPER,
            arguments: Vec::new(),
            argument_temporaries: Vec::new(),
        },
        Statement::Branch {
            condition: Condition {
                left: Value::Constant(1),
                comparison: Comparison::Equal,
                right: Value::Constant(1),
                left_temporary: Q,
                right_temporary: R,
            },
            if_false: HALT,
        },
        Statement::Jump { target: HALT },
        Statement::Return(None),
        Statement::Timed {
            constraint: CycleConstraint::AtMost(10_000),
            pad: false,
            interruptible: false,
            body: vec![Statement::Declare { place: P }],
            span: Span::new(0, 0),
        },
        // 1000 cycles, not a handful: `plan_delay` emits no loop at all for a delay short enough
        // to fill with filler, and this sample is about the loop.
        Statement::Delay {
            cycles: 1000,
            span: Span::new(0, 0),
        },
        Statement::SyncExact,
    ];
    // One per `raster_ir::Statement` variant, so a variant added to `expectation` without a sample
    // here fails too.
    assert_eq!(corpus.len(), 10);

    for statement in corpus {
        let expected = expectation(&statement);
        let result = generate(&program_whose_timed_block_contains(statement.clone()));
        match expected {
            InsideATimedBlock::Refused => assert!(
                matches!(
                    result,
                    Err(CodegenError::Timing {
                        error: raster_timing::TimingError::ControlFlowInRegion { .. },
                        ..
                    })
                ),
                "{statement:?} must be refused inside a timed block, got {result:?}"
            ),
            InsideATimedBlock::Straight => assert!(
                result.is_ok(),
                "{statement:?} is costable inside a timed block, got {result:?}"
            ),
        }
    }
}

#[test]
fn timed_frame_requires_sync_before_rendering_ppu_write() {
    let instructions = instructions(
        r#"
            main { var value: u8 = 0 }
            frame bars using timed {
                at scanline 60 { ppu.addr = $3f }
            }
        "#,
    );

    let sync = instructions
        .iter()
        .position(|instruction| {
            instruction.opcode == BIT_ABSOLUTE && instruction.operand == Some(0x2002)
        })
        .expect("a timed frame de-jitters against the PPU before it runs a handler");
    let write = instructions
        .iter()
        .position(|instruction| {
            instruction.opcode == STA_ABSOLUTE && instruction.operand == Some(0x2006)
        })
        .expect("the handler writes a PPU register");

    assert!(
        sync < write,
        "the frame's PPU write must follow the synchronization, not precede it"
    );
}

#[test]
fn a_timed_frame_delays_from_the_origin_to_each_handlers_scanline() {
    let output = generate_source(
        r#"
            main { ppu.mask = 0 }
            frame bars using timed {
                at scanline 60 { ppu.addr = $3f }
                at scanline 120 { ppu.addr = $00 }
            }
        "#,
    );
    let instructions: Vec<_> = output
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            FixedBankItem::Instruction { instruction, .. } => Some(*instruction),
            FixedBankItem::Label(_) | FixedBankItem::Data(_) => None,
        })
        .collect();

    let pass = raster_timing::plan_timed_frame(&[60, 120], 3);
    assert!(pass.handlers.iter().all(|handler| handler.delay_cycles > 0));
    assert_eq!(
        timed_region_costs(&instructions),
        vec![114; 2 * raster_timing::FRAMES_PER_PASS as usize]
    );
}

#[test]
fn a_handler_that_does_not_fit_its_scanline_names_its_cost_and_its_budget() {
    let syntax = parse(
        r#"
            main { ppu.mask = 0 }
            frame bars using timed {
                at scanline 60 {
                    cycles(200) pad { ppu.addr = $3f }
                }
            }
        "#,
    )
    .expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    let error = generate(&lower(&typed).expect("fixture should lower"))
        .expect_err("two hundred cycles do not fit a scanline");

    assert!(
        matches!(
            error,
            CodegenError::Timing {
                error: raster_timing::TimingError::OverBudget { budget: 114, .. },
                ..
            }
        ),
        "expected an over-budget scanline body, found {error:?}"
    );
}

#[test]
fn nothing_unmasks_irqs_before_main_has_run() {
    const CLI: u8 = 0x58;

    let irq_source = r#"
        main {
            ppu.ctrl = $10
            ppu.mask = $18
        }
        frame bars using irq {
            at scanline 32 { ppu.mask = $1e }
            at scanline 64 { ppu.mask = $18 }
        }
    "#;
    let timed_source = r#"
        main {
            ppu.mask = 0
        }
        frame bars using timed {
            every 8 scanlines from 0 to 239 { ppu.mask = 0 }
        }
    "#;

    let mut irq_frame_had_a_cli = false;

    for (strategy, source) in [("using irq", irq_source), ("using timed", timed_source)] {
        let syntax = parse(source).expect("fixture should parse");
        let typed = analyze(&syntax).expect("fixture should analyze");
        let ir = lower(&typed).expect("fixture should lower");
        let halt = ir
            .main
            .as_ref()
            .expect("the fixture has a `main`")
            .halt_label;
        let output = generate(&ir).expect("fixture should generate");

        let halt_index = output
            .program
            .items
            .iter()
            .position(|item| matches!(item, FixedBankItem::Label(label) if label.0 == halt.0))
            .expect("codegen emits `main`'s halt label");

        for (index, item) in output.program.items.iter().enumerate() {
            let FixedBankItem::Instruction { instruction, .. } = item else {
                continue;
            };
            if instruction.opcode != CLI {
                continue;
            }
            if strategy == "using irq" {
                irq_frame_had_a_cli = true;
            }
            assert!(
                index > halt_index,
                "a `CLI` is emitted at item {index}, before `main`'s halt label at item \
                 {halt_index}.\n\n\
                 rasterc's MMC3 bank tracking assumes nothing runs between two of `main`'s\n\
                 statements: it folds a `mmc3.bank_select` and judges the `mmc3.bank_data`\n\
                 write that follows it against the register that select named. If interrupts\n\
                 are unmasked while `main` is still running, a `frame` handler's own bank\n\
                 select can land between the two and the write moves a window the author\n\
                 never named — silently, because the tracking still believes the fold.\n\n\
                 That needs a warning before this change ships. Spec section 9.4 says\n\
                 `main` is uninterrupted; either that paragraph changes with this, or\n\
                 this does not ship."
            );
        }
    }

    assert!(
        irq_frame_had_a_cli,
        "the `frame ... using irq` fixture emitted no `CLI` at all, so this test\n\
         checked nothing. Either the arming sequence stopped unmasking interrupts,\n\
         or the fixture stopped lowering to an IRQ chain. Fix the fixture rather\n\
         than deleting the assertion."
    );
}
