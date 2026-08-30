use std::{
    io::Cursor,
    path::PathBuf,
    process::{Command, Stdio},
};

use rasterc::run;

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
fn readable_source_without_at_reports_unavailable_compilation() {
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());

    assert_eq!(
        run(
            vec![fixture("plain.raster").display().to_string()],
            &mut stdout,
            &mut stderr,
        ),
        Err(1)
    );
    assert_eq!(stdout.into_inner(), b"");
    assert_eq!(
        stderr.into_inner(),
        b"error: compilation is not available yet\n"
    );
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
            "error: unexpected character `@`\n --> {}:1:1\n  |\n1 | @\n  | ^ unexpected character `@`\n",
            fixture("invalid-at.raster").display()
        )
    );
}
