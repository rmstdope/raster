//! The per-event budget table of spec section 7.2, as an author reads it in the terminal.

use std::{fs, io::Cursor};

use rasterc::run;

#[path = "common/scratch.rs"]
mod scratch;
use scratch::Scratch;

/// Build `source` and return the result with stdout and stderr.
fn build(source: &str) -> (Result<(), i32>, String, String) {
    // Scratch names are stamped with the clock, whose resolution is coarse enough for two
    // parallel tests to collide, so each test also names itself.
    let label = std::thread::current()
        .name()
        .unwrap_or("budget")
        .replace("::", "-");
    let directory = Scratch::new(&label);
    let input = directory.path().join("bars.raster");
    fs::write(&input, source).expect("the fixture is writable");
    let mut stdout = Cursor::new(Vec::new());
    let mut stderr = Cursor::new(Vec::new());
    let result = run(vec![input.display().to_string()], &mut stdout, &mut stderr);
    (
        result,
        String::from_utf8(stdout.into_inner()).expect("UTF-8"),
        String::from_utf8(stderr.into_inner()).expect("UTF-8"),
    )
}

fn program(using: &str, first: &str, second: &str) -> String {
    format!(
        "main {{\n    ppu.ctrl = $10\n    ppu.mask = $18\n}}\nframe bars {using} {{\n    at scanline 60 {{\n{first}\n    }}\n    every 8 scanlines from 96 to 200 {{\n{second}\n    }}\n}}\n"
    )
}

/// Everything from the blank line after the summary on.
fn table_of(stdout: &str) -> &str {
    stdout
        .split_once("\n\n")
        .expect("a blank line follows the summary")
        .1
}

#[test]
fn an_irq_frame_prints_its_table_after_the_summary() {
    let (result, stdout, _) = build(&program("using irq", "ppu.mask = $1f", "ppu.mask = $1e"));

    assert_eq!(result, Ok(()));
    assert!(stdout.contains("    entry  reset $E000"), "{stdout}");
    assert_eq!(
        table_of(&stdout),
        "frame bars (using irq, mmc3)\n  scanline  60             handler  6 cycles   window   9   3 spare\n  scanline  96-200 every 8 handler  6 cycles   window   9   3 spare\n"
    );
}

#[test]
fn an_omitted_using_is_timed_and_says_by_default() {
    let (result, stdout, _) = build(&program("", "ppu.mask = $1f", "ppu.mask = $1e"));

    assert_eq!(result, Ok(()));
    assert_eq!(
        table_of(&stdout),
        "frame bars (using timed by default, mmc3)\n  scanline  60             handler 15 cycles   window 114   99 spare\n  scanline  96-200 every 8 handler 15 cycles   window 114   99 spare\n"
    );
}

#[test]
fn a_written_timed_does_not_say_by_default() {
    let (_, stdout, _) = build(&program("using timed", "ppu.mask = $1f", "ppu.mask = $1e"));

    assert!(
        table_of(&stdout).starts_with("frame bars (using timed, mmc3)\n"),
        "{stdout}"
    );
}

#[test]
fn a_program_with_no_frame_prints_no_table() {
    let (result, stdout, _) = build("main {\n    ppu.mask = $18\n}\n");

    assert_eq!(result, Ok(()));
    assert!(!stdout.contains("\nframe "), "{stdout}");
    assert!(stdout.lines().last().unwrap().contains("entry"), "{stdout}");
    assert!(!stdout.ends_with("\n\n"), "{stdout}");
}

#[test]
fn the_table_follows_the_cycles_rows() {
    let source = "main {\n    ppu.ctrl = $10\n    ppu.mask = $18\n    sync exact\n    cycles(?) hblank {\n        ppu.mask = $18\n    }\n}\nframe bars using irq {\n    at scanline 60 {\n        ppu.mask = $1f\n    }\n}\n";
    let (result, stdout, _) = build(source);

    assert_eq!(result, Ok(()));
    let cycles = stdout.find("   cycles  ").expect("a cycles row");
    let table = stdout.find("\n\nframe bars").expect("a table");
    assert!(cycles < table, "{stdout}");
    assert!(stdout.ends_with("3 spare\n"), "{stdout}");
}

#[test]
fn a_handler_over_its_window_prints_the_table_before_the_count() {
    let (result, stdout, stderr) = build(&program(
        "using irq",
        "        ppu.mask = $1f\n        ppu.scroll = 0",
        "        ppu.mask = $1e",
    ));

    assert_eq!(result, Err(1));
    assert_eq!(stdout, "");
    assert!(stderr.contains("error: an `irq` handler exceeds its hblank"));
    let (diagnostics, tail) = stderr
        .split_once("\nframe bars")
        .expect("the table follows the diagnostic");
    assert!(!diagnostics.contains("could not compile"));
    let (table, count) = tail
        .split_once("error: could not compile ")
        .expect("the count line comes last");
    assert_eq!(
        table,
        " (using irq, mmc3)\n  scanline  60             handler 12 cycles   window   9   OVER BY 3\n  scanline  96-200 every 8 handler  6 cycles   window   9   3 spare\n\n"
    );
    assert!(count.ends_with("bars.raster (1 error)\n"), "{count}");
}

