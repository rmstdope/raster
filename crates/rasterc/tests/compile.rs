use rasterc::compile_source;

mod common;
use common::demo_source;

#[test]
fn compiles_the_demo_to_the_same_rom_as_m1() {
    let rom = compile_source(&demo_source()).expect("the demo compiles");

    assert_eq!(rom.image, raster_link::m1_solid_backdrop_rom());
    assert_eq!(rom.code_len, 160);
    assert_eq!(rom.runtime_len, 132);
    assert_eq!(rom.code_len - rom.runtime_len, 28);
}

const SUPPORTED_SUBSET: &str = concat!(
    "this release compiles `main`, `fn`, `if`, `while`, `for`, u8\n",
    "arithmetic, and `ppu.*` / `mmc3.*` register writes; timed regions\n",
    "with `cycles`, `pad`, `sync exact` and `wait cycles`; and one\n",
    "`frame` of `every ... scanlines` events"
);

#[test]
fn reports_every_error_from_the_failing_stage() {
    let diagnostics =
        compile_source("frame display { at vblank {} }\nmain {\n    loop {}\n    wait vblank\n}\n")
            .expect_err("three unsupported constructs are three errors");

    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        [
            "`at vblank` is not supported yet",
            "`loop` is not supported yet",
            "only `wait cycles` is supported yet; frame waits arrive with frame scheduling",
        ]
    );

    // The subset is listed once per run, beside the first refusal.
    assert_eq!(diagnostics[0].notes, [SUPPORTED_SUBSET]);
    assert!(diagnostics[1].notes.is_empty());
    assert!(diagnostics[2].notes.is_empty());
}

#[test]
fn stops_at_the_first_failing_stage() {
    let diagnostics =
        compile_source("@\nmain {\n    wait vblank\n}\n").expect_err("a syntax error is an error");

    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        ["expected a top-level declaration"]
    );
    assert!(diagnostics[0].notes.is_empty());
}

#[test]
fn a_program_with_no_main_says_where_a_rom_starts() {
    let diagnostics =
        compile_source("const A: u8 = $01\n").expect_err("a ROM needs somewhere to start");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "this program has no `main` block");
    assert_eq!(diagnostics[0].span, None);
    assert_eq!(
        diagnostics[0].notes,
        ["add `main { ... }` to give the ROM somewhere to start"]
    );
}

#[test]
fn too_many_variables_names_the_zero_page_that_ran_out() {
    let declarations = (0..241)
        .map(|index| format!("var value{index}: u8"))
        .collect::<Vec<_>>()
        .join("\n");
    let diagnostics = compile_source(&format!("{declarations}\nmain {{}}\n"))
        .expect_err("the zero page is finite");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "too many variables for the zero page"
    );
    // This one does point somewhere: the declaration that did not fit.
    assert!(diagnostics[0].span.is_some());
    assert_eq!(
        diagnostics[0].notes,
        ["the zero page holds 240 variables, from $10 to $FF"]
    );
}

#[test]
fn a_program_too_large_names_the_bank_it_does_not_fit() {
    let body = "    ppu.data = $01\n".repeat(2000);
    let diagnostics =
        compile_source(&format!("main {{\n{body}}}\n")).expect_err("the fixed bank is finite");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "the program does not fit the MMC3 fixed bank"
    );
    assert_eq!(diagnostics[0].span, None);
    assert_eq!(diagnostics[0].notes.len(), 3);
    assert!(
        diagnostics[0].notes[0].ends_with(" bytes of code, and $E000-$FFFF holds 8186"),
        "unexpected note: {}",
        diagnostics[0].notes[0]
    );
    // The overshoot depends on what 2000 statements compile to, so the figures
    // are left to `a_program_too_big_for_the_bank_is_told_how_much_of_its_own_has_to_go`
    // and only the shape is pinned here.
    assert!(
        diagnostics[0].notes[1].starts_with("132 of those are the reset runtime, so ")
            && diagnostics[0].notes[1].ends_with(" bytes of your own\nhave to go"),
        "unexpected note: {}",
        diagnostics[0].notes[1]
    );
    assert_eq!(
        diagnostics[0].notes[2],
        "PRG bank switching is not supported yet, so all code lives\nin the fixed bank"
    );
}

