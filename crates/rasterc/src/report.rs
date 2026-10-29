//! What `rasterc` prints: the build summary on success, the diagnostics and a
//! count on failure.

use raster_codegen::FrameBudget;
use raster_diag::{render, Diagnostic, Severity, SourceFile};
use raster_ir::FrameStrategy;
use raster_link::MMC3_FIXED_BANK_CODE_SIZE;

use crate::{chr::ChrBankSummary, compile::Rom};

/// Labels are right-aligned in nine columns and followed by two spaces, so every
/// value in the block begins in the same column.
fn row(label: &str, value: &str) -> String {
    format!("{label:>9}  {value}\n")
}

/// The per-event budget table of spec section 7.2: a heading naming the frame and the strategy it
/// was built with, then one row per event.
///
/// Every column is padded to its widest entry. The handler and window numbers keep a minimum width
/// (two and three digits) so the usual tables read the same whatever they hold; the verdict is the
/// last column and is never padded, so no row ends in a space.
pub fn frame_table(frame: &FrameBudget) -> String {
    let strategy = match frame.strategy {
        FrameStrategy::Irq => "irq",
        FrameStrategy::Timed => "timed",
    };
    let by_default = if frame.strategy_written {
        ""
    } else {
        " by default"
    };
    let mut table = format!(
        "frame {} (using {strategy}{by_default}, mmc3)\n",
        frame.name
    );

    // `scanline  60`: two spaces, so the scanline numbers of every row start in one column.
    let labels: Vec<String> = frame
        .rows
        .iter()
        .map(|row| row.label.replacen("scanline ", "scanline  ", 1))
        .collect();
    let widest = |numbers: &mut dyn Iterator<Item = u32>, minimum: usize| {
        numbers
            .map(|number| number.to_string().len())
            .fold(minimum, usize::max)
    };
    let label_width = labels.iter().map(String::len).max().unwrap_or(0);
    let handler_width = widest(&mut frame.rows.iter().map(|row| row.handler_cycles), 2);
    let window_width = widest(&mut frame.rows.iter().map(|row| row.window_cycles), 3);
    let spare_width = widest(
        &mut frame
            .rows
            .iter()
            .filter_map(|row| row.window_cycles.checked_sub(row.handler_cycles)),
        0,
    );

    for (row, label) in frame.rows.iter().zip(&labels) {
        let verdict = match row.window_cycles.checked_sub(row.handler_cycles) {
            Some(spare) => format!("{spare:>spare_width$} spare"),
            None => format!("OVER BY {}", row.handler_cycles - row.window_cycles),
        };
        table.push_str(&format!(
            "  {label:<label_width$} handler {handler:>handler_width$} cycles   window {window:>window_width$}   {verdict}\n",
            handler = row.handler_cycles,
            window = row.window_cycles,
        ));
    }
    table
}

/// The build summary. Paths are echoed as the author typed them, never resolved.
pub fn summary(input: &str, output: &str, rom: &Rom) -> String {
    [
        row(
            "Compiled",
            &match rom.warnings.len() {
                0 => format!("{input} -> {output}"),
                count => format!("{input} -> {output}  ({})", plural(count, "warning")),
            },
        ),
        row("mapper", "MMC3 (4), NTSC"),
        row("prg", "32 KiB, 4 banks of 8 KiB"),
        chr_rows(&rom.chr_banks),
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
        // After the whole summary block, behind one blank line; a program with no
        // frame prints nothing here, byte for byte what it always has.
        rom.frame
            .as_ref()
            .map(|frame| format!("\n{}", frame_table(frame)))
            .unwrap_or_default(),
    ]
    .concat()
}

