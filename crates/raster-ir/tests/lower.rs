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

/// Every named register, with the three facts a diagnostic needs from it: the
/// name it has in source, its address, and whether a read of it is refused.
/// Written out here rather than derived, so the test fails when the compiler's
/// own table changes rather than agreeing with it by construction.
const REGISTERS: [(Register, &str, u16, bool); 16] = [
    (Register::PpuCtrl, "ppu.ctrl", 0x2000, true),
    (Register::PpuMask, "ppu.mask", 0x2001, true),
    (Register::PpuStatus, "ppu.status", 0x2002, false),
    (Register::PpuOamAddr, "ppu.oam_addr", 0x2003, true),
    (Register::PpuOamData, "ppu.oam_data", 0x2004, false),
    (Register::PpuScroll, "ppu.scroll", 0x2005, true),
    (Register::PpuAddr, "ppu.addr", 0x2006, true),
    (Register::PpuData, "ppu.data", 0x2007, false),
    (Register::Mmc3BankSelect, "mmc3.bank_select", 0x8000, true),
    (Register::Mmc3BankData, "mmc3.bank_data", 0x8001, true),
    (Register::Mmc3Mirroring, "mmc3.mirroring", 0xa000, true),
    (Register::Mmc3RamProtect, "mmc3.ram_protect", 0xa001, true),
    (Register::Mmc3IrqLatch, "mmc3.irq_latch", 0xc000, true),
    (Register::Mmc3IrqReload, "mmc3.irq_reload", 0xc001, true),
    (Register::Mmc3IrqDisable, "mmc3.irq_disable", 0xe000, true),
    (Register::Mmc3IrqEnable, "mmc3.irq_enable", 0xe001, true),
];

#[test]
fn the_register_table_names_every_register_and_says_which_read() {
    for (register, name, address, write_only) in REGISTERS {
        assert_eq!(register.name(), name);
        assert_eq!(register.address(), address);
        assert_eq!(register.is_write_only(), write_only, "{name}");
    }
    // Three of the sixteen read: $2002, $2004 and $2007. If this number moves,
    // a register has changed sides and the spec table in §9.5 has to move with
    // it.
    assert_eq!(REGISTERS.iter().filter(|row| !row.3).count(), 3);
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
            r#"
                main { ppu.mask = 0 }
                frame bars using irq { at scanline 60 { ppu.addr = $01 } }
            "#,
            "`using irq` is not supported yet",
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
fn a_compound_bank_select_cannot_be_folded_and_warns() {
    // `binary` builds a `Value::Binary` unconditionally, so a compound
    // assignment is never a constant to rasterc however plain it reads.
    let program = lower_source("main { mmc3.bank_select += $80 }");

    assert_eq!(program.warnings.len(), 1);
    assert_eq!(
        program.warnings[0].label,
        "rasterc cannot see this value here, so bits 6 and 7 are unknown"
    );
}
