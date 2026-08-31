//! The `tests/cycles` fixtures, compiled and checked against the cost the compiler predicts.
//!
//! The fixtures live at the repository root rather than beside this file because the
//! emulator-measurement bead runs the same sources and compares the compiler's prediction with what
//! a real 6502 spends. A prediction nothing independent agrees with is not evidence.

use std::{fs, path::PathBuf};

use raster_6502::{cycles, CycleContext, Instruction};
use raster_codegen::generate_with_isa;
use raster_ir::lower;
use raster_link::FixedBankItem;
use raster_sema::analyze;
use raster_syntax::parse;
use rasterc::compile_source;

/// `SEI` opens a region the compiler masked interrupts around, and `CLI` closes it.
const SEI: u8 = 0x78;
const CLI: u8 = 0x58;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cycles")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()))
}

/// The instructions between the region's `SEI` and `CLI` — exactly what was analysed.
fn timed_region(source: &str) -> Vec<Instruction> {
    let syntax = parse(source).expect("the fixture parses");
    let typed = analyze(&syntax).expect("the fixture analyses");
    // The same ISA policy `compile_source` builds a ROM under, so this measures what ships.
    let output = generate_with_isa(&lower(&typed).expect("the fixture lowers"), true)
        .expect("the fixture generates");
    let instructions: Vec<_> = output
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            FixedBankItem::Instruction { instruction, .. } => Some(*instruction),
            FixedBankItem::Label(_) => None,
        })
        .collect();
    let start = instructions
        .iter()
        .position(|instruction| instruction.opcode == SEI)
        .expect("a timed region is entered with interrupts masked");
    let end = instructions
        .iter()
        .position(|instruction| instruction.opcode == CLI)
        .expect("a timed region restores interrupts on the way out");
    instructions[start + 1..end].to_vec()
}

#[test]
fn an_exact_region_costs_exactly_its_budget() {
    let source = fixture("exact.raster");
    assert!(compile_source(&source).is_ok());
    assert_eq!(cost(&timed_region(&source)), 5);
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
    assert_eq!(cost(&timed_region(&source)), 6);
}

#[test]
fn a_report_region_reaches_the_build_summary_with_its_measured_cost() {
    let rom = compile_source(&fixture("report.raster")).expect("the fixture compiles");
    assert_eq!(rom.reports, vec![("hblank".to_owned(), 6)]);
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
    assert_eq!(diagnostics[0].label, "block costs 6 cycles, budget is 4");
    assert!(
        diagnostics[0].span.is_some(),
        "the diagnostic is source-spanned"
    );
}

fn cost(instructions: &[Instruction]) -> u32 {
    cycles(instructions, CycleContext::default())
}
