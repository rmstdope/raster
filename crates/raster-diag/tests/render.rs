use raster_diag::{render, Diagnostic, SourceFile, Span};

#[test]
fn renders_first_line_primary_span() {
    let source = SourceFile::new("fixture.raster", "@\n");
    let diagnostic = Diagnostic::error(
        "unexpected character `@`",
        Span::new(0, 1),
        "unexpected character `@`",
    );

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "error: unexpected character `@`\n",
            " --> fixture.raster:1:1\n",
            "  |\n",
            "1 | @\n",
            "  | ^ unexpected character `@`\n"
        )
    );
}

#[test]
fn renders_later_line_and_column() {
    let source = SourceFile::new("fixture.raster", "first\n  @ later\n");
    let diagnostic = Diagnostic::error("invalid", Span::new(8, 9), "here");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "error: invalid\n",
            " --> fixture.raster:2:3\n",
            "  |\n",
            "2 |   @ later\n",
            "  |   ^ here\n"
        )
    );
}

#[test]
fn renders_zero_width_span() {
    let source = SourceFile::new("fixture.raster", "abc\n");
    let diagnostic = Diagnostic::error("missing", Span::new(2, 2), "expected item");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "error: missing\n",
            " --> fixture.raster:1:3\n",
            "  |\n",
            "1 | abc\n",
            "  |   ^ expected item\n"
        )
    );
}

#[test]
fn renders_notes_under_the_caret() {
    let source = SourceFile::new("fixture.raster", "@\n");
    let diagnostic = Diagnostic::error("m", Span::new(0, 1), "l").with_note("first\nsecond");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "error: m\n",
            " --> fixture.raster:1:1\n",
            "  |\n",
            "1 | @\n",
            "  | ^ l\n",
            "  = note: first\n",
            "          second\n"
        )
    );
}

#[test]
fn widens_the_gutter_for_a_two_digit_line() {
    let source = SourceFile::new(
        "fixture.raster",
        format!("{}frame every 8 scanlines {{\n", "\n".repeat(11)),
    );
    let diagnostic = Diagnostic::error(
        "`frame` blocks are not supported yet",
        Span::new(11, 34),
        "`frame` blocks are not supported yet",
    );

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "error: `frame` blocks are not supported yet\n",
            "  --> fixture.raster:12:1\n",
            "   |\n",
            "12 | frame every 8 scanlines {\n",
            "   | ^^^^^^^^^^^^^^^^^^^^^^^ `frame` blocks are not supported yet\n"
        )
    );
}

#[test]
fn clamps_a_span_past_the_end_of_the_source() {
    let source = SourceFile::new("fixture.raster", "abc\n");
    let span = Span::clamped(10, 20, &source.source);

    assert_eq!(span, Span::new(4, 4));

    let diagnostic = Diagnostic::error("past the end", span, "past the end");
    assert!(render(&source, &diagnostic).starts_with("error: past the end\n"));
}

#[test]
fn clamps_a_span_to_character_boundaries() {
    let source = SourceFile::new("fixture.raster", "aé\n");
    let span = Span::clamped(2, 3, &source.source);

    assert_eq!(span, Span::new(1, 3));

    let diagnostic = Diagnostic::error("inside a character", span, "inside a character");
    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "error: inside a character\n",
            " --> fixture.raster:1:2\n",
            "  |\n",
            "1 | aé\n",
            "  |  ^ inside a character\n"
        )
    );
}

#[test]
fn renders_a_diagnostic_without_a_span() {
    let source = SourceFile::new("fixture.raster", "main {}\n");
    let diagnostic = Diagnostic::without_span("the program does not fit the MMC3 fixed bank")
        .with_note("8402 bytes of code, and $E000-$FFFF holds 8186")
        .with_note("PRG bank switching is not supported yet, so all code lives\nin the fixed bank");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "error: the program does not fit the MMC3 fixed bank\n",
            "  = note: 8402 bytes of code, and $E000-$FFFF holds 8186\n",
            "  = note: PRG bank switching is not supported yet, so all code lives\n",
            "          in the fixed bank\n"
        )
    );
}

