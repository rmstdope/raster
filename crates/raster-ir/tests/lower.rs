use std::collections::BTreeSet;

use raster_diag::Refusal;
use raster_ir::{lower, PlaceKind, Register, Statement};
use raster_sema::analyze;
use raster_syntax::parse;

fn lower_source(source: &str) -> raster_ir::Program {
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    lower(&typed).expect("fixture should lower")
}

fn lower_errors(source: &str) -> Vec<raster_ir::LowerError> {
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    lower(&typed).expect_err("fixture should not lower").errors
}

/// The whole failure, rather than just its errors: a lowering that fails can
/// still have found warnings on the way, and some rules are about both at once.
fn lower_failure(source: &str) -> raster_ir::LowerFailure {
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    lower(&typed).expect_err("fixture should not lower")
}

/// Every named register, with the four facts a diagnostic needs from it: the
/// name it has in source, its address, whether a read of it is refused, and
/// whether a write of it is refused. Written out here rather than derived, so
/// the test fails when the compiler's own table changes rather than agreeing
/// with it by construction.
const REGISTERS: [(Register, &str, u16, bool, bool); 17] = [
    (Register::PpuCtrl, "ppu.ctrl", 0x2000, true, false),
    (Register::PpuMask, "ppu.mask", 0x2001, true, false),
    (Register::PpuStatus, "ppu.status", 0x2002, false, true),
    (Register::PpuOamAddr, "ppu.oam_addr", 0x2003, true, false),
    (Register::PpuOamData, "ppu.oam_data", 0x2004, false, false),
    (Register::PpuScroll, "ppu.scroll", 0x2005, true, false),
    (Register::PpuAddr, "ppu.addr", 0x2006, true, false),
    (Register::PpuData, "ppu.data", 0x2007, false, false),
    (Register::PpuOamDma, "ppu.oam_dma", 0x4014, true, false),
    (
        Register::Mmc3BankSelect,
        "mmc3.bank_select",
        0x8000,
        true,
        false,
    ),
    (
        Register::Mmc3BankData,
        "mmc3.bank_data",
        0x8001,
        true,
        false,
    ),
    (
        Register::Mmc3Mirroring,
        "mmc3.mirroring",
        0xa000,
        true,
        false,
    ),
    (
        Register::Mmc3RamProtect,
        "mmc3.ram_protect",
        0xa001,
        true,
        false,
    ),
    (
        Register::Mmc3IrqLatch,
        "mmc3.irq_latch",
        0xc000,
        true,
        false,
    ),
    (
        Register::Mmc3IrqReload,
        "mmc3.irq_reload",
        0xc001,
        true,
        false,
    ),
    (
        Register::Mmc3IrqDisable,
        "mmc3.irq_disable",
        0xe000,
        true,
        false,
    ),
    (
        Register::Mmc3IrqEnable,
        "mmc3.irq_enable",
        0xe001,
        true,
        false,
    ),
];

#[test]
fn the_register_table_names_every_register_and_says_which_read_and_write() {
    for (register, name, address, write_only, read_only) in REGISTERS {
        assert_eq!(register.name(), name);
        assert_eq!(register.address(), address);
        assert_eq!(register.is_write_only(), write_only, "{name}");
        assert_eq!(register.is_read_only(), read_only, "{name}");
        // The two are independent facts, not opposites: $2004 and $2007 are
        // false for both, because they read and write.
        assert!(!(write_only && read_only), "{name}");
    }
    // Three of the seventeen read: $2002, $2004 and $2007. One of the
    // seventeen cannot be written: $2002. If either number moves, a register
    // has changed sides and the spec table in §9.5 has to move with it.
    assert_eq!(REGISTERS.iter().filter(|row| !row.3).count(), 3);
    assert_eq!(REGISTERS.iter().filter(|row| row.4).count(), 1);
}

/// The one address the cost model matches on and codegen stores to must be one
/// fact, not two that happen to agree today.
#[test]
fn the_dma_port_the_cost_model_watches_is_the_one_the_register_names() {
    assert_eq!(
        Register::PpuOamDma.address(),
        raster_timing::OAM_DMA_PORT,
        "the register's address and the port the stall is charged for are the same fact"
    );
}

#[test]
fn lowers_scoped_control_flow_and_calls() {
    let program = lower_source(
        r#"
            const LIMIT: u8 = 3
            var output: u8
            fn increment(value: u8) -> u8 {
                if value < LIMIT { return value + 1 } else { return value }
            }
            main {
                var value: u8 = 0
                for index in 0..LIMIT { value = increment(value) }
                while value != 0 { output = value; value = value - 1 }
            }
        "#,
    );

    assert_eq!(program.functions.len(), 1);
    assert!(program.main.is_some());
    assert!(program
        .main
        .as_ref()
        .unwrap()
        .statements
        .iter()
        .any(|statement| matches!(statement, Statement::Branch { .. })));
    assert!(program
        .functions
        .iter()
        .flat_map(|function| function.statements.iter())
        .any(|statement| matches!(statement, Statement::Branch { .. })));

    let places: BTreeSet<_> = program.places.iter().map(|place| place.place).collect();
    assert_eq!(places.len(), program.places.len());
    assert!(program
        .places
        .iter()
        .any(|place| place.kind == PlaceKind::Counter));
}

/// The line each construct sits on, so an expectation names a place in the
/// fixture rather than a word in a message. `LowerError.span` is a byte offset.
fn line_of(source: &str, offset: u32) -> usize {
    // Counted over bytes rather than `source[..offset]`, which panics when an offset lands inside a
    // multi-byte character. A panic in a test helper reads as a compiler bug rather than as a moved
    // expectation, and `\n` cannot occur inside a UTF-8 sequence, so the count is the same.
    source.as_bytes()[..offset as usize]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

/// Every construct here is one the specification defines and this release does
/// not compile, so every one of them must be refused as `NotInThisRelease` —
/// not because of how its message is worded, which is free to change, but
/// because of what kind of refusal it is.
#[test]
fn rejects_all_accepted_forms_not_supported_by_initial_codegen() {
    let source = r#"
            group state { var line: u8 }
            var table: [2]u8
            var word: u16
            var flag: bool = true
            asm fn raw() {}
            frame display { at vblank {} }
            main {
                table[0] = 1
                word = 2
                flag = false
                "text"
                'x'
                wait vblank
                loop {
                    break
                    continue
                }
            }
        "#;
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    let errors = lower(&typed)
        .expect_err("unsupported forms must be diagnosed")
        .errors;

    // group, array, u16, bool, `asm`, `at vblank`, the array assignment, the
    // string, the character, `wait vblank`, `loop`, `break` and `continue` —
    // one construct per line of the fixture above, which is why `break` and
    // `continue` sit on lines of their own.
    for line in [2, 3, 4, 5, 6, 7, 9, 12, 13, 14, 15, 16, 17] {
        assert!(
            errors
                .iter()
                .any(|error| line_of(source, error.span.start) == line
                    && error.refusal == Refusal::NotInThisRelease),
            "line {line} holds a construct this release does not have: {errors:?}"
        );
    }
    assert!(errors
        .iter()
        .all(|error| error.span.end >= error.span.start));
}

#[test]
fn rejects_for_steps_that_wrap_before_the_range_ends() {
    let errors = lower_errors("main { for index in 250..255 step 10 {} }");

    assert!(errors
        .iter()
        .any(|error| error.message.contains("overflow")));
}

#[test]
fn rejects_direct_and_mutually_recursive_calls() {
    for source in [
        "fn recurse() { recurse() } main {}",
        "fn first() { second() } fn second() { first() } main {}",
    ] {
        let errors = lower_errors(source);
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("recursive")),
            "expected recursion error in {errors:?}"
        );
    }
}

#[test]
fn lowers_forward_non_recursive_calls() {
    let program = lower_source(
        r#"
            fn first() { second() }
            fn second() {}
            main { first() }
        "#,
    );

    assert_eq!(program.functions.len(), 2);
}