#[test]
fn a_cycle_annotated_function_that_returns_still_names_the_real_refusal() {
    let diagnostics = compile_source("fn f() -> u8 cycles(20) {\n    return 1\n}\nmain { }\n")
        .expect_err("function timing specifications are not supported yet");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "function timing specifications are not supported"
    );
}

#[test]
fn a_frame_too_large_for_the_bank_says_the_schedule_is_emitted_three_times() {
    // Every handler fits its scanline; there are simply too many of them for the bank.
    let diagnostics = compile_source(
        "main { ppu.mask = 0 }\n\
         frame bars using timed {\n\
         \x20   every 1 scanlines from 0 to 239 { ppu.mask = 0 }\n\
         }\n",
    )
    .expect_err("the fixed bank is finite");

    assert_eq!(
        diagnostics[0].message,
        "the program does not fit the MMC3 fixed bank"
    );
    assert!(
        diagnostics[0]
            .notes
            .iter()
            .any(|note| note.contains("costs three times its own size")),
        "the author is owed the reason their program measures three times its size: {:?}",
        diagnostics[0].notes
    );
}

#[test]
fn a_frame_wait_says_what_the_release_can_build() {
    let diagnostics = compile_source("main {\n    wait vblank\n}\n")
        .expect_err("frame waits are not in this release");

    assert_eq!(
        diagnostics[0].message,
        "only `wait cycles` is supported yet; frame waits arrive with frame scheduling"
    );
    assert_eq!(diagnostics[0].notes, [SUPPORTED_SUBSET]);
}

#[test]
fn a_string_expression_says_what_the_release_can_build() {
    let diagnostics = compile_source("main {\n    \"text\"\n}\n")
        .expect_err("string expressions are not in this release");

    assert!(
        diagnostics
            .iter()
            .any(|d| d.message == "string expressions are not supported"
                && d.notes == [SUPPORTED_SUBSET]),
        "{diagnostics:?}"
    );
}

const TIMED_REGION_COST: &str = concat!(
    "a timed region is costed as straight-line code; loops, branches\n",
    "and calls will be admitted once their cost can be measured"
);

#[test]
fn a_timed_region_says_why_it_cannot_charge_a_loop() {
    let diagnostics = compile_source(
        "var level: u8\nmain {\n    sync exact\n    cycles(20) pad {\n        level = level >> 1\n    }\n}\n",
    )
    .expect_err("a shift compiles to a loop");

    assert_eq!(
        diagnostics[0].message,
        "a shift inside a timed region compiles to a loop whose cost is not yet proven"
    );
    assert_eq!(diagnostics[0].notes, [TIMED_REGION_COST]);
}

#[test]
fn a_hardware_wait_inside_a_timed_region_carries_no_note() {
    let diagnostics = compile_source(
        "main {\n    sync exact\n    cycles(20) pad {\n        wait vblank\n    }\n}\n",
    )
    .expect_err("a vblank wait has no provable cost");

    let waited = diagnostics
        .iter()
        .find(|d| d.message == "`wait vblank` has no provable cost inside a timed region")
        .expect("the vblank wait is refused");
    assert!(
        waited.notes.is_empty(),
        "a wait has no cost to measure ever, so the note would promise nothing: {:?}",
        waited.notes
    );
}

#[test]
fn a_bank_select_warning_does_not_fail_the_build() {
    let rom = compile_source("main { mmc3.bank_select = $80 }")
        .expect("a warning does not fail the build");

    assert_eq!(rom.warnings.len(), 1);
    let warning = &rom.warnings[0];
    assert_eq!(warning.severity, raster_diag::Severity::Warning);
    assert_eq!(
        warning.message,
        "this bank select changes the MMC3 mapping mode"
    );
    assert!(warning.span.is_some());
}

