use raster_diag::Refusal;
use raster_sema::analyze;
use raster_syntax::parse;

fn errors(source: &str) -> Vec<String> {
    errors_with_spans(source)
        .into_iter()
        .map(|error| error.message)
        .collect()
}

/// Some refusals are asserted on their span as well as their message, and need the errors whole.
fn errors_with_spans(source: &str) -> Vec<raster_sema::SemanticError> {
    let program = parse(source).expect("fixture should parse");
    analyze(&program).expect_err("fixture should be semantically invalid")
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
fn rejects_unprovable_timed_block() {
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
        "`helper` cannot be called inside a timed block yet",
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
fn accepts_a_provable_timed_block() {
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
            "`for` loop inside a timed block",
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
        ("total = total << 3", "shift inside a timed block"),
        ("total = total >> 3", "shift inside a timed block"),
        ("sync exact", "belongs before a timed block"),
        ("wait cycles(20)", "spends its cycles in a loop"),
        ("helper()", "cannot be called inside a timed block yet"),
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

const RETURN_IN_A_TIMED_BLOCK: &str = "`return` inside a timed block jumps out before the block has spent its budget and before the interrupt flag is restored, so it belongs after the block rather than inside one";

const RETURN_IN_AN_INTERRUPTIBLE_TIMED_BLOCK: &str = "`return` inside a timed block jumps out before the block has spent its budget, so it belongs after the block rather than inside one";

#[test]
fn return_inside_a_timed_block_is_refused() {
    let source = "main { cycles(20) pad { return } }";
    let diagnostics = errors_with_spans(source);

    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].message, RETURN_IN_A_TIMED_BLOCK);
    assert_eq!(
        &source[diagnostics[0].span.start as usize..diagnostics[0].span.end as usize],
        "return"
    );
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
fn return_inside_an_interruptible_timed_block_omits_the_interrupt_clause() {
    // An `interruptible` block emits no `PHP`/`SEI`/`PLP`, so there is no interrupt flag to
    // restore and the longer sentence would be false of it.
    let diagnostics = errors_with_spans("main { cycles(20) pad interruptible { return } }");

    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(
        diagnostics[0].message,
        RETURN_IN_AN_INTERRUPTIBLE_TIMED_BLOCK
    );
    assert!(!diagnostics[0].message.contains("interrupt flag"));
}

#[test]
fn return_inside_a_report_block_is_refused() {
    // `cycles(?)` carries no budget, but it prints a measured cost — and a block that jumps out
    // has no single cost to print.
    let diagnostics = errors_with_spans("main { cycles(?) hblank { return } }");

    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].message, RETURN_IN_A_TIMED_BLOCK);
}

#[test]
fn return_outside_a_timed_block_is_accepted() {
    let program = parse("fn f() -> u8 { return 1 }\nmain { }").expect("fixture should parse");
    analyze(&program).expect("a `return` outside a timed block is ordinary");
}

#[test]
fn return_inside_a_cycle_annotated_function_is_left_to_the_lowering_refusal() {
    // A function's own `cycles(...)` annotation is not a block: `lower` refuses such a function
    // outright, and there is no block for the `return` to go after, so the refusal's advice would
    // be unfollowable. It would also be the only error reported, hiding the accurate one — sema
    // never reaches `lower`.
    let program = parse("fn f() -> u8 cycles(20) {\n    return 1\n}\nmain { }\n")
        .expect("fixture should parse");

    analyze(&program).expect("a cycle-annotated function is `lower`'s to refuse, not sema's");
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
        "`wait cycles` inside a timed block",
        "branch arms inside a timed block",
        "a `while` loop's trip count cannot be proven inside a timed block",
    ] {
        assert!(
            diagnostics.iter().any(|message| message.contains(expected)),
            "expected `{expected}` in {diagnostics:?}"
        );
    }
}

#[test]
fn return_inside_a_frame_handler_is_refused_like_any_other_timed_block() {
    // A handler is emitted through the same `timed_region` a `cycles(...) { }` block is, with
    // `pad` set and `interruptible` clear, so a `return` leaves before the budget is spent and
    // before the `PLP`. Without this it would reach the analyser's backstop instead, whose message
    // asks the author to report the file as a compiler bug.
    let diagnostics = errors_with_spans(
        "main { ppu.mask = 0 }\nframe bars using timed {\n    at scanline 10 {\n        return\n    }\n}\n",
    );

    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].message, RETURN_IN_A_TIMED_BLOCK);
}