#[test]
fn rejects_u8_functions_that_can_fall_through() {
    let errors = lower_errors("fn value() -> u8 {} main {}");
    assert!(errors
        .iter()
        .any(|error| error.message.contains("fall through")));

    lower_source(
        r#"
            fn first_value() -> u8 {
                for index in 0..1 { return 1 }
            }
            main {}
        "#,
    );

    lower_source(
        r#"
            fn scoped_loop_value() -> u8 {
                {
                    const LIMIT: u8 = 1
                    for index in 0..LIMIT { return 1 }
                }
            }
            main {}
        "#,
    );

    let errors = lower_errors(
        r#"
            fn value() -> u8 {
                for index in 0..1 {}
            }
            main {}
        "#,
    );
    assert!(errors
        .iter()
        .any(|error| error.message.contains("fall through")));

    let errors = lower_errors(
        r#"
            fn value(limit: u8) -> u8 {
                while limit != 0 { return 1 }
            }
            main {}
        "#,
    );
    assert!(errors
        .iter()
        .any(|error| error.message.contains("fall through")));

    lower_source(
        r#"
            fn choose(value: u8) -> u8 {
                if value == 0 { return 1 } else { return 2 }
            }
            main {}
        "#,
    );
}

#[test]
fn lowers_a_timed_frame_into_sorted_visible_scanline_events() {
    let program = lower_source(
        r#"
            main { ppu.mask = 0 }
            frame bars using timed {
                at scanline 120 { ppu.addr = $02 }
                at scanline 60 { ppu.addr = $01 }
                every 8 scanlines from 96 to 112 { ppu.addr = $03 }
            }
        "#,
    );

    let frame = program.frame.as_ref().expect("the frame lowers");
    assert_eq!(frame.name, "bars");
    assert_eq!(
        frame
            .events
            .iter()
            .map(|event| event.scanline)
            .collect::<Vec<_>>(),
        vec![60, 96, 104, 112, 120],
    );
    assert!(frame
        .events
        .iter()
        .all(|event| event.body.iter().any(|statement| matches!(
            statement,
            Statement::Assign {
                destination: raster_ir::Destination::Register(raster_ir::Register::PpuAddr),
                ..
            }
        ))));
}

#[test]
fn rejects_frame_events_outside_the_visible_scanline_range() {
    let errors = lower_errors(
        r#"
            main { ppu.mask = 0 }
            frame bars using timed {
                at scanline 240 { ppu.addr = $01 }
                every 8 scanlines from 232 to 248 { ppu.addr = $02 }
            }
        "#,
    );

    assert_eq!(errors.len(), 2);
    assert!(errors
        .iter()
        .all(|error| error.message.contains("visible scanline")));
}

#[test]
fn rejects_frame_forms_the_timed_lowering_does_not_cover() {
    for (source, expected) in [
        (
            // `timed` and `irq` are the two strategies this release lowers; the rest of spec
            // section 7.1's table is still refused by name.
            r#"
                main { ppu.mask = 0 }
                frame bars using sprite0 { at scanline 60 { ppu.addr = $01 } }
            "#,
            "`using sprite0` is not supported yet",
        ),
        (
            r#"
                main { ppu.mask = 0 }
                frame bars using timed { at vblank { ppu.addr = $01 } }
            "#,
            "`at vblank` is not supported yet",
        ),
        (
            r#"
                main { ppu.mask = 0 }
                frame bars using timed { at scanline 60 { ppu.addr = $01 } }
                frame more using timed { at scanline 90 { ppu.addr = $02 } }
            "#,
            "only one `frame` is supported yet",
        ),
        (
            r#"
                main { ppu.mask = 0 }
                frame bars using timed {
                    at scanline 60 { ppu.addr = $01 }
                    at scanline 60 { ppu.addr = $02 }
                }
            "#,
            "two frame events share scanline 60",
        ),
    ] {
        let errors = lower_errors(source);
        assert!(
            errors.iter().any(|error| error.message == expected),
            "expected `{expected}`, found {errors:?}"
        );
    }
}

#[test]
fn a_frame_interval_wider_than_the_picture_is_one_occurrence_rather_than_a_wrap() {
    let program = lower_source(
        r#"
            main { ppu.mask = 0 }
            frame bars using timed {
                every 4294967295 scanlines from 238 to 239 { ppu.mask = 0 }
            }
        "#,
    );
    let frame = program.frame.as_ref().expect("the frame lowers");

    // `to` is bounded by the visible picture and the interval by nothing, so walking the range
    // with an unchecked add panics a debug build and, worse, wraps a release one into a schedule
    // the author never wrote.
    assert_eq!(
        frame
            .events
            .iter()
            .map(|event| event.scanline)
            .collect::<Vec<_>>(),
        vec![238],
    );
}

#[test]
fn a_construct_this_release_does_not_have_is_refused_as_such() {
    let errors = lower_errors("var table: [2]u8\nmain {}\n");

    assert!(!errors.is_empty(), "the array must be refused at all");
    assert!(
        errors
            .iter()
            .all(|error| error.refusal == Refusal::NotInThisRelease),
        "an array is a construct the release does not have: {errors:?}"
    );
}

#[test]
fn a_bank_select_that_inverts_the_chr_map_is_a_warning() {
    let program = lower_source("main { mmc3.bank_select = $80 }");

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this bank select changes the MMC3 mapping mode"
    );
    assert_eq!(
        warning.label,
        "bit 7 swaps the two pattern tables from here on"
    );
    assert_eq!(
        warning.notes,
        [
            "reset chose CHR A12 inversion off, so pattern table 0 is at\nPPU $0000; clearing bit 7 keeps that map",
            "bits 6 and 7 take effect from whichever bank select was\nwritten last, not from the bank data that follows",
        ]
    );
}

#[test]
fn a_bank_select_that_changes_prg_mode_is_a_warning() {
    let program = lower_source("main { mmc3.bank_select = $46 }");

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this bank select changes the MMC3 mapping mode"
    );
    assert_eq!(
        warning.label,
        "bit 6 moves the fixed PRG bank from $C000 to $8000"
    );
    assert_eq!(
        warning.notes,
        [
            "reset chose PRG mode 0, a linear 32 KiB map with this code\nin the fixed bank at $E000; clearing bit 6 keeps that map",
            "bits 6 and 7 take effect from whichever bank select was\nwritten last, not from the bank data that follows",
        ]
    );
}

#[test]
fn a_bank_select_that_sets_both_mode_bits_is_one_warning() {
    let program = lower_source("main { mmc3.bank_select = $C0 }");

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this bank select changes the MMC3 mapping mode"
    );
    assert_eq!(
        warning.label,
        "bit 6 moves the fixed PRG bank and bit 7 swaps the pattern tables"
    );
    assert_eq!(
        warning.notes,
        [
            "reset chose PRG mode 0 and CHR A12 inversion off: a linear\n32 KiB map, and pattern table 0 at PPU $0000",
            "bits 6 and 7 take effect from whichever bank select was\nwritten last, not from the bank data that follows",
        ]
    );
}

#[test]
fn a_bank_select_rasterc_cannot_fold_is_a_warning() {
    for source in [
        "main { var which: u8 = 3\n mmc3.bank_select = which }",
        "main { mmc3.bank_select = $40 | $06 }",
        "fn pick() -> u8 { return 3 }\nmain { mmc3.bank_select = pick() }",
    ] {
        let program = lower_source(source);

        assert_eq!(program.warnings.len(), 1, "for {source:?}");
        let warning = &program.warnings[0];
        assert_eq!(
            warning.message,
            "rasterc cannot tell whether this bank select changes the mapping mode",
            "for {source:?}"
        );
        assert_eq!(
            warning.label, "rasterc cannot see this value here, so bits 6 and 7 are unknown",
            "for {source:?}"
        );
        assert_eq!(
            warning.notes,
            ["bit 6 is PRG mode and bit 7 is CHR A12 inversion; keeping\nboth clear keeps the map reset chose"],
            "for {source:?}"
        );
    }
}

#[test]
fn repointing_a_bank_window_is_silent() {
    let program = lower_source("main { mmc3.bank_select = 0\n mmc3.bank_data = 3 }");

    assert!(program.warnings.is_empty());
}

#[test]
fn a_const_bank_select_is_folded_like_a_literal() {
    let program = lower_source("const INVERT: u8 = $80\nmain { mmc3.bank_select = INVERT }");

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "bit 7 swaps the two pattern tables from here on"
    );
}

#[test]
fn warnings_survive_a_failed_lowering() {
    let source = "main { mmc3.bank_select = $80\n loop {} }";
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");

    let failure = lower(&typed).expect_err("`loop` is not supported yet");

    assert_eq!(failure.errors.len(), 1);
    assert_eq!(failure.errors[0].message, "`loop` is not supported yet");
    assert_eq!(failure.warnings.len(), 1);
    assert_eq!(
        failure.warnings[0].label,
        "bit 7 swaps the two pattern tables from here on"
    );
}