#[test]
fn a_failed_build_reports_its_warnings_beside_its_errors() {
    let diagnostics = compile_source("main {\n    mmc3.bank_select = $80\n    loop {}\n}\n")
        .expect_err("`loop` is not supported yet");

    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].severity, raster_diag::Severity::Warning);
    assert_eq!(
        diagnostics[0].message,
        "this bank select changes the MMC3 mapping mode"
    );
    assert_eq!(
        diagnostics[0].notes,
        [
            "reset chose CHR A12 inversion off, so pattern table 0 is at\nPPU $0000; clearing bit 7 keeps that map",
            "bits 6 and 7 take effect from whichever bank select was\nwritten last, not from the bank data that follows",
        ]
    );
    assert_eq!(diagnostics[1].severity, raster_diag::Severity::Error);
    assert_eq!(diagnostics[1].message, "`loop` is not supported yet");
    assert_eq!(diagnostics[1].notes, [SUPPORTED_SUBSET]);
}

#[test]
fn a_build_that_fails_after_lowering_still_reports_its_warnings() {
    // Lowers cleanly and fails in codegen, so the warning has to survive a
    // stage that is not `lower`. The author who fixes the zero page is the one
    // who needed the mapping-mode warning.
    let mut source = String::from("main {\n    mmc3.bank_select = $80\n");
    for index in 0..300 {
        source.push_str(&format!("    var v{index}: u8 = 1\n"));
    }
    source.push_str("}\n");

    let diagnostics = compile_source(&source).expect_err("the zero page is exhausted");

    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].severity, raster_diag::Severity::Warning);
    assert_eq!(
        diagnostics[0].message,
        "this bank select changes the MMC3 mapping mode"
    );
    assert_eq!(diagnostics[1].severity, raster_diag::Severity::Error);
    assert_eq!(
        diagnostics[1].message,
        "too many variables for the zero page"
    );
}

#[test]
fn a_link_failure_still_reports_its_warnings() {
    // Lowers and generates, and overflows the fixed bank at link time — the
    // other post-lowering arm.
    let mut source = String::from("main {\n    mmc3.bank_select = $80\n");
    for _ in 0..3000 {
        source.push_str("    ppu.mask = 1\n");
    }
    source.push_str("}\n");

    let diagnostics = compile_source(&source).expect_err("the fixed bank overflows");

    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].severity, raster_diag::Severity::Warning);
    assert_eq!(diagnostics[1].severity, raster_diag::Severity::Error);
    assert_eq!(
        diagnostics[1].message,
        "the program does not fit the MMC3 fixed bank"
    );
}

/// The byte count the bank refusal leads with, parsed out of its first note.
fn reported_size(source: &str) -> usize {
    let diagnostics = compile_source(source).expect_err("the fixed bank is finite");
    let note = &diagnostics
        .iter()
        .find(|d| d.message == "the program does not fit the MMC3 fixed bank")
        .expect("the bank refusal")
        .notes[0];
    note.split_whitespace().next().unwrap().parse().unwrap()
}

#[test]
fn the_refusal_figure_grows_with_the_program_after_the_overflow_point() {
    // Every `if` lays down a label, and the refusal used to fire at the first one
    // past the limit - so fifty-nine of these sixty blocks went uncounted and the
    // author was told to delete far less than they had to.
    let stores = "    ppu.mask = 1\n".repeat(1700);
    let plain = format!("main {{\n{stores}}}\n");
    let branchy = format!(
        "main {{\n{stores}{}}}\n",
        "    if 1 == 1 { ppu.mask = 1 }\n".repeat(60)
    );

    let grew = reported_size(&branchy) - reported_size(&plain);
    assert!(
        grew >= 600,
        "sixty `if` blocks after the overflow point moved the reported size by {grew} bytes"
    );
}