/// Every refusal a timed block raises, and the kind it must carry.
///
/// A `TimedRegionCost` refusal is a cost-model gap: the construct is fine, and a timed block costed as
/// straight-line code cannot price it, so `rasterc` says so beside the diagnostic. The four
/// `Rejected` ones deliberately say nothing — two hardware waits have no cost to measure ever, so
/// "once their cost can be measured" would promise what the compiler cannot deliver, and `sync
/// exact` in the wrong place and a `return` inside a block are placement rules whose messages
/// already say where to put the construct.
///
/// This is a table rather than a sentence in the source because the kind is what `rasterc` asks:
/// moving one of these back to an unclassified refusal takes the note away silently, which is the
/// failure this bead exists to close.
///
/// This corpus is the counterpart of the "rasterc today" column in section 6.3 of
/// docs/raster-language-spec.md: one fixture per refusal `raster-sema` raises by name inside a
/// timed block. Per refusal rather than per operator on purpose — `*`, `/` and `%` share one arm
/// and one message here, as do `<<` and `>>`, so a row each would exercise the same code twice.
/// Rule 4's other half, the refusal a cycle-annotated function meets when it is lowered, is not
/// raised in this crate and is pinned by
/// `a_cycle_annotated_function_that_returns_still_names_the_real_refusal` in
/// crates/rasterc/tests/compile.rs. A row added or removed here is an edit to that column, and a
/// construct this test stops covering is that column going stale. Nothing enforces the tie but
/// this sentence — a machine-readable list in the spec was considered and declined, because it
/// puts a block of construct names in front of an author.
#[test]
fn every_timed_block_refusal_carries_the_kind_that_decides_its_note() {
    use raster_diag::Refusal::{Rejected, TimedRegionCost};

    let cases: &[(Refusal, &str, &str)] = &[
        (
            TimedRegionCost,
            "an unbounded loop has no provable cycle cost inside a timed block",
            "main {\n    cycles(20) pad {\n        loop {}\n    }\n}\n",
        ),
        (
            TimedRegionCost,
            "branch arms inside a timed block cannot yet be balanced",
            "var level: u8\nmain {\n    cycles(20) pad {\n        if level == 1 { level = 2 }\n    }\n}\n",
        ),
        (
            TimedRegionCost,
            "a `while` loop's trip count cannot be proven inside a timed block",
            "var level: u8\nmain {\n    cycles(20) pad {\n        while level != 0 { level = level - 1 }\n    }\n}\n",
        ),
        (
            TimedRegionCost,
            "a `for` loop inside a timed block compiles to a loop whose cost is not yet proven",
            "var level: u8\nmain {\n    cycles(20) pad {\n        for index in 0..3 { level = level + 1 }\n    }\n}\n",
        ),
        (
            TimedRegionCost,
            "`wait cycles` inside a timed block spends its cycles in a loop",
            "main {\n    cycles(20) pad {\n        wait cycles(4)\n    }\n}\n",
        ),
        (
            TimedRegionCost,
            "multiplication, division and remainder inside a timed block compile to loops",
            "var level: u8\nmain {\n    cycles(20) pad {\n        level = level * 2\n    }\n}\n",
        ),
        (
            TimedRegionCost,
            "a shift inside a timed block compiles to a loop whose cost is not yet proven",
            "var level: u8\nmain {\n    cycles(20) pad {\n        level = level >> 1\n    }\n}\n",
        ),
        (
            TimedRegionCost,
            "cannot be called inside a timed block yet",
            "fn helper() {}\nmain {\n    cycles(20) pad {\n        helper()\n    }\n}\n",
        ),
        (
            Rejected,
            "`wait scanline` has no provable cost inside a timed block",
            "main {\n    cycles(20) pad {\n        wait scanline 10\n    }\n}\n",
        ),
        (
            Rejected,
            "`wait vblank` has no provable cost inside a timed block",
            "main {\n    cycles(20) pad {\n        wait vblank\n    }\n}\n",
        ),
        (
            Rejected,
            "`sync exact` waits an unpredictable number of cycles",
            "main {\n    cycles(20) pad {\n        sync exact\n    }\n}\n",
        ),
        (
            Rejected,
            "`return` inside a timed block jumps out before the block has spent its budget",
            "main {\n    cycles(20) pad {\n        return\n    }\n}\n",
        ),
    ];

    for (refusal, expected, source) in cases {
        let errors = errors_with_spans(source);
        let refused = errors
            .iter()
            .find(|error| error.message.contains(expected))
            .unwrap_or_else(|| panic!("expected `{expected}` in {errors:?}"));
        assert_eq!(
            refused.refusal, *refusal,
            "`{expected}` decides its note by this kind"
        );
    }
}

/// One of every construct a straight-line block cannot charge, in a single `cycles` block, so
/// that eleven of the twelve refusals fire from one program. The twelfth — `sync exact` is
/// required before a PPU-writing block — cannot join them: it fires only when no `sync exact`
/// precedes the block, and this fixture contains one inside it.
///
/// The guard below only sees the messages these two fixtures provoke, so **a refusal added to
/// `raster-sema` on a construct that is not here is not guarded**: add the construct to this
/// fixture at the same time, and raise the guard's expected count with it.
const REFUSALS_INSIDE_A_BLOCK: &str = r#"
    fn helper() { }
    var count: u8
    main {
        cycles(100) {
            loop { count = 1 }
            while count < 3 { count = count + 1 }
            if count == 1 { count = 2 }
            for i in 0..10 { count = count + 1 }
            helper()
            count = count * count
            count = count << 3
            wait vblank
            wait scanline 96
            wait cycles(20)
            sync exact
        }
    }
"#;

/// A PPU-writing block with no `sync exact` before it: the one message that a program carrying
/// `sync exact` can never produce.
const PPU_WRITE_WITHOUT_SYNC: &str = r#"
    var count: u8
    main {
        cycles(100) pad {
            ppu.mask = count
        }
    }
"#;

