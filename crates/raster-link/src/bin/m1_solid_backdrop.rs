use std::{env, fs, path::PathBuf, process};

fn main() {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let Some(output_path) = arguments.next().map(PathBuf::from) else {
        usage_and_exit();
    };

    if arguments.next().is_some() {
        usage_and_exit();
    }

    if let Err(error) = fs::write(&output_path, raster_link::m1_solid_backdrop_rom()) {
        eprintln!("error: could not write {}: {error}", output_path.display());
        process::exit(1);
    }
}

fn usage_and_exit() -> ! {
    eprintln!("Usage: m1_solid_backdrop <OUTPUT.nes>");
    process::exit(2);
}