#[test]
fn a_bank_select_rasterc_cannot_fold_still_warns() {
    // `binary` builds a `Value::Binary` unconditionally, so a value rasterc
    // could fold by eye is not a constant to it. This used to be shown with
    // `mmc3.bank_select += $80`, which is now refused outright for reading
    // $8000; a plain assignment of a binary expression makes the same point.
    let program = lower_source("main { mmc3.bank_select = $40 | $06 }");

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "rasterc cannot see this value here, so bits 6 and 7 are unknown"
    );
}

#[test]
fn a_bank_data_write_after_selecting_r6_warns() {
    let program = lower_source("main { mmc3.bank_select = 6\n mmc3.bank_data = 2 }");

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this write repoints the PRG window at $8000"
    );
    assert_eq!(
        warning.label,
        "R6 is selected, so this replaces $8000-$9FFF"
    );
    assert_eq!(
        warning.notes,
        [
            "reset chose a linear 32 KiB map with R6 = 0 and R7 = 1; the\nbytes at $8000 are not the ones it mapped from here on",
            "PRG bank switching is not supported yet: banks 0 to 2 hold $FF,\nand bank 3 is a second view of the fixed bank at $E000",
        ]
    );
}

#[test]
fn a_bank_data_write_after_selecting_r7_warns() {
    let program = lower_source("main { mmc3.bank_select = 7\n mmc3.bank_data = 2 }");

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this write repoints the PRG window at $A000"
    );
    assert_eq!(
        warning.label,
        "R7 is selected, so this replaces $A000-$BFFF"
    );
    assert_eq!(
        warning.notes,
        [
            "reset chose a linear 32 KiB map with R6 = 0 and R7 = 1; the\nbytes at $A000 are not the ones it mapped from here on",
            "PRG bank switching is not supported yet: banks 0 to 2 hold $FF,\nand bank 3 is a second view of the fixed bank at $E000",
        ]
    );
}

#[test]
fn a_bank_data_write_to_a_chr_register_is_silent() {
    for register in 0..=5u8 {
        let source = format!("main {{ mmc3.bank_select = {register}\n mmc3.bank_data = 3 }}");
        let program = lower_source(&source);

        assert!(program.warnings.is_empty(), "for {source:?}");
    }
}

#[test]
fn a_bare_bank_data_write_lands_on_the_register_reset_selected() {
    let program = lower_source("main { mmc3.bank_data = 0 }");

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this write repoints the PRG window at $A000"
    );
    assert_eq!(
        warning.label,
        "nothing selects a register before this, and reset selected R7 last"
    );
    assert_eq!(
        warning.notes,
        [
            "reset chose a linear 32 KiB map with R6 = 0 and R7 = 1; the\nbytes at $A000 are not the ones it mapped from here on",
            "write `mmc3.bank_select` with 0 to 5 first to point this at a CHR window",
        ]
    );
}

#[test]
fn a_bank_select_with_mode_bits_still_names_its_register() {
    let program = lower_source("main { mmc3.bank_select = $46\n mmc3.bank_data = 2 }");

    assert_eq!(program.warnings.len(), 2);
    assert_eq!(
        program.warnings[0].message,
        "this bank select changes the MMC3 mapping mode"
    );
    assert_eq!(
        program.warnings[1].label,
        "R6 is selected, so this replaces $8000-$9FFF"
    );
}

#[test]
fn a_bank_data_write_after_an_unfoldable_select_warns_softly() {
    let program =
        lower_source("main { var pick: u8 = 6\n mmc3.bank_select = pick\n mmc3.bank_data = 2 }");

    assert_eq!(program.warnings.len(), 2);
    let warning = &program.warnings[1];
    assert_eq!(
        warning.message,
        "rasterc cannot tell which bank register this write lands on"
    );
    assert_eq!(
        warning.label,
        "the last bank select before this is not one rasterc can see"
    );
    assert_eq!(
        warning.notes,
        [
            "R6 and R7 are the two 8 KiB PRG windows; a write landing on\neither one repoints $8000 or $A000",
            "selecting with a literal 0 to 5 immediately before the write\nkeeps the map reset chose",
        ]
    );
}

#[test]
fn a_bank_select_rasterc_cannot_fold_leaves_the_selection_unknown() {
    // `binary` builds a `Value::Binary` unconditionally, so a selection rasterc
    // could fold by eye is not a constant to it. This used to be shown with
    // `mmc3.bank_select += 1`, which `raster-1t9` now refuses outright for
    // reading $8000; a plain assignment of a binary expression is the same
    // unfoldable selection and makes the same point.
    let program = lower_source("main { mmc3.bank_select = 6 | 1\n mmc3.bank_data = 2 }");

    assert_eq!(program.warnings.len(), 2);
    assert_eq!(
        program.warnings[1].label,
        "the last bank select before this is not one rasterc can see"
    );
}

#[test]
fn branches_that_select_different_registers_leave_the_selection_unknown() {
    let program = lower_source(
        "main { var c: u8 = 1\n if c != 0 { mmc3.bank_select = 6 } else { mmc3.bank_select = 0 }\n mmc3.bank_data = 2 }",
    );

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "the last bank select before this is not one rasterc can see"
    );
}

#[test]
fn an_if_with_no_else_that_selects_leaves_the_selection_unknown() {
    let program = lower_source(
        "main { var c: u8 = 1\n mmc3.bank_select = 0\n if c != 0 { mmc3.bank_select = 6 }\n mmc3.bank_data = 2 }",
    );

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "the last bank select before this is not one rasterc can see"
    );
}

#[test]
fn branches_that_select_the_same_register_keep_it() {
    let program = lower_source(
        "main { var c: u8 = 1\n if c != 0 { mmc3.bank_select = 0 } else { mmc3.bank_select = 0 }\n mmc3.bank_data = 2 }",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn a_loop_body_that_selects_judges_its_own_bank_data_write_as_unknown() {
    let program = lower_source(
        "main { var n: u8 = 3\n mmc3.bank_select = 0\n while n != 0 { mmc3.bank_data = n\n mmc3.bank_select = 6 } }",
    );

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "the last bank select before this is not one rasterc can see"
    );
}

#[test]
fn a_loop_body_that_cannot_select_keeps_the_selection_from_outside_it() {
    let program = lower_source(
        "main { var n: u8 = 3\n mmc3.bank_select = 0\n while n != 0 { mmc3.bank_data = n } }",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn a_for_body_that_selects_carries_its_selection_past_the_loop() {
    let program =
        lower_source("main { for i in 0..3 { mmc3.bank_select = 6 }\n mmc3.bank_data = 2 }");

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "R6 is selected, so this replaces $8000-$9FFF"
    );
}

#[test]
fn a_bank_data_warning_survives_a_failed_lowering() {
    let source = "main { mmc3.bank_select = 6\n mmc3.bank_data = 2\n loop {} }";
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");

    let failure = lower(&typed).expect_err("`loop` is not supported yet");

    assert_eq!(failure.errors.len(), 1);
    assert_eq!(failure.errors[0].message, "`loop` is not supported yet");
    assert_eq!(failure.warnings.len(), 1);
    assert_eq!(
        failure.warnings[0].label,
        "R6 is selected, so this replaces $8000-$9FFF"
    );
}

#[test]
fn a_bank_data_write_in_a_function_says_any_register_may_be_selected() {
    let program = lower_source(
        "fn set_chr(tile: u8) { mmc3.bank_data = tile }\nmain { mmc3.bank_select = 0\n set_chr(3) }",
    );

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "this function can be called with any register selected"
    );
    assert_eq!(
        program.warnings[0].notes[1],
        "selecting with a literal 0 to 5 in this function, before the\nwrite, keeps the map reset chose"
    );
}

#[test]
fn a_call_that_cannot_select_leaves_the_selection_alone() {
    let program = lower_source(
        "var counter: u8\nfn bump() { counter = counter + 1 }\nmain { mmc3.bank_select = 0\n bump()\n mmc3.bank_data = 3 }",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn a_call_that_can_select_makes_the_selection_unknown() {
    let program = lower_source(
        "fn tick() { mmc3.bank_select = 6 }\nmain { mmc3.bank_select = 0\n tick()\n mmc3.bank_data = 3 }",
    );

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "the last bank select before this is not one rasterc can see"
    );
}

#[test]
fn a_call_that_can_only_select_through_another_call_makes_the_selection_unknown() {
    // Declared in both orders, because the fixed point is order-independent by
    // construction and a reader should not have to take that on trust.
    for source in [
        "fn tick() { mmc3.bank_select = 6 }\nfn outer() { tick() }\nmain { mmc3.bank_select = 0\n outer()\n mmc3.bank_data = 3 }",
        "fn outer() { tick() }\nfn tick() { mmc3.bank_select = 6 }\nmain { mmc3.bank_select = 0\n outer()\n mmc3.bank_data = 3 }",
    ] {
        let program = lower_source(source);

        assert_eq!(program.warnings.len(), 1, "for {source:?}");
        assert_eq!(
            program.warnings[0].label,
            "the last bank select before this is not one rasterc can see",
            "for {source:?}"
        );
    }
}

#[test]
fn a_loop_body_that_only_selects_through_a_call_judges_itself_as_unknown() {
    let program = lower_source(
        "fn tick() { mmc3.bank_select = 6 }\nmain { var n: u8 = 3\n mmc3.bank_select = 0\n while n != 0 { mmc3.bank_data = n\n tick() } }",
    );

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "the last bank select before this is not one rasterc can see"
    );
}

#[test]
fn a_bank_data_write_in_a_frame_handler_says_a_handler_runs_with_any_register_selected() {
    let program = lower_source(
        "main { ppu.mask = 0 }\nframe bars using timed { every 8 scanlines from 0 to 239 { mmc3.bank_data = 1 } }",
    );

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "a frame handler runs with any register selected"
    );
    assert_eq!(
        program.warnings[0].notes[1],
        "selecting with a literal 0 to 5 in the handler, before the\nwrite, keeps the map reset chose"
    );
}

