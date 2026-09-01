use raster_6502::{AddressingMode, Instruction};
use raster_timing::{analyze, leaves_straight_line, CycleConstraint, TimedRegion, TimingError};

/// `LDA #$00`, two cycles.
fn load_immediate() -> Instruction {
    Instruction {
        opcode: 0xa9,
        mode: AddressingMode::Immediate,
        operand: Some(0),
    }
}

/// `STA $0010`, four cycles.
fn store_absolute() -> Instruction {
    Instruction {
        opcode: 0x8d,
        mode: AddressingMode::Absolute,
        operand: Some(0x0010),
    }
}

/// `JMP $0000`, three cycles, and the plainest thing that leaves the straight line.
fn jmp_absolute() -> Instruction {
    Instruction {
        opcode: 0x4c,
        mode: AddressingMode::Absolute,
        operand: Some(0),
    }
}

fn region(constraint: CycleConstraint, instructions: Vec<Instruction>) -> TimedRegion {
    TimedRegion {
        constraint,
        pad: false,
        interruptible: false,
        instructions,
    }
}

#[test]
fn exact_budget_rejects_under_and_over_budget() {
    let instructions = vec![load_immediate(), store_absolute()];

    assert_eq!(
        analyze(
            &region(CycleConstraint::Exact(6), instructions.clone()),
            true
        ),
        Ok(raster_timing::TimingReport {
            label: None,
            measured_cycles: 6,
            padding: Vec::new(),
        })
    );
    assert_eq!(
        analyze(
            &region(CycleConstraint::Exact(8), instructions.clone()),
            true
        ),
        Err(TimingError::UnderBudget {
            measured_cycles: 6,
            budget: 8,
        })
    );
    assert_eq!(
        analyze(
            &region(CycleConstraint::Exact(4), instructions.clone()),
            true
        ),
        Err(TimingError::OverBudget {
            measured_cycles: 6,
            budget: 4,
            oam_dma_cycles: 0,
        })
    );
}

#[test]
fn upper_bound_permits_a_cheaper_region_and_rejects_an_expensive_one() {
    let instructions = vec![load_immediate(), store_absolute()];

    assert_eq!(
        analyze(
            &region(CycleConstraint::AtMost(8), instructions.clone()),
            true
        )
        .expect("a region within its upper bound is accepted")
        .measured_cycles,
        6
    );
    assert_eq!(
        analyze(&region(CycleConstraint::AtMost(5), instructions), true),
        Err(TimingError::OverBudget {
            measured_cycles: 6,
            budget: 5,
            oam_dma_cycles: 0,
        })
    );
}

#[test]
fn report_constraint_returns_the_measured_total_under_its_label() {
    let report = analyze(
        &region(
            CycleConstraint::Report {
                label: "hblank".to_owned(),
            },
            vec![load_immediate(), store_absolute()],
        ),
        true,
    )
    .expect("a report constraint never fails");

    assert_eq!(report.label.as_deref(), Some("hblank"));
    assert_eq!(report.measured_cycles, 6);
    assert!(report.padding.is_empty());
}

#[test]
fn indexed_reads_and_branches_are_charged_their_worst_case_by_the_cost_model() {
    // `LDA $1234,X` costs four cycles, or five when the index crosses a page.
    let indexed = Instruction {
        opcode: 0xbd,
        mode: AddressingMode::AbsoluteX,
        operand: Some(0x1234),
    };
    // `BNE` costs two cycles, three when taken and four when the target crosses a page.
    let branch = Instruction {
        opcode: 0xd0,
        mode: AddressingMode::Relative,
        operand: Some(0),
    };

    // The branch cannot go through `analyze`, which refuses a region containing one — the worst
    // case of a branch is exactly what a flat sum cannot prove. `worst_case_cycles` is still the
    // worst-case model, and is called directly on slices `analyze` never sees.
    assert_eq!(raster_timing::worst_case_cycles(&[indexed, branch]), 9);
    // And `analyze` costs a region with that model rather than a plainer one.
    assert_eq!(
        analyze(
            &region(
                CycleConstraint::Report {
                    label: "worst".to_owned()
                },
                vec![indexed]
            ),
            true,
        )
        .expect("a report constraint never fails")
        .measured_cycles,
        5
    );
}

