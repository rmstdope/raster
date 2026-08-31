//! The `tests/cycles` fixtures, compiled and checked against the cost the compiler predicts.
//!
//! The fixtures live at the repository root rather than beside this file because the
//! emulator-measurement bead runs the same sources and compares the compiler's prediction with what
//! a real 6502 spends. A prediction nothing independent agrees with is not evidence.

use std::{fs, path::PathBuf};

use raster_6502::Instruction;
use raster_codegen::generate_with_isa;
use raster_ir::lower;
use raster_link::FixedBankItem;
use raster_sema::analyze;
use raster_syntax::parse;
use rasterc::compile_source;

/// `PHP` opens a region the compiler masked interrupts around, and `PLP` restores the flag it saved.
/// Both are inside the measured window: the budget pays for the masking.
const PHP: u8 = 0x08;
const PLP: u8 = 0x28;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cycles")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()))
}

/// Every instruction the fixture generates, in order.
fn emitted(source: &str) -> Vec<Instruction> {
    let syntax = parse(source).expect("the fixture parses");
    let typed = analyze(&syntax).expect("the fixture analyses");
    // The same ISA policy `compile_source` builds a ROM under, so this measures what ships.
    let output = generate_with_isa(&lower(&typed).expect("the fixture lowers"), true)
        .expect("the fixture generates");
    output
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            FixedBankItem::Instruction { instruction, .. } => Some(*instruction),
            FixedBankItem::Label(_) => None,
        })
        .collect()
}

/// The instructions between the region's `SEI` and `PLP` — exactly what was analysed.
fn timed_region(source: &str) -> Vec<Instruction> {
    let instructions = emitted(source);
    let start = instructions
        .iter()
        .position(|instruction| instruction.opcode == PHP)
        .expect("a timed region saves the interrupt flag on the way in");
    let end = instructions
        .iter()
        .position(|instruction| instruction.opcode == PLP)
        .expect("a timed region restores it on the way out");
    instructions[start..=end].to_vec()
}

#[test]
fn an_exact_region_costs_exactly_its_budget() {
    let source = fixture("exact.raster");
    assert!(compile_source(&source).is_ok());
    assert_eq!(cost(&timed_region(&source)), 14);
}

#[test]
fn a_padded_scanline_region_is_filled_to_one_hundred_and_fourteen_cycles() {
    let source = fixture("padded.raster");
    compile_source(&source)
        .map(|_| ())
        .expect("the fixture compiles");
    assert_eq!(cost(&timed_region(&source)), 114);
}

#[test]
fn an_upper_bound_region_is_left_at_its_own_cost() {
    let source = fixture("upper-bound.raster");
    assert!(compile_source(&source).is_ok());
    assert_eq!(cost(&timed_region(&source)), 15);
}

#[test]
fn a_report_region_reaches_the_build_summary_with_its_measured_cost() {
    let rom = compile_source(&fixture("report.raster")).expect("the fixture compiles");
    assert_eq!(rom.reports, vec![("hblank".to_owned(), 15)]);
}

#[test]
fn a_delay_fixture_compiles_to_a_rom() {
    compile_source(&fixture("delay.raster"))
        .map(|_| ())
        .expect("the fixture compiles");
}

#[test]
fn an_over_budget_fixture_names_its_cost_and_its_budget() {
    let diagnostics =
        compile_source(&fixture("over-budget.raster")).expect_err("six cycles do not fit four");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "timed block exceeds its budget");
    assert_eq!(diagnostics[0].label, "block costs 15 cycles, budget is 4");
    assert!(
        diagnostics[0].span.is_some(),
        "the diagnostic is source-spanned"
    );
}

/// The cost the compiler itself holds a region to. Re-deriving the sum here would only prove that
/// this file agrees with itself.
fn cost(instructions: &[Instruction]) -> u32 {
    raster_timing::worst_case_cycles(instructions)
}
