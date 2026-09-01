//! What `rasterc` prints: the build summary on success, the diagnostics and a
//! count on failure.

use raster_diag::{render, Diagnostic, Severity, SourceFile};
use raster_link::MMC3_FIXED_BANK_CODE_SIZE;

use crate::compile::Rom;

/// Labels are right-aligned in nine columns and followed by two spaces, so every
/// value in the block begins in the same column.
fn row(label: &str, value: &str) -> String {
    format!("{label:>9}  {value}\n")
}

/// The build summary. Paths are echoed as the author typed them, never resolved.
pub fn summary(input: &str, output: &str, rom: &Rom) -> String {
    [
        row("Compiled", &format!("{input} -> {output}")),
        row("mapper", "MMC3 (4), NTSC"),
        row("prg", "32 KiB, 4 banks of 8 KiB"),
        row("chr", "8 KiB RAM"),
        row(
            "fixed",
            &format!(
                "$E000-$FFFF, {} of {MMC3_FIXED_BANK_CODE_SIZE} bytes used",
                rom.code_len
            ),
        ),
        row(
            "yours",
            &format!("{} bytes", rom.code_len - rom.runtime_len),
        ),
        row(
            "runtime",
            &format!(
                "{} bytes, the reset sequence around your program",
                rom.runtime_len
            ),
        ),
        row(
            "entry",
            &format!(
                "reset ${:04X}, nmi ${:04X}, irq ${:04X}",
                rom.vectors.reset, rom.vectors.nmi, rom.vectors.irq
            ),
        ),
        // What every `cycles(?)` region measured, which is the whole point of
        // writing one: the author asked, so the answer belongs in the summary.
        rom.reports
            .iter()
            .map(|(label, measured)| row("cycles", &format!("{label}: {measured} cycles")))
            .collect(),
        // Absent at zero: a row every build carries to say nothing is a row
        // nobody reads.
        if rom.warnings.is_empty() {
            String::new()
        } else {
            row("warnings", &rom.warnings.len().to_string())
        },
    ]
    .concat()
}

/// Every diagnostic, separated by a blank line, then a count — so an
/// unsupported-construct-heavy file is one read rather than a run-fix-run
/// treadmill.
pub fn diagnostics(name: &str, source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut rendered = diagnostics_only(name, source, diagnostics);
    let errors = count(diagnostics, Severity::Error);
    let warnings = count(diagnostics, Severity::Warning);
    // Omitted at zero, so a build with no warnings prints exactly what it has
    // always printed.
    let counted = if warnings == 0 {
        plural(errors, "error")
    } else {
        format!(
            "{}, {}",
            plural(errors, "error"),
            plural(warnings, "warning")
        )
    };
    rendered.push_str(&format!("error: could not compile {name} ({counted})\n"));
    rendered
}

/// Every diagnostic, separated by a blank line, and nothing else — what a
/// successful build prints for the warnings it found.
pub fn diagnostics_only(name: &str, source: &str, diagnostics: &[Diagnostic]) -> String {
    let file = SourceFile::new(name, source);
    let mut rendered = String::new();
    for diagnostic in diagnostics {
        rendered.push_str(&render(&file, diagnostic));
        rendered.push('\n');
    }
    rendered
}

fn count(diagnostics: &[Diagnostic], severity: Severity) -> usize {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == severity)
        .count()
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}