fn padded(constraint: CycleConstraint, instructions: Vec<Instruction>) -> TimedRegion {
    TimedRegion {
        constraint,
        pad: true,
        interruptible: false,
        instructions,
    }
}

fn padding_for(budget: u32, legal_isa: bool) -> Vec<Instruction> {
    analyze(
        &padded(CycleConstraint::Exact(budget), vec![load_immediate()]),
        legal_isa,
    )
    .expect("a padded region within its budget is accepted")
    .padding
}

/// The cost the padding itself carries, measured through the same cost table.
fn cost_of(instructions: &[Instruction]) -> u32 {
    raster_6502::cycles(instructions, raster_6502::CycleContext::default())
}

#[test]
fn pad_fills_exact_budget_with_legal_or_compact_nops() {
    // `LDA #$00` costs two, so a budget of N leaves N - 2 cycles to fill.
    for budget in [2, 4, 5, 6, 7, 8, 9, 20, 33] {
        let padding = padding_for(budget, true);
        assert_eq!(
            cost_of(&padding),
            budget - 2,
            "legal padding for a budget of {budget}"
        );
        assert!(
            padding
                .iter()
                .all(|instruction| raster_6502::opcode(instruction.opcode).official),
            "legal padding for a budget of {budget} uses only official opcodes"
        );
    }

    for budget in [2, 4, 5, 6, 7, 8, 9, 20, 33] {
        let padding = padding_for(budget, false);
        assert_eq!(
            cost_of(&padding),
            budget - 2,
            "compact padding for a budget of {budget}"
        );
    }
}

#[test]
fn compact_padding_is_never_longer_than_legal_padding() {
    for budget in 4..64 {
        assert!(
            padding_for(budget, false).len() <= padding_for(budget, true).len(),
            "compact padding for a budget of {budget}"
        );
    }
}

#[test]
fn a_padded_block_over_its_budget_is_rejected_rather_than_shortened() {
    assert_eq!(
        analyze(
            &padded(
                CycleConstraint::Exact(4),
                vec![load_immediate(), store_absolute()]
            ),
            true,
        ),
        Err(TimingError::OverBudget {
            measured_cycles: 6,
            budget: 4,
            oam_dma_cycles: 0,
        })
    );
}

#[test]
fn a_single_unreachable_cycle_of_padding_is_rejected() {
    assert_eq!(
        analyze(
            &padded(CycleConstraint::Exact(3), vec![load_immediate()]),
            true,
        ),
        Err(TimingError::UnreachablePadding { remaining: 1 })
    );
}

#[test]
fn an_upper_bound_with_pad_is_filled_to_its_bound() {
    let report = analyze(
        &padded(CycleConstraint::AtMost(10), vec![load_immediate()]),
        true,
    )
    .expect("a padded region within its upper bound is accepted");

    assert_eq!(report.measured_cycles, 2);
    assert_eq!(cost_of(&report.padding), 8);
}

#[test]
fn delay_for_two_cycles_and_large_budget_has_exact_cost() {
    for requested in [
        2, 3, 4, 5, 6, 7, 8, 20, 100, 1792, 1793, 1800, 29780, 100_000,
    ] {
        let plan = raster_timing::plan_delay(requested, false)
            .unwrap_or_else(|_| panic!("a delay of {requested} cycles is synthesizable"));
        assert_eq!(
            raster_timing::delay_cycles(&plan),
            requested,
            "delay of {requested} cycles"
        );
    }
}

