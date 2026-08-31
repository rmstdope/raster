use std::path::PathBuf;

use rasterc::compile_source;

fn demo_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp/demo.raster");
    std::fs::read_to_string(&path).expect("the demo example is readable")
}

#[test]
fn compiles_the_demo_to_the_same_rom_as_m1() {
    let rom = compile_source(&demo_source()).expect("the demo compiles");

    assert_eq!(rom.image, raster_link::m1_solid_backdrop_rom());
    assert_eq!(rom.code_len, 100);
}

const SUPPORTED_SUBSET: &str = concat!(
    "this release compiles `main`, `fn`, `if`, `while`, `for`, u8\n",
    "arithmetic and `ppu.*` / `mmc3.*` register writes"
);

#[test]
fn reports_every_error_from_the_failing_stage() {
    let diagnostics = compile_source(
        "frame display { at vblank {} }\nmain {\n    cycles(2) {}\n    wait vblank\n}\n",
    )
    .expect_err("three unsupported constructs are three errors");

    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        [
            "`frame` blocks are not supported yet",
            "`cycles` blocks are not supported yet",
            "`wait` statements are not supported yet",
        ]
    );

    // The subset is listed once per run, beside the first refusal.
    assert_eq!(diagnostics[0].notes, [SUPPORTED_SUBSET]);
    assert!(diagnostics[1].notes.is_empty());
    assert!(diagnostics[2].notes.is_empty());
}

#[test]
fn stops_at_the_first_failing_stage() {
    let diagnostics =
        compile_source("@\nmain {\n    wait vblank\n}\n").expect_err("a syntax error is an error");

    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        ["expected a top-level declaration"]
    );
    assert!(diagnostics[0].notes.is_empty());
}