#[test]
fn a_loop_body_that_selects_judges_a_bank_data_write_after_it_as_unknown() {
    // `loop` is refused, so this goes through `parse`/`analyze`/`lower` by
    // hand: the warning still has to reach the author, and the `Loop` arm's
    // pre-scan is what makes it the soft one rather than `Known(0)`.
    let source =
        "main { mmc3.bank_select = 0\n loop { mmc3.bank_select = 6 }\n mmc3.bank_data = 2 }";
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");

    let failure = lower(&typed).expect_err("`loop` is not supported yet");

    assert_eq!(failure.warnings.len(), 1);
    assert_eq!(
        failure.warnings[0].label,
        "the last bank select before this is not one rasterc can see"
    );
}

#[test]
fn reading_a_write_only_register_is_refused() {
    let errors = lower_errors("main {\n    var m: u8 = ppu.mask\n}\n");

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "`ppu.mask` cannot be read");
    assert_eq!(
        errors[0].label.as_deref(),
        Some("$2001 is a write-only port")
    );
    assert_eq!(
        errors[0].notes,
        [
            "reading $2001 returns whatever was last on the PPU's data bus,\nnot the last value written",
            "keep what you wrote in a variable of your own\nand write the whole value",
        ]
    );
    assert_eq!(errors[0].refusal, Refusal::Rejected);
    // `var m: u8 = ppu.mask` — line 2 begins at byte 7, and `ppu.mask` is
    // sixteen characters further along.
    assert_eq!(errors[0].span.start, 23);
    assert_eq!(errors[0].span.end, 31);
}

#[test]
fn a_read_of_the_dma_port_does_not_claim_it_is_a_ppu_port() {
    let errors = lower_errors("main {\n    var v: u8 = ppu.oam_dma\n}\n");

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "`ppu.oam_dma` cannot be read");
    assert_eq!(
        errors[0].notes[0],
        "$4014 does not read back at all: it is a write-only trigger on\nthe CPU bus, and a read of it returns whatever was last on\nthat bus"
    );
    // $4014 is neither a PPU port nor a mapper port, so neither of the other
    // two sentences may be told about it.
    assert!(
        !errors[0].notes[0].contains("PPU's data bus"),
        "the PPU sentence is false about $4014: {:?}",
        errors[0].notes[0]
    );
    assert!(!errors[0].notes[0].contains("PRG"));
}

#[test]
fn a_compound_assignment_to_a_write_only_register_is_refused() {
    let source = "main {\n    mmc3.bank_select += $80\n}\n";
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");

    let failure = lower(&typed).expect_err("$8000 does not read back");

    assert_eq!(failure.errors.len(), 1);
    assert_eq!(
        failure.errors[0].message,
        "`mmc3.bank_select` cannot be read"
    );
    assert_eq!(
        failure.errors[0].label.as_deref(),
        Some("$8000 is a write-only port")
    );
    assert_eq!(
        failure.errors[0].notes,
        [
            "`+=` reads its destination before it writes, so this reads $8000",
            "reading $8000 returns a byte of your own program from the PRG\nbank mapped there, not the last value written",
            "keep what you wrote in a variable of your own\nand write the whole value",
        ]
    );
    // The carets cover the destination only: `mmc3.bank_select` is sixteen
    // characters, starting eleven into line 2, which itself starts at byte 7.
    assert_eq!(failure.errors[0].span.start, 11);
    assert_eq!(failure.errors[0].span.end, 27);
    // The mapping-mode warning does not also fire: the statement is being
    // rewritten, so what its value would have been is moot, and one fault gets
    // one message.
    assert!(failure.warnings.is_empty());
}

#[test]
fn every_compound_operator_names_itself_in_the_refusal() {
    for (source, spelling) in [
        ("main { ppu.mask += 1 }", "+="),
        ("main { ppu.mask -= 1 }", "-="),
        ("main { ppu.mask *= 1 }", "*="),
        ("main { ppu.mask /= 1 }", "/="),
    ] {
        let errors = lower_errors(source);
        assert_eq!(errors.len(), 1, "{source}");
        // All three notes, not just the operator's: this is the only place a
        // compound assignment meets the PPU-port form of `dead_read_note`.
        assert_eq!(
            errors[0].notes,
            [
                format!(
                    "`{spelling}` reads its destination before it writes, so this reads $2001"
                ),
                "reading $2001 returns whatever was last on the PPU's data bus,\nnot the last value written"
                    .to_owned(),
                "keep what you wrote in a variable of your own\nand write the whole value".to_owned(),
            ],
            "{source}"
        );
    }
}

#[test]
fn every_write_only_register_refuses_a_read_and_every_readable_one_does_not() {
    for (_register, name, _address, write_only, _read_only) in REGISTERS {
        let source = format!("main {{ var v: u8 = {name} }}");
        let syntax = parse(&source).expect("fixture should parse");
        let typed = analyze(&syntax).expect("fixture should analyze");
        let result = lower(&typed);
        // The table's own verdict column, not `is_write_only()`: this test is
        // about what lowering does, and the function it would otherwise ask is
        // the one `the_register_table_names_every_register_and_says_which_read`
        // is there to pin.
        if write_only {
            let failure = result.expect_err(name);
            assert_eq!(failure.errors.len(), 1, "{name}");
            assert_eq!(
                failure.errors[0].message,
                format!("`{name}` cannot be read")
            );
        } else {
            assert!(result.is_ok(), "{name} reads");
        }
    }
}

#[test]
fn two_write_only_reads_on_one_line_are_two_errors_in_source_order() {
    let errors = lower_errors("main {\n    ppu.ctrl = ppu.mask | ppu.scroll\n}\n");

    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].message, "`ppu.mask` cannot be read");
    assert_eq!(errors[1].message, "`ppu.scroll` cannot be read");
    // `lower_value`'s `Infix` arm lowers left before right, so the errors come
    // out the way the author reads the line.
    assert!(errors[0].span.start < errors[1].span.start);
}

#[test]
fn writing_the_read_only_register_is_refused() {
    const SOURCE: &str = "main {\n    ppu.mask = $1E\n    ppu.status = 0\n}\n";

    let errors = lower_errors(SOURCE);

    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.message, "`ppu.status` cannot be written");
    assert_eq!(error.label.as_deref(), Some("$2002 is a read-only port"));
    assert_eq!(
        error.notes,
        [
            "writing $2002 changes nothing on the PPU: it is a status\nport, and the CPU can only read it",
            "there is no value that makes this store do something;\ndelete the line",
        ]
    );
    assert_eq!(error.refusal, Refusal::Rejected);
    // The carets cover the destination and nothing else. Existing tests in this
    // file assert the two offsets as numbers; slicing the source says what the
    // numbers mean.
    assert_eq!(
        &SOURCE[error.span.start as usize..error.span.end as usize],
        "ppu.status"
    );
}

#[test]
fn a_compound_assignment_to_the_read_only_register_is_refused() {
    let errors = lower_errors("main {\n    ppu.status += 1\n}\n");

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "`ppu.status` cannot be written");
    assert_eq!(
        errors[0].label.as_deref(),
        Some("$2002 is a read-only port")
    );
    // Three notes, the operator first. The read of $2002 is legal and is not
    // mentioned: raster-bm1 owns that question, not this bead.
    assert_eq!(errors[0].notes.len(), 3);
    assert_eq!(
        errors[0].notes[0],
        "`+=` writes its destination, so this writes $2002"
    );
}

