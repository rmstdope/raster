//! What `rasterc` prints: the build summary on success, the diagnostics and a
//! count on failure.

use raster_diag::{render, Diagnostic, SourceFile};
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
            "entry",
            &format!(
                "reset ${:04X}, nmi ${:04X}, irq ${:04X}",
                rom.vectors.reset, rom.vectors.nmi, rom.vectors.irq
            ),
        ),
    ]
    .concat()
}

/// Every diagnostic, separated by a blank line, then a count — so an
/// unsupported-construct-heavy file is one read rather than a run-fix-run
/// treadmill.
pub fn diagnostics(name: &str, source: &str, diagnostics: &[Diagnostic]) -> String {
    let file = SourceFile::new(name, source);
    let mut rendered = String::new();
    for diagnostic in diagnostics {
        rendered.push_str(&render(&file, diagnostic));
        rendered.push('\n');
    }
    let plural = if diagnostics.len() == 1 {
        "error"
    } else {
        "errors"
    };
    rendered.push_str(&format!(
        "error: could not compile {name} ({} {plural})\n",
        diagnostics.len()
    ));
    rendered
}
