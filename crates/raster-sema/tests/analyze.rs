use raster_sema::analyze;
use raster_syntax::parse;

fn errors(source: &str) -> Vec<String> {
    let program = parse(source).expect("fixture should parse");
    analyze(&program)
        .expect_err("fixture should be semantically invalid")
        .into_iter()
        .map(|error| error.message)
        .collect()
}

#[test]
fn resolves_scopes_and_reports_unknown_or_duplicate_names() {
    let diagnostics = errors(
        r#"
            var value: u8
            var value: u8
            fn render(value: u8) {
                { var value: u8 = 1 }
                missing = value
            }
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("duplicate declaration `value`"))
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("unknown name `missing`"))
    );
}

#[test]
fn types_mvp_expressions_assignments_and_register_access() {
    let valid = r#"
        var buffer: [4]u8
        var enabled: bool = true
        fn set(value: u8) -> u8 { return value }
        main {
            var value: u8 = 1
            buffer[0] = set(value)
            ppu.mask = buffer[0]
            if enabled { value = 2 }
        }
    "#;
    let program = parse(valid).expect("valid fixture should parse");
    assert!(analyze(&program).is_ok(), "valid MVP source should analyze");

    let diagnostics = errors(
        r#"
            const LIMIT: u8 = 1
            fn set(value: u8) {}
            main {
                var value: u8 = 1
                value = true
                LIMIT = 2
                set()
                ppu.no_such_register = value
                if value {}
            }
        "#,
    );
    for expected in [
        "assignment operands must have compatible types",
        "constants are not assignable",
        "expects 1 arguments",
        "unknown ppu register",
        "condition must have type bool",
    ] {
        assert!(
            diagnostics.iter().any(|message| message.contains(expected)),
            "expected `{expected}` in {diagnostics:?}"
        );
    }
}

#[test]
fn evaluates_constants_and_requires_static_bounds() {
    let program = parse(
        r#"
            const LIMIT: u8 = 2 + 2
            var buffer: [LIMIT]u8
            main { for index in 0..LIMIT { cycles(LIMIT) {} } }
            frame display { every LIMIT scanlines from 0 to 240 {} }
        "#,
    )
    .expect("valid fixture should parse");
    assert_eq!(
        analyze(&program)
            .expect("constants and static bounds should analyze")
            .constants
            .get("LIMIT"),
        Some(&4)
    );

    let diagnostics = errors(
        r#"
            var dynamic: u16
            var empty: [0]u8
            const SMALL: u8 = 256
            main {
                for index in 4..0 {}
                cycles(dynamic) {}
            }
            frame display { every 0 scanlines from 10 to 10 {} }
        "#,
    );
    for expected in [
        "array length must be greater than zero",
        "constant value overflows",
        "for range start must be less than its end",
        "cycle bound must be a compile-time constant",
        "frame interval must be greater than zero",
        "frame range start must be less than its end",
    ] {
        assert!(
            diagnostics.iter().any(|message| message.contains(expected)),
            "expected `{expected}` in {diagnostics:?}"
        );
    }
}

#[test]
fn rejects_semantically_invalid_mvp_fixture_with_all_errors() {
    let diagnostics = errors(
        r#"
            const VALUE: u8 = 1
            const VALUE: u8 = 2
            fn render(value: u8) -> bool { return 1 }
            main {
                unknown = true
                VALUE = 2
                render(true, 2)
                cycles(unknown) {}
            }
        "#,
    );
    for expected in [
        "duplicate declaration `VALUE`",
        "unknown name `unknown`",
        "constants are not assignable",
        "expects 1 arguments",
        "return expression does not match",
        "cycle bound must be a compile-time constant",
    ] {
        assert!(
            diagnostics.iter().any(|message| message.contains(expected)),
            "expected `{expected}` in {diagnostics:?}"
        );
    }
}

#[test]
fn validates_function_bounds_employs_and_integer_contexts() {
    let diagnostics = errors(
        r#"
            group state { var line: u8 }
            var dynamic: u8
            fn timer() cycles(dynamic) {}
            unsafe asm fn upload() employs(missing_group) {}
            fn byte() -> u8 { return 256 }
            fn takes_byte(value: u8) {}
            main {
                var byte: u8 = 256
                byte = 256
                takes_byte(256)
                var table: [2]u8
                var index: u8 = 0
                table[index] = byte
            }
        "#,
    );
    for expected in [
        "cycle bound must be a compile-time constant",
        "unknown name `missing_group`",
        "integer literal overflows u8",
    ] {
        assert!(
            diagnostics.iter().any(|message| message.contains(expected)),
            "expected `{expected}` in {diagnostics:?}"
        );
    }

    let program = parse(
        r#"
            group state { var line: u8 }
            unsafe asm fn upload() employs(state) {}
        "#,
    )
    .expect("group fixture should parse");
    assert!(analyze(&program).is_ok());
}
