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

/// Why the compiler refused a program, in a form the compiler itself can test.
///
/// The wording of a refusal is free to change; this is not. `rasterc` chooses
/// which note to attach by asking this, so a reworded message cannot silently
/// stop explaining itself — which is exactly what a `message.ends_with(...)`
/// test allowed, twice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The language specification defines this construct and this release does
    /// not compile it *anywhere*. These carry the list of what the release can
    /// build instead.
    NotInThisRelease,
    /// The construct is fine, and a timed region cannot charge its cost,
    /// because the region is costed as straight-line code. Only the cost-model
    /// gaps: a hardware wait has no cost to measure ever, and a placement rule
    /// is a rule, so both of those are `Rejected` even though their messages
    /// mention a timed region.
    TimedRegionCost,
    /// A mistake in the program, or something Raster does not intend to do.
    /// Carries no note: the message already says what to do instead, and a
    /// list of supported constructs beside it would be noise.
    Rejected,
}

/// Whether a diagnostic fails the build.
///
/// An `Error` refuses the program; a `Warning` names something that has stopped
/// being true and lets the build finish. This is not `Refusal`: a `Refusal` says
/// *why* the compiler refused, so every `Refusal` belongs to an error and a
/// warning carries none.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    /// The word `render` prints before the message, and the word `report`
    /// counts by.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Where in the source this is about, when there is such a place. Some
    /// failures — a program with no `main`, a program too large for its bank —
    /// are about the whole compilation rather than any one span, and render
    /// without a location line.
    pub span: Option<Span>,
    pub label: String,
    pub notes: Vec<String>,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// A span `render` can always draw: inside `source`, ordered, and on
    /// character boundaries. Compiler spans arrive from several crates and one of
    /// them is a default span for a declaration with no name, so a caller must be
    /// able to make a span safe without repeating the reasoning. A panic here
    /// would show the author a backtrace instead of their mistake.
    pub fn clamped(start: usize, end: usize, source: &str) -> Self {
        let end = end.min(source.len());
        let start = start.min(end);
        Self {
            start: floor_char_boundary(source, start),
            end: floor_char_boundary(source, end),
        }
    }
}

fn floor_char_boundary(source: &str, mut offset: usize) -> usize {
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
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
            severity: Severity::Error,
            message: message.into(),
            span: Some(span),
            label: label.into(),
            notes: Vec::new(),
        }
    }

    /// A diagnostic that does not fail the build.
    ///
    /// There is no `warning_without_span`: no warning the compiler emits today
    /// lacks a span, and a constructor with no caller is a constructor nobody
    /// has thought about.
    pub fn warning(message: impl Into<String>, span: Span, label: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span: Some(span),
            label: label.into(),
            notes: Vec::new(),
        }
    }

    /// A diagnostic about the compilation rather than about a place in the
    /// source.
    ///
    /// The label mirrors the message, as it does for every diagnostic this
    /// compiler builds. `render` does not draw a label without a span, but a
    /// field that disagrees with the message would be a trap for whatever reads
    /// it next.
    pub fn without_span(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            severity: Severity::Error,
            label: message.clone(),
            message,
            span: None,
            notes: Vec::new(),
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

const NOTE_MARKER: &str = " = note: ";

pub fn render(source: &SourceFile, diagnostic: &Diagnostic) -> String {
    let Some(Span { start, end }) = diagnostic.span else {
        let mut rendered = format!("{}: {}\n", diagnostic.severity.prefix(), diagnostic.message);
        for note in &diagnostic.notes {
            rendered.push_str(&render_note(note, " "));
        }
        return rendered;
    };
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

    let gutter = " ".repeat(line_number.to_string().len());
    let mut rendered = format!(
        "{}: {}\n{gutter}--> {}:{}:{}\n{gutter} |\n{} | {}\n{gutter} | {}{} {}\n",
        diagnostic.severity.prefix(),
        diagnostic.message,
        source.name,
        line_number,
        column,
        line_number,
        line,
        " ".repeat(column - 1),
        "^".repeat(underline_width),
        diagnostic.label,
    );
    for note in &diagnostic.notes {
        rendered.push_str(&render_note(note, &gutter));
    }
    rendered
}

/// Renders one note, aligning its `=` under the gutter's `|` so a note reads the
/// same whatever the line number's width. Later physical lines line up under the
/// note's text rather than under `= note:`.
fn render_note(note: &str, gutter: &str) -> String {
    let mut rendered = String::new();
    for (index, line) in note.split('\n').enumerate() {
        if index == 0 {
            rendered.push_str(gutter);
            rendered.push_str(NOTE_MARKER);
        } else {
            rendered.push_str(&" ".repeat(gutter.len() + NOTE_MARKER.len()));
        }
        rendered.push_str(line);
        rendered.push('\n');
    }
    rendered
}