#[test]
fn no_refusal_calls_a_timed_block_a_timed_region() {
    let mut diagnostics = errors(REFUSALS_INSIDE_A_BLOCK);
    diagnostics.extend(errors(PPU_WRITE_WITHOUT_SYNC));

    // `errors` only panics when a fixture produces no errors at all, so a construct that stops
    // being refused would drop out of the two filters below in silence and leave them passing
    // over less than they were written to cover. Twelve is what the two fixtures provoke: eleven
    // from the block, one from the PPU write.
    assert_eq!(
        diagnostics.len(),
        12,
        "the fixtures should provoke all twelve refusals; a lower count means this guard is \
         covering less than it reads as covering: {diagnostics:?}"
    );

    let region: Vec<&String> = diagnostics
        .iter()
        .filter(|message| message.contains("timed region"))
        .collect();
    assert!(
        region.is_empty(),
        "`timed block` is the word that ships; these still say `timed region`: {region:?}"
    );

    // Deliberately the bare word rather than `the region`: no shipped diagnostic names the
    // construct a region at all any more, so `a region`, `this region` and `the region's` are
    // caught too — the shapes a future second reference is as likely to take.
    let bare: Vec<&String> = diagnostics
        .iter()
        .filter(|message| message.contains(" region"))
        .collect();
    assert!(
        bare.is_empty(),
        "a message that opens with `timed block` must not call it a region later: {bare:?}"
    );
}

/// The four blocks an image asset exposes, read from inside `main`. Nothing is done
/// with them — they are blocks of ROM data, not values — so naming them must simply
/// analyze.
#[test]
fn an_asset_declares_a_name_with_four_members() {
    let source = r#"
        asset image picture = png("art/picture.png") { kind: background }
        main {
            picture.tiles
            picture.nametable
            picture.attributes
            picture.palette
        }
    "#;
    let program = parse(source).expect("fixture should parse");
    let errors = analyze(&program)
        .err()
        .map(|errors| {
            errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(errors, Vec::<String>::new());
}

#[test]
fn refuses_an_unknown_asset_member() {
    let source = r#"
        asset image picture = png("art/picture.png")
        main { picture.sprites }
    "#;
    let found = errors_with_spans(source);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "unknown asset member `sprites`; an image asset has `tiles`, `nametable`, `attributes` and `palette`"
    );
    assert_eq!(
        &source[found[0].span.start as usize..found[0].span.end as usize],
        "sprites"
    );
    assert_eq!(found[0].refusal, Refusal::Rejected);
}

/// The existing refusal, which the asset arm must not swallow: a member of a `u8`
/// is still a register-namespace mistake.
#[test]
fn refuses_a_member_of_something_that_is_not_a_namespace_or_asset() {
    let messages = errors("main { var value: u8 = 1\n value.mask = 2 }");
    assert!(
        messages
            .iter()
            .any(|message| message == "member access requires a register namespace"),
        "expected the register-namespace refusal, got {messages:?}"
    );
}

/// Every field value this release does not build is refused rather than ignored: a
/// compiler that accepts `dedup: false` and deduplicates anyway is worse than one
/// that says it cannot.
#[test]
fn refuses_asset_fields_this_release_does_not_build() {
    for (field, message, refusal, caret) in [
        (
            "kind: sprites",
            "only `kind: background` is supported yet",
            Refusal::AssetNotInThisRelease,
            "sprites",
        ),
        (
            "palette: auto(2)",
            "only `palette: auto(4)` is supported yet",
            Refusal::AssetNotInThisRelease,
            "auto(2)",
        ),
        (
            "dedup: false",
            "only `dedup: true` is supported yet",
            Refusal::AssetNotInThisRelease,
            "false",
        ),
        (
            "sharpness: 3",
            "unknown asset field; this release knows `kind`, `palette` and `dedup`",
            Refusal::Rejected,
            "sharpness",
        ),
    ] {
        let source = format!("asset image picture = png(\"p.png\") {{ {field} }}\nmain {{ }}");
        let found = errors_with_spans(&source);
        assert_eq!(found.len(), 1, "for `{field}`, got {found:?}");
        assert_eq!(found[0].message, message, "for `{field}`");
        assert_eq!(found[0].refusal, refusal, "for `{field}`");
        assert_eq!(
            &source[found[0].span.start as usize..found[0].span.end as usize],
            caret,
            "for `{field}`"
        );
    }
}

#[test]
fn refuses_an_asset_field_set_twice() {
    let source =
        "asset image picture = png(\"p.png\") { kind: background  kind: background }\nmain { }";
    let found = errors_with_spans(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "this asset field is set twice");
    assert_eq!(found[0].refusal, Refusal::Rejected);
    // The caret is on the second `kind`, which is the one to delete.
    assert_eq!(found[0].span.start as usize, source.rfind("kind").unwrap());
}

#[test]
fn refuses_an_asset_loader_and_kind_this_release_does_not_build() {
    for (source, message, caret) in [
        (
            "asset image picture = fam(\"p.fam\")\nmain { }",
            "only `png` is supported yet",
            "fam",
        ),
        (
            "asset sound jingle = png(\"p.png\")\nmain { }",
            "only `asset image` is supported yet",
            "sound",
        ),
    ] {
        let found = errors_with_spans(source);
        assert_eq!(found.len(), 1, "for `{source}`, got {found:?}");
        assert_eq!(found[0].message, message);
        assert_eq!(found[0].refusal, Refusal::AssetNotInThisRelease);
        assert_eq!(
            &source[found[0].span.start as usize..found[0].span.end as usize],
            caret
        );
    }
}

/// A field and a member the language specification defines (§8.1) and this release
/// does not build are refused as not-yet-built, not as the author's invention. A
/// developer copying §8.1 verbatim must not be told they made `compress` up.
#[test]
fn refuses_spec_defined_asset_surface_as_not_yet_built() {
    let source = "asset image picture = png(\"p.png\") { compress: none }\nmain { }";
    let found = errors_with_spans(source);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "only `kind`, `palette` and `dedup` are supported yet"
    );
    assert_eq!(found[0].refusal, Refusal::AssetNotInThisRelease);
    assert_eq!(
        &source[found[0].span.start as usize..found[0].span.end as usize],
        "compress"
    );

    let source = "asset image picture = png(\"p.png\")\nmain { picture.tile_count }";
    let found = errors_with_spans(source);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "only `tiles`, `nametable`, `attributes` and `palette` are supported yet"
    );
    assert_eq!(found[0].refusal, Refusal::AssetNotInThisRelease);
    assert_eq!(
        &source[found[0].span.start as usize..found[0].span.end as usize],
        "tile_count"
    );
}

