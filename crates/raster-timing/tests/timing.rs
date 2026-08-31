use raster_6502::{AddressingMode, Instruction};
use raster_timing::{analyze, CycleConstraint, TimedRegion, TimingError};

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
fn indexed_reads_and_branches_are_charged_their_worst_case() {
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

    assert_eq!(
        analyze(
            &region(
                CycleConstraint::Report {
                    label: "worst".to_owned()
                },
                vec![indexed, branch]
            ),
            true,
        )
        .expect("a report constraint never fails")
        .measured_cycles,
        9
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