#[test]
fn several_handlers_over_are_one_diagnostic_each_and_one_table() {
    let (result, _, stderr) = build(&program(
        "using irq",
        "        ppu.mask = $1f\n        ppu.scroll = 0",
        "        ppu.mask = $1e\n        ppu.scroll = 0\n        ppu.scroll = 0",
    ));

    assert_eq!(result, Err(1));
    assert_eq!(stderr.matches("exceeds its hblank").count(), 2, "{stderr}");
    assert_eq!(stderr.matches("OVER BY").count(), 2, "{stderr}");
    assert_eq!(stderr.matches("\nframe bars").count(), 1, "{stderr}");
    assert!(stderr.contains("(2 errors)\n"), "{stderr}");
}

#[test]
fn a_timed_handler_over_its_scanline_prints_the_table_too() {
    let (result, _, stderr) = build(&program(
        "using timed",
        "        cycles(200) pad {\n            ppu.addr = $3f\n        }",
        "        ppu.mask = $1e",
    ));

    assert_eq!(result, Err(1));
    assert!(
        stderr.contains("timed block exceeds its budget"),
        "{stderr}"
    );
    assert!(stderr.contains("window 114   OVER BY "), "{stderr}");
    assert!(
        stderr.contains("99 spare\n\nerror: could not compile"),
        "{stderr}"
    );
}

#[test]
fn a_failure_that_is_not_a_handler_overrun_prints_no_table() {
    let mut source = String::from("main {\n");
    for _ in 0..1700 {
        source.push_str("    ppu.mask = $18\n");
    }
    source.push_str(
        "}\nframe bars using timed {\n    at scanline 60 {\n        ppu.mask = $1f\n    }\n}\n",
    );
    let (result, _, stderr) = build(&source);

    assert_eq!(result, Err(1));
    assert!(
        stderr.contains("does not fit the MMC3 fixed bank"),
        "{stderr}"
    );
    assert!(!stderr.contains("frame bars"), "{stderr}");
}

#[test]
fn rows_follow_the_order_the_frame_block_writes_them() {
    let source = "main {\n    ppu.ctrl = $10\n    ppu.mask = $18\n}\nframe bars using irq {\n    every 8 scanlines from 96 to 200 {\n        ppu.mask = $1e\n    }\n    at scanline 0 {\n        ppu.mask = $1f\n    }\n}\n";
    let (_, stdout, _) = build(source);

    let every = stdout
        .find("scanline  96-200 every 8")
        .expect("the every row");
    let at = stdout.find("scanline  0 ").expect("the at row");
    assert!(every < at, "{stdout}");
}

#[test]
fn an_earlier_overrun_is_still_told_when_a_later_error_stops_the_build() {
    let source = "main {\n    ppu.ctrl = $10\n    ppu.mask = $18\n}\nframe bars using irq {\n    at scanline 60 {\n        ppu.mask = $1f\n        ppu.scroll = 0\n    }\n    at scanline 100 {\n        cycles(4) {\n            ppu.mask = $1f\n        }\n    }\n}\n";
    let (result, _, stderr) = build(source);

    assert_eq!(result, Err(1));
    assert!(stderr.contains("exceeds its hblank"), "{stderr}");
    assert!(
        stderr.contains("timed block exceeds its budget"),
        "{stderr}"
    );
    assert!(stderr.contains("(2 errors)"), "{stderr}");
    assert!(
        !stderr.contains("frame bars"),
        "no table when a handler was not measured: {stderr}"
    );
}

#[test]
fn the_table_follows_the_warnings_row() {
    let (result, stdout, _) = build(&program("using timed", "ppu.ctrl = $80", "ppu.mask = $1e"));

    assert_eq!(result, Ok(()));
    let warnings = stdout.find(" warnings  ").expect("a warnings row");
    let table = stdout.find("\n\nframe bars").expect("a table");
    assert!(warnings < table, "{stdout}");
}

#[test]
fn consecutive_scanlines_show_the_tighter_timed_window() {
    let source = "main {\n    ppu.mask = $18\n}\nframe bars using timed {\n    every 1 scanlines from 10 to 14 {\n        ppu.mask = $1e\n    }\n}\n";
    let (_, stdout, _) = build(source);

    assert!(stdout.contains("window 113   98 spare"), "{stdout}");
}
