use rasterc::compile_source;

mod common;
use common::demo_source;

#[test]
fn compiles_the_demo_to_the_same_rom_as_m1() {
    let rom = compile_source(&demo_source()).expect("the demo compiles");

    assert_eq!(rom.image, raster_link::m1_solid_backdrop_rom());
    assert_eq!(rom.code_len, 160);
}

const SUPPORTED_SUBSET: &str = concat!(
    "this release compiles `main`, `fn`, `if`, `while`, `for`, u8\n",
    "arithmetic and `ppu.*` / `mmc3.*` register writes"
);

#[test]
fn reports_every_error_from_the_failing_stage() {
    let diagnostics =
        compile_source("frame display { at vblank {} }\nmain {\n    loop {}\n    wait vblank\n}\n")
            .expect_err("three unsupported constructs are three errors");

    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        [
            "`at vblank` is not supported yet",
            "`loop` is not supported yet",
            "only `wait cycles` is supported yet; frame waits arrive with frame scheduling",
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

#[test]
fn a_program_with_no_main_says_where_a_rom_starts() {
    let diagnostics =
        compile_source("const A: u8 = $01\n").expect_err("a ROM needs somewhere to start");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "this program has no `main` block");
    assert_eq!(diagnostics[0].span, None);
    assert_eq!(
        diagnostics[0].notes,
        ["add `main { ... }` to give the ROM somewhere to start"]
    );
}

#[test]
fn too_many_variables_names_the_zero_page_that_ran_out() {
    let declarations = (0..241)
        .map(|index| format!("var value{index}: u8"))
        .collect::<Vec<_>>()
        .join("\n");
    let diagnostics = compile_source(&format!("{declarations}\nmain {{}}\n"))
        .expect_err("the zero page is finite");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "too many variables for the zero page"
    );
    // This one does point somewhere: the declaration that did not fit.
    assert!(diagnostics[0].span.is_some());
    assert_eq!(
        diagnostics[0].notes,
        ["the zero page holds 240 variables, from $10 to $FF"]
    );
}

#[test]
fn a_program_too_large_names_the_bank_it_does_not_fit() {
    let body = "    ppu.data = $01\n".repeat(2000);
    let diagnostics =
        compile_source(&format!("main {{\n{body}}}\n")).expect_err("the fixed bank is finite");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "the program does not fit the MMC3 fixed bank"
    );
    assert_eq!(diagnostics[0].span, None);
    assert_eq!(diagnostics[0].notes.len(), 2);
    assert!(
        diagnostics[0].notes[0].ends_with(" bytes of code, and $E000-$FFFF holds 8186"),
        "unexpected note: {}",
        diagnostics[0].notes[0]
    );
    assert_eq!(
        diagnostics[0].notes[1],
        "PRG bank switching is not supported yet, so all code lives\nin the fixed bank"
    );
}

#[test]
fn a_cycle_annotated_function_that_returns_still_names_the_real_refusal() {
    let diagnostics = compile_source("fn f() -> u8 cycles(20) {\n    return 1\n}\nmain { }\n")
        .expect_err("function timing specifications are not supported yet");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "function timing specifications are not supported"
    );
}

#[test]
fn a_frame_too_large_for_the_bank_says_the_schedule_is_emitted_three_times() {
    // Every handler fits its scanline; there are simply too many of them for the bank.
    let diagnostics = compile_source(
        "main { ppu.mask = 0 }\n\
         frame bars using timed {\n\
         \x20   every 1 scanlines from 0 to 239 { ppu.mask = 0 }\n\
         }\n",
    )
    .expect_err("the fixed bank is finite");

    assert_eq!(
        diagnostics[0].message,
        "the program does not fit the MMC3 fixed bank"
    );
    assert!(
        diagnostics[0]
            .notes
            .iter()
            .any(|note| note.contains("costs three times its own size")),
        "the author is owed the reason their program measures three times its size: {:?}",
        diagnostics[0].notes
    );
}

/// The counter clocks on filtered PPU A12 rises, so both pattern tables in one half is a ROM that
/// builds and never interrupts. The diagnostic names the value that did it, because `$00` and `$18`
/// are both plausible things to have written on purpose.
#[test]
fn irq_frame_rejects_same_half_background_and_sprite_patterns() {
    let source = "main {\n    ppu.ctrl = $18\n    ppu.mask = $1e\n}\n\
                  \nframe bars using irq {\n    at scanline 60 { ppu.data = $12 }\n}\n";
    let diagnostics = compile_source(source).expect_err("A12 never rises with one half configured");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "`using irq` needs the background and sprite pattern tables in opposite halves, \
         and `ppu.ctrl = $18` puts both at $1000"
    );
    assert!(
        diagnostics[0].span.is_some(),
        "the diagnostic is source-spanned"
    );
}

