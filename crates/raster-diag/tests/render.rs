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
