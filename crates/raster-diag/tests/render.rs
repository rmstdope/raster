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