#[test]
fn irq_frame_rejects_a_schedule_the_ppu_would_never_clock() {
    let source = "main {\n    ppu.ctrl = $08\n    ppu.mask = $06\n}\n\
                  \nframe bars using irq {\n    at scanline 60 { ppu.data = $12 }\n}\n";
    let diagnostics = compile_source(source).expect_err("a PPU that fetches nothing moves no A12");

    assert_eq!(
        diagnostics[0].message,
        "`using irq` needs rendering enabled, and `ppu.mask = $06` enables neither the \
         background nor the sprites"
    );
}

/// Rendering one half is rendering: the sprite pattern fetches happen either way, so a
/// background-only split - the commonest MMC3 IRQ program there is - compiles.
#[test]
fn irq_frame_accepts_a_background_only_schedule() {
    let source = "main {\n    ppu.ctrl = $08\n    ppu.mask = $0a\n}\n\
                  \nframe bars using irq {\n    at scanline 60 { ppu.mask = $2a }\n}\n";

    compile_source(source)
        .map(|_| ())
        .expect("background rendering alone still clocks the counter");
}

/// A configuration written through a call in value position is read like any other: the walk
/// follows the program, and a call is a call wherever its result goes.
#[test]
fn irq_frame_reads_a_ppu_configuration_written_through_a_value_call() {
    let spoiled = "fn spoil() -> u8 {\n    ppu.mask = $00\n    return 1\n}\n\
                   \nvar x: u8\n\nmain {\n    ppu.ctrl = $08\n    ppu.mask = $1e\n\
                   \n    x = spoil()\n}\n\
                   \nframe bars using irq {\n    at scanline 60 { ppu.mask = $3e }\n}\n";
    let diagnostics =
        compile_source(spoiled).expect_err("the last write to ppu.mask turned rendering back off");
    assert!(
        diagnostics[0]
            .message
            .starts_with("`using irq` needs rendering enabled"),
        "found {:?}",
        diagnostics[0].message
    );

    let configured =
        "fn setup() -> u8 {\n    ppu.ctrl = $08\n    ppu.mask = $1e\n    return 0\n}\n\
                      \nvar seed: u8 = setup()\n\nmain {\n    ppu.data = $00\n}\n\
                      \nframe bars using irq {\n    at scanline 60 { ppu.mask = $3e }\n}\n";
    compile_source(configured)
        .map(|_| ())
        .expect("an initializer's call configures the PPU as surely as a statement does");
}

/// `return` in an IRQ handler would leave the interrupt without its `RTI`, three bytes of stack at
/// a time — a ROM that runs for eighty frames and then wanders. It is refused for the same reason
/// a `return` inside any handler is, which is why this asserts the refusal rather than a message of
/// its own.
#[test]
fn a_frame_handler_cannot_return() {
    let source = "main {\n    ppu.ctrl = $08\n    ppu.mask = $1e\n}\n\
                  \nframe bars using irq {\n    at scanline 60 {\n        ppu.mask = $3e\n\
                  \n        return\n    }\n}\n";
    let diagnostics = compile_source(source).expect_err("a handler has nowhere to return to");

    assert!(
        diagnostics[0]
            .message
            .starts_with("`return` inside a timed block jumps out"),
        "found {:?}",
        diagnostics[0].message
    );
    assert!(diagnostics[0].span.is_some());
}

/// A register the compiler cannot read is refused rather than assumed: guessing would either
/// refuse a correct program or pass one that silently never interrupts.
#[test]
fn irq_frame_rejects_a_ppu_configuration_it_cannot_prove() {
    let source = "var setting: u8 = $1e\n\nmain {\n    ppu.ctrl = $08\n    ppu.mask = setting\n}\n\
                  \nframe bars using irq {\n    at scanline 60 { ppu.data = $12 }\n}\n";
    let diagnostics = compile_source(source).expect_err("a variable mask cannot be checked");

    assert_eq!(
        diagnostics[0].message,
        "`using irq` needs a constant `ppu.mask` before the frame, and this program's last \
         write to it is not one"
    );
}