/// The specification's explicit-palette form parses, so it is refused by name
/// rather than by token — one message, saying what is supported instead.
#[test]
fn refuses_the_explicit_palette_by_name() {
    let source =
        "asset image picture = png(\"p.png\") { palette: [ $0F, $30, $21, $11 ] }\nmain { }";
    let found = errors_with_spans(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "only `palette: auto(4)` is supported yet");
    assert_eq!(found[0].refusal, Refusal::AssetNotInThisRelease);
    assert_eq!(
        &source[found[0].span.start as usize..found[0].span.end as usize],
        "[ $0F, $30, $21, $11 ]"
    );
}

const RETURN_IN_AN_IRQ_HANDLER: &str = "an `irq` handler cannot `return`";

fn irq_frame(handler: &str) -> String {
    format!("main {{ }}\nframe bars using irq {{\n    at scanline 60 {{ {handler} }}\n}}\n")
}

#[test]
fn return_inside_an_irq_handler_gets_its_own_refusal_spanned_on_the_return() {
    let source = irq_frame("return");
    let diagnostics = errors_with_spans(&source);

    assert_eq!(diagnostics.len(), 1, "{:?}", diagnostics);
    let error = &diagnostics[0];
    assert_eq!(error.message, RETURN_IN_AN_IRQ_HANDLER);
    assert_eq!(
        error.label.as_deref(),
        Some("`return` jumps past the handler's `RTI`")
    );
    assert_eq!(
        error.notes,
        [
            "a handler ends by acknowledging the interrupt, arming the next\n\
             event and running `RTI`; a `return` skips all three, so no later\n\
             event in the schedule runs",
            "let the handler run to its last statement instead",
        ]
    );
    assert_eq!(
        &source[error.span.start as usize..error.span.end as usize],
        "return"
    );
}

#[test]
fn every_return_in_an_irq_handler_is_refused_whatever_it_returns() {
    let diagnostics = errors_with_spans(&irq_frame("return 1\n return\n return"));
    assert_eq!(diagnostics.len(), 3, "{:?}", diagnostics);
    assert!(
        diagnostics
            .iter()
            .all(|error| error.message == RETURN_IN_AN_IRQ_HANDLER)
    );
}

/// A `return`'s value makes no difference to the refusal, so it is not also judged against a
/// return type the handler does not have (one error per `return`) - but it is still read, so an
/// unknown name in it is still reported, and the span is the whole statement, value included.
#[test]
fn a_return_value_in_an_irq_handler_is_read_but_not_typed_against_the_handler() {
    let source = irq_frame("return missing");
    let diagnostics = errors_with_spans(&source);

    assert_eq!(diagnostics.len(), 2, "{:?}", diagnostics);
    assert_eq!(diagnostics[0].message, RETURN_IN_AN_IRQ_HANDLER);
    assert_eq!(
        &source[diagnostics[0].span.start as usize..diagnostics[0].span.end as usize],
        "return missing"
    );
    assert!(
        diagnostics[1].message.contains("missing"),
        "{:?}",
        diagnostics[1]
    );
}

#[test]
fn a_cycles_block_inside_an_irq_handler_keeps_the_timed_block_sentence() {
    let diagnostics = errors_with_spans(&irq_frame("cycles(20) pad { return }"));
    assert_eq!(diagnostics.len(), 1, "{:?}", diagnostics);
    assert_eq!(diagnostics[0].message, RETURN_IN_A_TIMED_BLOCK);
    assert!(diagnostics[0].label.is_none());
}

#[test]
fn return_in_a_timed_frame_handler_keeps_the_timed_block_sentence() {
    let diagnostics =
        errors_with_spans("main { }\nframe bars using timed {\n    at scanline 60 { return }\n}\n");
    assert_eq!(diagnostics.len(), 1, "{:?}", diagnostics);
    assert_eq!(diagnostics[0].message, RETURN_IN_A_TIMED_BLOCK);
}

#[test]
fn a_call_stays_refused_in_an_irq_handler() {
    let source =
        "fn helper() { }\nmain { }\nframe bars using irq {\n    at scanline 60 { helper() }\n}\n";
    let diagnostics = errors_with_spans(source);
    assert_eq!(diagnostics.len(), 1, "{:?}", diagnostics);
    assert!(
        diagnostics[0]
            .message
            .starts_with("`helper` cannot be called inside a timed block yet"),
        "{:?}",
        diagnostics[0]
    );
    assert_eq!(diagnostics[0].refusal, Refusal::TimedRegionCost);
}

#[test]
fn if_and_while_stay_refused_in_an_irq_handler() {
    for (statement, expected) in [
        (
            "if true { }",
            "branch arms inside a timed block cannot yet be balanced, because each path through them costs a different number of cycles",
        ),
        (
            "while true { }",
            "a `while` loop's trip count cannot be proven inside a timed block",
        ),
    ] {
        let diagnostics = errors_with_spans(&irq_frame(statement));
        assert_eq!(diagnostics.len(), 1, "{statement}: {:?}", diagnostics);
        assert_eq!(diagnostics[0].message, expected);
        assert_eq!(diagnostics[0].refusal, Refusal::TimedRegionCost);
    }
}