/// The tile memory: today's one row for CHR RAM, or the cartridge's ROM total and a row per bank.
fn chr_rows(banks: &[ChrBankSummary]) -> String {
    if banks.is_empty() {
        return row("chr", "8 KiB RAM");
    }
    let mut rows = row(
        "chr",
        &format!(
            "{} KiB ROM, {} of 8 KiB",
            banks.len() * 8,
            plural(banks.len(), "bank")
        ),
    );
    for bank in banks {
        let label = format!("bank {}", bank.number);
        rows.push_str(&if bank.contents.is_empty() {
            row(&label, "empty")
        } else {
            row(
                &label,
                &format!(
                    "{}, {}, {} of 8192 bytes",
                    bank.contents.join(" + "),
                    plural(bank.tiles, "tile"),
                    bank.bytes_used
                ),
            )
        });
    }
    rows
}

/// Every diagnostic, separated by a blank line, then a count — so an
/// unsupported-construct-heavy file is one read rather than a run-fix-run
/// treadmill.
pub fn diagnostics(
    name: &str,
    source: &str,
    diagnostics: &[Diagnostic],
    frame: Option<&FrameBudget>,
) -> String {
    let mut rendered = diagnostics_only(name, source, diagnostics);
    // The budget table sits after the last diagnostic and before the count, which stays last.
    if let Some(frame) = frame {
        rendered.push_str(&frame_table(frame));
        rendered.push('\n');
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use raster_codegen::{EventBudget, FrameBudget};
    use raster_ir::FrameStrategy;

    fn budget(strategy: FrameStrategy, written: bool, rows: &[(&str, u32, u32)]) -> FrameBudget {
        FrameBudget {
            name: "bars".to_owned(),
            strategy,
            strategy_written: written,
            rows: rows
                .iter()
                .map(|&(label, handler_cycles, window_cycles)| EventBudget {
                    label: label.to_owned(),
                    handler_cycles,
                    window_cycles,
                })
                .collect(),
        }
    }

    #[test]
    fn an_irq_frame_that_fits_prints_its_spare_cycles() {
        let table = frame_table(&budget(
            FrameStrategy::Irq,
            true,
            &[("scanline 60", 6, 9), ("scanline 96-200 every 8", 6, 9)],
        ));

        assert_eq!(
            table,
            "frame bars (using irq, mmc3)\n\
             \x20 scanline  60             handler  6 cycles   window   9   3 spare\n\
             \x20 scanline  96-200 every 8 handler  6 cycles   window   9   3 spare\n"
        );
    }

    #[test]
    fn an_omitted_using_says_timed_by_default() {
        let table = frame_table(&budget(
            FrameStrategy::Timed,
            false,
            &[("scanline 60", 15, 114)],
        ));

        assert_eq!(
            table,
            "frame bars (using timed by default, mmc3)\n\
             \x20 scanline  60 handler 15 cycles   window 114   99 spare\n"
        );
    }

    #[test]
    fn a_handler_that_exactly_fills_its_window_has_zero_spare() {
        let table = frame_table(&budget(FrameStrategy::Irq, true, &[("scanline 60", 9, 9)]));

        assert!(table.ends_with("window   9   0 spare\n"), "{table}");
    }

    #[test]
    fn a_handler_over_its_window_says_by_how_much_in_words() {
        let table = frame_table(&budget(
            FrameStrategy::Irq,
            true,
            &[("scanline 60", 12, 9), ("scanline 96", 6, 9)],
        ));

        assert_eq!(
            table,
            "frame bars (using irq, mmc3)\n\
             \x20 scanline  60 handler 12 cycles   window   9   OVER BY 3\n\
             \x20 scanline  96 handler  6 cycles   window   9   3 spare\n"
        );
    }

    #[test]
    fn spare_numbers_line_up_and_no_row_ends_in_a_space() {
        let table = frame_table(&budget(
            FrameStrategy::Timed,
            true,
            &[("scanline 60", 105, 114), ("scanline 96", 15, 114)],
        ));

        assert!(table.contains("window 114    9 spare\n"), "{table}");
        assert!(table.contains("window 114   99 spare\n"), "{table}");
        assert!(table.lines().all(|line| !line.ends_with(' ')));
    }
}
