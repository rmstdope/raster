//! The compiler pipeline, with no filesystem in it.
//!
//! Nothing here reads or writes a file, so the whole of what `rasterc` does to a
//! source is testable from a string.

use raster_codegen::{generate_with_isa, CodegenError};
use raster_diag::{Diagnostic, Refusal, Span};
use raster_ir::{lower, FrameStrategy};
use raster_link::{link_mmc3_program, mmc3_reset_runtime_bytes, InterruptVectors, LinkError};
use raster_sema::analyze;
use raster_syntax::parse;
use raster_timing::TimingError;

/// What this release of the compiler can build, listed once per run beside the
/// first construct it had to refuse for not being in it.
const SUPPORTED_SUBSET: &str = "this release compiles `main`, `fn`, `if`, `while`, `for`, u8\n\
                                arithmetic, and `ppu.*` / `mmc3.*` register writes; timed blocks\n\
                                with `cycles`, `pad`, `sync exact` and `wait cycles`; and one\n\
                                `frame` of `every ... scanlines` events";

/// Why a timed region refuses a construct that is fine everywhere else, said
/// once per run beside the first of them.
const TIMED_REGION_COST: &str = "a timed block is costed as straight-line code; loops, branches\n\
                                 and calls will be admitted once their cost can be measured";

/// Whether this release restricts itself to official opcodes.
///
/// Codegen and the linker must agree: padding synthesized from the undocumented
/// `NOP` forms is shorter, but the assembler refuses it under a legal ISA, and
/// the two disagreeing shows up as an internal compiler error rather than as a
/// choice anyone made. One constant, both call sites, until `--legal-isa`
/// becomes a flag and threads a value through instead.
const LEGAL_ISA: bool = true;

/// The zero page raster allocates from, `$10` through `$FF`.
const ZERO_PAGE_VARIABLES: usize = 0x100 - 0x10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rom {
    pub image: Vec<u8>,
    pub code_len: usize,
    /// How many of `code_len` the reset runtime contributed. `code_len` minus
    /// this is what the author's source compiled to, which is the number the
    /// summary leads with.
    pub runtime_len: usize,
    pub vectors: InterruptVectors,
    /// The measured cost of each `cycles(?)` region, which the summary prints.
    pub reports: Vec<(String, u32)>,
    /// The hazards the compiler saw and did not refuse. A ROM with warnings is
    /// still a ROM: these are printed, and counted, and the build succeeds.
    pub warnings: Vec<Diagnostic>,
}

/// Compile `source` into a ROM, or into every diagnostic the first failing stage
/// produced.
///
/// Stages run in order and the first one with errors is the last one to run, so
/// a file with both a parse error and an unsupported construct reports only the
/// parse errors: the later stages were never given a program worth judging.
pub fn compile_source(source: &str) -> Result<Rom, Vec<Diagnostic>> {
    let syntax = parse(source).map_err(|errors| {
        spanned(
            errors
                .into_iter()
                .map(|e| (e.message, e.span, Refusal::Rejected)),
            source,
        )
    })?;
    let typed = analyze(&syntax).map_err(|errors| {
        spanned(
            errors.into_iter().map(|e| (e.message, e.span, e.refusal)),
            source,
        )
    })?;
    // Warnings first, then errors: a warning is about what the author wrote
    // before the stage gave up, and the errors are why it did.
    let mut ir = lower(&typed).map_err(|failure| {
        let mut diagnostics = warned(failure.warnings, source);
        diagnostics.extend(lowered(failure.errors, source));
        diagnostics
    })?;
    // Every failing stage after lowering reports the warnings lowering found,
    // for the reason the whole run does: the author who fixes the error is the
    // author who needed the warning, and they only get one look at it.
    let warnings = warned(std::mem::take(&mut ir.warnings), source);
    let output = generate_with_isa(&ir, LEGAL_ISA)
        .map_err(|error| beside(&warnings, codegen_diagnostic(error, source)))?;
    // Only `using timed` pays the three-frame pass the fixed-bank note describes; an `irq`
    // frame emits its handlers once.
    let timed_frame = ir
        .frame
        .as_ref()
        .is_some_and(|frame| frame.strategy == FrameStrategy::Timed);
    let rom = link_mmc3_program(&output.program, output.main, output.irq, LEGAL_ISA)
        .map_err(|error| beside(&warnings, link_diagnostic(error, timed_frame)))?;

    Ok(Rom {
        image: rom.image,
        code_len: rom.code_len,
        runtime_len: rom.runtime_len,
        vectors: rom.vectors,
        reports: output.reports,
        warnings,
    })
}