#[test]
fn every_compound_operator_names_itself_in_the_write_refusal() {
    for spelling in ["+=", "-=", "*=", "/="] {
        let source = format!("main {{\n    ppu.status {spelling} 1\n}}\n");
        let errors = lower_errors(&source);

        assert_eq!(errors.len(), 1, "{spelling}");
        assert_eq!(
            errors[0].notes[0],
            format!("`{spelling}` writes its destination, so this writes $2002"),
        );
    }
}

#[test]
fn a_bad_write_and_a_bad_read_on_one_line_are_two_errors_in_source_order() {
    let errors = lower_errors("main {\n    ppu.status = ppu.mask\n}\n");

    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].message, "`ppu.status` cannot be written");
    assert_eq!(errors[1].message, "`ppu.mask` cannot be read");
    // A refused write still lowers its right-hand side, so the author sees both
    // mistakes in one build. Nothing sorts the errors; they come out in the
    // order they were pushed, which is the order the line is read.
    assert!(errors[0].span.start < errors[1].span.start);
}

#[test]
fn every_register_but_the_read_only_one_accepts_a_write() {
    for (register, name, _address, _write_only, read_only) in REGISTERS {
        // `= 0` raises no warning for fifteen of the sixteen; `mmc3.bank_data
        // = 0` warns (raster-rid), which this test does not assert on, because
        // `result.is_ok()` is unaffected by warnings. `mmc3.bank_select = 0` is
        // the one worth naming: it is silent because bits 6 and 7 are clear,
        // and a different constant would fold to a warning, so the test would
        // pass for a reason it does not state.
        let source = format!("main {{ {name} = 0 }}");
        let syntax = parse(&source).expect("fixture should parse");
        let typed = analyze(&syntax).expect("fixture should analyze");
        let result = lower(&typed);
        if read_only {
            let failure = result.expect_err(name);
            assert_eq!(failure.errors.len(), 1, "{name}");
            assert_eq!(
                failure.errors[0].message,
                format!("`{name}` cannot be written")
            );
        } else {
            assert!(result.is_ok(), "{name} is written");
        }
        assert_eq!(register.is_read_only(), read_only, "{name}");
    }
}

#[test]
fn reading_the_read_only_register_is_still_fine() {
    // The mirror of `writing_a_write_only_register_is_still_fine`. $2002 reads
    // perfectly well and this bead does not change that; its read side effects
    // are raster-bm1. If this goes red, the check has been attached to
    // `lower_value`'s `Member` arm instead of to the destination.
    let program = lower_source(
        "main {\n    var s: u8 = ppu.status\n    ppu.oam_data = $20\n    ppu.data = $0F\n}\n",
    );
    assert!(program.main.is_some());
}

#[test]
fn writing_a_write_only_register_is_still_fine() {
    // The whole point of the rule is that it is about reads. Every one of the
    // thirteen may still be written, and this is the test that goes red if the
    // check is ever attached to `lower_destination` instead.
    let program = lower_source(
        "main {\n    ppu.mask = $1E\n    mmc3.bank_select = $06\n    ppu.addr = $00\n}\n",
    );
    assert!(program.main.is_some());
}

#[test]
fn a_bank_select_whose_value_was_refused_does_not_also_warn() {
    // Q7 again, on the route the plan's site list did not name. `+=` is not the
    // only way a bank select can carry a refused read: a plain assignment of an
    // expression that reads a write-only register refuses too, and the same
    // reasoning applies word for word — the statement is being rewritten, so
    // what its value would have been is moot, and the warning is unactionable
    // until the read is fixed.
    let source = "main {\n    mmc3.bank_select = ppu.mask | $80\n}\n";
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");

    let failure = lower(&typed).expect_err("$2001 does not read back");

    assert_eq!(failure.errors.len(), 1);
    assert_eq!(failure.errors[0].message, "`ppu.mask` cannot be read");
    assert!(failure.warnings.is_empty());
}

#[test]
fn a_compound_assignment_to_anything_that_reads_is_untouched() {
    // The other half of the rule: `+=` on a place, and on a register that does
    // read, still lowers. Nothing else in the suite lowers a compound
    // assignment successfully any more, so this is what goes red if
    // `destination_read` ever refuses a `Destination::Place`, or if the
    // refusal path fires one branch too wide.
    let program = lower_source("main {\n    var n: u8 = 1\n    n += 2\n    ppu.mask = n\n}\n");
    assert!(program.main.is_some());

    // `ppu.oam_data`, not `ppu.data`: raster-hqh refuses a compound assignment
    // to $2007 because the read it performs is buffered, so it is no longer an
    // example of a register that reads.
    let readable = lower_source("main {\n    ppu.oam_data += 1\n}\n");
    assert!(readable.main.is_some());
}

#[test]
fn a_bank_select_whose_value_was_refused_leaves_the_selection_unknown() {
    // The refused read lowers to a `Value::Constant(0)` placeholder, which is
    // not a byte the author wrote and must not be read as one: taken at face
    // value it would say R0 is selected and the `mmc3.bank_data` write below
    // would warn about nothing. Skipping the update instead would leave the
    // selection at whatever preceded it, which is just as untrue. rasterc
    // genuinely cannot see this select, so the selection is unknown.
    let source = "main {\n    mmc3.bank_select = ppu.mask\n    mmc3.bank_data = 2\n}\n";
    let syntax = parse(source).expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");

    let failure = lower(&typed).expect_err("$2001 does not read back");

    assert_eq!(failure.errors.len(), 1);
    assert_eq!(failure.errors[0].message, "`ppu.mask` cannot be read");
    // The bank-select warning is suppressed by Q7 — it is about the value, and
    // the value is a placeholder. The bank-data warning is not: it is about
    // which register is selected, which is a fact about the statements before
    // it and stays true and actionable once the read is fixed.
    assert_eq!(failure.warnings.len(), 1);
    assert_eq!(
        failure.warnings[0].label,
        "the last bank select before this is not one rasterc can see"
    );
}

#[test]
fn a_lone_ppu_data_read_is_a_warning() {
    let source = "main {\n    var tile: u8 = ppu.data\n}\n";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this `ppu.data` read gives you the byte before the one you asked for"
    );
    assert_eq!(
        warning.label,
        "nothing next to this read primes the PPU's read buffer"
    );
    assert_eq!(
        warning.notes,
        [
            "$2007 hands back what the previous read fetched, and loads the\nbyte at this address for the next read",
            "read `ppu.data` twice in a row, discard the first and keep the\nsecond; a palette address, $3F00 to $3FFF, is not buffered and\nreads back at once",
        ]
    );
    // The span covers `ppu.data` and nothing else.
    assert_eq!(line_of(source, warning.span.start), 2);
}

#[test]
fn a_primed_ppu_data_read_is_silent() {
    let program =
        lower_source("main {\n    var discard: u8 = ppu.data\n    var tile: u8 = ppu.data\n}\n");

    assert!(program.warnings.is_empty());
}

