use raster_6502::AddressingMode;
use raster_codegen::{generate, CodegenError};
use raster_ir::lower;
use raster_link::{
    link_mmc3_program, FixedBankItem, Label, RelocationKind, INES_HEADER_SIZE,
    MMC3_FIXED_BANK_SIZE, MMC3_PRG_ROM_SIZE,
};
use raster_sema::analyze;
use raster_syntax::parse;

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
    let rom = link_mmc3_program(&output.program, output.main, true).unwrap();
    let fixed_bank = INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE;
    // The fixed bank now opens with the runtime's reset sequence, and the
    // program's own first bytes follow the 71-byte prologue.
    assert_eq!(
        &rom.image[fixed_bank..fixed_bank + 4],
        &[0x78, 0xd8, 0xa2, 0x40]
    );
    assert_eq!(
        &rom.image[fixed_bank + 71..fixed_bank + 76],
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
