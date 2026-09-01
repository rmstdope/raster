use std::{
    fs,
    io::{Cursor, Error, ErrorKind, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use rasterc::run;

mod common;
use common::{demo_source, Scratch};

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

/// The eight rows every successful build prints, then whatever this build adds
/// after them. `reset` is always `$E000`, and `nmi` and `irq` share an address
/// for a program with no `frame`. `yours` is the author's own share of
/// `code_len`; the runtime's is the rest, which is 132 for every program.
fn summary_of(
    input: &Path,
    output: &Path,
    code_len: usize,
    yours: usize,
    handler: u16,
    trailing: &str,
) -> String {
    let runtime = code_len - yours;
    format!(
        " Compiled  {} -> {}\n   mapper  MMC3 (4), NTSC\n      prg  32 KiB, 4 banks of 8 KiB\n      chr  8 KiB RAM\n    fixed  $E000-$FFFF, {code_len} of 8186 bytes used\n    yours  {yours} bytes\n  runtime  {runtime} bytes, the reset sequence around your program\n    entry  reset $E000, nmi ${handler:04X}, irq ${handler:04X}\n{trailing}",
        input.display(),
        output.display()
    )
}

/// The demo's summary, which three tests assert byte for byte.
fn summary(input: &Path, output: &Path) -> String {
    summary_of(input, output, 160, 28, 0xE09F, "")
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
    let directory = Scratch::new("beside");
    let input = directory.path().join("demo.raster");
    let output = directory.path().join("demo.nes");
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
        let directory = Scratch::new("flag");
        let input = directory.path().join("demo.raster");
        let output = directory.path().join("rom.nes");
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
    let directory = Scratch::new("overwrite");
    let input = directory.path().join("demo.raster");
    let output = directory.path().join("demo.nes");
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
    let directory = Scratch::new("missing");
    let input = directory.path().join("demo.raster");
    let output = directory.path().join("build").join("demo.nes");
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
    assert!(!directory.path().join("build").exists());
}

#[test]
fn refuses_to_overwrite_its_own_input() {
    let directory = Scratch::new("selfwrite");
    let input = directory.path().join("demo.nes");
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

#[test]
fn refuses_to_overwrite_its_own_input_under_another_spelling() {
    let directory = Scratch::new("samefile");
    let input = directory.path().join("demo.raster");
    fs::write(&input, demo_source()).unwrap();
    // The same file, named through `.`: a string comparison does not see it, and
    // the author's source is what gets destroyed.
    let output = directory.path().join(".").join("demo.raster");

    let (result, stdout, stderr) = run_capturing(vec![
        input.display().to_string(),
        "-o".to_owned(),
        output.display().to_string(),
    ]);

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            "error: refusing to overwrite the input file {}\n",
            input.display()
        )
    );
    assert_eq!(fs::read_to_string(&input).unwrap(), demo_source());
}

#[test]
fn reports_several_errors_separated_by_a_blank_line_and_a_count() {
    let directory = Scratch::new("errors");
    let input = directory.path().join("bad.raster");
    fs::write(
        &input,
        "frame display { at vblank {} }\nmain {\n    loop {}\n    wait vblank\n}\n",
    )
    .unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            concat!(
                "error: `at vblank` is not supported yet\n",
                " --> {path}:1:20\n",
                "  |\n",
                "1 | frame display {{ at vblank {{}} }}\n",
                "  |                    ^^^^^^ `at vblank` is not supported yet\n",
                "  = note: this release compiles `main`, `fn`, `if`, `while`, `for`, u8\n",
                "          arithmetic, and `ppu.*` / `mmc3.*` register writes; timed blocks\n",
                "          with `cycles`, `pad`, `sync exact` and `wait cycles`; and one\n",
                "          `frame` of `every ... scanlines` events\n",
                "\n",
                "error: `loop` is not supported yet\n",
                " --> {path}:3:5\n",
                "  |\n",
                "3 |     loop {{}}\n",
                "  |     ^^^^^^^ `loop` is not supported yet\n",
                "\n",
                "error: only `wait cycles` is supported yet; frame waits arrive with frame scheduling\n",
                " --> {path}:4:5\n",
                "  |\n",
                "4 |     wait vblank\n",
                "  |     ^^^^^^^^^^^ only `wait cycles` is supported yet; frame waits arrive with frame scheduling\n",
                "\n",
                "error: could not compile {path} (3 errors)\n"
            ),
            path = input.display()
        )
    );
}

#[test]
fn timing_overage_diagnostic_names_cost_budget_and_span() {
    let directory = Scratch::new("overbudget");
    let input = directory.path().join("hblank.raster");
    fs::write(
        &input,
        "main {\n    sync exact\n    cycles(<= 4) {\n        ppu.mask = 1\n    }\n}\n",
    )
    .unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            concat!(
                "error: timed block exceeds its budget\n",
                " --> {path}:3:5\n",
                "  |\n",
                "3 |     cycles(<= 4) {{\n",
                "  |     ^^^^^^^^^^^^ block costs 15 cycles, budget is 4\n",
                "  = note: an indexed read that may cross a page and a branch that may be\n",
                "          taken are both charged their worst case\n",
                "\n",
                "error: could not compile {path} (1 error)\n",
            ),
            path = input.display()
        )
    );
}

#[test]
fn a_report_region_prints_its_measured_cost_in_the_build_summary() {
    let directory = Scratch::new("cyclesreport");
    let input = directory.path().join("report.raster");
    let output = directory.path().join("report.nes");
    fs::write(
        &input,
        "main {\n    sync exact\n    cycles(?) hblank {\n        ppu.mask = 1\n    }\n}\n",
    )
    .unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Ok(()));
    assert_eq!(stderr, "");
    assert!(
        stdout.contains("   cycles  hblank: 15 cycles\n"),
        "the build summary reports a `cycles(?)` region, got:\n{stdout}"
    );
    assert!(output.exists());
}