#[test]
fn three_ppu_data_reads_in_a_row_are_all_silent() {
    let program = lower_source(
        "main {\n    var a: u8 = ppu.data\n    var b: u8 = ppu.data\n    var c: u8 = ppu.data\n}\n",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn two_ppu_data_reads_in_one_statement_are_each_other_s_neighbour() {
    let program = lower_source("main {\n    var sum: u8 = ppu.data + ppu.data\n}\n");

    assert!(program.warnings.is_empty());
}

#[test]
fn a_statement_between_two_ppu_data_reads_breaks_the_pair() {
    let program = lower_source(
        "main {\n    var a: u8 = ppu.data\n    ppu.addr = $20\n    var b: u8 = ppu.data\n}\n",
    );

    assert_eq!(program.warnings.len(), 2);
}

#[test]
fn a_neighbour_never_crosses_a_block_boundary() {
    let program = lower_source(
        "main {\n    var flag: u8 = 1\n    if flag != 0 {\n        var a: u8 = ppu.data\n    }\n    var b: u8 = ppu.data\n}\n",
    );

    assert_eq!(program.warnings.len(), 2);
}

#[test]
fn a_compound_assignment_to_ppu_data_is_refused() {
    let errors = lower_errors("main {\n    ppu.data += 1\n}\n");

    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "`ppu.data` cannot be the destination of a compound assignment"
    );
    assert_eq!(
        errors[0].label.as_deref(),
        Some("`+=` reads $2007 before it writes, and that read is buffered")
    );
    assert_eq!(
        errors[0].notes,
        [
            "the byte it would add to is the one at the previous address,\nnot the one at the address you are writing",
            "read the byte you want into a variable of your own, add to\nthat, and write the whole value",
        ]
    );
    assert_eq!(errors[0].refusal, Refusal::Rejected);
}

#[test]
fn every_compound_operator_names_itself_in_the_ppu_data_refusal() {
    for spelling in ["+=", "-=", "*=", "/="] {
        let errors = lower_errors(&format!("main {{\n    ppu.data {spelling} 1\n}}\n"));

        assert_eq!(errors.len(), 1, "{spelling}");
        assert_eq!(
            errors[0].label.as_deref(),
            Some(
                format!("`{spelling}` reads $2007 before it writes, and that read is buffered")
                    .as_str()
            ),
            "{spelling}"
        );
    }
}

#[test]
fn a_refused_compound_assignment_is_not_a_neighbour() {
    // `ppu.data += 1` writes $2007; the read it makes of its own destination is
    // refused. Neither is a read that primes the buffer for the line below, so
    // that line still warns.
    let failure = lower_failure("main {\n    ppu.data += 1\n    var a: u8 = ppu.data\n}\n");

    assert_eq!(failure.errors.len(), 1);
    assert_eq!(failure.warnings.len(), 1);
}

#[test]
fn a_ppu_data_read_in_a_global_initializer_has_no_neighbour() {
    // A global initializer runs at reset, before any `ppu.addr` write, and has
    // no neighbouring statement to prime it — so both of these warn even though
    // they are written one under the other.
    let program = lower_source("var a: u8 = ppu.data\nvar b: u8 = ppu.data\nmain { }\n");

    assert_eq!(program.warnings.len(), 2);
}

#[test]
fn a_ppu_data_write_is_not_a_priming_neighbour() {
    // The trap the plan names: a write to $2007 is not a read of it, so it
    // primes nothing, and the read below it is still lone. Without this, the
    // warning is silenced on the line after every `ppu.data = ...` and no other
    // test in the workspace notices.
    let program = lower_source("main {\n    ppu.data = $21\n    var a: u8 = ppu.data\n}\n");

    assert_eq!(program.warnings.len(), 1);
}

#[test]
fn an_if_condition_that_reads_ppu_data_is_a_neighbour() {
    // A condition is evaluated where its statement sits, so it primes the read
    // above it. The `if` body is a block and never a neighbour; that is
    // `a_neighbour_never_crosses_a_block_boundary`.
    let program = lower_source(
        "main {\n    var flag: u8 = 0\n    var a: u8 = ppu.data\n    if ppu.data != 0 { flag = 2 }\n}\n",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn a_while_condition_that_reads_ppu_data_is_a_neighbour() {
    let program =
        lower_source("main {\n    var a: u8 = ppu.data\n    while ppu.data != 0 { }\n}\n");

    assert!(program.warnings.is_empty());
}

#[test]
fn a_returned_ppu_data_read_is_a_neighbour() {
    let program = lower_source(
        "fn fetch() -> u8 {\n    var a: u8 = ppu.data\n    return ppu.data\n}\nmain { fetch() }\n",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn a_ppu_data_read_inside_an_assignment_destination_is_still_a_read() {
    // Only the destination itself is a write. A `ppu.data` read that is part of
    // working out *where* to write is an ordinary read, and primes the line
    // above it. Arrays are refused by this release, so the program does not
    // lower — but the warnings it found on the way still say what the rule is.
    let failure = lower_failure(
        "var table: [2]u8\nmain {\n    var a: u8 = ppu.data\n    table[ppu.data] = 5\n}\n",
    );

    assert!(failure.warnings.is_empty(), "{:?}", failure.warnings);
}

#[test]
fn a_ppu_status_read_inside_an_address_pair_warns() {
    let source = "main {\n    ppu.addr = $3f\n    var s: u8 = ppu.status\n    ppu.addr = $00\n}\n";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this `ppu.status` read leaves your `ppu.addr` pair half written"
    );
    assert_eq!(
        warning.label,
        "the `ppu.addr` write below this becomes a second high byte"
    );
    assert_eq!(
        warning.notes,
        [
            "$2005 and $2006 share one write latch, and reading $2002 puts\nit back to expecting a high byte",
            "the PPU never sees the low byte, so it reads and writes at an\naddress you did not ask for",
            "read `ppu.status` above the pair or below it, not inside it",
        ]
    );
    assert_eq!(line_of(source, warning.span.start), 3);
}

#[test]
fn a_ppu_status_read_before_a_complete_pair_is_silent() {
    let program = lower_source(
        "main {\n    var s: u8 = ppu.status\n    ppu.addr = $3f\n    ppu.addr = $00\n}\n",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn a_ppu_status_read_with_no_pair_open_is_silent() {
    let program = lower_source("main {\n    ppu.mask = $1e\n    var s: u8 = ppu.status\n}\n");

    assert!(program.warnings.is_empty());
}

/// What pins the ordering inside one statement: the read is applied to the
/// latch before the write is, because codegen emits the value and then the
/// store.
///
/// The trailing read is what makes this a pin rather than a description. With
/// the write applied first, the pair opened on line 2 is still open when line 4
/// reads $2002, and this correct program warns; with the read applied first,
/// line 3 closes the pair the first line opened and line 4 is silent. The
/// two-line fixture the plan named warns under neither order.
#[test]
fn a_read_feeding_the_write_that_opens_a_pair_is_silent() {
    let program = lower_source(
        "main {\n    ppu.addr = ppu.status\n    ppu.addr = $00\n    var s: u8 = ppu.status\n}\n",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn a_ppu_status_read_inside_a_scroll_pair_warns_in_scroll_words() {
    let source =
        "main {\n    ppu.scroll = $10\n    var s: u8 = ppu.status\n    ppu.scroll = $20\n}\n";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this `ppu.status` read leaves your `ppu.scroll` pair half written"
    );
    assert_eq!(
        warning.label,
        "the `ppu.scroll` write below this becomes a second X scroll"
    );
    assert_eq!(
        warning.notes,
        [
            "$2005 and $2006 share one write latch, and reading $2002 puts\nit back to expecting an X scroll",
            "the PPU never sees the Y scroll, so the picture scrolls\nsomewhere you did not ask for",
            "read `ppu.status` above the pair or below it, not inside it",
        ]
    );
    assert_eq!(line_of(source, warning.span.start), 3);
}

/// The two registers share one latch, so two writes close a pair whichever
/// registers they were.
#[test]
fn an_address_write_then_a_scroll_write_closes_the_pair() {
    let program = lower_source(
        "main {\n    ppu.addr = $3f\n    ppu.scroll = $10\n    var s: u8 = ppu.status\n}\n",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn sync_exact_inside_an_address_pair_warns() {
    let source = "main {\n    ppu.addr = $3f\n    sync exact\n    ppu.addr = $00\n}\n";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "`sync exact` leaves your `ppu.addr` pair half written"
    );
    assert_eq!(
        warning.label,
        "`sync exact` polls $2002, and that resets the latch mid-pair"
    );
    assert_eq!(
        warning.notes,
        [
            "$2005 and $2006 share one write latch, and reading $2002 puts\nit back to expecting a high byte",
            "the PPU never sees the low byte, so it reads and writes at an\naddress you did not ask for",
            "put `sync exact` above the pair or below it, not inside it",
        ]
    );
    // The carets cover `sync exact` itself, not the lines around it.
    assert_eq!(line_of(source, warning.span.start), 3);
    assert_eq!(line_of(source, warning.span.end), 3);
}

#[test]
fn sync_exact_outside_a_pair_is_silent() {
    let program =
        lower_source("main {\n    ppu.addr = $3f\n    ppu.addr = $00\n    sync exact\n}\n");

    assert!(program.warnings.is_empty());
}

#[test]
fn a_pair_is_not_tracked_into_a_nested_block() {
    let program = lower_source(
        "main {\n    var flag: u8 = 1\n    ppu.addr = $3f\n    if flag > 0 {\n        var s: u8 = ppu.status\n    }\n    ppu.addr = $00\n}\n",
    );

    assert!(program.warnings.is_empty());
}

/// The latch goes back to closed after a block rasterc cannot see through, so
/// the read below is silent rather than wrong about a pair that may or may not
/// still be open.
#[test]
fn a_pair_is_not_tracked_across_a_nested_block() {
    let program = lower_source(
        "main {\n    var flag: u8 = 1\n    ppu.addr = $3f\n    if flag > 0 {\n        ppu.mask = 0\n    }\n    var s: u8 = ppu.status\n}\n",
    );

    assert!(program.warnings.is_empty());
}

#[test]
fn a_pair_is_not_tracked_across_a_call() {
    let program = lower_source(
        "fn paint() {\n    ppu.mask = $1e\n}\n\nmain {\n    ppu.addr = $3f\n    paint()\n    var s: u8 = ppu.status\n}\n",
    );

    assert!(program.warnings.is_empty());
}

/// A condition is evaluated where its statement sits, so a poll loop between
/// the two halves of a pair breaks it like any other read.
#[test]
fn a_condition_that_reads_ppu_status_breaks_a_pair() {
    let source =
        "main {\n    ppu.addr = $3f\n    while ppu.status < $80 {\n    }\n    ppu.addr = $00\n}\n";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].message,
        "this `ppu.status` read leaves your `ppu.addr` pair half written"
    );
    // The carets are on the `ppu.status` in the condition, not on the `while`.
    assert_eq!(line_of(source, program.warnings[0].span.start), 3);
}

/// The first read is what breaks the pair; the second finds the latch already
/// back at its first write. The span is asserted rather than only the count,
/// because the count passes whichever read it lands on.
#[test]
fn two_ppu_status_reads_in_one_statement_warn_once_on_the_first() {
    let source =
        "main {\n    ppu.addr = $3f\n    var s: u8 = ppu.status + ppu.status\n    ppu.addr = $00\n}\n";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    let span = program.warnings[0].span;
    assert_eq!(line_of(source, span.start), 3);
    assert_eq!(
        &source[span.start as usize..span.end as usize],
        "ppu.status"
    );
    // The first of the two, at column 17, not the second at column 30.
    assert_eq!(span.start as usize, source.find("ppu.status").unwrap());
}

/// A compound assignment reads its own destination, so it really does break a
/// pair. The carets cover the destination, which is the width raster-1t9 chose
/// for a compound assignment.
///
/// Read off the failure rather than off a `Program`: `raster-xeo` merged while
/// this bead was in review and now refuses a write to `ppu.status` outright, so
/// this fixture no longer lowers. The classification is still this bead's to
/// get right — a warning survives a failed lowering, and which statements read
/// $2002 is not a fact about whether the program compiles.
#[test]
fn a_compound_assignment_to_ppu_status_is_a_read() {
    let source = "main {\n    ppu.addr = $3f\n    ppu.status += 1\n    ppu.addr = $00\n}\n";
    let failure = lower_failure(source);

    assert_eq!(failure.warnings.len(), 1);
    assert_eq!(
        failure.warnings[0].message,
        "this `ppu.status` read leaves your `ppu.addr` pair half written"
    );
    let span = failure.warnings[0].span;
    assert_eq!(
        &source[span.start as usize..span.end as usize],
        "ppu.status"
    );
}

/// A plain write is not a read, and this bead must not turn it into one.
///
/// `raster-xeo` merged while this bead was in review and now refuses the write
/// itself, so the fixture fails to lower and the assertion is on the failure's
/// warnings. That is still exactly the property this bead owns: whatever
/// `raster-xeo` decides about the store, the latch rule must not add a second
/// diagnostic claiming a read that never happened.
#[test]
fn a_plain_write_to_ppu_status_is_not_a_read() {
    let failure =
        lower_failure("main {\n    ppu.addr = $3f\n    ppu.status = 0\n    ppu.addr = $00\n}\n");

    assert!(failure.warnings.is_empty());
}

#[test]
fn a_ppu_status_read_directly_before_sync_exact_warns() {
    let source = "main {\n    var s: u8 = ppu.status\n    sync exact\n}\n";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this `ppu.status` read costs you a frame at the `sync exact` below"
    );
    assert_eq!(
        warning.label,
        "reading $2002 clears the vblank flag the poll is waiting for"
    );
    assert_eq!(
        warning.notes,
        [
            "the flag is set once a frame, and any read of $2002 clears it,\nwhoever does the reading",
            "`sync exact` therefore waits for the next frame rather than\nthis one",
            "put `sync exact` first, and read `ppu.status` after it",
        ]
    );
    assert_eq!(line_of(source, warning.span.start), 2);
}

/// The rule looks at the very next statement and no further: a read and a sync
/// with anything between them is a stale flag either way.
#[test]
fn a_ppu_status_read_two_statements_before_sync_exact_is_silent() {
    let program =
        lower_source("main {\n    var s: u8 = ppu.status\n    ppu.mask = $1e\n    sync exact\n}\n");

    assert!(program.warnings.is_empty());
}

/// The loop only exits by consuming a set flag, so the sync below it always
/// waits a frame.
#[test]
fn a_poll_loop_directly_before_sync_exact_warns() {
    let source = "main {\n    while ppu.status < $80 {\n    }\n    sync exact\n}\n";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].message,
        "this `ppu.status` read costs you a frame at the `sync exact` below"
    );
    assert_eq!(line_of(source, program.warnings[0].span.start), 2);
}

/// Two faults whose fixes point in opposite directions — move the read down
/// past the pair, move it up past the sync — so the author has to see both.
#[test]
fn a_read_that_trips_both_rules_warns_twice_with_the_latch_first() {
    let program =
        lower_source("main {\n    ppu.addr = $3f\n    var s: u8 = ppu.status\n    sync exact\n}\n");

    assert_eq!(program.warnings.len(), 2);
    assert_eq!(
        program.warnings[0].message,
        "this `ppu.status` read leaves your `ppu.addr` pair half written"
    );
    assert_eq!(
        program.warnings[1].message,
        "this `ppu.status` read costs you a frame at the `sync exact` below"
    );
    assert_eq!(program.warnings[0].span, program.warnings[1].span);
}

/// `sync exact` closes the pair it just broke, exactly as a `ppu.status` read
/// does: its own poll has already put the latch back to expecting a high byte,
/// so the read below it is correct and must stay silent.
///
/// The pin for the `effect.sync` half of the line that clears the latch. Test
/// `a_read_that_trips_both_rules_warns_twice_with_the_latch_first` is the same
/// pin for the `effect.read` half; without this one, dropping `|| effect.sync`
/// leaves the whole suite green while this program grows a second warning
/// blaming a read that is right.
#[test]
fn a_read_after_a_sync_that_broke_a_pair_is_silent_about_the_read() {
    let program =
        lower_source("main {\n    ppu.addr = $3f\n    sync exact\n    var s: u8 = ppu.status\n}\n");

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].message,
        "`sync exact` leaves your `ppu.addr` pair half written"
    );
}

/// The only agreed string combination `half_written_pair` composes that no
/// other test covers: the `sync exact` message and label with the scroll pair's
/// two notes.
#[test]
fn sync_exact_inside_a_scroll_pair_warns_in_scroll_words() {
    let program =
        lower_source("main {\n    ppu.scroll = $10\n    sync exact\n    ppu.scroll = $20\n}\n");

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "`sync exact` leaves your `ppu.scroll` pair half written"
    );
    assert_eq!(
        warning.label,
        "`sync exact` polls $2002, and that resets the latch mid-pair"
    );
    assert_eq!(
        warning.notes,
        [
            "$2005 and $2006 share one write latch, and reading $2002 puts\nit back to expecting an X scroll",
            "the PPU never sees the Y scroll, so the picture scrolls\nsomewhere you did not ask for",
            "put `sync exact` above the pair or below it, not inside it",
        ]
    );
}

