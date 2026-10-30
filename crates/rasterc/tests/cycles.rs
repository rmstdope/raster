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
    // Instructions only, in layout order: the islands a loop is kept on are laid out after all the
    // code, behind a page alignment.
    output
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            raster_link::FixedBankItem::Instruction { instruction, .. } => Some(*instruction),
            _ => None,
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

/// Every timed region in the emitted stream, in order.
///
/// A frame fixture has one per handler per frame of the pass, so it can never go through
/// [`timed_region`] — and it has nothing to measure against either, because the measurement half
/// of this file executes the ROM to its first `PHP`. What a frame's regions are compared with is
/// the schedule `raster_timing::plan_timed_frame` places them on.
fn timed_regions(source: &str) -> Vec<Vec<Instruction>> {
    let instructions = emitted(source);
    let mut regions = Vec::new();
    let mut start = None;
    for (index, instruction) in instructions.iter().enumerate() {
        match instruction.opcode {
            PHP => start = Some(index),
            PLP => {
                if let Some(start) = start.take() {
                    regions.push(instructions[start..=index].to_vec());
                }
            }
            _ => {}
        }
    }
    regions
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
fn a_vram_addr_write_costs_twelve_cycles_inside_an_exact_region() {
    let source = fixture("vram-addr.raster");
    assert!(compile_source(&source).is_ok());
    // The region's 21 cycles are the nine of interrupt masking and the twelve of the pair.
    assert_eq!(cost(&timed_region(&source)), 21);
    let opcodes: Vec<u8> = timed_region(&source)
        .iter()
        .map(|instruction| instruction.opcode)
        .collect();
    // PHP SEI, LDA #$3F STA $2006, LDA #$00 STA $2006, PLP: nothing between the two stores.
    assert_eq!(opcodes, [PHP, 0x78, 0xA9, 0x8D, 0xA9, 0x8D, PLP]);
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

/// A fixture whose measured cost is its prediction, or one cycle less.
///
/// Every other fixture in this file is exact, and that is the point of them. This one holds an
/// OAM DMA, whose 513-or-514-cycle stall the compiler charges at 514 because it cannot know the
/// parity of the cycle the write lands on. One cycle of slack is the uncertainty made visible;
/// two would be a bug in the cost model.
fn assert_prediction_is_spent_within_one(name: &str) {
    let source = fixture(name);
    let predicted = cost(&timed_region(&source));
    let actual = measured(name, &source);
    let difference = i64::from(actual) - i64::from(predicted);

    assert!(
        (-1..=0).contains(&difference),
        "{name}: the compiler predicted {predicted} cycles and the region spent {actual}, \
         a difference of {difference} — an OAM DMA may come in one cycle under and no more"
    );
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
        "vram-addr.raster",
    ] {
        assert_prediction_is_spent(name);
    }
}

/// Deliberately not in `every_timed_fixture_spends_exactly_what_the_compiler_predicted`: that
/// list is the exact-agreement guarantee, and this is the one construct rasterc emits whose
/// predicted cost is not necessarily its cost.
#[test]
fn an_oam_dma_fixture_spends_its_prediction_or_one_cycle_less() {
    assert_prediction_is_spent_within_one("oam-dma.raster");
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

    // `DelayStep::Loop` closes on a `JMP` so that only one branch is ever taken, and the loop runs
    // in an island kept on one page, so that branch never pays for crossing one: the delay spends
    // exactly what was planned, wherever the linker puts it.
    compare("delay-bracketed.raster", predicted, actual);
}

#[test]
fn a_frame_schedule_is_sorted_and_every_occurrence_gets_its_own_scanline_body() {
    let source = fixture("frame-schedule.raster");
    assert!(compile_source(&source).is_ok());

    // Three `at` and `every` occurrences between them — 40, 96, 104, 112, 200 — each padded to a
    // whole scanline, and the schedule run in each of the three frames one pass covers.
    let bodies: Vec<_> = timed_regions(&source)
        .iter()
        .map(|region| cost(region))
        .collect();
    assert_eq!(
        bodies,
        vec![114; 5 * raster_timing::FRAMES_PER_PASS as usize]
    );
}

#[test]
fn a_frame_handler_that_does_not_fit_its_scanline_names_its_cost_and_its_budget() {
    let diagnostics = compile_source(&fixture("frame-over-budget.raster"))
        .expect_err("two hundred cycles do not fit a scanline");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "timed block exceeds its budget");
    assert!(
        diagnostics[0].label.ends_with("budget is 114"),
        "the budget is the scanline's, found {:?}",
        diagnostics[0].label
    );
    assert!(
        diagnostics[0].span.is_some(),
        "the diagnostic is source-spanned"
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

#[test]
fn a_return_inside_a_timed_block_is_refused_rather_than_mistimed() {
    let diagnostics = compile_source(&fixture("return-in-a-block.raster"))
        .expect_err("a block that jumps out has no provable cost");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "`return` inside a timed block jumps out before the block has spent its budget and before \
         the interrupt flag is restored, so it belongs after the block rather than inside one"
    );
    assert!(
        diagnostics[0].span.is_some(),
        "the diagnostic is source-spanned"
    );
}

#[test]
fn an_unsynchronized_ppu_write_is_refused_outside_a_frame() {
    let diagnostics = compile_source(&fixture("unsynchronized.raster"))
        .expect_err("a timed PPU write needs something to align it");

    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics[0]
            .message
            .starts_with("`sync exact` is required"),
        "found {:?}",
        diagnostics[0].message
    );
}

#[test]
fn a_table_read_spends_exactly_the_budget_its_region_was_held_to() {
    // Not `assert_prediction_is_spent`: `cost` charges every indexed read a page crossing, and
    // this one is proven not to make one. The budget is what the compiler accepted the region at.
    let source = fixture("table-read.raster");
    compare(
        "table-read.raster",
        21,
        measured("table-read.raster", &source),
    );
}

