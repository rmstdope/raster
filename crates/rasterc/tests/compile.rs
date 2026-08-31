use rasterc::compile_source;

mod common;
use common::demo_source;

#[test]
fn compiles_the_demo_to_the_same_rom_as_m1() {
    let rom = compile_source(&demo_source()).expect("the demo compiles");

    assert_eq!(rom.image, raster_link::m1_solid_backdrop_rom());
    assert_eq!(rom.code_len, 100);
}

const SUPPORTED_SUBSET: &str = concat!(
    "this release compiles `main`, `fn`, `if`, `while`, `for`, u8\n",
    "arithmetic and `ppu.*` / `mmc3.*` register writes"
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
            "`frame` blocks are not supported yet",
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
    assert_eq!(diagnostics[0].notes.len(), 2);
    assert!(
        diagnostics[0].notes[0].ends_with(" bytes of code, and $E000-$FFFF holds 8186"),
        "unexpected note: {}",
        diagnostics[0].notes[0]
    );
    assert_eq!(
        diagnostics[0].notes[1],
        "PRG bank switching is not supported yet, so all code lives\nin the fixed bank"
    );
}

#[test]
fn the_control_flow_backstop_is_a_diagnostic_and_not_an_internal_error() {
    let diagnostics = compile_source("main {\n    cycles(30) pad {\n        return\n    }\n}\n")
        .expect_err("a timed block that jumps has no provable cost");

    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];

    // Exactly, because `codegen_diagnostic`'s fallback would produce a message beginning
    // "internal compiler error: " and this is what pins the variant onto a real arm.
    assert_eq!(
        diagnostic.message,
        "rasterc cannot prove this timed block's cost"
    );
    assert_eq!(
        diagnostic.label,
        "this block compiles to a branch or a jump ($4C)"
    );
    assert_eq!(
        diagnostic.notes,
        [
            "a timed block is costed by adding up its instructions, so one\nthat jumps has no single cost",
            "rasterc should have refused the construct that produced this\nwith a clearer message; please report this file",
        ]
    );
}