/// A `timed` schedule synchronizes once and counts forward, so a cycle it did not spend itself is
/// never recovered. NMI costs it thirteen every frame, out of cycles it had already allocated —
/// the ROM still builds, because every other hazard rasterc can see but not prove is a warning.
#[test]
fn a_timed_frame_with_nmi_on_is_a_warning() {
    let program = lower_source(
        "main {\n\
         ppu.ctrl = $88\n\
         ppu.mask = $1e\n\
         }\n\
         frame bars using timed {\n\
         every 8 scanlines from 0 to 239 { ppu.mask = $1e }\n\
         }",
    );

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this program enables NMI, and a `timed` frame cannot afford one"
    );
    assert_eq!(
        warning.label,
        "this schedule counts cycles from one synchronization onwards"
    );
    assert_eq!(
        warning.notes,
        [
            "`ppu.ctrl` holds $88 when the frame starts, and bit 7 is NMI;\neach NMI costs the schedule 13 cycles it has already spent",
            "clear bit 7 to keep the schedule, or use `using irq`, which\nsynchronizes on every scanline it fires",
        ]
    );
}

/// A value rasterc could not fold is said so rather than passed over: the shape
/// `mmc3.bank_select` already uses for a value it cannot see.
#[test]
fn a_timed_frame_whose_ppu_ctrl_cannot_be_folded_is_a_warning() {
    let program = lower_source(
        "var flags: u8\n\
         main {\n\
         ppu.ctrl = flags\n\
         ppu.mask = $1e\n\
         }\n\
         frame bars using timed {\n\
         every 8 scanlines from 0 to 239 { ppu.mask = $1e }\n\
         }",
    );

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "rasterc cannot tell whether this program enables NMI"
    );
    assert_eq!(
        warning.label,
        "this schedule counts cycles from one synchronization onwards"
    );
    assert_eq!(
        warning.notes,
        [
            "the last write to `ppu.ctrl` before the frame is not a value\nrasterc can see, so bit 7 is unknown",
            "bit 7 is NMI, and each NMI costs the schedule 13 cycles it has\nalready spent; writing a constant with bit 7 clear keeps it",
        ]
    );
}

