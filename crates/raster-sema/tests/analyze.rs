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

/// Every refusal a timed region raises, and the kind it must carry.
///
/// A `TimedRegionCost` refusal is a cost-model gap: the construct is fine, and a region costed as
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
fn every_timed_region_refusal_carries_the_kind_that_decides_its_note() {
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

/// The cascade raster-3o3 exists to remove. `oam.addr = 0` reported four errors
/// — ``unknown name `oam` ``, `member access requires a register namespace`,
/// ``unknown name `oam` `` a second time, and `assignment target must be ...` —
/// for one line, and not one of them named the register the author wanted.
#[test]
fn an_unknown_register_namespace_is_one_error_not_four() {
    let errors = errors_with_spans("main {\n    zzz.thing = 0\n}\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "there is no `zzz` namespace");
    assert_eq!(
        errors[0].label.as_deref(),
        Some("rasterc has `ppu` and `mmc3`")
    );
}

/// `apu.pulse1.volume` ran the cascade once per member level, for six errors.
#[test]
fn a_two_level_member_on_an_unknown_namespace_is_still_one_error() {
    let errors = errors_with_spans("main {\n    zzz.one.two = $BF\n}\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "there is no `zzz` namespace");
}

/// `ppu.oam.addr` produced four errors: the inner level twice and the outer
/// twice. Only the deepest wrong step is worth reporting.
#[test]
fn a_two_level_member_on_a_real_namespace_reports_the_inner_level_once() {
    let errors = errors_with_spans("main {\n    ppu.oam.addr = 0\n}\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "unknown ppu register `oam`");
}

/// A member on an ordinary variable produced two errors, the second of which
/// said nothing the first had not. The label names what `x` actually is.
#[test]
fn a_member_on_a_variable_names_what_the_variable_is() {
    let errors = errors_with_spans("main {\n    var x: u8 = 0\n    x.y = 1\n}\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "member access requires a register namespace"
    );
    assert_eq!(
        errors[0].label.as_deref(),
        Some("`x` is a variable, not a register namespace")
    );
}

/// A member on a real register: `ppu.ctrl.x`. The inner level is fine and the
/// outer one is not, and the message is the one this release always used.
#[test]
fn a_member_on_a_register_is_refused_with_the_message_it_always_had() {
    let errors = errors_with_spans("main {\n    ppu.ctrl.x = 1\n}\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "member access requires a register namespace"
    );
}

/// §9.1 named $2003 and $2004 `oam.addr` and `oam.data` until raster-3o3, and
/// an author who knows the hardware as OAMADDR types them anyway. rasterc has
/// both ports and says which name reaches them.
///
/// The span is the whole member expression, not the namespace: the label is a
/// substitution the author performs, and it does not read as one if the carets
/// cover `oam` alone. Agreed with the navigator; see the bead's plan.
#[test]
fn the_specifications_oam_names_say_which_spelling_reaches_the_port() {
    for (source, port, spelling) in [
        ("main {\n    oam.addr = 0\n}\n", "$2003", "ppu.oam_addr"),
        ("main {\n    oam.data = $18\n}\n", "$2004", "ppu.oam_data"),
    ] {
        let errors = errors_with_spans(source);
        assert_eq!(errors.len(), 1, "{source}");
        assert_eq!(errors[0].message, "there is no `oam` namespace");
        assert_eq!(
            errors[0].label.as_deref(),
            Some(format!("{port} is spelled `{spelling}`").as_str())
        );
        assert_eq!(errors[0].refusal, Refusal::Rejected);
        // The whole member expression, `oam.addr`, from column 5 of line 2.
        assert_eq!(errors[0].span.end - errors[0].span.start, 8);
    }
}

/// A read is the same expression somewhere else, and says the same thing.
#[test]
fn reading_a_renamed_register_says_the_same_thing_as_writing_one() {
    let errors = errors_with_spans("main {\n    var y: u8 = oam.addr\n}\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "there is no `oam` namespace");
    assert_eq!(
        errors[0].label.as_deref(),
        Some("$2003 is spelled `ppu.oam_addr`")
    );
}

/// §9.1's note names three things it once implied and rasterc cannot do. Two of
/// them are reachable by name, and both promise later — so both are
/// `NotInThisRelease` and carry the "this release compiles ..." note, which is
/// the rule `crates/rasterc/src/compile.rs` states: never leave "yet"
/// unexplained.
///
/// The OAM DMA message names the *feature*, not a spelling. §9.1 no longer
/// shows `oam.dma`, and nothing has chosen between `oam.dma` and
/// `ppu.oam_dma`; naming one here would settle that in a terminal message.
#[test]
fn hardware_the_specification_names_and_this_release_lacks_promises_later() {
    for (source, message, label) in [
        (
            "main {\n    oam.dma = $02\n}\n",
            "OAM DMA is not supported yet",
            "$4014 stalls the CPU for 513 or 514 cycles",
        ),
        (
            "main {\n    apu.pulse1.volume = $BF\n}\n",
            "the `apu` registers are not supported yet",
            "the sound driver owns the APU today (§8.3)",
        ),
    ] {
        let errors = errors_with_spans(source);
        assert_eq!(errors.len(), 1, "{source}");
        assert_eq!(errors[0].message, message);
        assert_eq!(errors[0].label.as_deref(), Some(label));
        assert_eq!(errors[0].refusal, Refusal::NotInThisRelease);
    }
}

/// `apu` is a whole namespace this release does not have, so every member of it
/// answers the same way — `NOT_YET`'s `None` member.
#[test]
fn every_apu_register_answers_the_same_way() {
    for source in [
        "main {\n    apu.pulse1.volume = $BF\n}\n",
        "main {\n    apu.status = 0\n}\n",
        "main {\n    apu.triangle.linear = 1\n}\n",
    ] {
        let errors = errors_with_spans(source);
        assert_eq!(errors.len(), 1, "{source}");
        assert_eq!(
            errors[0].message,
            "the `apu` registers are not supported yet"
        );
    }
}

/// The label a chain rooted at nothing carries is the namespace list. That the
/// list agrees with `NAMESPACES` is checked in `raster-sema`'s own unit tests,
/// where the private const is visible — an integration test can only compare
/// the label against a third copy of the list, which guards nothing. Review
/// finding 3 on PR #47.
#[test]
fn an_unrooted_namespace_is_labelled_with_the_namespace_list() {
    let errors = errors_with_spans("main {\n    zzz.thing = 0\n}\n");
    assert_eq!(
        errors[0].label.as_deref(),
        Some("rasterc has `ppu` and `mmc3`")
    );
}

/// §9.1 showed `ppu.status` on a line of its own, commented `// read`, since
/// draft 0.1, and this release has no such statement. Authors will keep writing
/// it, so the message names the form that does compile.
#[test]
fn a_bare_register_read_names_the_assignment_that_works() {
    let errors = errors_with_spans("main {\n    ppu.status\n}\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "a register read cannot stand on its own as a statement"
    );
    assert_eq!(
        errors[0].label.as_deref(),
        Some("assign it to a variable: `var s: u8 = ppu.status`")
    );
}

/// The label names the register that was written, not a fixed example.
#[test]
fn the_bare_read_label_names_the_register_that_was_read() {
    let errors = errors_with_spans("main {\n    mmc3.irq_disable\n}\n");
    assert_eq!(
        errors[0].label.as_deref(),
        Some("assign it to a variable: `var s: u8 = mmc3.irq_disable`")
    );
}

/// Assignments and calls are `Infix` and `Call`, not `Member`, so neither is
/// caught by the new check.
#[test]
fn a_register_write_and_a_call_are_still_statements() {
    let program = parse("fn f() -> void { }\nmain {\n    ppu.ctrl = $80\n    f()\n}\n")
        .expect("fixture should parse");
    analyze(&program).expect("a write and a call are valid statements");
}

/// Review finding 2 on PR #47. Collapsing the cascade must not swallow a real
/// error that happens to sit inside a member chain's base. `nope` is unknown
/// whatever the member access turns out to be, and an author told only about
/// the member access learns about `nope` on the next build instead of this one.
#[test]
fn an_error_inside_a_member_chains_base_is_still_reported() {
    let errors = errors_with_spans("main {\n    var t: [4]u8\n    t[nope].y = 1\n}\n");
    let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        vec![
            "unknown name `nope`",
            "member access requires a register namespace"
        ]
    );
}

/// And the base's own errors are reported once, not twice: the duplicate came
/// from `ensure_assignable` re-evaluating the base, which no longer happens.
#[test]
fn a_member_chains_base_is_evaluated_once() {
    let errors = errors_with_spans("main {\n    var t: [4]u8\n    t[nope].y = 1\n}\n");
    assert_eq!(
        errors
            .iter()
            .filter(|e| e.message == "unknown name `nope`")
            .count(),
        1
    );
}

/// Review finding 4 on PR #47. `oam` and `apu` are matched by name, so a local
/// variable of either name used to be told "there is no `oam` namespace" — of
/// a name the author had just declared. What a *bound* name is takes precedence
/// over what the specification once called it.
#[test]
fn a_variable_shadowing_a_specification_namespace_is_named_for_what_it_is() {
    for (source, root) in [
        ("main {\n    var oam: u8 = 0\n    oam.addr = 1\n}\n", "oam"),
        (
            "main {\n    var apu: u8 = 0\n    apu.status = 1\n}\n",
            "apu",
        ),
    ] {
        let errors = errors_with_spans(source);
        assert_eq!(errors.len(), 1, "{source}");
        assert_eq!(
            errors[0].message,
            "member access requires a register namespace"
        );
        assert_eq!(
            errors[0].label.as_deref(),
            Some(format!("`{root}` is a variable, not a register namespace").as_str())
        );
    }
}
