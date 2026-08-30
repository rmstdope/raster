use std::process::ExitCode;

fn main() -> ExitCode {
    match rasterc::run(
        std::env::args().skip(1),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
    }
}
