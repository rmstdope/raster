use std::{
    fs,
    io::{Cursor, Error, ErrorKind, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use rasterc::run;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn scratch_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rasterc-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("the scratch directory is creatable");
    path
}

fn copy_fixture_into(directory: &Path, name: &str) -> PathBuf {
    let destination = directory.join(name);
    fs::copy(fixture(name), &destination).expect("the fixture is copyable");
    destination
}

#[test]
fn no_input_writes_usage_and_returns_code_two() {
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());

    assert_eq!(run(Vec::new(), &mut stdout, &mut stderr), Err(2));
    assert_eq!(stdout.into_inner(), b"");
    assert_eq!(stderr.into_inner(), b"Usage: rasterc <INPUT.raster>\n");
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
fn rasterc_compiles_backdrop_fixture_to_requested_output() {
    let directory = scratch_directory("backdrop");
    let input = copy_fixture_into(&directory, "backdrop.raster");
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());

    assert_eq!(
        run(vec![input.display().to_string()], &mut stdout, &mut stderr),
        Ok(())
    );
    assert_eq!(stdout.into_inner(), b"");
    assert_eq!(stderr.into_inner(), b"");

    let rom =
        fs::read(directory.join("backdrop.nes")).expect("the ROM is written beside its source");
    assert_eq!(&rom[..4], b"NES\x1a");
    assert_eq!(rom.len(), 16 + 32 * 1024);

    fs::remove_dir_all(&directory).expect("the scratch directory is removable");
}

#[test]
fn unreadable_input_reports_an_actionable_error() {
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());
    let missing = std::env::temp_dir().join("rasterc-does-not-exist.raster");

    assert_eq!(
        run(
            vec![missing.display().to_string()],
            &mut stdout,
            &mut stderr
        ),
        Err(1)
    );
    assert_eq!(stdout.into_inner(), b"");
    let message = String::from_utf8(stderr.into_inner()).expect("stderr is valid UTF-8");
    assert!(
        message.starts_with(&format!("error: could not read {}: ", missing.display())),
        "unexpected message: {message}"
    );
}

#[test]
fn unwritable_output_reports_an_actionable_error() {
    let directory = scratch_directory("unwritable");
    let input = copy_fixture_into(&directory, "backdrop.raster");
    let output = directory.join("backdrop.nes");
    fs::create_dir(&output).expect("a directory can stand where the ROM would go");
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());

    assert_eq!(
        run(vec![input.display().to_string()], &mut stdout, &mut stderr),
        Err(1)
    );
    assert_eq!(stdout.into_inner(), b"");
    let message = String::from_utf8(stderr.into_inner()).expect("stderr is valid UTF-8");
    assert!(
        message.starts_with(&format!("error: could not write {}: ", output.display())),
        "unexpected message: {message}"
    );

    fs::remove_dir_all(&directory).expect("the scratch directory is removable");
}

#[test]
fn invalid_at_input_reports_a_source_spanned_diagnostic() {
    let output = Command::new(env!("CARGO_BIN_EXE_rasterc"))
        .arg(fixture("invalid-at.raster"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("rasterc starts");

    assert!(!output.status.success());
    assert_eq!(output.stdout, b"");
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is valid UTF-8"),
        format!(
            "error: expected a top-level declaration\n --> {}:1:1\n  |\n1 | @\n  | ^ expected a top-level declaration\n",
            fixture("invalid-at.raster").display()
        )
    );
    assert!(!fixture("invalid-at.nes").exists());
}

#[test]
fn semantic_error_reports_a_source_spanned_diagnostic() {
    let directory = scratch_directory("semantic");
    let input = copy_fixture_into(&directory, "undefined-name.raster");
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());

    assert_eq!(
        run(vec![input.display().to_string()], &mut stdout, &mut stderr),
        Err(1)
    );
    assert_eq!(stdout.into_inner(), b"");
    let message = String::from_utf8(stderr.into_inner()).expect("stderr is valid UTF-8");
    assert!(
        message.contains("`missing`"),
        "unexpected message: {message}"
    );
    assert!(
        message.contains(&format!(" --> {}:2:16\n", input.display())),
        "unexpected message: {message}"
    );
    assert!(!directory.join("undefined-name.nes").exists());

    fs::remove_dir_all(&directory).expect("the scratch directory is removable");
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
