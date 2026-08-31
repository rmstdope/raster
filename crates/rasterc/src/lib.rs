use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use raster_codegen::{generate, CodegenError};
use raster_diag::{render, Diagnostic, SourceFile, Span};
use raster_ir::{lower, LowerError};
use raster_link::{link_mmc3_ines, LinkError};
use raster_sema::{analyze, SemanticError};
use raster_syntax::{parse, ParseError, Span as SourceSpan};

const USAGE: &str = "Usage: rasterc <INPUT.raster>\n";

pub fn run(
    args: impl IntoIterator<Item = String>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), i32> {
    let args: Vec<_> = args.into_iter().collect();
    match args.as_slice() {
        [] => {
            write(stderr, USAGE)?;
            Err(2)
        }
        [help] if help == "-h" || help == "--help" => {
            write(stderr, USAGE)?;
            Err(2)
        }
        [version] if version == "--version" => {
            write(stdout, &format!("rasterc {}\n", env!("CARGO_PKG_VERSION")))?;
            Ok(())
        }
        [path] => compile(path, stderr),
        _ => {
            write(stderr, USAGE)?;
            Err(2)
        }
    }
}

/// The ROM `rasterc input.raster` writes: the source path with a `.nes` extension.
pub fn rom_path(source: &Path) -> PathBuf {
    source.with_extension("nes")
}

fn compile(path: &str, stderr: &mut dyn Write) -> Result<(), i32> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            write(stderr, &format!("error: could not read {path}: {error}\n"))?;
            return Err(1);
        }
    };
    let file = SourceFile::new(path, source.clone());

    let syntax = stage(parse(&source), stderr, &file)?;
    let typed = stage(analyze(&syntax), stderr, &file)?;
    let ir = stage(lower(&typed), stderr, &file)?;
    let generated = match generate(&ir) {
        Ok(generated) => generated,
        Err(error) => {
            let (message, span) = describe_codegen_error(&error);
            return match span {
                Some(span) => report(stderr, &file, [(message, span)]),
                None => {
                    write(stderr, &format!("error: {message}\n"))?;
                    Err(1)
                }
            };
        }
    };
    let rom = match link_mmc3_ines(&generated.program, generated.entry_points, true) {
        Ok(rom) => rom,
        Err(error) => {
            write(stderr, &format!("error: {}\n", describe_link_error(&error)))?;
            return Err(1);
        }
    };

    let output = rom_path(Path::new(path));
    match fs::write(&output, &rom) {
        Ok(()) => Ok(()),
        Err(error) => {
            write(
                stderr,
                &format!("error: could not write {}: {error}\n", output.display()),
            )?;
            Err(1)
        }
    }
}

/// A front-end stage: its errors are source-spanned, so each renders as its own diagnostic.
fn stage<T, E: SpannedError>(
    result: Result<T, Vec<E>>,
    stderr: &mut dyn Write,
    file: &SourceFile,
) -> Result<T, i32> {
    match result {
        Ok(value) => Ok(value),
        Err(errors) => {
            report(
                stderr,
                file,
                errors.iter().map(|error| (error.message(), error.span())),
            )?;
            Err(1)
        }
    }
}

fn report(
    stderr: &mut dyn Write,
    file: &SourceFile,
    errors: impl IntoIterator<Item = (String, SourceSpan)>,
) -> Result<(), i32> {
    for (message, span) in errors {
        let span = clamp(span, file.source.len());
        let diagnostic = Diagnostic::error(message.clone(), span, message);
        write(stderr, &render(file, &diagnostic))?;
    }
    Err(1)
}

/// What the front end's three error types have in common: a message and where it points.
trait SpannedError {
    fn message(&self) -> String;
    fn span(&self) -> SourceSpan;
}

macro_rules! spanned_error {
    ($($type:ty),+ $(,)?) => {
        $(impl SpannedError for $type {
            fn message(&self) -> String {
                self.message.clone()
            }

            fn span(&self) -> SourceSpan {
                self.span
            }
        })+
    };
}

spanned_error!(ParseError, SemanticError, LowerError);

/// Spans reported past the end of the source would panic the renderer, so hold them inside it.
fn clamp(span: SourceSpan, length: usize) -> Span {
    let start = (span.start as usize).min(length);
    let end = (span.end as usize).clamp(start, length);
    Span::new(start, end)
}

fn describe_codegen_error(error: &CodegenError) -> (String, Option<SourceSpan>) {
    match error {
        CodegenError::MissingMain => ("a program needs a `main` block to compile".to_owned(), None),
        CodegenError::ZeroPageExhausted { span } => (
            "too many variables for zero page".to_owned(),
            Some(*span),
        ),
        CodegenError::UnknownPlace { place } => {
            (format!("internal error: unknown place {}", place.0), None)
        }
        CodegenError::UnknownFunction { label } => {
            (format!("internal error: unknown function {}", label.0), None)
        }
        CodegenError::WrongArgumentCount {
            label,
            expected,
            actual,
        } => (
            format!(
                "internal error: function {} takes {expected} arguments but was called with {actual}",
                label.0
            ),
            None,
        ),
    }
}

fn describe_link_error(error: &LinkError) -> String {
    match error {
        LinkError::DuplicateLabel { label } => {
            format!("internal error: duplicate label {}", label.0)
        }
        LinkError::UndefinedLabel { label } => {
            format!("internal error: undefined label {}", label.0)
        }
        LinkError::RelativeBranchOutOfRange { from, target } => format!(
            "a branch at ${from:04x} cannot reach ${target:04x}: the program is too large to branch across"
        ),
        LinkError::FixedBankTooLarge { actual, maximum } => format!(
            "the program needs {actual} bytes but the fixed bank holds {maximum}"
        ),
        LinkError::EntryPointOutsideCode { vector, address } => format!(
            "the {vector} entry point at ${address:04x} lies outside the fixed bank's code"
        ),
        LinkError::Assemble(error) => format!("internal error: {error:?}"),
    }
}

fn write(writer: &mut dyn Write, text: &str) -> Result<(), i32> {
    match writer.write_all(text.as_bytes()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Err(0),
        Err(_) => Err(1),
    }
}
