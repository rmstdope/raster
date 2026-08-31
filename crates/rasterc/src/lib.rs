use std::{
    fs,
    io::{ErrorKind, Write},
    path::PathBuf,
};

mod compile;
mod report;

pub use compile::{compile_source, Rom};

const USAGE: &str = "Usage: rasterc <INPUT.raster> [-o <OUTPUT.nes>]\n";

const HELP: &str = concat!(
    "rasterc ",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "Compile a Raster source file into an NES ROM.\n",
    "\n",
    "Usage: rasterc <INPUT.raster> [options]\n",
    "\n",
    "Options:\n",
    "  -o, --output <OUTPUT.nes>  write the ROM here\n",
    "                             (default: the input with a .nes extension)\n",
    "      --version              print the version and exit\n",
    "  -h, --help                 print this help and exit\n",
);

enum Invocation {
    Help,
    Version,
    Compile { input: String, output: String },
}

pub fn run(
    args: impl IntoIterator<Item = String>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), i32> {
    let args: Vec<_> = args.into_iter().collect();
    match arguments(&args) {
        Some(Invocation::Help) => {
            write(stdout, HELP)?;
            Ok(())
        }
        Some(Invocation::Version) => {
            write(stdout, &format!("rasterc {}\n", env!("CARGO_PKG_VERSION")))?;
            Ok(())
        }
        Some(Invocation::Compile { input, output }) => compile(&input, &output, stdout, stderr),
        None => {
            write(stderr, USAGE)?;
            Err(2)
        }
    }
}

/// Four flags do not earn a dependency. `None` is a usage error: no input, a
/// second positional, a repeated or valueless `-o`, or a flag we do not know.
fn arguments(args: &[String]) -> Option<Invocation> {
    let mut input: Option<&str> = None;
    let mut output: Option<&str> = None;
    let mut remaining = args.iter();

    while let Some(argument) = remaining.next() {
        match argument.as_str() {
            "-h" | "--help" => return Some(Invocation::Help),
            "--version" => return Some(Invocation::Version),
            "-o" | "--output" => {
                if output.is_some() {
                    return None;
                }
                output = Some(remaining.next()?.as_str());
            }
            flag if flag.starts_with('-') => return None,
            positional => {
                if input.is_some() {
                    return None;
                }
                input = Some(positional);
            }
        }
    }

    let input = input?;
    let output = output.map_or_else(
        || {
            PathBuf::from(input)
                .with_extension("nes")
                .display()
                .to_string()
        },
        str::to_owned,
    );
    Some(Invocation::Compile {
        input: input.to_owned(),
        output,
    })
}

fn compile(
    input: &str,
    output: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), i32> {
    // Reachable as `rasterc demo.nes`, and it would destroy the author's source.
    // Refuse before compiling rather than after.
    if output == input {
        write(
            stderr,
            &format!("error: refusing to overwrite the input file {input}\n"),
        )?;
        return Err(1);
    }

    let source = match fs::read_to_string(input) {
        Ok(source) => source,
        Err(error) => {
            write(stderr, &format!("error: could not read {input}: {error}\n"))?;
            return Err(1);
        }
    };

    let rom = match compile_source(&source) {
        Ok(rom) => rom,
        Err(found) => {
            write(stderr, &report::diagnostics(input, &source, &found))?;
            return Err(1);
        }
    };

    // The ROM is complete in memory before anything is written, so a failed
    // write never leaves a partial file behind.
    if let Err(error) = fs::write(output, &rom.image) {
        write(
            stderr,
            &format!("error: could not write {output}: {error}\n"),
        )?;
        return Err(1);
    }

    write(stdout, &report::summary(input, output, &rom))
}

fn write(writer: &mut dyn Write, text: &str) -> Result<(), i32> {
    match writer.write_all(text.as_bytes()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Err(0),
        Err(_) => Err(1),
    }
}