/// Every error from one stage, as diagnostics. `Span::clamped` is the only way
/// spans reach `raster_diag` from here: they arrive as `u32` offsets from four
/// crates, one of which has a default span for a declaration with no name, and a
/// panic in the renderer would show the author a backtrace instead of a mistake.
fn spanned(
    errors: impl Iterator<Item = (String, raster_syntax::Span, Refusal)>,
    source: &str,
) -> Vec<Diagnostic> {
    noted(
        errors
            .map(|(message, span, refusal)| {
                let span = Span::clamped(span.start as usize, span.end as usize, source);
                (Diagnostic::error(message.clone(), span, message), refusal)
            })
            .collect(),
    )
}

/// One stage's error, behind the warnings found before it. Warnings first and
/// errors after, the same order `compile_source` gives a failed lowering.
fn beside(warnings: &[Diagnostic], error: Diagnostic) -> Vec<Diagnostic> {
    let mut diagnostics = warnings.to_vec();
    diagnostics.push(error);
    diagnostics
}

/// Lowering's warnings, as diagnostics. `Span::clamped` for the same reason
/// `spanned` uses it: offsets arrive as `u32` from another crate, and a panic in
/// the renderer would show the author a backtrace instead of a hazard.
///
/// Warnings never pass through `noted`: a note there belongs to a `Refusal`, and
/// a warning refuses nothing.
fn warned(warnings: Vec<raster_ir::LowerWarning>, source: &str) -> Vec<Diagnostic> {
    warnings
        .into_iter()
        .map(|warning| {
            let span = Span::clamped(
                warning.span.start as usize,
                warning.span.end as usize,
                source,
            );
            let mut diagnostic = Diagnostic::warning(warning.message, span, warning.label);
            diagnostic.notes = warning.notes;
            diagnostic
        })
        .collect()
}

/// Lowering's errors, as diagnostics. Unlike a parse or a semantic error, one
/// of these may carry a label and notes of its own — the write-only register
/// refusal names the port under the carets and says what a read of that address
/// returns — so it cannot go through `spanned`, whose label mirrors the message.
/// It still goes through `noted`, because it still carries a `Refusal`.
///
/// `Span::clamped` for the same reason `spanned` uses it: offsets arrive as
/// `u32` from another crate, and a panic in the renderer would show the author a
/// backtrace instead of a mistake.
fn lowered(errors: Vec<raster_ir::LowerError>, source: &str) -> Vec<Diagnostic> {
    noted(
        errors
            .into_iter()
            .map(|error| {
                let raster_ir::LowerError {
                    message,
                    label,
                    notes,
                    span,
                    refusal,
                } = error;
                let span = Span::clamped(span.start as usize, span.end as usize, source);
                let label = label.unwrap_or_else(|| message.clone());
                let mut diagnostic = Diagnostic::error(message, span, label);
                diagnostic.notes = notes;
                (diagnostic, refusal)
            })
            .collect(),
    )
}

/// The note a refusal of this kind carries, if it carries one.
fn note_for(refusal: Refusal) -> Option<&'static str> {
    match refusal {
        Refusal::NotInThisRelease => Some(SUPPORTED_SUBSET),
        Refusal::TimedRegionCost => Some(TIMED_REGION_COST),
        Refusal::Rejected => None,
    }
}