/// One failing register access is one error, with the carets under the whole
/// access and a label that names the spelling that works. Before raster-3o3 each
/// of these produced a cascade, two of whose errors were byte for byte equal.
#[test]
fn a_renamed_register_gets_one_error_naming_its_spelling() {
    for (source, port, spelling) in [
        ("main { oam.addr = 0 }", "$2003", "ppu.oam_addr"),
        ("main { oam.data = 0 }", "$2004", "ppu.oam_data"),
        ("main { oam.dma = $02 }", "$4014", "ppu.oam_dma"),
    ] {
        let found = errors_with_spans(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert_eq!(found[0].message, "there is no `oam` namespace");
        assert_eq!(
            found[0].label.as_deref(),
            Some(format!("{port} is spelled `{spelling}`").as_str())
        );
        assert_eq!(found[0].refusal, Refusal::Rejected, "{source}");
        assert!(found[0].notes.is_empty());
        let start = source.find("oam").unwrap();
        assert_eq!(
            &source[found[0].span.start as usize..found[0].span.end as usize],
            &source[start..source.find(" =").unwrap()]
        );
    }
}

#[test]
fn another_oam_member_gets_the_general_namespace_error() {
    let found = errors_with_spans("main { oam.foo = 0 }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "there is no `oam` namespace");
    assert_eq!(
        found[0].label.as_deref(),
        Some("rasterc has `ppu`, `mmc3` and `apu`")
    );
}

#[test]
fn a_variable_called_oam_is_not_told_there_is_no_namespace() {
    let found = errors_with_spans("main { var oam: u8 = 1\n oam.addr = 0 }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "member access requires a register namespace"
    );
    assert_eq!(
        found[0].label.as_deref(),
        Some("`oam` is a variable, not a register namespace")
    );
}

const APU_NAMES: [&str; 16] = [
    "apu.pulse1.volume",
    "apu.pulse1.sweep",
    "apu.pulse1.timer_lo",
    "apu.pulse1.timer_hi",
    "apu.pulse2.volume",
    "apu.pulse2.sweep",
    "apu.pulse2.timer_lo",
    "apu.pulse2.timer_hi",
    "apu.triangle.linear",
    "apu.triangle.timer_lo",
    "apu.triangle.timer_hi",
    "apu.noise.volume",
    "apu.noise.period",
    "apu.noise.length",
    "apu.status",
    "apu.frame_counter",
];

#[test]
fn every_apu_register_name_analyzes() {
    for name in APU_NAMES {
        let program = parse(&format!("main {{ {name} = $01 }}")).expect("fixture should parse");
        assert!(analyze(&program).is_ok(), "{name} should analyze");
    }
}

/// One error, its words, the label under the caret, and the caret covering the
/// whole register name.
fn assert_one_apu_error(source: &str, register: &str, message: &str, label: &str) {
    let found = errors_with_spans(source);
    assert_eq!(found.len(), 1, "{source}: {found:?}");
    assert_eq!(found[0].message, message, "{source}");
    assert_eq!(found[0].label.as_deref(), Some(label), "{source}");
    let start = source
        .find(register)
        .expect("the register is in the source") as u32;
    assert_eq!(
        (found[0].span.start, found[0].span.end),
        (start, start + register.len() as u32),
        "{source}: the carets cover the whole name"
    );
}

#[test]
fn an_unknown_apu_channel_is_one_error() {
    assert_one_apu_error(
        "main { apu.pulse3.volume = $BF }",
        "apu.pulse3.volume",
        "unknown apu channel `pulse3`",
        "the APU has pulse1, pulse2, triangle and noise",
    );
}

#[test]
fn an_unknown_register_in_a_channel_lists_that_channels_registers() {
    for (channel, list) in [
        ("pulse1", "volume, sweep, timer_lo and timer_hi"),
        ("pulse2", "volume, sweep, timer_lo and timer_hi"),
        ("triangle", "linear, timer_lo and timer_hi"),
        ("noise", "volume, period and length"),
    ] {
        let register = format!("apu.{channel}.volum");
        assert_one_apu_error(
            &format!("main {{ {register} = $BF }}"),
            &register,
            &format!("unknown apu.{channel} register `volum`"),
            &format!("{channel} has {list}"),
        );
    }
}

#[test]
fn a_channel_used_as_a_register_is_one_error() {
    assert_one_apu_error(
        "main { apu.pulse1 = $BF }",
        "apu.pulse1",
        "`apu.pulse1` is a channel, not a register",
        "write one of its registers, such as `apu.pulse1.volume`",
    );
}

#[test]
fn a_channel_register_named_directly_under_apu_points_at_its_channel() {
    for (name, example) in [
        ("volume", "apu.pulse1.volume"),
        ("sweep", "apu.pulse1.sweep"),
        ("timer_lo", "apu.pulse1.timer_lo"),
        ("timer_hi", "apu.pulse1.timer_hi"),
        ("linear", "apu.triangle.linear"),
        ("period", "apu.noise.period"),
        ("length", "apu.noise.length"),
    ] {
        let register = format!("apu.{name}");
        assert_one_apu_error(
            &format!("main {{ {register} = $BF }}"),
            &register,
            &format!("unknown apu register `{name}`"),
            &format!("`{name}` belongs to a channel, such as `{example}`"),
        );
    }
}

#[test]
fn a_name_that_belongs_to_nothing_under_apu_lists_what_apu_has() {
    assert_one_apu_error(
        "main { apu.foo = $BF }",
        "apu.foo",
        "unknown apu register `foo`",
        "apu has the registers status and frame_counter, and the channels pulse1, pulse2, triangle and noise",
    );
}

#[test]
fn the_dmc_registers_are_not_supported_yet_at_any_depth() {
    for register in ["apu.dmc.level", "apu.dmc.freq", "apu.dmc"] {
        let source = format!("main {{ {register} = 64 }}");
        assert_one_apu_error(
            &source,
            register,
            "the DMC registers are not supported yet",
            "how sample playback's cycle theft meets timed blocks is still open (§15.2)",
        );
        assert_eq!(
            errors_with_spans(&source)[0].refusal,
            Refusal::NotInThisRelease
        );
    }
}

#[test]
fn a_register_has_no_members_under_apu_either() {
    let found = errors_with_spans("main { apu.pulse1.volume.x = 1 }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "member access requires a register namespace"
    );
}

#[test]
fn apu_errors_are_the_same_in_a_read() {
    assert_one_apu_error(
        "main { var v: u8 = apu.pulse3.volume }",
        "apu.pulse3.volume",
        "unknown apu channel `pulse3`",
        "the APU has pulse1, pulse2, triangle and noise",
    );
}

#[test]
fn a_deeper_chain_on_a_real_namespace_gets_one_error() {
    for source in ["main { ppu.oam.addr = 0 }", "main { ppu.ctrl.x = 0 }"] {
        let found = errors_with_spans(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
    }
}

#[test]
fn an_unknown_namespace_in_a_read_gets_one_error() {
    let found = errors_with_spans("main { var v: u8 = foo.bar }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "there is no `foo` namespace");
}

/// An asset's member is a block of ROM data, so it is no assignment target. The
/// register-member guard must not wave it through.
#[test]
fn an_asset_member_is_not_an_assignment_target() {
    for statement in ["picture.tiles = 1", "picture.tiles += 1"] {
        let found = errors_with_spans(&format!(
            "asset image picture = png(\"p.png\")\nmain {{ {statement} }}"
        ));
        assert!(
            found.iter().any(|e| e.message
                == "assignment target must be a mutable variable, array element, or register member"),
            "{statement}: {found:?}"
        );
    }
}

#[test]
fn a_member_of_a_variable_is_one_error() {
    let found = errors_with_spans("main { var v: u8 = 1\n v.y = 1 }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "member access requires a register namespace"
    );
}

#[test]
fn a_member_of_a_call_still_reports_the_unknown_name_inside_it() {
    let messages = errors("main { var v: u8 = 1\n v = nope().x }");
    assert!(
        messages.iter().any(|m| m == "unknown name `nope`"),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m == "member access requires a register namespace")
    );
}

#[test]
fn an_unknown_ppu_member_keeps_its_message() {
    let found = errors_with_spans("main { ppu.crtl = 1 }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "unknown ppu register `crtl`");
}

// ---- vram_addr and the raw $2006 port (raster-agi) ----

#[test]
fn vram_addr_resolves_and_takes_a_whole_address() {
    let program = parse("main {\n    vram_addr = $3F00\n}\n").expect("fixture should parse");
    analyze(&program).expect("the virtual register is a known name");
}

#[test]
fn a_16_bit_number_on_the_raw_port_is_the_one_byte_at_a_time_error() {
    let found = errors_with_spans("main {\n    ppu.addr = $3F00\n}\n");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "ppu.addr is written one byte at a time");
    assert_eq!(
        found[0].label.as_deref(),
        Some("$3F00 does not fit in a byte")
    );
    assert_eq!(
        found[0].notes,
        ["help: to set the whole address in one line, write `vram_addr = $3F00`"]
    );
    // `    ppu.addr = ` is eleven characters after the line's first byte.
    assert_eq!((found[0].span.start, found[0].span.end), (22, 27));
}

#[test]
fn the_raw_port_error_quotes_the_number_as_written() {
    let found = errors_with_spans("main {\n    ppu.addr = 16128\n}\n");
    assert_eq!(
        found[0].label.as_deref(),
        Some("16128 does not fit in a byte")
    );
    assert_eq!(
        found[0].notes,
        ["help: to set the whole address in one line, write `vram_addr = 16128`"]
    );
}

#[test]
fn another_register_keeps_the_generic_overflow_error() {
    let found = errors("main {\n    ppu.mask = $3F00\n}\n");
    assert_eq!(found, ["integer literal overflows u8"]);
}

#[test]
fn a_byte_on_the_raw_port_still_analyzes() {
    let program = parse("main {\n    ppu.addr = $3F\n    ppu.addr = $00\n}\n").expect("parses");
    analyze(&program).expect("the raw pair is unchanged");
}

#[test]
fn vram_addr_in_a_timed_region_requires_sync_exact_like_any_ppu_write() {
    let found = errors("main {\n    cycles(100) pad {\n        vram_addr = $3F00\n    }\n}\n");
    assert!(
        found
            .iter()
            .any(|message| message.contains("`sync exact` is required")),
        "got {found:?}"
    );
}

#[test]
fn declaring_vram_addr_at_the_top_level_says_it_is_built_in() {
    let found = errors("var vram_addr: u8 = 1\nmain {\n}\n");
    assert_eq!(
        found,
        ["`vram_addr` is a built-in virtual register and cannot be declared"]
    );
}

#[test]
fn a_local_named_vram_addr_in_a_timed_region_needs_no_sync_exact() {
    let program = parse(
        "main {\n    var vram_addr: u8 = 1\n    cycles(30) pad {\n        vram_addr = 2\n    }\n}\n",
    )
    .expect("parses");
    analyze(&program).expect("a variable is not a PPU write");
}

const PICTURE: &str = "asset image picture = png(\"art/picture.png\") { kind: background }\n";

/// The errors of `PICTURE` followed by `rest`, each with the text its span covers.
fn picture_errors(rest: &str) -> Vec<(String, String)> {
    let source = format!("{PICTURE}{rest}");
    let program = parse(&source).expect("fixture should parse");
    match analyze(&program) {
        Ok(_) => Vec::new(),
        Err(found) => found
            .into_iter()
            .map(|error| {
                (
                    error.message,
                    source[error.span.start as usize..error.span.end as usize].to_owned(),
                )
            })
            .collect(),
    }
}

fn pair(message: &str, text: &str) -> (String, String) {
    (message.to_owned(), text.to_owned())
}

#[test]
fn a_chrrom_bank_of_asset_tiles_analyzes_and_the_loads_are_calls() {
    assert_eq!(
        picture_errors(
            "chrrom bank 0 { picture.tiles }\n\
             main {\n load_palette(picture.palette)\n load_nametable(picture.nametable, picture.attributes)\n }"
        ),
        []
    );
}

#[test]
fn refuses_a_chrrom_bank_number_an_mmc3_cannot_address() {
    assert_eq!(
        picture_errors("chrrom bank 32 { picture.tiles }\nmain { }"),
        [pair("an MMC3 cartridge holds banks 0 to 31", "32")]
    );
    assert_eq!(
        picture_errors("chrrom bank 31 { picture.tiles }\nmain { }"),
        []
    );
    assert_eq!(
        picture_errors("chrrom bank $1F { picture.tiles }\nmain { }"),
        []
    );
}

#[test]
fn refuses_a_chrrom_bank_declared_twice() {
    let found = picture_errors("chrrom bank 1 { }\nchrrom bank 1 { }\nmain { }");
    assert_eq!(found, [pair("`chrrom bank 1` is declared twice", "1")]);
}

#[test]
fn refuses_anything_in_a_chrrom_bank_that_is_not_an_assets_tiles() {
    let message = "a `chrrom` bank holds tiles, such as `picture.tiles`";
    assert_eq!(
        picture_errors("chrrom bank 0 { picture.nametable }\nmain { }"),
        [pair(message, "picture.nametable")]
    );
    assert_eq!(
        picture_errors("chrrom bank 0 { 7 }\nmain { }"),
        [pair(message, "7")]
    );
    assert_eq!(
        picture_errors("chrrom bank 0 { ppu.ctrl }\nmain { }"),
        [pair(message, "ppu.ctrl")]
    );
    assert_eq!(
        picture_errors("chrrom bank 0 { missing.tiles }\nmain { }"),
        [pair("unknown name `missing`", "missing")]
    );
}

#[test]
fn refuses_one_assets_tiles_placed_twice() {
    assert_eq!(
        picture_errors(
            "chrrom bank 0 { picture.tiles }\nchrrom bank 1 { picture.tiles }\nmain { }"
        ),
        [pair(
            "`picture.tiles` is already placed in a `chrrom` bank",
            "picture.tiles"
        )]
    );
}

#[test]
fn the_load_calls_check_their_argument_count() {
    assert_eq!(
        picture_errors("main { load_palette() }"),
        [pair(
            "function `load_palette` expects 1 arguments but received 0",
            "load_palette"
        )]
    );
    assert_eq!(
        picture_errors("main { load_nametable(picture.nametable) }"),
        [pair(
            "function `load_nametable` expects 2 arguments but received 1",
            "load_nametable"
        )]
    );
}

#[test]
fn the_load_calls_are_refused_inside_a_timed_block() {
    let source = format!("{PICTURE}main {{ cycles(500) pad {{ load_palette(picture.palette) }} }}");
    let program = parse(&source).expect("fixture should parse");
    let found = analyze(&program).expect_err("a load is a loop");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0]
            .message
            .contains("`load_palette` cannot be called inside a timed block")
    );
    assert_eq!(found[0].refusal, Refusal::TimedRegionCost);
}

#[test]
fn a_function_of_the_same_name_shadows_the_builtin() {
    assert_eq!(
        picture_errors("fn load_palette(x: u8) { }\nmain { load_palette(1) }"),
        []
    );
}

/// §8.3's music fields are defined by the specification and not built: refused
/// as not yet built, never as invented.
#[test]
fn refuses_music_asset_fields_as_not_yet_built() {
    for field in ["driver: famistudio", "channels: 2a03"] {
        let source = format!("asset music song = fam(\"m.fms\") {{ {field} }}\nmain {{ }}");
        let found = errors_with_spans(&source);
        let ours: Vec<_> = found
            .iter()
            .filter(|e| e.message.contains("supported yet") && e.message.starts_with("only `kind`"))
            .collect();
        assert_eq!(ours.len(), 1, "for `{field}`, got {found:?}");
        assert_eq!(ours[0].refusal, Refusal::AssetNotInThisRelease);
        let name = field.split(':').next().unwrap();
        assert_eq!(
            &source[ours[0].span.start as usize..ours[0].span.end as usize],
            name
        );
        assert!(
            found.iter().all(|e| !e.message.contains("unknown")),
            "{found:?}"
        );
    }
}

#[test]
fn reports_other_music_asset_fields_as_unknown_music_fields() {
    for field in [
        "kind: background",
        "palette: auto(4)",
        "dedup: true",
        "compress: none",
        "tempo: 3",
    ] {
        let source = format!("asset music song = fam(\"m.fms\") {{ {field} }}\nmain {{ }}");
        let found = errors_with_spans(&source);
        let ours: Vec<_> = found
            .iter()
            .filter(|e| e.message.starts_with("unknown"))
            .collect();
        assert_eq!(ours.len(), 1, "for `{field}`, got {found:?}");
        assert_eq!(
            ours[0].message,
            "unknown music asset field; a music asset has `driver` and `channels`"
        );
        assert_eq!(ours[0].refusal, Refusal::Rejected);
    }
}

#[test]
fn spec_8_3_example_prints_four_errors_none_unknown() {
    let source = "asset music song = fam(\"music/demo.fms\") { driver: famistudio  channels: 2a03 }\nmain { }";
    let found = errors_with_spans(source);
    let messages: Vec<_> = found.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "only `asset image` is supported yet",
            "only `png` is supported yet",
            "only `kind`, `palette` and `dedup` are supported yet",
            "only `kind`, `palette` and `dedup` are supported yet",
        ]
    );
}

