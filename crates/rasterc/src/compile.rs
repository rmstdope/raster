//! The compiler pipeline, with no filesystem in it.
//!
//! Nothing here reads or writes a file, so the whole of what `rasterc` does to a
//! source is testable from a string.

use raster_codegen::{generate, CodegenError};
use raster_diag::{Diagnostic, Span};
use raster_ir::lower;
use raster_link::{link_mmc3_program, InterruptVectors, LinkError, MMC3_FIXED_BANK_CODE_SIZE};
use raster_sema::analyze;
use raster_syntax::parse;

/// What this release of the compiler can build, listed once per run beside the
/// first construct it had to refuse.
const SUPPORTED_SUBSET: &str = "this release compiles `main`, `fn`, `if`, `while`, `for`, u8\n\
                                arithmetic and `ppu.*` / `mmc3.*` register writes";
const UNSUPPORTED_SUFFIX: &str = "not supported yet";

/// The zero page raster allocates from, `$10` through `$FF`.
const ZERO_PAGE_VARIABLES: usize = 0x100 - 0x10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rom {
    pub image: Vec<u8>,
    pub code_len: usize,
    pub vectors: InterruptVectors,
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
    let output = generate(&ir).map_err(|error| noted(vec![codegen_diagnostic(error, source)]))?;
    let rom = link_mmc3_program(&output.program, output.main, true)
        .map_err(|error| noted(vec![link_diagnostic(error)]))?;

    Ok(Rom {
        image: rom.image,
        code_len: rom.code_len,
        vectors: rom.vectors,
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
        // `lower` rejects the programs that would reach the remaining variants
        // before codegen ever sees them.
        error => internal_compiler_error(&error),
    }
}

fn link_diagnostic(error: LinkError) -> Diagnostic {
    match error {
        LinkError::FixedBankTooLarge { actual, .. } => {
            Diagnostic::without_span("the program does not fit the MMC3 fixed bank")
                .with_note(format!(
                    "{actual} bytes of code, and $E000-$FFFF holds {MMC3_FIXED_BANK_CODE_SIZE}"
                ))
                .with_note(
                    "PRG bank switching is not supported yet, so all code lives\n\
                     in the fixed bank",
                )
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