/// Say once, beside the first refusal of its kind, why the compiler said no.
/// Twice would be noise, and never would leave "yet" unexplained.
///
/// Stages run in order and the first failing stage is the last to run, so a run
/// cannot today contain both a semantic refusal and a lowering one. This does
/// not rely on that: each kind is said once, in the order the diagnostics
/// arrive, so a refusal moved between stages keeps its note.
fn noted(diagnostics: Vec<(Diagnostic, Refusal)>) -> Vec<Diagnostic> {
    let mut said: Vec<Refusal> = Vec::new();
    diagnostics
        .into_iter()
        .map(|(mut diagnostic, refusal)| {
            if let Some(note) = note_for(refusal) {
                if !said.contains(&refusal) {
                    said.push(refusal);
                    diagnostic.notes.push(note.to_owned());
                }
            }
            diagnostic
        })
        .collect()
}

fn codegen_diagnostic(error: CodegenError, source: &str) -> Diagnostic {
    match error {
        CodegenError::MissingMain => Diagnostic::without_span("this program has no `main` block")
            .with_note("add `main { ... }` to give the ROM somewhere to start"),
        CodegenError::ZeroPageExhausted { span } => Diagnostic::error(
            "too many variables for the zero page",
            Span::clamped(span.start as usize, span.end as usize, source),
            "too many variables for the zero page",
        )
        .with_note(format!(
            "the zero page holds {ZERO_PAGE_VARIABLES} variables, from $10 to $FF"
        )),
        CodegenError::Timing { error, span } => timing_diagnostic(error, span, source),
        // `lower` rejects the programs that would reach the remaining variants
        // before codegen ever sees them.
        error => internal_compiler_error(&error),
    }
}

/// The budget diagnostic of spec section 14: what it cost, what the budget was,
/// and the `cycles(...)` header it belongs to.
fn timing_diagnostic(error: TimingError, span: raster_syntax::Span, source: &str) -> Diagnostic {
    let at = |message: &str, label: String| {
        Diagnostic::error(
            message.to_owned(),
            Span::clamped(span.start as usize, span.end as usize, source),
            label,
        )
    };
    match error {
        TimingError::OverBudget {
            measured_cycles,
            budget,
        } => at(
            "timed block exceeds its budget",
            format!("block costs {measured_cycles} cycles, budget is {budget}"),
        )
        .with_note(
            "an indexed read that may cross a page and a branch that may be
taken are both charged their worst case",
        ),
        TimingError::UnderBudget {
            measured_cycles,
            budget,
        } => at(
            "timed block does not fill its budget",
            format!("block costs {measured_cycles} cycles, budget is {budget}"),
        )
        .with_note(format!(
            "`pad` would fill the remaining {} cycles",
            budget - measured_cycles
        )),
        TimingError::UnreachablePadding { remaining } => at(
            "a timed block cannot be padded to its budget",
            format!("this block is {remaining} cycle short of its budget"),
        )
        .with_note(
            "no instruction costs a single cycle, so widen the budget by one or
add a cycle of work",
        ),
        TimingError::DelayTooShort { requested_cycles } => at(
            "`wait cycles` needs at least two cycles",
            format!("a delay of {requested_cycles} cycles was asked for"),
        )
        .with_note("the shortest instruction the 6502 has costs two cycles"),
        TimingError::ControlFlowInRegion { opcode, .. } => at(
            "rasterc cannot prove this timed block's cost",
            format!("this block compiles to a branch or a jump (${opcode:02X})"),
        )
        .with_note(
            "a timed block is costed by adding up its instructions, so one
that jumps has no single cost",
        )
        .with_note(
            "rasterc should have refused the construct that produced this
with a clearer message; please report this file",
        ),
        TimingError::IrqHandlerOverHblank {
            measured_cycles,
            budget,
        } => at(
            "an `irq` handler exceeds its hblank",
            format!("handler costs {measured_cycles} cycles, the hblank leaves {budget}"),
        )
        .with_note(
            "the MMC3 asserts its interrupt near the end of the scanline before
the one this handler runs on, so a store made once the window has
closed lands part-way along a visible row",
        )
        .with_note(
            "split the work across two events on consecutive scanlines to give
each a window of its own, or use `using timed`, which gives a
handler the whole scanline at the cost of the frame's entire CPU time",
        ),
    }
}

