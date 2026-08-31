use std::collections::BTreeSet;

use raster_diag::Refusal;
use raster_ir::{lower, PlaceKind, Statement};
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
    lower(&typed).expect_err("fixture should not lower")
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
    source[..offset as usize].matches('\n').count() + 1
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
    let errors = lower(&typed).expect_err("unsupported forms must be diagnosed");

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

    assert!(
        errors
            .iter()
            .all(|error| error.refusal == Refusal::NotInThisRelease),
        "an array is a construct the release does not have: {errors:?}"
    );
}
