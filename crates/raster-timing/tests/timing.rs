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
        analyze(&region(CycleConstraint::Exact(6), instructions.clone()), true),
        Ok(raster_timing::TimingReport {
            label: None,
            measured_cycles: 6,
            padding: Vec::new(),
        })
    );
    assert_eq!(
        analyze(&region(CycleConstraint::Exact(8), instructions.clone()), true),
        Err(TimingError::UnderBudget {
            measured_cycles: 6,
            budget: 8,
        })
    );
    assert_eq!(
        analyze(&region(CycleConstraint::Exact(4), instructions.clone()), true),
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
        analyze(&region(CycleConstraint::AtMost(8), instructions.clone()), true)
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