/// A `ppu.ctrl` written by a function `main` calls is as good as one written in `main`: the
/// configuration is read through the calls in the order they run.
#[test]
fn irq_frame_reads_a_ppu_configuration_written_by_a_called_function() {
    let source = "fn configure() {\n    ppu.ctrl = $08\n    ppu.mask = $1e\n}\n\
                  \nmain {\n    configure()\n}\n\
                  \nframe bars using irq {\n    at scanline 60 { ppu.data = $12 }\n}\n";

    compile_source(source)
        .map(|_| ())
        .expect("the configuration a called function wrote is the one the frame inherits");
}

/// Where `address` sits in a linked image: the fixed bank is the last 8 KiB of PRG ROM, and it is
/// mapped at `$E000`.
fn fixed_bank_offset(address: u16) -> usize {
    use raster_link::{
        INES_HEADER_SIZE, MMC3_FIXED_BANK_SIZE, MMC3_FIXED_BANK_START, MMC3_PRG_ROM_SIZE,
    };
    INES_HEADER_SIZE + MMC3_PRG_ROM_SIZE - MMC3_FIXED_BANK_SIZE
        + usize::from(address - MMC3_FIXED_BANK_START)
}

/// A `frame ... using irq` is a chain of handlers, and every link of it is asserted here as bytes.
///
/// The order is the one spec section 7.3 requires and hardware does not forgive: the latch at
/// `$C000` before the reload request at `$C001`, and the acknowledgement at `$E000` before the
/// re-arm at `$E001`. Acknowledging after enabling would leave the line asserted and the console
/// would take the same interrupt for ever.
///
/// Nothing here is reached by falling through: the handlers sit past the frame loop's own `JMP`,
/// and control arrives through `$FFFE` — which is why the vectors and the `$E000` placement are
/// part of the same test rather than a separate one.
#[test]
fn irq_lowering_acknowledges_before_rearming_and_returns_from_fixed_bank() {
    use raster_link::MMC3_FIXED_BANK_START;

    let source = "main {\n    ppu.ctrl = $08\n    ppu.mask = $1e\n}\n\
                  \nframe bars using irq {\n    at scanline 60 { ppu.data = $12 }\n\
                  \n    at scanline 120 { ppu.data = $21 }\n}\n";
    let rom = compile_source(source).expect("the fixture compiles");
    let image = &rom.image;
    let at = |address: u16, length: usize| {
        let offset = fixed_bank_offset(address);
        &image[offset..offset + length]
    };

    // The IRQ vector is the frame's own entry, in the fixed bank, and not the runtime's bare `RTI`
    // that `$FFFA` still points at.
    assert!(rom.vectors.irq >= MMC3_FIXED_BANK_START);
    assert_ne!(rom.vectors.irq, rom.vectors.nmi);
    assert_eq!(
        at(rom.vectors.nmi, 1),
        &[0x40],
        "the NMI vector is an `RTI`"
    );
    // `JMP ($000E)`: the entry dispatches through the two RAM bytes each handler leaves pointing at
    // its successor, which is what makes one vector serve a whole chain.
    assert_eq!(at(rom.vectors.irq, 3), &[0x6c, 0x0e, 0x00]);

    // The frame loop arms the first handler in vblank: the dispatch vector, then the latch, the
    // reload request, and the enable.
    let armed = image
        .windows(8)
        .position(|window| {
            window[0] == 0xa9
                && window[2..4] == [0x85, 0x0e]
                && window[4] == 0xa9
                && window[6..8] == [0x85, 0x0f]
        })
        .expect("the frame loop points the dispatch vector at its first handler");
    let first = u16::from_le_bytes([image[armed + 1], image[armed + 5]]);
    assert!(
        first >= MMC3_FIXED_BANK_START,
        "a handler is in the fixed bank"
    );
    assert_eq!(
        &image[armed + 8..armed + 8 + 11],
        // Armed from the pre-render line, an IRQ on scanline 60 is a latch of 60.
        &[0xa9, 60, 0x8d, 0x00, 0xc0, 0x8d, 0x01, 0xc0, 0x8d, 0x01, 0xe0],
        "the arming sequence latches, requests a reload, and enables"
    );

    // The first handler: its body, then the chain to the second, then back out of the interrupt.
    let chained = at(first, 30);
    assert_eq!(
        &chained[..20],
        &[
            0x48, // PHA — the spin loop the frame runs in holds nothing else
            0xa9, 0x12, 0x8d, 0x07, 0x20, // ppu.data = $12
            0xa9, 59, // 120 - 60 scanlines away is a latch of 59
            0x8d, 0x00, 0xc0, // $C000: the latch
            0x8d, 0x01, 0xc0, // $C001: reload on the next A12 rise
            0x8d, 0x00, 0xe0, // $E000: acknowledge and disable, before ...
            0x8d, 0x01, 0xe0, // ... $E001 re-arms the line
        ]
    );
    // The dispatch vector, left pointing at the handler that runs next.
    assert_eq!(chained[20], 0xa9);
    assert_eq!(&chained[22..24], &[0x85, 0x0e]);
    assert_eq!(chained[24], 0xa9);
    assert_eq!(&chained[26..28], &[0x85, 0x0f]);
    assert_eq!(&chained[28..30], &[0x68, 0x40], "PLA then RTI");
    let second = u16::from_le_bytes([chained[21], chained[25]]);

    // The last handler wraps: same shape, and a latch that counts across the vblank the counter
    // does not clock, back to the first handler of the next frame.
    assert!(second >= MMC3_FIXED_BANK_START);
    let wrapping = at(second, 30);
    assert_eq!(
        &wrapping[..20],
        &[
            0x48, //
            0xa9, 0x21, 0x8d, 0x07, 0x20, // ppu.data = $21
            0xa9, 180, // 241 counted rises a frame, less the 60 the schedule spans
            0x8d, 0x00, 0xc0, 0x8d, 0x01, 0xc0, // the latch, then the reload request
            0x8d, 0x00, 0xe0, 0x8d, 0x01, 0xe0, // acknowledge, then re-arm
        ]
    );
    assert_eq!(
        u16::from_le_bytes([wrapping[21], wrapping[25]]),
        first,
        "the last handler points the chain back at the first"
    );
    assert_eq!(&wrapping[28..30], &[0x68, 0x40], "PLA then RTI");
}