/// A link failure, as a diagnostic. `has_timed_frame` is not decoration: a timed frame emits its
/// schedule once per frame of its pass, so an author whose program overflows the bank is otherwise
/// shown a byte count three times the size of anything they wrote, with nothing to explain it.
fn link_diagnostic(error: LinkError, has_timed_frame: bool) -> Diagnostic {
    match error {
        LinkError::FixedBankTooLarge { actual, maximum } => {
            let runtime = mmc3_reset_runtime_bytes();
            // Only ever constructed when `actual > maximum`; saturating rather
            // than a subtraction that would panic in debug on a path nothing
            // can reach.
            let over = actual.saturating_sub(maximum);
            let diagnostic =
                Diagnostic::without_span("the program does not fit the MMC3 fixed bank")
                    .with_note(format!(
                        "{actual} bytes of code, and $E000-$FFFF holds {maximum}"
                    ))
                    .with_note(format!(
                        "{runtime} of those are the reset runtime, so {over} bytes of your own\n\
                         have to go"
                    ))
                    .with_note(
                        "PRG bank switching is not supported yet, so all code lives\n\
                     in the fixed bank",
                    );
            if has_timed_frame {
                diagnostic.with_note(
                    "a `frame ... using timed` emits its schedule once per frame of its\n\
                     three-frame pass, so a handler costs three times its own size",
                )
            } else {
                diagnostic
            }
        }
        LinkError::RelativeBranchOutOfRange { from, target } => {
            Diagnostic::without_span("a branch is too far for the 6502").with_note(format!(
                "the branch at ${from:04X} must reach ${target:04X}, and 6502 branches\n\
                 reach 127 bytes forward or 128 back"
            ))
        }
        error => internal_compiler_error(&error),
    }
}

fn internal_compiler_error(error: &impl std::fmt::Debug) -> Diagnostic {
    Diagnostic::without_span(format!("internal compiler error: {error:?}"))
        .with_note("this is a bug in rasterc, not in your program")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The figures are a real overflow: 1700 `ppu.mask = 1` statements compile
    /// to 8635 bytes, of which 449 are over the bank. Pinned here rather than
    /// through `compile_source` so the strings are asserted without a 1700-line
    /// fixture, and so a codegen change elsewhere cannot silently move them.
    #[test]
    fn the_overflow_refusal_says_how_many_of_the_author_s_own_bytes_have_to_go() {
        let diagnostic = link_diagnostic(
            LinkError::FixedBankTooLarge {
                actual: 8635,
                maximum: 8186,
            },
            false,
        );

        assert_eq!(
            diagnostic.message,
            "the program does not fit the MMC3 fixed bank"
        );
        assert_eq!(
            diagnostic.notes,
            [
                "8635 bytes of code, and $E000-$FFFF holds 8186",
                "132 of those are the reset runtime, so 449 bytes of your own\nhave to go",
                "PRG bank switching is not supported yet, so all code lives\nin the fixed bank",
            ]
        );
    }

    /// The backstop is by design unreachable from any source `raster-sema` already refuses, so its
    /// rendering is pinned here rather than through `compile_source`. Asserting `message` exactly
    /// is what pins the variant onto a real arm: `codegen_diagnostic`'s
    /// `error => internal_compiler_error(&error)` fallback produces a message beginning
    /// "internal compiler error: ", which would fail this.
    #[test]
    fn the_control_flow_backstop_is_a_diagnostic_and_not_an_internal_error() {
        let diagnostic = codegen_diagnostic(
            CodegenError::Timing {
                error: TimingError::ControlFlowInRegion {
                    index: 2,
                    opcode: 0x4c,
                },
                span: raster_syntax::Span::new(7, 17),
            },
            "main { cycles(30) pad { return } }",
        );

        assert_eq!(
            diagnostic.message,
            "rasterc cannot prove this timed block's cost"
        );
        assert_eq!(
            diagnostic.label,
            "this block compiles to a branch or a jump ($4C)"
        );
        assert_eq!(
            diagnostic.notes,
            [
                "a timed block is costed by adding up its instructions, so one\nthat jumps has no single cost",
                "rasterc should have refused the construct that produced this\nwith a clearer message; please report this file",
            ]
        );
    }
}
