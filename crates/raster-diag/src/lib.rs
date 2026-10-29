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
    /// An `asset` construct the language defines and this release does not
    /// build. Carries the note about what this release reads for assets, not
    /// the general list of constructs.
    AssetNotInThisRelease,
    /// A correct `asset` that the release reads and does not yet place in the
    /// ROM. Carries no note: the line the author wrote is already the one the
    /// release allows, so a list of what is allowed would only describe it back.
    AssetNotPlaced,
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
    /// One line of advice, rendered last after a bare gutter line as `= help: ...`.
    pub help: Option<String>,
    /// A second place the primary one is about, drawn above it with a dashed
    /// underline so it reads without colour. Only drawn when it lies on an
    /// earlier line than the primary span.
    pub secondary: Option<(Span, String)>,
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
            help: None,
            secondary: None,
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
            help: None,
            secondary: None,
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
            help: None,
            secondary: None,
        }
    }

    /// Marks a second span above the primary one, with its own label.
    pub fn with_secondary(mut self, span: Span, label: impl Into<String>) -> Self {
        self.secondary = Some((span, label.into()));
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

const NOTE_MARKER: &str = " = note: ";
const HELP_MARKER: &str = " = help: ";

/// A note that begins with this is rendered as `= help:` rather than `= note:`:
/// advice on what to write instead, as opposed to a fact about the refusal.
pub const HELP_PREFIX: &str = "help: ";

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

    let primary = locate(&source.source, start, end);
    let secondary = diagnostic
        .secondary
        .as_ref()
        .filter(|(span, _)| {
            span.start <= span.end
                && span.end <= source.source.len()
                && source.source.is_char_boundary(span.start)
                && source.source.is_char_boundary(span.end)
        })
        .map(|(span, label)| (locate(&source.source, span.start, span.end), label))
        .filter(|(place, _)| place.number <= primary.number);

    let gutter = " ".repeat(primary.number.to_string().len());
    let mut rendered = format!(
        "{}: {}\n{gutter}--> {}:{}:{}\n{gutter} |\n",
        diagnostic.severity.prefix(),
        diagnostic.message,
        source.name,
        primary.number,
        primary.column,
    );
    let width = gutter.len();
    if let Some((place, label)) = secondary {
        rendered.push_str(&source_line(place.number, place.text, width));
        rendered.push_str(&marker_line(&place, '-', label, width));
        let between = (primary.number - place.number).saturating_sub(1);
        if place.number == primary.number {
            // Same line: it was printed once above, and the primary marker
            // goes under the secondary's.
            rendered.push_str(&marker_line(&primary, '^', &diagnostic.label, width));
            push_notes_and_help(&mut rendered, diagnostic, &gutter);
            return rendered;
        } else if between <= MAX_SHOWN_BETWEEN {
            for number in place.number + 1..primary.number {
                let text = source.source.lines().nth(number - 1).unwrap_or("");
                rendered.push_str(&source_line(number, text, width));
            }
        } else {
            rendered.push_str("...\n");
        }
    }
    rendered.push_str(&source_line(primary.number, primary.text, width));
    rendered.push_str(&marker_line(&primary, '^', &diagnostic.label, width));
    push_notes_and_help(&mut rendered, diagnostic, &gutter);
    rendered
}

fn push_notes_and_help(rendered: &mut String, diagnostic: &Diagnostic, gutter: &str) {
    for note in &diagnostic.notes {
        rendered.push_str(&render_note(note, gutter));
    }
    if let Some(help) = &diagnostic.help {
        rendered.push_str(&format!("{gutter} |\n"));
        rendered.push_str(&render_note(&format!("{HELP_PREFIX}{help}"), gutter));
    }
}

/// Renders one note, aligning its `=` under the gutter's `|` so a note reads the
/// same whatever the line number's width. Later physical lines line up under the
/// note's text rather than under `= note:`.
fn render_note(note: &str, gutter: &str) -> String {
    let (marker, note) = match note.strip_prefix(HELP_PREFIX) {
        Some(advice) => (HELP_MARKER, advice),
        None => (NOTE_MARKER, note),
    };
    let mut rendered = String::new();
    for (index, line) in note.split('\n').enumerate() {
        if index == 0 {
            rendered.push_str(gutter);
            rendered.push_str(marker);
        } else {
            rendered.push_str(&" ".repeat(gutter.len() + marker.len()));
        }
        rendered.push_str(line);
        rendered.push('\n');
    }
    rendered
}

/// How many lines between a secondary span and the primary one are shown; more
/// than this and a single `...` line stands for all of them.
const MAX_SHOWN_BETWEEN: usize = 2;

/// Where a span starts: the line it is on and how much of that line it marks.
struct Placed<'a> {
    number: usize,
    text: &'a str,
    column: usize,
    width: usize,
}

fn locate(source: &str, start: usize, end: usize) -> Placed<'_> {
    let line_start = source[..start].rfind('\n').map_or(0, |newline| newline + 1);
    let number = source[..line_start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |offset| start + offset);
    Placed {
        number,
        text: source[line_start..line_end].trim_end_matches('\r'),
        column: source[line_start..start].chars().count() + 1,
        width: source[start..end.min(line_end)].chars().count().max(1),
    }
}

fn source_line(number: usize, text: &str, gutter: usize) -> String {
    format!("{number:>gutter$} | {text}\n")
}

fn marker_line(place: &Placed<'_>, mark: char, label: &str, gutter: usize) -> String {
    format!(
        "{} | {}{} {label}\n",
        " ".repeat(gutter),
        " ".repeat(place.column - 1),
        mark.to_string().repeat(place.width),
    )
}
