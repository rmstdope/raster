use std::collections::BTreeSet;

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

#[test]
fn rejects_all_accepted_forms_not_supported_by_initial_codegen() {
    let syntax = parse(
        r#"
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
                cycles(2) {}
                wait vblank
                sync schedule
                loop { break; continue }
            }
        "#,
    )
    .expect("fixture should parse");
    let typed = analyze(&syntax).expect("fixture should analyze");
    let errors = lower(&typed).expect_err("unsupported forms must be diagnosed");

    for expected in [
        "group",
        "array",
        "u16",
        "bool",
        "assembly",
        "frame",
        "indexing",
        "string",
        "character",
        "timing",
        "wait",
        "sync",
        "loop",
        "break",
        "continue",
    ] {
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "expected `{expected}` in {errors:?}"
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

    lower_source(
        r#"
            fn choose(value: u8) -> u8 {
                if value == 0 { return 1 } else { return 2 }
            }
            main {}
        "#,
    );
}
