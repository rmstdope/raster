//! The `tests/cycles` fixtures, compiled and checked against the cost the compiler predicts.
//!
//! The fixtures live at the repository root rather than beside this file because the same sources
//! are run under an emulator here too, comparing the compiler's prediction with what a real 6502
//! spends. A prediction nothing independent agrees with is not evidence.
//!
//! Only the marked window is measured — never whole-ROM execution — so reset, the two-vblank wait
//! and the emulator's own startup fall outside every count.

use std::{fs, path::PathBuf};

use raster_6502::Instruction;
use raster_codegen::generate_with_isa;
use raster_emu::{cycles_between, Window};
use raster_ir::lower;
use raster_link::FixedBankItem;
use raster_sema::analyze;
use raster_syntax::parse;
use raster_timing::{delay_cycles, plan_delay};
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

/// The cycles the ROM's first timed region actually spends, measured by executing it.
fn measured(name: &str, source: &str) -> u32 {
    let rom = compile_source(source).expect("the fixture compiles");
    cycles_between(name, &rom.image, Window::new(PHP, PLP))
        .unwrap_or_else(|error| panic!("{name} is measurable: {error}"))
}

/// How a prediction and a measurement disagree, or `None` when they do not.
///
/// A bare `assert_eq!` would name two numbers and leave whoever reads the failure to work out
/// which fixture produced them and which way round they went, so the message says all four things
/// a mismatch is about: the fixture, what was predicted, what was spent, and the gap.
fn disagreement(name: &str, predicted: u32, actual: u32) -> Option<String> {
    (predicted != actual).then(|| {
        format!(
            "{name}: the compiler predicted {predicted} cycles and the region spent {actual}, \
             a difference of {}",
            i64::from(actual) - i64::from(predicted)
        )
    })
}

/// A fixture whose predicted cost a 6502 is expected to spend exactly.
///
/// The prediction and the measurement come from opposite ends of the compiler — one from the
/// instruction cost table, one from executing the ROM that table produced — so their agreement is
/// evidence rather than a tautology.
fn assert_prediction_is_spent(name: &str) {
    let source = fixture(name);
    let predicted = cost(&timed_region(&source));
    let actual = measured(name, &source);

    if let Some(message) = disagreement(name, predicted, actual) {
        panic!("{message}");
    }
}

/// One instruction with no operand, for costing a marker the window includes.
fn bare(opcode: u8) -> Instruction {
    Instruction {
        opcode,
        mode: raster_6502::opcode(opcode).mode,
        operand: None,
    }
}

#[test]
fn exact_padded_region_executes_for_114_cycles() {
    let source = fixture("padded.raster");
    assert_eq!(cost(&timed_region(&source)), 114, "the prediction is 114");
    assert_eq!(measured("padded.nes", &source), 114);
}

#[test]
fn every_timed_fixture_spends_exactly_what_the_compiler_predicted() {
    for name in [
        "exact.raster",
        "padded.raster",
        "upper-bound.raster",
        "report.raster",
    ] {
        assert_prediction_is_spent(name);
    }
}

#[test]
fn predictions_cover_the_branch_a_delay_loop_takes() {
    let source = fixture("delay-bracketed.raster");
    let planned = delay_cycles(&plan_delay(1000, true).expect("a thousand cycles are reachable"));
    assert_eq!(planned, 1000, "the plan spends what the source asked for");

    // The window runs from the `PLP` closing the first region to the `PHP` opening the second,
    // so it costs those two instructions as well as the delay between them.
    let predicted = planned + cost(&[bare(PLP), bare(PHP)]);
    let actual = cycles_between(
        "delay-bracketed.nes",
        &compile_source(&source).expect("the fixture compiles").image,
        Window::new(PLP, PHP),
    )
    .expect("the fixture is measurable");

    // `DelayStep::Loop` closes on a `JMP` so that only one branch is ever taken, and the compiler
    // reserves exactly one cycle for that branch crossing a page. Asserting the whole range is
    // asserting the guarantee; asserting a single number would assert a linker layout instead.
    assert!(
        (predicted..=predicted + 1).contains(&actual),
        "delay-bracketed.raster: the compiler predicted {predicted} cycles and the delay spent \
         {actual}, more than the one cycle a page-crossing branch is allowed"
    );
}

#[test]
fn measurement_failure_identifies_fixture_and_delta() {
    let message = disagreement("padded.raster", 114, 118).expect("118 cycles are not 114");
    assert!(message.contains("padded.raster"), "{message}");
    assert!(message.contains("114"), "{message}");
    assert!(message.contains("118"), "{message}");
    assert!(message.contains("a difference of 4"), "{message}");

    let under = disagreement("exact.raster", 14, 12).expect("12 cycles are not 14");
    assert!(under.contains("a difference of -2"), "{under}");

    assert_eq!(disagreement("padded.raster", 114, 114), None);
}
