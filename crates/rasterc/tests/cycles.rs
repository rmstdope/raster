//! The `tests/cycles` fixtures, compiled and checked against the cost the compiler predicts.
//!
//! The fixtures live at the repository root rather than beside this file because the same sources
//! are run under an emulator here too, comparing the compiler's prediction with what a real 6502
//! spends. A prediction nothing independent agrees with is not evidence.
//!
//! Only the marked window is measured — never whole-ROM execution — so reset, the two-vblank wait
//! and the emulator's own startup fall outside every count.

use std::{
    fs,
    panic::{self, UnwindSafe},
    path::PathBuf,
};

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
///
/// The measurement half of this file finds its region by *executing* the ROM until a `PHP`, while
/// this half finds it by scanning what codegen *emitted*; the two coincide only while a fixture
/// has exactly one region, in `main`. `raster-codegen` emits every function before `main`, and it
/// supports nested regions, so either would silently make these two halves describe different
/// code — and a nested region would make first-`PHP`-to-first-`PLP` no region at all. A fixture
/// that grows a second pair fails here rather than passing while comparing nonsense.
fn timed_region(source: &str) -> Vec<Instruction> {
    let instructions = emitted(source);
    let pairs = |opcode: u8| {
        instructions
            .iter()
            .filter(|instruction| instruction.opcode == opcode)
            .count()
    };
    assert_eq!(
        (pairs(PHP), pairs(PLP)),
        (1, 1),
        "this file compares one region per fixture, and its two halves would pick different ones"
    );
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

/// Compare a prediction with a measurement, failing with everything a mismatch is about.
///
/// Every measurement in this file goes through here, so the diagnostic the plan asked for cannot
/// be bypassed by a test that reaches for `assert_eq!` instead: two anonymous numbers would leave
/// whoever reads the failure to work out which fixture produced them and which way round.
fn compare(name: &str, predicted: u32, actual: u32) {
    assert!(
        predicted == actual,
        "{name}: the compiler predicted {predicted} cycles and the region spent {actual}, \
         a difference of {}",
        i64::from(actual) - i64::from(predicted)
    );
}

/// A fixture whose predicted cost a 6502 is expected to spend exactly.
///
/// The prediction and the measurement come from opposite ends of the compiler — one from the
/// instruction cost table, one from executing the ROM that table produced — so their agreement is
/// evidence rather than a tautology.
fn assert_prediction_is_spent(name: &str) {
    let source = fixture(name);
    compare(name, cost(&timed_region(&source)), measured(name, &source));
}

/// The message a failing comparison actually fails with.
///
/// Asserting the formatter's output directly would leave the diagnostic green after it stopped
/// being reached; this goes through the panic [`compare`] raises, which is the path a drifting
/// fixture takes. The hook is silenced only so a passing run does not print a backtrace that
/// reads like a failure.
fn failure_of(comparison: impl FnOnce() + UnwindSafe) -> String {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let payload = panic::catch_unwind(comparison);
    panic::set_hook(previous);

    match payload
        .expect_err("the comparison fails")
        .downcast::<String>()
    {
        Ok(message) => *message,
        Err(_) => panic!("the failure carries a message"),
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
    let predicted = cost(&timed_region(&source));

    assert_eq!(predicted, 114, "one NTSC scanline is 114 cycles");
    compare(
        "padded.raster",
        predicted,
        measured("padded.raster", &source),
    );
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
fn a_delay_spends_its_planned_cycles_across_the_branch_its_loop_takes() {
    let source = fixture("delay-bracketed.raster");
    let planned = delay_cycles(&plan_delay(1000, true).expect("a thousand cycles are reachable"));
    assert_eq!(planned, 1000, "the plan spends what the source asked for");

    // The window runs from the `PLP` closing the first region to the `PHP` opening the second,
    // so it costs those two instructions as well as the delay between them.
    let predicted = planned + cost(&[bare(PLP), bare(PHP)]);
    let actual = cycles_between(
        "delay-bracketed.raster",
        &compile_source(&source).expect("the fixture compiles").image,
        Window::new(PLP, PHP),
    )
    .expect("the fixture is measurable");

    // `DelayStep::Loop` closes on a `JMP` so that only one branch is ever taken, and the compiler
    // reserves exactly one cycle for that branch crossing a page. Asserting the whole range is
    // asserting the guarantee; asserting a single number would assert a linker layout instead.
    //
    // Which also means the upper end is unexercised: this loop happens to sit within one page, so
    // the reserved cycle is charged by `worst_case_cycles` and measured by nothing. Forcing the
    // branch across a page needs alignment control the linker cannot yet express.
    assert!(
        (predicted..=predicted + 1).contains(&actual),
        "delay-bracketed.raster: the compiler predicted {predicted} cycles and the delay spent \
         {actual}, more than the one cycle a page-crossing branch is allowed"
    );
}

#[test]
fn measurement_failure_identifies_fixture_and_delta() {
    let over = failure_of(|| compare("padded.raster", 114, 118));
    assert!(over.contains("padded.raster"), "{over}");
    assert!(over.contains("114"), "{over}");
    assert!(over.contains("118"), "{over}");
    assert!(over.contains("a difference of 4"), "{over}");

    let under = failure_of(|| compare("exact.raster", 14, 12));
    assert!(under.contains("a difference of -2"), "{under}");

    compare("padded.raster", 114, 114);
}
