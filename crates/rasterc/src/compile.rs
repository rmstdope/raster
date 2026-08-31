//! The compiler pipeline, with no filesystem in it.
//!
//! Nothing here reads or writes a file, so the whole of what `rasterc` does to a
//! source is testable from a string.

use raster_codegen::{generate_with_isa, CodegenError};
use raster_diag::{Diagnostic, Span};
use raster_ir::{lower, FrameStrategy};
use raster_link::{link_mmc3_program, InterruptVectors, LinkError};
use raster_sema::analyze;
use raster_syntax::parse;
use raster_timing::TimingError;

/// What this release of the compiler can build, listed once per run beside the
/// first construct it had to refuse.
const SUPPORTED_SUBSET: &str = "this release compiles `main`, `fn`, `if`, `while`, `for`, u8\n\
                                arithmetic and `ppu.*` / `mmc3.*` register writes";
const UNSUPPORTED_SUFFIX: &str = "not supported yet";

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
    pub vectors: InterruptVectors,
    /// The measured cost of each `cycles(?)` region, which the summary prints.
    pub reports: Vec<(String, u32)>,
}

/// Compile `source` into a ROM, or into every diagnostic the first failing stage
/// produced.
///
/// Stages run in order and the first one with errors is the last one to run, so
/// a file with both a parse error and an unsupported construct reports only the
/// parse errors: the later stages were never given a program worth judging.
pub fn compile_source(source: &str) -> Result<Rom, Vec<Diagnostic>> {
    let syntax = parse(source)
        .map_err(|errors| spanned(errors.into_iter().map(|e| (e.message, e.span)), source))?;
    let typed = analyze(&syntax)
        .map_err(|errors| spanned(errors.into_iter().map(|e| (e.message, e.span)), source))?;
    let ir = lower(&typed)
        .map_err(|errors| spanned(errors.into_iter().map(|e| (e.message, e.span)), source))?;
    let output = generate_with_isa(&ir, LEGAL_ISA)
        .map_err(|error| noted(vec![codegen_diagnostic(error, source)]))?;
    let rom = link_mmc3_program(&output.program, output.main, output.irq, LEGAL_ISA).map_err(
        |error| {
            let timed_frame = ir
                .frame
                .as_ref()
                .is_some_and(|frame| frame.strategy == FrameStrategy::Timed);
            noted(vec![link_diagnostic(error, timed_frame)])
        },
    )?;

    Ok(Rom {
        image: rom.image,
        code_len: rom.code_len,
        vectors: rom.vectors,
        reports: output.reports,
    })
}

/// Every error from one stage, as diagnostics. `Span::clamped` is the only way
/// spans reach `raster_diag` from here: they arrive as `u32` offsets from four
/// crates, one of which has a default span for a declaration with no name, and a
/// panic in the renderer would show the author a backtrace instead of a mistake.
fn spanned(
    errors: impl Iterator<Item = (String, raster_syntax::Span)>,
    source: &str,
) -> Vec<Diagnostic> {
    noted(
        errors
            .map(|(message, span)| {
                let span = Span::clamped(span.start as usize, span.end as usize, source);
                Diagnostic::error(message.clone(), span, message)
            })
            .collect(),
    )
}

/// Say once, beside the first construct this release had to refuse, what it can
/// build instead. Twice would be noise, and never would leave "yet" unexplained.
fn noted(mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    if let Some(first) = diagnostics
        .iter_mut()
        .find(|diagnostic| diagnostic.message.ends_with(UNSUPPORTED_SUFFIX))
    {
        first.notes.push(SUPPORTED_SUBSET.to_owned());
    }
    diagnostics
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
    }
}

/// A link failure, as a diagnostic. `has_timed_frame` is not decoration: a timed frame emits its
/// schedule once per frame of its pass, so an author whose program overflows the bank is otherwise
/// shown a byte count three times the size of anything they wrote, with nothing to explain it.
fn link_diagnostic(error: LinkError, has_timed_frame: bool) -> Diagnostic {
    match error {
        LinkError::FixedBankTooLarge { actual, maximum } => {
            let diagnostic =
                Diagnostic::without_span("the program does not fit the MMC3 fixed bank")
                    .with_note(format!(
                        "{actual} bytes of code, and $E000-$FFFF holds {maximum}"
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