#[test]
fn every_delay_in_a_representative_range_costs_exactly_what_was_asked() {
    for requested in 2..600 {
        for legal_isa in [true, false] {
            let plan = raster_timing::plan_delay(requested, legal_isa)
                .unwrap_or_else(|_| panic!("a delay of {requested} cycles is synthesizable"));
            assert_eq!(
                raster_timing::delay_cycles(&plan),
                requested,
                "delay of {requested} cycles, legal_isa = {legal_isa}"
            );
        }
    }
}

#[test]
fn a_delay_below_two_cycles_is_rejected() {
    for requested in [0, 1] {
        assert_eq!(
            raster_timing::plan_delay(requested, false),
            Err(TimingError::DelayTooShort {
                requested_cycles: requested
            })
        );
    }
}

#[test]
fn a_long_delay_stays_a_handful_of_steps() {
    let plan = raster_timing::plan_delay(29780, false).expect("a frame-length delay");
    assert!(
        plan.len() <= 3,
        "a frame-length delay is {} steps",
        plan.len()
    );
}

/// A counted loop takes its branch exactly once, whatever its iteration count. That is the whole
/// reason it closes on a `JMP`: a `DEX`/`BNE` loop takes its branch on every pass but the last, and
/// each taken branch carries a page-crossing penalty nothing at this level can prove away.
#[test]
fn a_counted_loop_takes_its_branch_once_however_long_it_runs() {
    for iterations in [1, 2, 37, 256] {
        let step = raster_timing::DelayStep::Loop {
            outer: None,
            inner: iterations,
        };
        // `LDX` 2, then per pass `DEX` 2 and `BEQ` 2, plus `JMP` 3 on every pass but the last,
        // plus the one cycle the single taken branch costs.
        let expected = 2 + iterations * 4 + (iterations - 1) * 3 + 1;
        assert_eq!(
            raster_timing::delay_cycles(std::slice::from_ref(&step)),
            expected,
            "a loop of {iterations} iterations"
        );
    }
}

/// Every instruction the predicate must name, built from the opcode table so the test cannot
/// disagree with the cost model about what an instruction is.
fn instruction(code: u8) -> Instruction {
    let definition = raster_6502::opcode(code);
    Instruction {
        opcode: code,
        mode: definition.mode,
        operand: (definition.bytes > 1).then_some(0),
    }
}

#[test]
fn leaves_straight_line_accepts_every_branch_jump_call_and_return() {
    for code in [
        0x10, 0x30, 0x50, 0x70, 0x90, 0xb0, 0xd0, 0xf0, // every branch
        0x00, // BRK
        0x20, // JSR
        0x40, // RTI
        0x4c, // JMP $nnnn
        0x60, // RTS
        0x6c, // JMP ($nnnn)
    ] {
        assert!(
            leaves_straight_line(&instruction(code)),
            "${code:02X} leaves the straight line"
        );
    }

    for code in [
        0xea, 0x85, 0x04, 0x14, // the filler synthesize_padding emits
        0x08, 0x78, 0x28, // PHP, SEI, PLP — the block's own prologue and epilogue
        0xa9, 0x8d, // LDA #, STA $nnnn
    ] {
        assert!(
            !leaves_straight_line(&instruction(code)),
            "${code:02X} stays in the straight line"
        );
    }
}

#[test]
fn a_region_containing_a_jump_is_refused_under_every_constraint() {
    let instructions = vec![load_immediate(), jmp_absolute(), store_absolute()];

    for constraint in [
        CycleConstraint::Exact(20),
        CycleConstraint::AtMost(20),
        CycleConstraint::Report {
            label: "hblank".to_owned(),
        },
    ] {
        assert_eq!(
            analyze(&region(constraint.clone(), instructions.clone()), true),
            Err(TimingError::ControlFlowInRegion {
                index: 1,
                opcode: 0x4c,
            }),
            "{constraint:?} is refused a region that jumps"
        );
    }
}