/// Written on some paths and not others is its own first note, and not the unfoldable one: they
/// are two different things for the author to go and fix.
#[test]
fn a_timed_frame_whose_ppu_ctrl_is_written_on_some_paths_is_a_warning() {
    let program = lower_source(
        "main {\n\
         var x: u8 = 1\n\
         if x != 0 { ppu.ctrl = $08 }\n\
         ppu.mask = $1e\n\
         }\n\
         frame bars using timed {\n\
         every 8 scanlines from 0 to 239 { ppu.mask = $1e }\n\
         }",
    );

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "rasterc cannot tell whether this program enables NMI"
    );
    assert_eq!(
        warning.notes[0],
        "`ppu.ctrl` is written on some paths through this program and not\nothers, so bit 7 is unknown"
    );
    assert_ne!(
        warning.notes[0],
        "the last write to `ppu.ctrl` before the frame is not a value\nrasterc can see, so bit 7 is unknown"
    );
}

/// A handler write is worse than a clean `main`: NMI goes on for every frame after the first, and
/// the program-level check sees nothing at all, because it reads what holds *before* the frame.
#[test]
fn a_handler_that_enables_nmi_is_a_warning() {
    let source = "main {\n\
                  ppu.ctrl = $08\n\
                  ppu.mask = $1e\n\
                  }\n\
                  frame bars using timed {\n\
                  every 8 scanlines from 0 to 239 { ppu.ctrl = $88 }\n\
                  }";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this write enables NMI, and a `timed` frame cannot afford one"
    );
    assert_eq!(
        warning.label,
        "bit 7 is NMI, and this handler runs on every frame"
    );
    assert_eq!(
        warning.notes,
        [
            "the schedule is counted from one synchronization and never\nre-checked; each NMI costs it 13 cycles it has already spent",
            "clear bit 7 to keep the schedule, or use `using irq`, which\nsynchronizes on every scanline it fires",
        ]
    );
    // The carets are on the handler's own write, not on the frame's strategy.
    assert_eq!(
        &source[warning.span.start as usize..warning.span.end as usize],
        "ppu.ctrl = $88"
    );
}

/// A handler write rasterc cannot fold gets the same treatment as one in `main`.
#[test]
fn a_handler_ppu_ctrl_write_rasterc_cannot_fold_is_a_warning() {
    let program = lower_source(
        "var flags: u8\n\
         main {\n\
         ppu.ctrl = $08\n\
         ppu.mask = $1e\n\
         }\n\
         frame bars using timed {\n\
         every 8 scanlines from 0 to 239 { ppu.ctrl = flags }\n\
         }",
    );

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "rasterc cannot tell whether this write enables NMI"
    );
    assert_eq!(
        warning.label,
        "this is not a value rasterc can see, so bit 7 is unknown"
    );
    assert_eq!(
        warning.notes,
        [
            "bit 7 is NMI, and this handler runs on every frame; each NMI\ncosts the schedule 13 cycles it has already spent",
            "writing a constant with bit 7 clear keeps the schedule",
        ]
    );
}

/// An IRQ chain re-synchronizes on every scanline it fires, so NMI costs it nothing it cannot
/// recover — and §13's own flagship example turns NMI on. It has to keep compiling quietly.
#[test]
fn an_irq_frame_that_enables_nmi_is_silent() {
    let program = lower_source(
        "main {\n\
         ppu.ctrl = $88\n\
         ppu.mask = $1e\n\
         }\n\
         frame bars using irq {\n\
         at scanline 60 { ppu.mask = $1e }\n\
         }",
    );

    assert!(program.warnings.is_empty(), "{:?}", program.warnings);
}

/// The check is about what `ppu.ctrl` holds when the frame starts, not about every value it ever
/// held: a program that sets bit 7 and clears it again is correct.
#[test]
fn a_timed_frame_that_clears_bit_7_again_before_it_starts_is_silent() {
    let program = lower_source(
        "main {\n\
         ppu.ctrl = $88\n\
         ppu.ctrl = $08\n\
         ppu.mask = $1e\n\
         }\n\
         frame bars using timed {\n\
         every 8 scanlines from 0 to 239 { ppu.mask = $1e }\n\
         }",
    );

    assert!(program.warnings.is_empty(), "{:?}", program.warnings);
}

/// The reset runtime stores zero into `ppu.ctrl` before the author's code runs, so a program that
/// never writes it is provably NMI-off. There is no fourth "never written" verdict to give.
#[test]
fn a_timed_frame_with_no_ppu_ctrl_write_is_silent() {
    let program = lower_source(
        "main {\n\
         ppu.mask = $1e\n\
         }\n\
         frame bars using timed {\n\
         every 8 scanlines from 0 to 239 { ppu.mask = $1e }\n\
         }",
    );

    assert!(program.warnings.is_empty(), "{:?}", program.warnings);
}

/// An ordinary handler adjusting the nametable or the pattern half is a correct program, and a
/// warning that fired on it would be one an author learns to ignore.
#[test]
fn a_handler_write_with_bit_7_clear_is_silent() {
    let program = lower_source(
        "main {\n\
         ppu.mask = $1e\n\
         }\n\
         frame bars using timed {\n\
         every 8 scanlines from 0 to 239 { ppu.ctrl = $08 }\n\
         }",
    );

    assert!(program.warnings.is_empty(), "{:?}", program.warnings);
}

/// An omitted `using` clause means `timed` (spec section 7.1), so the same program earns the same
/// warning. There is no `timed` to underline, so the carets fall back to the frame item's own
/// span, which begins at the `frame` keyword.
#[test]
fn a_frame_with_no_using_clause_earns_the_same_warning() {
    let source = "frame bars {\n\
                  every 8 scanlines from 0 to 239 { ppu.mask = $1e }\n\
                  }\n\
                  main {\n\
                  ppu.ctrl = $88\n\
                  ppu.mask = $1e\n\
                  }";
    let program = lower_source(source);

    assert_eq!(program.warnings.len(), 1);
    let warning = &program.warnings[0];
    assert_eq!(
        warning.message,
        "this program enables NMI, and a `timed` frame cannot afford one"
    );
    assert!(
        source[warning.span.start as usize..warning.span.end as usize].starts_with("frame bars"),
        "the carets take the frame's own span, not a strategy that is not written"
    );
}

/// The NMI check runs after lowering, so its warnings are appended to whatever lowering already
/// found — whatever the line numbers say. The frame is declared first here and the bank select
/// last, so source order and emission order disagree and this pins the one that ships.
#[test]
fn an_nmi_warning_prints_after_a_bank_warning_whatever_the_line_numbers() {
    let program = lower_source(
        "frame bars {\n\
         every 8 scanlines from 0 to 239 { ppu.mask = $1e }\n\
         }\n\
         main {\n\
         ppu.ctrl = $88\n\
         mmc3.bank_select = $80\n\
         }",
    );

    assert_eq!(program.warnings.len(), 2);
    assert_eq!(
        program.warnings[0].message,
        "this bank select changes the MMC3 mapping mode"
    );
    assert_eq!(
        program.warnings[1].message,
        "this program enables NMI, and a `timed` frame cannot afford one"
    );
    assert!(
        program.warnings[1].span.start < program.warnings[0].span.start,
        "the fixture is only a test of ordering while the two disagree with source order"
    );
}
