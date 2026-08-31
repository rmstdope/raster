use std::{
    fs,
    io::{Cursor, Error, ErrorKind, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use rasterc::run;

/// A directory of this test's own, so parallel tests never share an output path.
fn scratch_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rasterc-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("the scratch directory is creatable");
    path
}

fn demo_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp/demo.raster");
    fs::read_to_string(path).expect("the demo example is readable")
}

fn run_capturing(args: Vec<String>) -> (Result<(), i32>, String, String) {
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());
    let result = run(args, &mut stdout, &mut stderr);
    (
        result,
        String::from_utf8(stdout.into_inner()).expect("stdout is valid UTF-8"),
        String::from_utf8(stderr.into_inner()).expect("stderr is valid UTF-8"),
    )
}

fn summary(input: &Path, output: &Path) -> String {
    format!(
        " Compiled  {} -> {}\n   mapper  MMC3 (4), NTSC\n      prg  32 KiB, 4 banks of 8 KiB\n      chr  8 KiB RAM\n    fixed  $E000-$FFFF, 100 of 8186 bytes used\n    entry  reset $E000, nmi $E063, irq $E063\n",
        input.display(),
        output.display()
    )
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn no_input_writes_usage_and_returns_code_two() {
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());

    assert_eq!(run(Vec::new(), &mut stdout, &mut stderr), Err(2));
    assert_eq!(stdout.into_inner(), b"");
    assert_eq!(
        stderr.into_inner(),
        b"Usage: rasterc <INPUT.raster> [-o <OUTPUT.nes>]\n"
    );
}

#[test]
fn version_writes_package_version() {
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());

    assert_eq!(
        run(vec!["--version".to_owned()], &mut stdout, &mut stderr),
        Ok(())
    );
    assert!(String::from_utf8(stdout.into_inner())
        .expect("stdout is valid UTF-8")
        .starts_with("rasterc "));
    assert_eq!(stderr.into_inner(), b"");
}

#[test]
fn unknown_top_level_item_reports_a_source_spanned_diagnostic() {
    let path = fixture("plain.raster");
    let (result, stdout, stderr) = run_capturing(vec![path.display().to_string()]);

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            concat!(
                "error: expected a top-level declaration\n",
                " --> {path}:1:1\n",
                "  |\n",
                "1 | screen {{}}\n",
                "  | ^^^^^^ expected a top-level declaration\n",
                "\n",
                "error: could not compile {path} (1 error)\n"
            ),
            path = path.display()
        )
    );
}

#[test]
fn invalid_at_input_reports_a_source_spanned_diagnostic() {
    let path = fixture("invalid-at.raster");
    let output = Command::new(env!("CARGO_BIN_EXE_rasterc"))
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("rasterc starts");

    assert!(!output.status.success());
    assert_eq!(output.stdout, b"");
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is valid UTF-8"),
        format!(
            concat!(
                "error: expected a top-level declaration\n",
                " --> {path}:1:1\n",
                "  |\n",
                "1 | @\n",
                "  | ^ expected a top-level declaration\n",
                "\n",
                "error: could not compile {path} (1 error)\n"
            ),
            path = path.display()
        )
    );
}

#[test]
fn broken_pipe_ends_without_a_panic() {
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = BrokenPipe;

    assert_eq!(run(Vec::new(), &mut stdout, &mut stderr), Err(0));
}

struct BrokenPipe;

impl Write for BrokenPipe {
    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
        Err(Error::from(ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn writes_the_rom_beside_the_input() {
    let directory = scratch_directory("beside");
    let input = directory.join("demo.raster");
    let output = directory.join("demo.nes");
    fs::write(&input, demo_source()).unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Ok(()));
    assert_eq!(stderr, "");
    assert_eq!(stdout, summary(&input, &output));
    assert_eq!(
        fs::read(&output).unwrap(),
        raster_link::m1_solid_backdrop_rom()
    );
}

#[test]
fn accepts_the_output_flag_before_and_after_the_input() {
    for arguments in [
        ["-o", "rom.nes", "demo.raster"],
        ["demo.raster", "-o", "rom.nes"],
    ] {
        let directory = scratch_directory("flag");
        let input = directory.join("demo.raster");
        let output = directory.join("rom.nes");
        fs::write(&input, demo_source()).unwrap();

        let arguments = arguments
            .iter()
            .map(|argument| match *argument {
                "demo.raster" => input.display().to_string(),
                "rom.nes" => output.display().to_string(),
                other => other.to_owned(),
            })
            .collect();
        let (result, stdout, stderr) = run_capturing(arguments);

        assert_eq!(result, Ok(()));
        assert_eq!(stderr, "");
        assert_eq!(stdout, summary(&input, &output));
        assert_eq!(
            fs::read(&output).unwrap(),
            raster_link::m1_solid_backdrop_rom()
        );
    }
}

#[test]
fn help_goes_to_stdout_and_succeeds() {
    for flag in ["-h", "--help"] {
        let (result, stdout, stderr) = run_capturing(vec![flag.to_owned()]);

        assert_eq!(result, Ok(()));
        assert_eq!(stderr, "");
        assert_eq!(
            stdout,
            concat!(
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
            )
        );
    }
}

#[test]
fn a_second_positional_is_a_usage_error() {
    for arguments in [
        vec!["one.raster", "two.raster"],
        vec!["one.raster", "-o", "a.nes", "-o", "b.nes"],
        vec!["one.raster", "-o"],
        vec!["--nonsense"],
    ] {
        let arguments = arguments.into_iter().map(str::to_owned).collect();
        let (result, stdout, stderr) = run_capturing(arguments);

        assert_eq!(result, Err(2));
        assert_eq!(stdout, "");
        assert_eq!(stderr, "Usage: rasterc <INPUT.raster> [-o <OUTPUT.nes>]\n");
    }
}

#[test]
fn overwrites_an_existing_rom_without_comment() {
    let directory = scratch_directory("overwrite");
    let input = directory.join("demo.raster");
    let output = directory.join("demo.nes");
    fs::write(&input, demo_source()).unwrap();
    fs::write(&output, b"stale").unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Ok(()));
    assert_eq!(stderr, "");
    assert_eq!(stdout, summary(&input, &output));
    assert_eq!(
        fs::read(&output).unwrap(),
        raster_link::m1_solid_backdrop_rom()
    );
}

#[test]
fn a_missing_parent_directory_is_an_error_rather_than_a_new_tree() {
    let directory = scratch_directory("missing");
    let input = directory.join("demo.raster");
    let output = directory.join("build").join("demo.nes");
    fs::write(&input, demo_source()).unwrap();

    let (result, stdout, stderr) = run_capturing(vec![
        input.display().to_string(),
        "-o".to_owned(),
        output.display().to_string(),
    ]);

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with(&format!("error: could not write {}: ", output.display())),
        "unexpected stderr: {stderr}"
    );
    assert!(!directory.join("build").exists());
}

#[test]
fn refuses_to_overwrite_its_own_input() {
    let directory = scratch_directory("selfwrite");
    let input = directory.join("demo.nes");
    fs::write(&input, demo_source()).unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            "error: refusing to overwrite the input file {}\n",
            input.display()
        )
    );
    // Refused before compiling, so the source is untouched.
    assert_eq!(fs::read_to_string(&input).unwrap(), demo_source());
}