#[test]
fn the_first_control_flow_instruction_is_the_one_reported() {
    let instructions = vec![
        instruction(0xf0),
        load_immediate(),
        jmp_absolute(),
        store_absolute(),
    ];

    assert_eq!(
        analyze(&region(CycleConstraint::AtMost(40), instructions), true),
        Err(TimingError::ControlFlowInRegion {
            index: 0,
            opcode: 0xf0,
        })
    );
}

/// What a caller spends returning to the top of the pass — `JMP absolute`, for the tests here.
const CLOSING_CYCLES: u32 = 3;

#[test]
fn a_pass_of_three_frames_is_a_whole_number_of_cpu_cycles() {
    // One frame is 89342 dots, which is 29780 CPU cycles and two dots: a loop that runs one
    // frame's worth of cycles cannot stay locked to the picture. Three of them can.
    assert_eq!(
        raster_timing::PASS_CYCLES * raster_timing::DOTS_PER_CPU_CYCLE,
        raster_timing::FRAMES_PER_PASS
            * raster_timing::SCANLINES_PER_FRAME
            * raster_timing::DOTS_PER_SCANLINE
    );
    assert_eq!(raster_timing::PASS_CYCLES, 89342);
}

#[test]
fn a_scanline_starts_a_whole_number_of_cycles_after_the_pass_origin() {
    // Vblank is twenty scanlines and the pre-render line is one more, so the visible picture
    // starts twenty-one scanlines after the origin the vblank poll establishes.
    assert_eq!(raster_timing::scanline_origin_cycles(0, 0), 2387);
    // 341 dots is 113.667 CPU cycles, so consecutive scanlines are 114, 113, 114 apart and every
    // three of them are exactly 341 cycles.
    let starts: Vec<_> = (0..4)
        .map(|scanline| raster_timing::scanline_origin_cycles(0, scanline))
        .collect();
    assert_eq!(starts, vec![2387, 2501, 2614, 2728]);
    assert_eq!(starts[3] - starts[0], 341);
    // The second and third frames of a pass are whole frames further on.
    assert_eq!(raster_timing::scanline_origin_cycles(1, 0), 32168);
}

#[test]
fn a_timed_frame_schedule_puts_every_handler_on_its_own_scanline_in_every_frame() {
    let scanlines = [0, 8, 9, 10, 200];
    let pass = raster_timing::plan_timed_frame(&scanlines, CLOSING_CYCLES);

    assert_eq!(
        pass.handlers.len(),
        scanlines.len() * raster_timing::FRAMES_PER_PASS as usize,
        "the schedule runs in each frame of the pass"
    );

    let mut position = 0;
    let mut handlers = pass.handlers.iter();
    for frame in 0..raster_timing::FRAMES_PER_PASS {
        for scanline in scanlines {
            let handler = handlers.next().expect("a handler per scanline per frame");
            position += handler.delay_cycles;
            assert_eq!(
                position,
                raster_timing::scanline_origin_cycles(frame, scanline),
                "the handler for scanline {scanline} of frame {frame} starts where it does"
            );
            position += handler.budget_cycles;
        }
    }

    // The pass closes on the cycle it opened on, which is what keeps it locked to the picture —
    // and the jump that closes it is inside that budget, not on top of it.
    assert_eq!(
        position + pass.trailing_delay_cycles + CLOSING_CYCLES,
        raster_timing::PASS_CYCLES
    );
}

#[test]
fn adjacent_handlers_carry_the_correction_that_keeps_the_frame_budget_exact() {
    let pass = raster_timing::plan_timed_frame(&[0, 1, 2, 3], CLOSING_CYCLES);
    let budgets: Vec<_> = pass.handlers[..4]
        .iter()
        .map(|handler| handler.budget_cycles)
        .collect();

    // A flat 114 every line would drift a third of a cycle a line; 114, 113, 114 does not.
    assert_eq!(budgets, vec![114, 113, 114, 114]);
    assert_eq!(budgets[0] + budgets[1] + budgets[2], 341);
    // Back-to-back handlers leave no gap to delay through.
    assert!(pass.handlers[1..4]
        .iter()
        .all(|handler| handler.delay_cycles == 0));
}