#[test]
fn direct_operands_spend_exactly_the_budget_their_region_was_held_to() {
    // The budget, not `cost`: the region holds two table reads proven never to cross a page.
    let source = fixture("direct-operands.raster");
    compare(
        "direct-operands.raster",
        57,
        measured("direct-operands.raster", &source),
    );
}

#[test]
fn a_delay_spends_exactly_its_planned_cycles_wherever_its_loop_lands() {
    // `value = 1` is four bytes and `ppu.mask = 0` five, so 0 to 63 of one and 0 to 3 of the other
    // ahead of the delay move its loop through every position on a page, among them the ones where
    // the branch closing the loop crosses into the next. A timed frame never re-synchronizes, so
    // one cycle more there is a cycle more every pass.
    let planned = delay_cycles(&plan_delay(1000, true).expect("a thousand cycles are reachable"));
    let predicted = planned + cost(&[bare(PLP), bare(PHP)]);
    let mut wrong = Vec::new();
    for shift in (0..64).flat_map(|fours| (0..4).map(move |fives| (fours, fives))) {
        let source = format!(
            "main {{\n    var value: u8 = 0\n{}{}    cycles(?) before {{\n        value = 1\n    }}\n    wait cycles(1000)\n    cycles(?) after {{\n        value = 2\n    }}\n}}\n",
            "    value = 1\n".repeat(shift.0),
            "    ppu.mask = 0\n".repeat(shift.1)
        );
        let actual = cycles_between(
            "shifted-delay.raster",
            &compile_source(&source).expect("the fixture compiles").image,
            Window::new(PLP, PHP),
        )
        .expect("the fixture is measurable");
        if actual != predicted {
            wrong.push((shift, actual));
        }
    }
    assert!(
        wrong.is_empty(),
        "the delay was planned at {predicted} cycles and spent otherwise \
         ((four-byte, five-byte statements), cycles): {wrong:?}"
    );
}

/// What `wait cycles(cycles)` spends between the regions bracketing it, measured.
fn bracketed_delay(cycles: u32) -> u32 {
    let source = format!(
        "main {{\n    var value: u8 = 0\n    cycles(?) before {{\n        value = 1\n    }}\n    wait cycles({cycles})\n    cycles(?) after {{\n        value = 2\n    }}\n}}\n"
    );
    cycles_between(
        "bracketed-delay.raster",
        &compile_source(&source).expect("the fixture compiles").image,
        Window::new(PLP, PHP),
    )
    .expect("the fixture is measurable")
}

#[test]
fn a_delay_too_short_to_loop_on_its_own_page_is_spent_exactly() {
    // Seven cycles plan as a loop, and the jumps to an island and back would leave it one.
    for cycles in [7, 8, 12, 14] {
        compare(
            &format!("wait cycles({cycles})"),
            cycles + cost(&[bare(PLP), bare(PHP)]),
            bracketed_delay(cycles),
        );
    }
}

#[test]
fn a_delay_too_long_for_one_page_still_compiles() {
    // Nine million cycles, about five seconds, are more loops than one page holds, so each loop
    // has an island of its own. Too long to measure here, since the emulator stops at two million
    // instructions; how exactly each island spends its loop is measured by the delays above.
    compile_source("main {\n    wait cycles(9000000)\n}\n")
        .map(|_| ())
        .expect("a long delay compiles");

    // A million and a half is still several loops, each in its own island, and short enough to
    // measure: the jumps between them are paid for exactly.
    let cycles = 1_500_000;
    compare(
        "wait cycles(1500000)",
        cycles + cost(&[bare(PLP), bare(PHP)]),
        bracketed_delay(cycles),
    );
}

#[test]
fn a_delay_whose_loop_count_turns_on_its_jumps_is_spent_exactly() {
    // Paying six cycles of jumps for each of these lengths' loops leaves a length planned with one
    // loop fewer, and the compiler once searched back and forth between the two for ever.
    for cycles in [1812, 3613] {
        compare(
            &format!("wait cycles({cycles})"),
            cycles + cost(&[bare(PLP), bare(PHP)]),
            bracketed_delay(cycles),
        );
    }

fn constant_arithmetic_in_a_timed_region_is_folded_and_reported_at_its_cost() {
    let source = fixture("constant-arithmetic.raster");
    let rom = compile_source(&source).expect("constant arithmetic has a fixed cost");
    assert_eq!(rom.reports, vec![("c".to_owned(), 15)]);
    let region: Vec<(u8, Option<u16>)> = timed_region(&source)
        .iter()
        .map(|instruction| (instruction.opcode, instruction.operand))
        .collect();
    // PHP SEI, LDA #$06 STA $2007, PLP: the product is a constant, not a loop.
    assert_eq!(
        region,
        [
            (PHP, None),
            (0x78, None),
            (0xA9, Some(6)),
            (0x8D, Some(0x2007)),
            (PLP, None)
        ]
    );
    assert_prediction_is_spent("constant-arithmetic.raster");
}

#[test]
fn a_vram_addr_written_as_constant_arithmetic_builds_in_a_timed_region() {
    let source = fixture("constant-vram-addr.raster");
    let rom = compile_source(&source).expect("constant arithmetic has a fixed cost");
    assert_eq!(rom.reports, vec![("v".to_owned(), 21)]);
    let operands: Vec<Option<u16>> = timed_region(&source)
        .iter()
        .filter(|instruction| instruction.opcode == 0xA9)
        .map(|instruction| instruction.operand)
        .collect();
    assert_eq!(operands, [Some(0x20), Some(0x82)]);
    assert_prediction_is_spent("constant-vram-addr.raster");
}