#[test]
fn music_field_on_image_asset_and_sound_asset_keep_todays_messages() {
    let source = "asset image p = png(\"p.png\") { driver: famistudio }\nmain { }";
    let found = errors_with_spans(source);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "unknown asset field; this release knows `kind`, `palette` and `dedup`"
    );

    let source = "asset sound s = png(\"p.png\") { driver: x }\nmain { }";
    let messages: Vec<_> = errors_with_spans(source)
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert_eq!(
        messages,
        [
            "only `asset image` is supported yet",
            "unknown asset field; this release knows `kind`, `palette` and `dedup`",
        ]
    );
}

#[test]
fn music_asset_field_set_twice_and_no_fields() {
    let found = errors_with_spans("asset music s = fam(\"m\") { driver: a  driver: b }\nmain { }");
    let messages: Vec<_> = found.iter().map(|e| e.message.as_str()).skip(2).collect();
    assert_eq!(
        messages,
        [
            "only `kind`, `palette` and `dedup` are supported yet",
            "this asset field is set twice",
        ]
    );
    let found = errors_with_spans("asset music s = fam(\"m\")\nmain { }");
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn a_constant_array_keeps_its_bytes_for_lowering() {
    let program = parse(
        "const BASE: u8 = $10\nconst FADE: [4]u8 = [$01, BASE + 1, $21, 255]\nmain { ppu.mask = FADE[2] }",
    )
    .expect("fixture should parse");
    let typed = analyze(&program).expect("a constant array analyzes");
    assert_eq!(typed.tables["FADE"], [0x01, 0x11, 0x21, 0xff]);
}

#[test]
fn a_constant_array_may_hold_256_elements_and_no_more() {
    let elements = vec!["0"; 256].join(", ");
    let program = parse(&format!("const T: [256]u8 = [{elements}]\nmain {{ }}")).unwrap();
    assert_eq!(analyze(&program).expect("256 elements").tables["T"].len(), 256);

    let elements = vec!["0"; 257].join(", ");
    let diagnostics = errors(&format!("const T: [257]u8 = [{elements}]\nmain {{ }}"));
    assert_eq!(
        diagnostics,
        ["a constant array holds at most 256 elements, as many as a u8 index reaches"]
    );
}

#[test]
fn a_constant_array_literal_must_match_its_declared_length() {
    assert_eq!(
        errors("const T: [3]u8 = [1, 2]"),
        ["the array literal has 2 elements and `[3]u8` needs 3"]
    );
}

#[test]
fn a_constant_array_element_must_be_a_u8_constant() {
    assert_eq!(
        errors("var v: u8\nconst T: [2]u8 = [v, 256]"),
        [
            "array element must be a compile-time constant",
            "array element overflows u8",
        ]
    );
}

#[test]
fn a_constant_array_holds_u8_elements_only() {
    assert_eq!(
        errors("const T: [2]u16 = [1, 2]"),
        ["a constant array holds `u8` elements; declare it `[2]u8`"]
    );
}

#[test]
fn a_constant_array_needs_an_array_type_and_an_array_literal() {
    assert_eq!(
        errors("const T = [1, 2]"),
        ["a constant array needs its type written, as in `const T: [2]u8 = [...]`"]
    );
    assert_eq!(
        errors("const T: [2]u8 = 3"),
        ["a constant array is initialized with an array literal, `[...]`"]
    );
}

#[test]
fn an_array_literal_is_only_a_constant_s_initializer() {
    assert_eq!(
        errors("main { ppu.mask = [1, 2] }"),
        ["an array literal is only supported as a top-level `const` initializer"]
    );
    assert_eq!(
        errors("main {\n    const T: [2]u8 = [1, 2]\n}"),
        ["an array literal is only supported as a top-level `const` initializer"]
    );
}

#[test]
fn a_constant_array_element_cannot_be_assigned() {
    assert_eq!(
        errors("const T: [2]u8 = [1, 2]\nmain { T[0] = 3 }"),
        ["constants are not assignable"]
    );
}
