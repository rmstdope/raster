use std::{fs, io::Write};

use raster_diag::{render, Diagnostic, SourceFile, Span};

const USAGE: &str = "Usage: rasterc <INPUT.raster>\n";

pub fn run(
    args: impl IntoIterator<Item = String>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), i32> {
    let args: Vec<_> = args.into_iter().collect();
    match args.as_slice() {
        [] => {
            write(stderr, USAGE);
            Err(2)
        }
        [help] if help == "-h" || help == "--help" => {
            write(stderr, USAGE);
            Err(2)
        }
        [version] if version == "--version" => {
            write(stdout, &format!("rasterc {}\n", env!("CARGO_PKG_VERSION")));
            Ok(())
        }
        [path] => compile(path, stderr),
        _ => {
            write(stderr, USAGE);
            Err(2)
        }
    }
}

fn compile(path: &str, stderr: &mut dyn Write) -> Result<(), i32> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            write(stderr, &format!("error: could not read {path}: {error}\n"));
            return Err(1);
        }
    };

    if let Some(start) = source.find('@') {
        let diagnostic = Diagnostic::error(
            "unexpected character `@`",
            Span::new(start, start + '@'.len_utf8()),
            "unexpected character `@`",
        );
        write(stderr, &render(&SourceFile::new(path, source), &diagnostic));
        return Err(1);
    }

    write(stderr, "error: compilation is not available yet\n");
    Err(1)
}

fn write(writer: &mut dyn Write, text: &str) {
    writer
        .write_all(text.as_bytes())
        .expect("writing compiler output succeeds");
}