#[test]
fn a_handler_with_room_after_it_gets_one_scanline_body_and_a_delay_to_the_next() {
    let pass = raster_timing::plan_timed_frame(&[60, 120], CLOSING_CYCLES);

    assert_eq!(pass.handlers[0].budget_cycles, 114);
    assert_eq!(pass.handlers[1].budget_cycles, 114);
    assert_eq!(
        pass.handlers[0].delay_cycles,
        raster_timing::scanline_origin_cycles(0, 60)
    );
    assert_eq!(
        pass.handlers[1].delay_cycles,
        raster_timing::scanline_origin_cycles(0, 120)
            - raster_timing::scanline_origin_cycles(0, 60)
            - 114
    );
}

/// The MMC3 counter reloads on the A12 rise *after* the write that requested it, and asserts the
/// IRQ on the rise where the counter reaches zero — so a latch of `n` places the next IRQ `n + 1`
/// scanlines after the one being handled. Every case here is a delta between two events, which is
/// what a chained schedule is made of.
#[test]
fn mmc3_latch_for_delta_accounts_for_counter_offset() {
    for (delta, latch) in [
        (1u16, 0u8),
        (2, 1),
        (8, 7),
        (60, 59),
        (240, 239),
        (256, 255),
    ] {
        assert_eq!(
            raster_timing::mmc3_latch_for_delta(delta),
            latch,
            "an IRQ {delta} scanlines after this one needs a latch of {latch}"
        );
    }
}

/// The first IRQ of a frame is armed in vblank, so its reload lands on the pre-render line's own
/// A12 rise rather than one rise after an IRQ. That is one rise earlier than a chained link, and
/// it is exactly the difference that makes the first latch the scanline itself.
#[test]
fn the_first_irq_of_a_frame_is_armed_from_the_pre_render_line() {
    for (scanline, latch) in [(0u16, 0u8), (1, 1), (60, 60), (239, 239)] {
        assert_eq!(
            raster_timing::mmc3_latch_for_first_event(scanline),
            latch,
            "an IRQ on scanline {scanline} armed in vblank needs a latch of {latch}"
        );
    }
}

/// The counter clocks on filtered PPU A12 rises, and A12 only rises when a scanline fetches from
/// both halves of pattern memory. A layout with both tables in the same half is a ROM that looks
/// right and never takes an interrupt, which is the failure spec section 7.3 exists to prevent.
#[test]
fn mmc3_irq_needs_opposite_pattern_halves() {
    use raster_timing::{validate_mmc3_irq_frame, Mmc3IrqError, PpuConfiguration, RegisterState};

    let configuration = |ctrl: u8, mask: u8| PpuConfiguration {
        ctrl: RegisterState::Known(ctrl),
        mask: RegisterState::Known(mask),
    };

    // Background at $0000 and sprites at $1000, both halves of rendering on.
    assert_eq!(validate_mmc3_irq_frame(&configuration(0x08, 0x18)), Ok(()));
    // The other way round is just as valid.
    assert_eq!(validate_mmc3_irq_frame(&configuration(0x10, 0x1e)), Ok(()));

    for ctrl in [0x00, 0x18] {
        assert_eq!(
            validate_mmc3_irq_frame(&configuration(ctrl, 0x18)),
            Err(Mmc3IrqError::PatternTablesShareHalf { ctrl }),
            "${ctrl:02X} puts both pattern tables in one half"
        );
    }
}

