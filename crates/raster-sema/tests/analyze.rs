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

#[test]
fn rejects_unprovable_timed_region() {
    let diagnostics = errors(
        r#"
            fn helper() { }
            var count: u8
            main {
                cycles(100) {
                    loop { count = 1 }
                    while count < 3 { count = count + 1 }
                    if count == 1 { count = 2 }
                    helper()
                    count = count * count
                    wait vblank
                }
            }
        "#,
    );

    for expected in [
        "unbounded loop",
        "`while` loop",
        "branch arms",
        "`helper` cannot be called inside a timed region yet",
        "multiplication, division and remainder",
        "`wait vblank`",
    ] {
        assert!(
            diagnostics.iter().any(|message| message.contains(expected)),
            "expected a diagnostic mentioning {expected}, got {diagnostics:?}"
        );
    }
}

#[test]
fn accepts_a_provable_timed_region() {
    let program = parse(
        r#"
            var count: u8
            main {
                sync exact
                cycles(100) pad {
                    count = count + 1
                    ppu.mask = count
                }
            }
        "#,
    )
    .expect("fixture should parse");
    analyze(&program).expect("a provable timed region is accepted");
}

#[test]
fn requires_sync_exact_before_a_timed_region_that_writes_the_ppu() {
    let diagnostics = errors(
        r#"
            var count: u8
            main {
                cycles(100) pad {
                    ppu.mask = count
                }
            }
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("`sync exact` is required")),
        "got {diagnostics:?}"
    );
}

#[test]
fn rejects_an_unknown_sync_strategy() {
    let diagnostics = errors(
        r#"
            main {
                sync approximately
            }
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("unknown sync strategy `approximately`")),
        "got {diagnostics:?}"
    );
}

#[test]
fn rejects_a_report_region_without_a_label() {
    let diagnostics = errors(
        r#"
            main {
                cycles(?) { }
            }
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("`cycles(?)` needs a label")),
        "got {diagnostics:?}"
    );
}

/// Every construct that lowers to a loop the region's straight-line cost model would charge a
/// single pass. Each of these compiled clean before, and each was mistimed by hundreds of cycles.
#[test]
fn rejects_every_construct_a_straight_line_region_cannot_charge() {
    let cases = [
        (
            "for i in 0..10 { total = total + 1 }",
            "`for` loop inside a timed region",
        ),
        (
            "total = total * 3",
            "multiplication, division and remainder",
        ),
        (
            "total = total / 3",
            "multiplication, division and remainder",
        ),
        (
            "total = total % 3",
            "multiplication, division and remainder",
        ),
        ("total = total << 3", "shift inside a timed region"),
        ("total = total >> 3", "shift inside a timed region"),
        ("sync exact", "belongs before a timed region"),
        ("wait cycles(20)", "spends its cycles in a loop"),
        ("helper()", "cannot be called inside a timed region yet"),
    ];

    for (statement, expected) in cases {
        let diagnostics = errors(&format!(
            "fn helper() cycles(6) {{ }}\nvar total: u8\nmain {{ cycles(76) {{ {statement} }} }}"
        ));
        assert!(
            diagnostics.iter().any(|message| message.contains(expected)),
            "expected a diagnostic mentioning {expected} for `{statement}`, got {diagnostics:?}"
        );
    }
}

#[test]
fn sync_exact_need_not_sit_immediately_before_the_region_it_guards() {
    let program = parse(
        r#"
            var count: u8
            main {
                sync exact
                count = 1
                cycles(100) pad {
                    ppu.mask = count
                }
            }
        "#,
    )
    .expect("fixture should parse");
    analyze(&program).expect("a `sync exact` earlier in the block still guards the region");
}

#[test]
fn a_frame_handler_is_entered_synchronized_and_needs_no_sync_of_its_own() {
    // Outside a frame the rule of spec section 6.6 still bites.
    let diagnostics = errors(
        r#"
            main { cycles(114) pad { ppu.mask = $1e } }
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("`sync exact` is required")),
        "expected the jitter rule in {diagnostics:?}"
    );

    // Inside one it does not: the frame synchronizes on vblank and counts every handler's position
    // in cycles from there, which is the whole of what the construct is for.
    let program = parse(
        r#"
            main { ppu.mask = 0 }
            frame bars using timed {
                at scanline 60 { cycles(114) pad { ppu.mask = $1e } }
            }
        "#,
    )
    .expect("valid fixture should parse");
    analyze(&program).expect("a frame handler carries its own synchronization");
}

#[test]
fn the_jitter_rule_still_applies_to_a_program_that_declares_a_frame() {
    // A frame handler is entered synchronized; nothing after the frame is. If the flag the frame
    // sets were left set, section 6.6 would quietly stop applying to every program declaring one,
    // and the suite would stay green — so the frame is written ahead of `main` on purpose.
    let diagnostics = errors(
        r#"
            frame bars using timed {
                at scanline 60 { cycles(114) pad { ppu.mask = $1e } }
            }
            main { cycles(114) pad { ppu.mask = $1e } }
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("`sync exact` is required")),
        "expected the jitter rule in {diagnostics:?}"
    );
}

#[test]
fn a_frame_handler_is_a_timed_region_and_obeys_section_6_3() {
    // A handler is padded to the scanline it is scheduled on, so its cost has to be provable for
    // exactly the reason a `cycles` block's does — and by the same rules. A `wait` spends its
    // cycles in a loop the region is not costed for, and a branch's arms are not balanced, so
    // either would leave the whole schedule after it in the wrong place.
    let diagnostics = errors(
        r#"
            main { ppu.mask = 0 }
            frame bars using timed {
                at scanline 60 { wait cycles(200) }
                at scanline 90 { if 1 == 1 { ppu.mask = 1 } }
                at scanline 120 { var count: u8 = 2; while count != 0 { count = count - 1 } }
            }
        "#,
    );
    for expected in [
        "`wait cycles` inside a timed region",
        "branch arms inside a timed region",
        "a `while` loop's trip count cannot be proven inside a timed region",
    ] {
        assert!(
            diagnostics.iter().any(|message| message.contains(expected)),
            "expected `{expected}` in {diagnostics:?}"
        );
    }
}