/// An IRQ chain emits each handler once, so the note the timed lowering earns would be a lie here —
/// and a lie about a factor of three is exactly the kind an author cannot check.
#[test]
fn an_irq_frame_too_large_for_the_bank_is_not_told_its_schedule_is_tripled() {
    let diagnostics = compile_source(
        "main {\n    ppu.ctrl = $08\n    ppu.mask = $1e\n}\n\
         frame bars using irq {\n\
         \x20   every 1 scanlines from 0 to 239 {\n\
         \x20       ppu.mask = $1e\n    ppu.mask = $3e\n    ppu.mask = $1e\n\
         \x20   }\n\
         }\n",
    )
    .expect_err("the fixed bank is finite");

    assert_eq!(
        diagnostics[0].message,
        "the program does not fit the MMC3 fixed bank"
    );
    assert!(
        !diagnostics[0]
            .notes
            .iter()
            .any(|note| note.contains("three times")),
        "an IRQ chain emits each handler once: {:?}",
        diagnostics[0].notes
    );
}

/// A schedule of one event chains to itself: the wrap is the whole frame, and the vector it leaves
/// behind points at the handler that just ran.
#[test]
fn a_single_event_irq_chain_wraps_onto_itself() {
    use raster_link::MMC3_FIXED_BANK_START;

    let source = "main {\n    ppu.ctrl = $08\n    ppu.mask = $1e\n}\n\
                  \nframe bars using irq {\n    at scanline 60 { ppu.mask = $3e }\n}\n";
    let rom = compile_source(source).expect("the fixture compiles");
    let image = &rom.image;

    let armed = image
        .windows(8)
        .position(|window| {
            window[0] == 0xa9
                && window[2..4] == [0x85, 0x0e]
                && window[4] == 0xa9
                && window[6..8] == [0x85, 0x0f]
        })
        .expect("the arming points the dispatch vector at the only handler");
    let handler = u16::from_le_bytes([image[armed + 1], image[armed + 5]]);
    assert!(handler >= MMC3_FIXED_BANK_START);

    let offset = fixed_bank_offset(handler);
    let emitted = &image[offset..offset + 28];
    assert_eq!(
        &emitted[..20],
        &[
            0x48, //
            0xa9, 0x3e, 0x8d, 0x01, 0x20, // ppu.mask = $3e
            0xa9, 240, // a whole frame of counted rises, less the one it starts from
            0x8d, 0x00, 0xc0, 0x8d, 0x01, 0xc0, //
            0x8d, 0x00, 0xe0, 0x8d, 0x01, 0xe0,
        ]
    );
    assert_eq!(
        u16::from_le_bytes([emitted[21], emitted[25]]),
        handler,
        "the only handler chains to itself"
    );
}