/// Spec section 7.3 asks for rendering, not for both halves of it: the PPU runs its dot-257 to
/// dot-320 sprite pattern fetches whenever rendering is on, whether or not sprites are composited,
/// so a background-only split still clocks the counter. Measured on this project's own emulator.
#[test]
fn mmc3_irq_needs_rendering_enabled() {
    use raster_timing::{validate_mmc3_irq_frame, Mmc3IrqError, PpuConfiguration, RegisterState};

    let with_mask = |mask: u8| PpuConfiguration {
        ctrl: RegisterState::Known(0x08),
        mask: RegisterState::Known(mask),
    };

    for mask in [0x1e, 0x0a, 0x14, 0x08, 0x10] {
        assert_eq!(
            validate_mmc3_irq_frame(&with_mask(mask)),
            Ok(()),
            "${mask:02X} renders, so the PPU fetches and A12 moves"
        );
    }
    for mask in [0x00, 0x06, 0x01] {
        assert_eq!(
            validate_mmc3_irq_frame(&with_mask(mask)),
            Err(Mmc3IrqError::RenderingDisabled { mask }),
            "${mask:02X} renders nothing at all"
        );
    }
}

/// The rule the pre-frame check applies, exported so `raster-ir` can apply it to what a handler
/// stores. Either rendering bit alone still runs the sprite pattern fetches the counter clocks on;
/// the emphasis bits are not rendering and do not save a mask that clears both.
#[test]
fn either_half_of_rendering_keeps_the_counter_clocking() {
    use raster_timing::mask_enables_rendering;

    for mask in [0x1e, 0x0a, 0x14, 0x08, 0x10, 0x3e] {
        assert!(
            mask_enables_rendering(mask),
            "${mask:02X} renders, so the PPU fetches and A12 moves"
        );
    }
    for mask in [0x00, 0x06, 0x01, 0xe0] {
        assert!(
            !mask_enables_rendering(mask),
            "${mask:02X} renders nothing at all"
        );
    }
}

/// With 8x16 sprites the sprite half comes from bit 0 of each tile index and `ppu.ctrl` bit 3 is
/// ignored, so the A12 check has nothing to read. Refused by name rather than answered from a bit
/// the PPU is not looking at — a diagnostic that quotes a value must not misdescribe it.
#[test]
fn mmc3_irq_refuses_the_sprite_size_it_cannot_check() {
    use raster_timing::{validate_mmc3_irq_frame, Mmc3IrqError, PpuConfiguration, RegisterState};

    for ctrl in [0x20, 0x28, 0x30] {
        assert_eq!(
            validate_mmc3_irq_frame(&PpuConfiguration {
                ctrl: RegisterState::Known(ctrl),
                mask: RegisterState::Known(0x1e),
            }),
            Err(Mmc3IrqError::TallSpritesUncheckable { ctrl }),
            "${ctrl:02X} selects 8x16 sprites"
        );
    }
}

/// A register written from something other than a constant cannot be checked, and a compiler that
/// guessed would either refuse a correct program or pass a silent one.
#[test]
fn mmc3_irq_refuses_a_ppu_configuration_it_cannot_prove() {
    use raster_timing::{validate_mmc3_irq_frame, Mmc3IrqError, PpuConfiguration, RegisterState};

    assert_eq!(
        validate_mmc3_irq_frame(&PpuConfiguration {
            ctrl: RegisterState::Unproven,
            mask: RegisterState::Known(0x18),
        }),
        Err(Mmc3IrqError::UnprovenConfiguration {
            register: "ppu.ctrl"
        })
    );
    assert_eq!(
        validate_mmc3_irq_frame(&PpuConfiguration {
            ctrl: RegisterState::Known(0x08),
            mask: RegisterState::Unproven,
        }),
        Err(Mmc3IrqError::UnprovenConfiguration {
            register: "ppu.mask"
        })
    );
}