#[test]
fn renders_a_warning_with_the_same_gutter_as_an_error() {
    let source = SourceFile::new(
        "fixture.raster",
        "main {\n    ppu.mask = 0\n    mmc3.bank_select = $80\n}\n",
    );
    let diagnostic = Diagnostic::warning(
        "this bank select changes the MMC3 mapping mode",
        Span::new(28, 50),
        "bit 7 swaps the two pattern tables from here on",
    );

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "warning: this bank select changes the MMC3 mapping mode\n",
            " --> fixture.raster:3:5\n",
            "  |\n",
            "3 |     mmc3.bank_select = $80\n",
            "  |     ^^^^^^^^^^^^^^^^^^^^^^ bit 7 swaps the two pattern tables from here on\n"
        )
    );
}

#[test]
fn renders_a_secondary_marker_on_the_line_above_the_primary() {
    let source = SourceFile::new("demo.raster", "main {\n    tick()\n    write()\n}\n");
    let diagnostic = Diagnostic::warning("careful", Span::new(22, 29), "primary")
        .with_secondary(Span::new(11, 17), "secondary");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "warning: careful\n",
            " --> demo.raster:3:5\n",
            "  |\n",
            "2 |     tick()\n",
            "  |     ------ secondary\n",
            "3 |     write()\n",
            "  |     ^^^^^^^ primary\n"
        )
    );
}

#[test]
fn shows_the_lines_between_a_secondary_and_the_primary_when_there_are_two_or_fewer() {
    let source = SourceFile::new("demo.raster", "a()\nb\nc\nw()\n");
    let diagnostic = Diagnostic::warning("careful", Span::new(8, 11), "primary")
        .with_secondary(Span::new(0, 3), "secondary");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "warning: careful\n",
            " --> demo.raster:4:1\n",
            "  |\n",
            "1 | a()\n",
            "  | --- secondary\n",
            "2 | b\n",
            "3 | c\n",
            "4 | w()\n",
            "  | ^^^ primary\n"
        )
    );
}

#[test]
fn elides_the_lines_between_a_distant_secondary_and_the_primary() {
    let source = SourceFile::new("demo.raster", "a()\nb\nc\nd\nw()\n");
    let diagnostic = Diagnostic::warning("careful", Span::new(10, 13), "primary")
        .with_secondary(Span::new(0, 3), "secondary");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "warning: careful\n",
            " --> demo.raster:5:1\n",
            "  |\n",
            "1 | a()\n",
            "  | --- secondary\n",
            "...\n",
            "5 | w()\n",
            "  | ^^^ primary\n"
        )
    );
}

#[test]
fn widens_the_gutter_to_the_longest_line_number_when_a_secondary_is_drawn() {
    let mut text = String::new();
    for _ in 0..8 {
        text.push_str("x\n");
    }
    let call = text.len();
    text.push_str("a()\nb\nw()\n");
    let write = call + "a()\nb\n".len();
    let source = SourceFile::new("demo.raster", text);
    let diagnostic = Diagnostic::warning("careful", Span::new(write, write + 3), "primary")
        .with_secondary(Span::new(call, call + 3), "secondary");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "warning: careful\n",
            "  --> demo.raster:11:1\n",
            "   |\n",
            " 9 | a()\n",
            "   | --- secondary\n",
            "10 | b\n",
            "11 | w()\n",
            "   | ^^^ primary\n"
        )
    );
}

#[test]
fn draws_a_secondary_on_the_primarys_own_line_instead_of_dropping_it() {
    let source = SourceFile::new("demo.raster", "if pick() { w() }\n");
    let diagnostic = Diagnostic::warning("careful", Span::new(12, 15), "primary")
        .with_secondary(Span::new(3, 9), "secondary");

    assert_eq!(
        render(&source, &diagnostic),
        concat!(
            "warning: careful\n",
            " --> demo.raster:1:13\n",
            "  |\n",
            "1 | if pick() { w() }\n",
            "  |    ------ secondary\n",
            "  |             ^^^ primary\n"
        )
    );
}

#[test]
fn prints_crlf_sources_without_the_carriage_return() {
    let source = SourceFile::new("demo.raster", "a()\r\nb\r\nw()\r\n");
    let diagnostic = Diagnostic::warning("careful", Span::new(8, 11), "primary")
        .with_secondary(Span::new(0, 3), "secondary");

    assert!(!render(&source, &diagnostic).contains('\r'));
}