#[test]
fn a_block_one_cycle_short_of_its_budget_says_why_it_cannot_be_padded() {
    let directory = Scratch::new("unpaddable");
    let input = directory.path().join("short.raster");
    fs::write(
        &input,
        "main {\n    sync exact\n    cycles(16) pad {\n        ppu.mask = 1\n    }\n}\n",
    )
    .unwrap();

    let (_, _, stderr) = run_capturing(vec![input.display().to_string()]);

    assert!(
        stderr.contains("error: a timed block cannot be padded to its budget"),
        "got:\n{stderr}"
    );
    assert!(
        stderr.contains("no instruction costs a single cycle"),
        "got:\n{stderr}"
    );
}

#[test]
fn a_block_short_of_an_exact_budget_is_told_about_pad() {
    let directory = Scratch::new("underbudget");
    let input = directory.path().join("under.raster");
    fs::write(
        &input,
        "main {\n    sync exact\n    cycles(25) {\n        ppu.mask = 1\n    }\n}\n",
    )
    .unwrap();

    let (_, _, stderr) = run_capturing(vec![input.display().to_string()]);

    assert!(
        stderr.contains("error: timed block does not fill its budget"),
        "got:\n{stderr}"
    );
    assert!(
        stderr.contains("`pad` would fill the remaining 10 cycles"),
        "got:\n{stderr}"
    );
}

#[test]
fn a_bank_select_warning_prints_to_stderr_and_still_writes_a_rom() {
    let directory = Scratch::new("bankwarning");
    let input = directory.path().join("invert.raster");
    let output = directory.path().join("invert.nes");
    fs::write(
        &input,
        "main {\n    ppu.mask = 0\n    mmc3.bank_select = $80\n}\n",
    )
    .unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Ok(()));
    assert!(output.exists());
    assert_eq!(
        stderr,
        format!(
            concat!(
                "warning: this bank select changes the MMC3 mapping mode\n",
                " --> {path}:3:5\n",
                "  |\n",
                "3 |     mmc3.bank_select = $80\n",
                "  |     ^^^^^^^^^^^^^^^^^^^^^^ bit 7 swaps the two pattern tables from here on\n",
                "  = note: reset chose CHR A12 inversion off, so pattern table 0 is at\n",
                "          PPU $0000; clearing bit 7 keeps that map\n",
                "  = note: bits 6 and 7 take effect from whichever bank select was\n",
                "          written last, not from the bank data that follows\n",
                "\n",
            ),
            path = input.display()
        )
    );
    assert_eq!(
        stdout,
        summary_of(&input, &output, 145, 13, 0xE090, " warnings  1\n")
    );
}

#[test]
fn a_build_that_fails_counts_its_errors_and_its_warnings() {
    let directory = Scratch::new("warnandfail");
    let input = directory.path().join("both.raster");
    let output = directory.path().join("both.nes");
    fs::write(
        &input,
        "main {\n    mmc3.bank_select = $80\n    loop {}\n}\n",
    )
    .unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert!(!output.exists());
    assert!(
        stderr.starts_with("warning: this bank select changes the MMC3 mapping mode\n"),
        "the warning comes first, got:\n{stderr}"
    );
    assert!(
        stderr.ends_with(&format!(
            "error: could not compile {path} (1 error, 1 warning)\n",
            path = input.display()
        )),
        "both severities are counted, got:\n{stderr}"
    );
}

#[test]
fn two_warnings_and_an_error_are_counted_in_the_plural() {
    let directory = Scratch::new("plural");
    let input = directory.path().join("plural.raster");
    fs::write(
        &input,
        "main {\n    mmc3.bank_select = $80\n    mmc3.bank_select = $40\n    loop {}\n}\n",
    )
    .unwrap();

    let (result, _stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Err(1));
    assert!(
        stderr.ends_with(&format!(
            "error: could not compile {path} (1 error, 2 warnings)\n",
            path = input.display()
        )),
        "each severity is pluralised on its own count, got:\n{stderr}"
    );
}

#[test]
fn an_empty_main_is_three_bytes_of_the_author_s_own() {
    let directory = Scratch::new("emptymain");
    let input = directory.path().join("empty.raster");
    let output = directory.path().join("empty.nes");
    fs::write(&input, "main {\n}\n").unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Ok(()));
    assert_eq!(stderr, "");
    // The three bytes are the halt loop rasterc puts at the end of every `main`.
    // They are the author's: the line is drawn at the linker, so `yours` is
    // never 0 and never 1, and neither row needs a plural.
    assert_eq!(stdout, summary_of(&input, &output, 135, 3, 0xE086, ""));
}

#[test]
fn a_program_too_big_for_the_bank_is_told_how_much_of_its_own_has_to_go() {
    let directory = Scratch::new("overflow");
    let input = directory.path().join("huge.raster");
    let mut source = String::from("main {\n");
    for _ in 0..1700 {
        source.push_str("    ppu.mask = 1\n");
    }
    source.push_str("}\n");
    fs::write(&input, source).unwrap();

    let (result, stdout, stderr) = run_capturing(vec![input.display().to_string()]);

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("error: the program does not fit the MMC3 fixed bank"),
        "got:\n{stderr}"
    );
    assert!(
        stderr.contains("132 of those are the reset runtime, so "),
        "got:\n{stderr}"
    );
    assert!(
        stderr.contains("bytes of your own\n          have to go"),
        "got:\n{stderr}"
    );
}