/// The window an `irq` handler's body gets is the hblank the MMC3 leaves it, and it is a
/// measurement rather than a derivation — so this test's job is to hold the number still.
///
/// The bounds are asserted alongside the equality on purpose: a later edit that inflates the
/// constant to make some other test pass fails here rather than quietly widening the window a
/// running console does not actually offer. They are `const` assertions rather than runtime ones
/// because both sides are constants — which is also why they fail the build rather than the test.
#[test]
fn an_irq_handler_body_gets_what_is_left_of_the_scanline_it_interrupts() {
    use raster_timing::{IRQ_HANDLER_BODY_CYCLES, SCANLINE_BODY_CYCLES};

    /// `LDA #imm` (2) plus `STA $2001` (4): the cheapest thing a handler can usefully do.
    const ONE_REGISTER_STORE: u32 = 6;

    const _: () = assert!(
        IRQ_HANDLER_BODY_CYCLES > ONE_REGISTER_STORE,
        "a handler with no room beyond one register store is a strategy with no use"
    );
    const _: () = assert!(
        IRQ_HANDLER_BODY_CYCLES < SCANLINE_BODY_CYCLES,
        "the hblank is a fraction of the scanline a `using timed` handler gets"
    );

    assert_eq!(IRQ_HANDLER_BODY_CYCLES, 9);
}

/// `STA $4014`, four cycles of instruction and a 514-cycle stall behind them.
fn store_dma_port() -> Instruction {
    Instruction {
        opcode: 0x8d,
        mode: AddressingMode::Absolute,
        operand: Some(0x4014),
    }
}

#[test]
fn a_store_to_the_dma_port_is_charged_its_worst_case_stall() {
    let dma = store_dma_port();
    let ctrl = Instruction {
        opcode: 0x8d,
        mode: AddressingMode::Absolute,
        operand: Some(0x2000),
    };

    assert_eq!(raster_timing::worst_case_cycles(&[dma]), 4 + 514);
    // A store to any other absolute address is the instruction and nothing else.
    assert_eq!(raster_timing::worst_case_cycles(&[ctrl]), 4);
    // Two DMAs are two stalls: the charge accumulates rather than saturating.
    assert_eq!(raster_timing::worst_case_cycles(&[dma, dma]), 2 * (4 + 514));
}

#[test]
fn stx_and_sty_to_the_dma_port_start_a_dma_too() {
    // Codegen emits only `STA` today. The rule is about the hardware, which
    // starts a DMA on whatever writes the port.
    for opcode in [0x8e, 0x8c] {
        let store = Instruction {
            opcode,
            mode: AddressingMode::Absolute,
            operand: Some(0x4014),
        };
        assert_eq!(
            raster_timing::worst_case_cycles(&[store]),
            4 + 514,
            "opcode ${opcode:02x}"
        );
    }
}

/// Bit 7 of `ppu.ctrl` is NMI, and a `timed` schedule cannot spare the cycles one costs. The
/// verdict is about the bit and nothing else: the other bits an author sets alongside it — the
/// pattern halves, the nametable — are none of this check's business.
#[test]
fn nmi_is_on_when_ppu_ctrl_bit_7_reaches_the_frame() {
    use raster_timing::{timed_frame_nmi, RegisterState, TimedFrameNmi};

    assert_eq!(
        timed_frame_nmi(RegisterState::Known(0x88)),
        Some(TimedFrameNmi::On { ctrl: 0x88 })
    );
    assert_eq!(timed_frame_nmi(RegisterState::Known(0x08)), None);
    // The reset runtime's own zero: a program that never writes `ppu.ctrl` is provably NMI-off.
    assert_eq!(timed_frame_nmi(RegisterState::Known(0x00)), None);
}

/// A value rasterc could not fold and one written under a branch are two different things for the
/// author to go and fix, so they stay two verdicts rather than one "unknown".
#[test]
fn a_ppu_ctrl_rasterc_cannot_prove_is_its_own_verdict() {
    use raster_timing::{timed_frame_nmi, RegisterState, TimedFrameNmi};

    assert_eq!(
        timed_frame_nmi(RegisterState::Unproven),
        Some(TimedFrameNmi::Unproven)
    );
    assert_eq!(
        timed_frame_nmi(RegisterState::Conditional),
        Some(TimedFrameNmi::Conditional)
    );
}
