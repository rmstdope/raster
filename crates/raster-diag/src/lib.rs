#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFile {
    pub name: String,
    pub source: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
    pub label: String,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

impl SourceFile {
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
        }
    }
}

impl Diagnostic {
    pub fn error(message: impl Into<String>, span: Span, label: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span,
            label: label.into(),
        }
    }
}

pub fn render(source: &SourceFile, diagnostic: &Diagnostic) -> String {
    let Span { start, end } = diagnostic.span;
    assert!(
        start <= end,
        "diagnostic span start must not exceed its end"
    );
    assert!(
        end <= source.source.len(),
        "diagnostic span must be within the source"
    );
    assert!(
        source.source.is_char_boundary(start) && source.source.is_char_boundary(end),
        "diagnostic span offsets must be character boundaries"
    );

    let line_start = source.source[..start]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let line_number = source.source[..line_start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let line_end = source.source[start..]
        .find('\n')
        .map_or(source.source.len(), |offset| start + offset);
    let line = &source.source[line_start..line_end];
    let column = source.source[line_start..start].chars().count() + 1;
    let underline_end = end.min(line_end);
    let underline_width = source.source[start..underline_end].chars().count().max(1);

    format!(
        "error: {}\n --> {}:{}:{}\n  |\n{} | {}\n  | {}{} {}\n",
        diagnostic.message,
        source.name,
        line_number,
        column,
        line_number,
        line,
        " ".repeat(column - 1),
        "^".repeat(underline_width),
        diagnostic.label,
    )
}
