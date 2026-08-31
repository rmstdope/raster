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
