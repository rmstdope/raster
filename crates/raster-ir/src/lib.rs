use std::collections::{BTreeMap, BTreeSet};

use raster_diag::Refusal;
use raster_sema::TypedProgram;
use raster_syntax::{
    Block, CycleBound, Declaration, Expression as SyntaxExpression, Frame as SyntaxFrame,
    FrameEvent as SyntaxFrameEvent, FramePosition, Function as SyntaxFunction, Item, Keyword,
    Operator, Program as SyntaxProgram, Span, Spanned, Statement as SyntaxStatement, Type, Wait,
};
pub use raster_timing::CycleConstraint;
use raster_timing::{validate_mmc3_irq_frame, Mmc3IrqError, PpuConfiguration, RegisterState};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Place(pub u32);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Label(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaceKind {
    Global,
    Local,
    Parameter,
    Temporary,
    Counter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaceDefinition {
    pub place: Place,
    pub kind: PlaceKind,
    pub span: Span,
    pub explicit_zero_page: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Register {
    PpuCtrl,
    PpuMask,
    PpuStatus,
    PpuOamAddr,
    PpuOamData,
    PpuScroll,
    PpuAddr,
    PpuData,
    Mmc3BankSelect,
    Mmc3BankData,
    Mmc3Mirroring,
    Mmc3RamProtect,
    Mmc3IrqLatch,
    Mmc3IrqReload,
    Mmc3IrqDisable,
    Mmc3IrqEnable,
}

impl Register {
    pub const fn address(self) -> u16 {
        match self {
            Self::PpuCtrl => 0x2000,
            Self::PpuMask => 0x2001,
            Self::PpuStatus => 0x2002,
            Self::PpuOamAddr => 0x2003,
            Self::PpuOamData => 0x2004,
            Self::PpuScroll => 0x2005,
            Self::PpuAddr => 0x2006,
            Self::PpuData => 0x2007,
            Self::Mmc3BankSelect => 0x8000,
            Self::Mmc3BankData => 0x8001,
            Self::Mmc3Mirroring => 0xa000,
            Self::Mmc3RamProtect => 0xa001,
            Self::Mmc3IrqLatch => 0xc000,
            Self::Mmc3IrqReload => 0xc001,
            Self::Mmc3IrqDisable => 0xe000,
            Self::Mmc3IrqEnable => 0xe001,
        }
    }

    /// The register's name in source, which is what a diagnostic calls it.
    ///
    /// One-for-one with the match in `Lowerer::register`: every arm there maps
    /// a `("ns", "member")` pair to a variant, and every arm here spells that
    /// pair back. The table test in `tests/lower.rs` is what keeps the two in
    /// step.
    pub const fn name(self) -> &'static str {
        match self {
            Self::PpuCtrl => "ppu.ctrl",
            Self::PpuMask => "ppu.mask",
            Self::PpuStatus => "ppu.status",
            Self::PpuOamAddr => "ppu.oam_addr",
            Self::PpuOamData => "ppu.oam_data",
            Self::PpuScroll => "ppu.scroll",
            Self::PpuAddr => "ppu.addr",
            Self::PpuData => "ppu.data",
            Self::Mmc3BankSelect => "mmc3.bank_select",
            Self::Mmc3BankData => "mmc3.bank_data",
            Self::Mmc3Mirroring => "mmc3.mirroring",
            Self::Mmc3RamProtect => "mmc3.ram_protect",
            Self::Mmc3IrqLatch => "mmc3.irq_latch",
            Self::Mmc3IrqReload => "mmc3.irq_reload",
            Self::Mmc3IrqDisable => "mmc3.irq_disable",
            Self::Mmc3IrqEnable => "mmc3.irq_enable",
        }
    }

    /// Whether a read of this port returns anything to do with the register.
    ///
    /// Three of the sixteen can be read: $2002, $2004 and $2007. A read of any
    /// other returns whatever was last on the PPU's data bus, or — at $8000 and
    /// above — a byte of the PRG bank the mapper has at that address, which is
    /// a byte of the program itself.
    ///
    /// Both sides are listed rather than `!matches!(...)` on the three, so the
    /// match stays exhaustive with no `_` arm: a register added later cannot
    /// inherit a verdict nobody chose, because the compiler will not build
    /// until someone decides.
    pub const fn is_write_only(self) -> bool {
        match self {
            Self::PpuStatus | Self::PpuOamData | Self::PpuData => false,
            Self::PpuCtrl
            | Self::PpuMask
            | Self::PpuOamAddr
            | Self::PpuScroll
            | Self::PpuAddr
            | Self::Mmc3BankSelect
            | Self::Mmc3BankData
            | Self::Mmc3Mirroring
            | Self::Mmc3RamProtect
            | Self::Mmc3IrqLatch
            | Self::Mmc3IrqReload
            | Self::Mmc3IrqDisable
            | Self::Mmc3IrqEnable => true,
        }
    }

    /// Whether a write to this port reaches the register at all.
    ///
    /// One of the sixteen is read-only: $2002, the PPU's status port, which the
    /// CPU can only read. The PPU discards a store to it entirely — there is no
    /// register behind the address to hold the value, and no flag or latch the
    /// store moves.
    ///
    /// Both sides are listed rather than `matches!(self, Self::PpuStatus)`, for
    /// the same reason `is_write_only` lists both: the match stays exhaustive
    /// with no `_` arm, so a register added later cannot inherit a verdict
    /// nobody chose, because the compiler will not build until someone decides.
    ///
    /// This and `is_write_only` are independent facts, not opposites. $2004 and
    /// $2007 are false for both: they read and write.
    pub const fn is_read_only(self) -> bool {
        match self {
            Self::PpuStatus => true,
            Self::PpuCtrl
            | Self::PpuMask
            | Self::PpuOamAddr
            | Self::PpuOamData
            | Self::PpuScroll
            | Self::PpuAddr
            | Self::PpuData
            | Self::Mmc3BankSelect
            | Self::Mmc3BankData
            | Self::Mmc3Mirroring
            | Self::Mmc3RamProtect
            | Self::Mmc3IrqLatch
            | Self::Mmc3IrqReload
            | Self::Mmc3IrqDisable
            | Self::Mmc3IrqEnable => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    And,
    Or,
    Xor,
    ShiftLeft,
    ShiftRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOperator {
    Negate,
    Not,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Constant(u8),
    Place(Place),
    Register(Register),
    Unary {
        operator: UnaryOperator,
        operand: Box<Value>,
    },
    Binary {
        left: Box<Value>,
        operator: BinaryOperator,
        right: Box<Value>,
        left_temporary: Place,
        right_temporary: Place,
    },
    Call {
        target: Label,
        arguments: Vec<Value>,
        argument_temporaries: Vec<Place>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Comparison {
    const fn inverted(self) -> Self {
        match self {
            Self::Equal => Self::NotEqual,
            Self::NotEqual => Self::Equal,
            Self::Less => Self::GreaterEqual,
            Self::LessEqual => Self::Greater,
            Self::Greater => Self::LessEqual,
            Self::GreaterEqual => Self::Less,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Condition {
    pub left: Value,
    pub comparison: Comparison,
    pub right: Value,
    pub left_temporary: Place,
    pub right_temporary: Place,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Destination {
    Place(Place),
    Register(Register),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Statement {
    Declare {
        place: Place,
    },
    Label(Label),
    Assign {
        destination: Destination,
        value: Value,
    },
    Call {
        target: Label,
        arguments: Vec<Value>,
        argument_temporaries: Vec<Place>,
    },
    Branch {
        condition: Condition,
        if_false: Label,
    },
    Jump {
        target: Label,
    },
    Return(Option<Value>),
    /// A region whose generated code must satisfy a cycle budget.
    ///
    /// The body stays nested rather than flattened because its cost is measured as a unit, and
    /// codegen must be able to tell the region's own instructions from what surrounds them.
    Timed {
        constraint: CycleConstraint,
        pad: bool,
        interruptible: bool,
        body: Vec<Statement>,
        /// The `cycles(...)` header, which is what a budget diagnostic underlines.
        span: Span,
    },
    /// `wait cycles(N)`: spend exactly this many cycles doing nothing.
    Delay {
        cycles: u32,
        span: Span,
    },
    /// `sync exact`: de-jitter against the PPU before a timed region.
    SyncExact,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function {
    pub name: String,
    pub label: Label,
    pub parameters: Vec<Place>,
    pub statements: Vec<Statement>,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Main {
    pub label: Label,
    pub halt_label: Label,
    pub statements: Vec<Statement>,
    pub span: Span,
}

/// The first scanline of the NTSC visible picture. A timed frame schedules work on the visible
/// picture only: everything else is vblank, post-render or pre-render, where a raster effect has
/// nothing to change.
pub const FIRST_VISIBLE_SCANLINE: u32 = 0;
/// The last scanline of the NTSC visible picture — spec Appendix A.
pub const LAST_VISIBLE_SCANLINE: u32 = 239;

/// How a frame's schedule is realised — spec section 7.1's `using` clause.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameStrategy {
    /// A cycle-counted loop synchronized once against vblank.
    Timed,
    /// The MMC3 scanline counter, chained handler to handler.
    Irq,
}

/// A `frame` schedule, and the strategy that realises it.
///
/// Every `every N scanlines` range is already expanded into one event per occurrence and the whole
/// schedule is sorted by scanline, so codegen walks it forwards and never has to reason about
/// source order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub name: String,
    pub strategy: FrameStrategy,
    pub events: Vec<FrameEvent>,
    pub span: Span,
    /// The `using` clause, which is what a diagnostic about the strategy underlines. A frame that
    /// omitted the clause carries the frame's own span here.
    pub strategy_span: Span,
}

/// One scheduled handler, and the visible scanline it runs on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameEvent {
    pub scanline: u32,
    pub body: Vec<Statement>,
    /// The event in the source, which is what a budget diagnostic about this handler underlines.
    pub span: Span,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Program {
    pub places: Vec<PlaceDefinition>,
    pub global_initializers: Vec<Statement>,
    pub functions: Vec<Function>,
    pub main: Option<Main>,
    /// The one `frame` the program declares, if it declares one.
    pub frame: Option<Frame>,
    /// The hazards lowering saw and did not refuse, in source order.
    pub warnings: Vec<LowerWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LowerError {
    pub message: String,
    /// What the carets say. `None` mirrors the message, which is what every
    /// refusal did before one of them needed to name a fact the message does
    /// not — here, which port the register is.
    pub label: Option<String>,
    /// Notes this refusal carries in its own right, before `rasterc` adds the
    /// one its `Refusal` earns.
    pub notes: Vec<String>,
    pub span: Span,
    pub refusal: Refusal,
}

/// A hazard rasterc can see but will not refuse. Shaped like `LowerError` plus
/// the label and notes a diagnostic needs, and without a `Refusal`, which is a
/// property of refusing. This crate does not build the diagnostic itself;
/// `rasterc` does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LowerWarning {
    pub message: String,
    pub label: String,
    pub notes: Vec<String>,
    pub span: Span,
}

/// Lowering that produced errors, and the warnings it found on the way.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LowerFailure {
    pub errors: Vec<LowerError>,
    pub warnings: Vec<LowerWarning>,
}

/// Bit 6 of an MMC3 bank select: PRG mode.
const MMC3_PRG_MODE: u8 = 0b0100_0000;
/// Bit 7 of an MMC3 bank select: CHR A12 inversion.
const MMC3_CHR_INVERSION: u8 = 0b1000_0000;

/// Bits 0-2 of a bank select name the register. Bits 6 and 7 are the mode bits
/// `bank_select_warning` judges and say nothing about which register is named:
/// `$46` selects R6 *and* changes the PRG mode, and earns both warnings.
const MMC3_BANK_REGISTER: u8 = 0b0000_0111;

/// The register reset leaves selected. The MMC3 table in
/// `crates/raster-link/src/runtime.rs` ends `(0x07, MMC3_BANK_SELECT)` followed
/// by a bank data write, so R7 is what a `mmc3.bank_data` with no select before
/// it lands on. `raster-ir` cannot depend on `raster-link` - the dependency runs
/// the other way - so this is pinned instead by
/// `reset_leaves_r7_selected_for_the_lowering_pass` in
/// `crates/raster-link/tests/runtime.rs`.
const RESET_SELECTED_REGISTER: u8 = 7;

/// Which MMC3 bank register the next `mmc3.bank_data` write lands on, at the
/// point lowering has reached.
///
/// Three variants because there are three things to say to an author, not
/// because the lattice needs three: `ResetLeftR7` and `Known(7)` are the same
/// register and a different sentence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BankSelection {
    /// Nothing in the source has selected a register yet. Reset's last bank
    /// select was `$07`, so R7 is selected. Only `main` starts here.
    ResetLeftR7,
    /// Bits 0-2 of the last bank select rasterc could fold: 0 through 7.
    Known(u8),
    /// A select rasterc could not fold, two branches that disagree, a loop body
    /// that can select, a call that can reach a select, or the first statement
    /// of a body any caller or interrupt can reach.
    Unknown(Unseen),
}

/// Why the selection is unknown, which is the only thing that varies between
/// the three `cannot tell` labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Unseen {
    /// Something in this body: a computed select, a branch join, a loop, a call.
    InThisBody,
    /// The first statement of a `fn`, which any caller can reach.
    FunctionEntry,
    /// The first statement of a `frame` handler, which interrupts `main`.
    FrameEntry,
}

impl BankSelection {
    /// A folded bank select. The mask is here rather than at each call site so
    /// that `Known` cannot hold 8 or above by construction - which is what lets
    /// `bank_data_warning` read its last arm as R7 rather than as a fallback
    /// standing in for values it has no sentence for.
    fn known(bits: u8) -> Self {
        Self::Known(bits & MMC3_BANK_REGISTER)
    }

    /// The selection true of both paths reaching a join. Equal selections
    /// survive; anything else is unknown from here on.
    ///
    /// `ResetLeftR7` and `Known(RESET_SELECTED_REGISTER)` name the same
    /// register and differ only in which sentence the author is owed, so they
    /// join to the one that is true of both paths: R7 was selected, and it was
    /// not only reset that selected it. Without this, an `if` that selects R7
    /// explicitly would fall to `Unknown` against a path that never selected
    /// at all, and the author would be told rasterc cannot tell when it can.
    fn join(self, other: Self) -> Self {
        let named = |selection| match selection {
            Self::ResetLeftR7 => Self::known(RESET_SELECTED_REGISTER),
            other => other,
        };
        if self == other {
            self
        } else if named(self) == named(other) {
            named(self)
        } else {
            Self::Unknown(Unseen::InThisBody)
        }
    }
}

const BANK_SELECT_MODE_NOTE: &str = "bits 6 and 7 take effect from whichever bank select was\n\
                                     written last, not from the bank data that follows";

/// The lowest MMC3 port address. At or above it a named register sits in the
/// PRG window the mapper banks, so a read there is a read of the program;
/// below it, among the sixteen, every named register is a PPU port.
const MMC3_PORT_BASE: u16 = 0x8000;

const WRITE_THE_WHOLE_VALUE: &str = "keep what you wrote in a variable of your own\n\
                                     and write the whole value";

/// What a $2007 read actually does. Said the same way by the warning and by
/// the compound-assignment refusal, so the two cannot drift apart.
const PPU_DATA_BUFFER_NOTE: &str =
    "$2007 hands back what the previous read fetched, and loads the\n\
                                    byte at this address for the next read";

/// The sequence that gets the author the byte they asked for, and the one
/// address range the buffer does not apply to.
const PPU_DATA_READ_TWICE: &str =
    "read `ppu.data` twice in a row, discard the first and keep the\n\
                                   second; a palette address, $3F00 to $3FFF, is not buffered and\n\
                                   reads back at once";

/// Why a compound assignment to `ppu.data` cannot be made to work.
const PPU_DATA_COMPOUND_NOTE: &str =
    "the byte it would add to is the one at the previous address,\n\
                                      not the one at the address you are writing";

/// What to write instead of a compound assignment to `ppu.data`.
const PPU_DATA_KEEP_IT: &str = "read the byte you want into a variable of your own, add to\n\
                                that, and write the whole value";

/// What a read of a write-only port actually returns.
fn dead_read_note(register: Register) -> String {
    let address = register.address();
    if address >= MMC3_PORT_BASE {
        format!(
            "reading ${address:04X} returns a byte of your own program from the PRG\n\
             bank mapped there, not the last value written"
        )
    } else {
        format!(
            "reading ${address:04X} returns whatever was last on the PPU's data bus,\n\
             not the last value written"
        )
    }
}

/// How a register read reached lowering, which decides whether the refusal has
/// to say where the read came from.
#[derive(Clone, Copy)]
enum ReadSite {
    /// The author named the register in an expression: `var m: u8 = ppu.mask`.
    /// The read is on the line in front of them, so nothing explains it.
    Named,
    /// A compound assignment read its own destination: `ppu.mask += $18`.
    /// There is no read on that line at all, so the refusal says which operator
    /// made one. Carries the operator as the author wrote it.
    CompoundAssignment(&'static str),
}

/// How many times this statement reads `ppu.data` in its own right.
///
/// Does NOT descend into a nested block — an `if` body, a `while` body, a
/// `cycles(...)` region — because a read in there is separated from this
/// statement's neighbours by a branch or by a region boundary, and may not run
/// at all. It DOES look at an `if`'s or a `while`'s condition, which is
/// evaluated where the statement sits.
fn ppu_data_reads(statement: &Spanned<SyntaxStatement>) -> usize {
    match &statement.value {
        SyntaxStatement::Declaration(declaration) => declaration
            .initializer
            .as_ref()
            .map_or(0, ppu_data_reads_in),
        SyntaxStatement::Expression(expression) => ppu_data_reads_in(expression),
        SyntaxStatement::Return(Some(expression)) => ppu_data_reads_in(expression),
        SyntaxStatement::If { condition, .. } => ppu_data_reads_in(condition),
        SyntaxStatement::While { condition, .. } => ppu_data_reads_in(condition),
        // Counted for completeness rather than for reach: `raster-sema`
        // refuses a `for` range or step that is not a compile-time constant,
        // so neither can hold a `ppu.data` read today.
        SyntaxStatement::For { range, step, .. } => {
            ppu_data_reads_in(range) + step.as_ref().map_or(0, ppu_data_reads_in)
        }
        SyntaxStatement::Block(_)
        | SyntaxStatement::Loop(_)
        | SyntaxStatement::Cycles { .. }
        | SyntaxStatement::Wait(_)
        | SyntaxStatement::Sync(_)
        | SyntaxStatement::Break
        | SyntaxStatement::Continue
        | SyntaxStatement::Return(None) => 0,
    }
}

/// How many times this expression reads `ppu.data`.
///
/// A `Member` expression whose base is the name `ppu` and whose member is
/// `data` is one read; everything else recurses into its children. This is a
/// syntactic count and deliberately not `Lowerer::register` — it runs before
/// lowering the statement, so it must not report errors or touch any state.
///
/// A `ppu.data` that is *itself* the destination of an assignment or a compound
/// assignment is **not** a read: `ppu.data = $21` is a write, and `ppu.data +=
/// 1` reads $2007 only in a way this bead refuses outright. Counting either
/// would make a write look like a priming read and silence the warning on the
/// line beside it. Only the destination is excluded, not the whole left
/// subtree — `table[ppu.data] = 5` reads $2007 to work out where to write.
fn ppu_data_reads_in(expression: &Spanned<SyntaxExpression>) -> usize {
    match &expression.value {
        SyntaxExpression::Member { base, member } => {
            usize::from(is_ppu_data(&base.value, &member.value))
        }
        SyntaxExpression::Prefix { operand, .. } => ppu_data_reads_in(operand),
        SyntaxExpression::Infix {
            left,
            operator,
            right,
        } => {
            // Only the destination *itself* is the write. A `ppu.data` read
            // that is part of working out where to write — `table[ppu.data] =
            // 5` — is an ordinary read, and counting it is the difference
            // between a spurious warning on the line above and none.
            let left_reads = if assigns(operator.value) && is_ppu_data_member(&left.value) {
                0
            } else {
                ppu_data_reads_in(left)
            };
            left_reads + ppu_data_reads_in(right)
        }
        SyntaxExpression::Call { callee, arguments } => {
            ppu_data_reads_in(callee) + arguments.iter().map(ppu_data_reads_in).sum::<usize>()
        }
        SyntaxExpression::Index { base, index } => {
            ppu_data_reads_in(base) + ppu_data_reads_in(index)
        }
        SyntaxExpression::Range { start, end } => ppu_data_reads_in(start) + ppu_data_reads_in(end),
        SyntaxExpression::Name(_)
        | SyntaxExpression::Number(_)
        | SyntaxExpression::String(_)
        | SyntaxExpression::Character(_)
        | SyntaxExpression::Boolean(_) => 0,
    }
}

/// Whether this operator writes its left operand: plain assignment, or one of
/// the four compound assignments.
fn assigns(operator: Operator) -> bool {
    matches!(
        operator,
        Operator::Assign
            | Operator::PlusEqual
            | Operator::MinusEqual
            | Operator::StarEqual
            | Operator::SlashEqual
    )
}

/// Whether this expression is the `ppu.data` member access itself, rather than
/// something that merely contains one.
fn is_ppu_data_member(expression: &SyntaxExpression) -> bool {
    matches!(expression, SyntaxExpression::Member { base, member }
        if is_ppu_data(&base.value, &member.value))
}

/// Whether this `Member` expression's base and member spell `ppu.data`.
fn is_ppu_data(base: &SyntaxExpression, member: &str) -> bool {
    matches!(base, SyntaxExpression::Name(name) if name.value == "ppu") && member == "data"
}

const DELETE_THE_LINE: &str = "there is no value that makes this store do something;\n\
                               delete the line";

/// What a write to a read-only port actually does.
///
/// One shape today, because $2002 is the only read-only port among the sixteen.
/// `is_read_only`'s exhaustive match is what forces whoever adds a second one to
/// come back and decide whether this sentence still fits.
fn dead_write_note(register: Register) -> String {
    format!(
        "writing ${:04X} changes nothing on the PPU: it is a status\n\
         port, and the CPU can only read it",
        register.address()
    )
}

/// How a register write reached lowering, which decides whether the refusal has
/// to say where the write came from.
#[derive(Clone, Copy)]
enum WriteSite {
    /// The author wrote the assignment: `ppu.status = 0`. The write is on the
    /// line in front of them, so nothing explains it.
    Named,
    /// A compound assignment wrote its own destination: `ppu.status += 1`.
    /// There is no `=` on that line at all, so the refusal says which operator
    /// made the write. Carries the operator as the author wrote it.
    CompoundAssignment(&'static str),
}

/// Which of the two registers that share the $2005/$2006 write latch left a
/// pair half written.
///
/// Not `Register`: only these two can open a pair, and a match over all
/// sixteen would carry fourteen arms nobody can reach.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LatchPair {
    Address,
    Scroll,
}

impl LatchPair {
    /// The register as the author spells it: `ppu.addr` or `ppu.scroll`.
    const fn name(self) -> &'static str {
        match self {
            Self::Address => "ppu.addr",
            Self::Scroll => "ppu.scroll",
        }
    }

    /// What the carets say under a `ppu.status` read.
    const fn read_label(self) -> &'static str {
        match self {
            Self::Address => "the `ppu.addr` write below this becomes a second high byte",
            Self::Scroll => "the `ppu.scroll` write below this becomes a second X scroll",
        }
    }

    /// The note explaining the shared latch, in this register's own words.
    const fn shared_latch_note(self) -> &'static str {
        match self {
            Self::Address => "$2005 and $2006 share one write latch, and reading $2002 puts\n\
                              it back to expecting a high byte",
            Self::Scroll => "$2005 and $2006 share one write latch, and reading $2002 puts\n\
                             it back to expecting an X scroll",
        }
    }

    /// The note saying what the PPU never sees, in this register's own words.
    const fn lost_half_note(self) -> &'static str {
        match self {
            Self::Address => "the PPU never sees the low byte, so it reads and writes at an\n\
                              address you did not ask for",
            Self::Scroll => "the PPU never sees the Y scroll, so the picture scrolls\n\
                             somewhere you did not ask for",
        }
    }
}

/// What to do instead, when the author wrote the read.
const MOVE_THE_READ: &str = "read `ppu.status` above the pair or below it, not inside it";

/// What one statement does to the shared $2005/$2006 write latch, in its own
/// right — not counting anything inside a nested block.
#[derive(Clone, Copy)]
struct LatchEffect {
    /// The span of the **first** $2002 read this statement makes. `None` for a
    /// statement that reads no port.
    read: Option<Span>,
    /// The `ppu.addr` or `ppu.scroll` write this statement makes, if it makes
    /// one. A statement makes at most one, because `lower_expression_statement`
    /// recognises an assignment only as the whole statement.
    write: Option<LatchPair>,
}

/// Classify one statement. Purely syntactic and side-effect free: it runs
/// before the statement is lowered, so it must report no error and touch no
/// state. Mirrors `ppu_data_reads` arm for arm.
fn latch_effect(statement: &Spanned<SyntaxStatement>) -> LatchEffect {
    match &statement.value {
        SyntaxStatement::Declaration(declaration) => LatchEffect {
            read: declaration
                .initializer
                .as_ref()
                .and_then(ppu_status_read_in),
            write: None,
        },
        SyntaxStatement::Expression(expression) => LatchEffect {
            read: ppu_status_read_in(expression),
            write: latch_write_in(expression),
        },
        SyntaxStatement::Return(Some(expression)) => LatchEffect {
            read: ppu_status_read_in(expression),
            write: None,
        },
        SyntaxStatement::If { condition, .. } => LatchEffect {
            read: ppu_status_read_in(condition),
            write: None,
        },
        SyntaxStatement::While { condition, .. } => LatchEffect {
            read: ppu_status_read_in(condition),
            write: None,
        },
        // Classified for completeness rather than for reach, exactly as
        // `ppu_data_reads` does: `raster-sema` refuses a `for` range or step
        // that is not a compile-time constant, so neither can hold a read.
        SyntaxStatement::For { range, step, .. } => LatchEffect {
            read: ppu_status_read_in(range)
                .or_else(|| step.as_ref().and_then(ppu_status_read_in)),
            write: None,
        },
        SyntaxStatement::Block(_)
        | SyntaxStatement::Loop(_)
        | SyntaxStatement::Cycles { .. }
        | SyntaxStatement::Wait(_)
        | SyntaxStatement::Sync(_)
        | SyntaxStatement::Break
        | SyntaxStatement::Continue
        | SyntaxStatement::Return(None) => LatchEffect {
            read: None,
            write: None,
        },
    }
}

/// The span of the first `ppu.status` read in this expression, in source order.
///
/// Leftmost wins, so the recursion goes left before right, callee before
/// arguments, and base before index.
fn ppu_status_read_in(expression: &Spanned<SyntaxExpression>) -> Option<Span> {
    match &expression.value {
        SyntaxExpression::Member { base, member } => {
            is_ppu_status(&base.value, &member.value).then_some(expression.span)
        }
        SyntaxExpression::Prefix { operand, .. } => ppu_status_read_in(operand),
        SyntaxExpression::Infix {
            left,
            operator,
            right,
        } => {
            // A plain `ppu.status = 0` is a write the PPU throws away, which is
            // raster-xeo's bead and not a read. The four compound assignments
            // really do emit `LDA $2002` before their store, so the destination
            // itself is a read there, with the carets on `ppu.status`.
            let left_read = if assigns(operator.value) && is_ppu_status_member(&left.value) {
                match operator.value {
                    Operator::Assign => None,
                    _ => Some(left.span),
                }
            } else {
                ppu_status_read_in(left)
            };
            left_read.or_else(|| ppu_status_read_in(right))
        }
        SyntaxExpression::Call { callee, arguments } => ppu_status_read_in(callee)
            .or_else(|| arguments.iter().find_map(ppu_status_read_in)),
        SyntaxExpression::Index { base, index } => {
            ppu_status_read_in(base).or_else(|| ppu_status_read_in(index))
        }
        SyntaxExpression::Range { start, end } => {
            ppu_status_read_in(start).or_else(|| ppu_status_read_in(end))
        }
        SyntaxExpression::Name(_)
        | SyntaxExpression::Number(_)
        | SyntaxExpression::String(_)
        | SyntaxExpression::Character(_)
        | SyntaxExpression::Boolean(_) => None,
    }
}

/// The `ppu.addr` or `ppu.scroll` write this expression is, if it is one.
fn latch_write_in(expression: &Spanned<SyntaxExpression>) -> Option<LatchPair> {
    let SyntaxExpression::Infix {
        left, operator, ..
    } = &expression.value
    else {
        return None;
    };
    if !assigns(operator.value) {
        return None;
    }
    let SyntaxExpression::Member { base, member } = &left.value else {
        return None;
    };
    latch_pair(&base.value, &member.value)
}

/// The pair this `Member` expression's base and member open, if any.
fn latch_pair(base: &SyntaxExpression, member: &str) -> Option<LatchPair> {
    if !matches!(base, SyntaxExpression::Name(name) if name.value == "ppu") {
        return None;
    }
    match member {
        "addr" => Some(LatchPair::Address),
        "scroll" => Some(LatchPair::Scroll),
        _ => None,
    }
}

/// Whether this expression is the `ppu.status` member access itself, rather
/// than something that merely contains one.
fn is_ppu_status_member(expression: &SyntaxExpression) -> bool {
    matches!(expression, SyntaxExpression::Member { base, member }
        if is_ppu_status(&base.value, &member.value))
}

/// Whether this `Member` expression's base and member spell `ppu.status`.
fn is_ppu_status(base: &SyntaxExpression, member: &str) -> bool {
    matches!(base, SyntaxExpression::Name(name) if name.value == "ppu") && member == "status"
}

/// The warning a $2002 read earns when a `ppu.addr` or `ppu.scroll` pair is
/// half written.
fn half_written_pair(opener: LatchPair, span: Span) -> LowerWarning {
    LowerWarning {
        message: format!(
            "this `ppu.status` read leaves your `{}` pair half written",
            opener.name()
        ),
        label: opener.read_label().to_owned(),
        notes: vec![
            opener.shared_latch_note().to_owned(),
            opener.lost_half_note().to_owned(),
            MOVE_THE_READ.to_owned(),
        ],
        span,
    }
}

/// The warning a write to `mmc3.bank_select` earns, if it earns one.
///
/// `None` for a constant with bits 6 and 7 clear. That selects a bank register
/// without touching the mapping mode, which is the ordinary use of these
/// registers and the thing the reset map is built to survive; warning on it
/// would fire on every correct CHR animation, on every build.
fn bank_select_warning(value: &Value, span: Span) -> Option<LowerWarning> {
    let mode_change = |label: &str, reset_note: &str| LowerWarning {
        message: "this bank select changes the MMC3 mapping mode".to_owned(),
        label: label.to_owned(),
        notes: vec![reset_note.to_owned(), BANK_SELECT_MODE_NOTE.to_owned()],
        span,
    };
    let Value::Constant(bits) = value else {
        return Some(LowerWarning {
            message: "rasterc cannot tell whether this bank select changes the mapping mode"
                .to_owned(),
            label: "rasterc cannot see this value here, so bits 6 and 7 are unknown".to_owned(),
            notes: vec![
                "bit 6 is PRG mode and bit 7 is CHR A12 inversion; keeping\n\
                         both clear keeps the map reset chose"
                    .to_owned(),
            ],
            span,
        });
    };
    let prg = bits & MMC3_PRG_MODE != 0;
    let chr = bits & MMC3_CHR_INVERSION != 0;
    match (prg, chr) {
        (false, false) => None,
        (false, true) => Some(mode_change(
            "bit 7 swaps the two pattern tables from here on",
            "reset chose CHR A12 inversion off, so pattern table 0 is at\n\
             PPU $0000; clearing bit 7 keeps that map",
        )),
        (true, false) => Some(mode_change(
            "bit 6 moves the fixed PRG bank from $C000 to $8000",
            "reset chose PRG mode 0, a linear 32 KiB map with this code\n\
             in the fixed bank at $E000; clearing bit 6 keeps that map",
        )),
        (true, true) => Some(mode_change(
            "bit 6 moves the fixed PRG bank and bit 7 swaps the pattern tables",
            "reset chose PRG mode 0 and CHR A12 inversion off: a linear\n\
             32 KiB map, and pattern table 0 at PPU $0000",
        )),
    }
}

/// The warning a write to `mmc3.bank_data` earns, if it earns one.
///
/// `None` for R0-R5. Those are the CHR windows: repointing one is the ordinary
/// use of these registers and the thing the reset map is built to survive, so
/// warning on them would fire on every correct CHR animation, on every build.
fn bank_data_warning(selection: BankSelection, span: Span) -> Option<LowerWarning> {
    const RESET_MAP_NOTE: &str = "reset chose a linear 32 KiB map with R6 = 0 and R7 = 1; the\n\
                                  bytes at ";
    const NOT_SUPPORTED_NOTE: &str =
        "PRG bank switching is not supported yet: banks 0 to 2 hold $FF,\n\
         and bank 3 is a second view of the fixed bank at $E000";
    let repoints = |window: &str, label: &str, second_note: &str| LowerWarning {
        message: format!("this write repoints the PRG window at {window}"),
        label: label.to_owned(),
        notes: vec![
            format!("{RESET_MAP_NOTE}{window} are not the ones it mapped from here on"),
            second_note.to_owned(),
        ],
        span,
    };
    match selection {
        BankSelection::Known(0..=5) => None,
        BankSelection::Known(6) => Some(repoints(
            "$8000",
            "R6 is selected, so this replaces $8000-$9FFF",
            NOT_SUPPORTED_NOTE,
        )),
        // R7, and nothing else: `BankSelection::known` masks with
        // `MMC3_BANK_REGISTER`, so 8 and above cannot be constructed.
        BankSelection::Known(_) => Some(repoints(
            "$A000",
            "R7 is selected, so this replaces $A000-$BFFF",
            NOT_SUPPORTED_NOTE,
        )),
        BankSelection::ResetLeftR7 => Some(repoints(
            "$A000",
            "nothing selects a register before this, and reset selected R7 last",
            "write `mmc3.bank_select` with 0 to 5 first to point this at a CHR window",
        )),
        BankSelection::Unknown(unseen) => Some(LowerWarning {
            message: "rasterc cannot tell which bank register this write lands on".to_owned(),
            label: match unseen {
                Unseen::InThisBody => "the last bank select before this is not one rasterc can see",
                Unseen::FunctionEntry => "this function can be called with any register selected",
                Unseen::FrameEntry => "a frame handler runs with any register selected",
            }
            .to_owned(),
            notes: vec![
                "R6 and R7 are the two 8 KiB PRG windows; a write landing on\n\
                 either one repoints $8000 or $A000"
                    .to_owned(),
                match unseen {
                    Unseen::InThisBody => {
                        "selecting with a literal 0 to 5 immediately before the write\n\
                         keeps the map reset chose"
                    }
                    Unseen::FunctionEntry => {
                        "selecting with a literal 0 to 5 in this function, before the\n\
                         write, keeps the map reset chose"
                    }
                    Unseen::FrameEntry => {
                        "selecting with a literal 0 to 5 in the handler, before the\n\
                         write, keeps the map reset chose"
                    }
                }
                .to_owned(),
            ],
            span,
        }),
    }
}

pub fn lower(typed: &TypedProgram) -> Result<Program, LowerFailure> {
    let mut lowerer = Lowerer::new();
    lowerer.predeclare_labels(&typed.program);
    lowerer.predeclare_globals(&typed.program);
    lowerer.reject_recursive_calls(&typed.program);
    lowerer.selects_bank = functions_that_select(&typed.program);
    lowerer.lower_program(&typed.program);
    lowerer.check_mmc3_irq_frame();
    if lowerer.errors.is_empty() {
        lowerer.program.warnings = lowerer.warnings;
        Ok(lowerer.program)
    } else {
        Err(LowerFailure {
            errors: lowerer.errors,
            warnings: lowerer.warnings,
        })
    }
}

#[derive(Clone, Copy)]
enum Binding {
    Place(Place),
    Constant(u8),
}

#[derive(Clone, Copy)]
struct FunctionSignature {
    label: Label,
}

struct Lowerer {
    program: Program,
    errors: Vec<LowerError>,
    warnings: Vec<LowerWarning>,
    scopes: Vec<BTreeMap<String, Binding>>,
    functions: BTreeMap<String, FunctionSignature>,
    global_places: BTreeMap<u32, Place>,
    next_place: u32,
    next_label: u32,
    main_label: Option<(Label, Label)>,
    /// Which bank register the next `mmc3.bank_data` write lands on. Reset at
    /// the top of every entry point, so the order `lower_program` happens to
    /// visit items in does not matter.
    selection: BankSelection,
    /// Every function that can reach a `mmc3.bank_select` write, directly or
    /// through a call. A call to one of these makes `selection` unknown; a call
    /// to anything else leaves it alone.
    selects_bank: BTreeSet<String>,
    /// Whether the `ppu.data` read about to be lowered has another `ppu.data`
    /// read beside it: in the statement immediately before it, in the statement
    /// immediately after it, or a second time in its own statement.
    ///
    /// Set by `lower_statements` before each statement of a block, saved and
    /// restored around the block so a nested block does not inherit its
    /// parent's answer, and `false` everywhere else — a global `var`
    /// initializer reads at reset, before any `ppu.addr` write, and has no
    /// neighbouring statement to prime it.
    ppu_data_read_has_neighbour: bool,
}

impl Lowerer {
    fn new() -> Self {
        Self {
            program: Program::default(),
            errors: Vec::new(),
            warnings: Vec::new(),
            scopes: vec![BTreeMap::new()],
            functions: BTreeMap::new(),
            global_places: BTreeMap::new(),
            next_place: 0,
            next_label: 0,
            main_label: None,
            selection: BankSelection::Unknown(Unseen::InThisBody),
            selects_bank: BTreeSet::new(),
            ppu_data_read_has_neighbour: false,
        }
    }

    /// Refuse the program. The default kind is `Rejected`: a mistake, or
    /// something Raster does not intend to do. A construct the specification
    /// defines and this release does not compile goes through
    /// `not_in_this_release` instead, so that the author is told what the
    /// release *can* build.
    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.refuse(span, message, Refusal::Rejected);
    }

    /// Refuse a construct the specification defines and this release does not
    /// compile anywhere.
    fn not_in_this_release(&mut self, span: Span, message: impl Into<String>) {
        self.refuse(span, message, Refusal::NotInThisRelease);
    }

    fn refuse(&mut self, span: Span, message: impl Into<String>, refusal: Refusal) {
        self.errors.push(LowerError {
            message: message.into(),
            label: None,
            notes: Vec::new(),
            span,
            refusal,
        });
    }

    /// Refuse the program with a label and notes of the refusal's own, rather
    /// than the mirrored message every other refusal carries.
    ///
    /// `Refusal::Rejected`, which carries no note of its own: the message
    /// already says what to do instead, and a read of a write-only port is not
    /// a construct that arrives in a later release, so the supported-subset
    /// list beside it would be wrong as well as noisy.
    fn refuse_with(
        &mut self,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
        notes: Vec<String>,
    ) {
        self.errors.push(LowerError {
            message: message.into(),
            label: Some(label.into()),
            notes,
            span,
            refusal: Refusal::Rejected,
        });
    }

    fn fresh_label(&mut self) -> Label {
        let label = Label(self.next_label);
        self.next_label += 1;
        label
    }

    fn allocate_place(&mut self, kind: PlaceKind, span: Span, explicit_zero_page: bool) -> Place {
        let place = Place(self.next_place);
        self.next_place += 1;
        self.program.places.push(PlaceDefinition {
            place,
            kind,
            span,
            explicit_zero_page,
        });
        place
    }

    fn predeclare_labels(&mut self, syntax: &raster_syntax::Program) {
        for item in &syntax.items {
            match &item.value {
                Item::Function(function) => {
                    let label = self.fresh_label();
                    self.functions
                        .insert(function.name.value.clone(), FunctionSignature { label });
                }
                Item::Main(_) if self.main_label.is_none() => {
                    self.main_label = Some((self.fresh_label(), self.fresh_label()));
                }
                _ => {}
            }
        }
    }

    fn predeclare_globals(&mut self, syntax: &raster_syntax::Program) {
        for item in &syntax.items {
            let Item::Declaration(declaration) = &item.value else {
                continue;
            };
            if declaration.kind != Keyword::Var {
                continue;
            }
            if !self.declaration_is_u8(declaration) {
                continue;
            }
            let Some(name) = &declaration.name else {
                continue;
            };
            let explicit_zero_page = self.check_storage(declaration.storage.as_ref());
            let place = self.allocate_place(PlaceKind::Global, name.span, explicit_zero_page);
            self.global_places.insert(name.span.start, place);
            self.scopes[0].insert(name.value.clone(), Binding::Place(place));
        }
    }

    fn reject_recursive_calls(&mut self, syntax: &SyntaxProgram) {
        let call_graph = function_call_graph(syntax);
        let mut visit_states = BTreeMap::new();
        for function in call_graph.keys() {
            self.find_recursive_calls(function, &call_graph, &mut visit_states);
        }
    }

    fn find_recursive_calls(
        &mut self,
        function: &str,
        call_graph: &BTreeMap<String, Vec<FunctionCall>>,
        visit_states: &mut BTreeMap<String, VisitState>,
    ) {
        if visit_states.get(function) == Some(&VisitState::Visited) {
            return;
        }
        visit_states.insert(function.to_owned(), VisitState::Visiting);
        for call in &call_graph[function] {
            if !call_graph.contains_key(&call.target) {
                continue;
            }
            match visit_states.get(&call.target) {
                Some(VisitState::Visiting) => {
                    self.error(call.span, "recursive function calls are not supported");
                }
                Some(VisitState::Visited) => {}
                None => self.find_recursive_calls(&call.target, call_graph, visit_states),
            }
        }
        visit_states.insert(function.to_owned(), VisitState::Visited);
    }

    fn lower_program(&mut self, syntax: &raster_syntax::Program) {
        let mut main_count = 0usize;
        for item in &syntax.items {
            match &item.value {
                Item::Declaration(declaration) => self.lower_top_level_declaration(declaration),
                Item::Function(function) => self.lower_function(function, item.span),
                Item::Main(block) => {
                    main_count += 1;
                    if main_count > 1 {
                        self.error(item.span, "multiple `main` blocks are not supported");
                    } else {
                        self.lower_main(block);
                    }
                }
                Item::Target(_) => {
                    self.not_in_this_release(item.span, "`target` blocks are not supported yet")
                }
                Item::Import(_) => {
                    self.not_in_this_release(item.span, "`import` is not supported yet")
                }
                Item::Frame(frame) => self.lower_frame(frame, item.span),
                Item::Other(_) => self.error(item.span, "this top-level item is not supported"),
            }
        }
    }

    /// Lower a `frame ... using timed` into a sorted schedule of visible-scanline handlers.
    ///
    /// Only the shape codegen can realise reaches the IR: one frame, the `timed` strategy, and
    /// events on the visible picture. Everything else is refused here, with the span of the part
    /// that cannot be built, rather than left for codegen to discover with nothing to point at.
    fn lower_frame(&mut self, frame: &SyntaxFrame, span: Span) {
        if self.program.frame.is_some() {
            self.not_in_this_release(span, "only one `frame` is supported yet");
            return;
        }
        // An omitted strategy is the compiler's to choose (spec section 7.1).
        // `timed` and `irq` are the two this release lowers; `timed` needs nothing of the
        // mapper, so it is what an omitted clause means.
        let (strategy, strategy_span) = match &frame.strategy {
            Some(strategy) => match strategy.value.as_str() {
                "timed" => (FrameStrategy::Timed, strategy.span),
                "irq" => (FrameStrategy::Irq, strategy.span),
                other => {
                    self.not_in_this_release(
                        strategy.span,
                        format!("`using {other}` is not supported yet"),
                    );
                    return;
                }
            },
            None => (FrameStrategy::Timed, span),
        };

        let mut events = Vec::new();
        for event in &frame.events {
            match &event.value {
                SyntaxFrameEvent::At { position, body } => match position {
                    FramePosition::Vblank(span) => {
                        self.not_in_this_release(*span, "`at vblank` is not supported yet");
                    }
                    FramePosition::Scanline(value) => {
                        if let Some(scanline) = self.visible_scanline(value) {
                            let body = self.lower_frame_body(body);
                            events.push(FrameEvent {
                                scanline,
                                body,
                                span: event.span,
                            });
                        }
                    }
                },
                SyntaxFrameEvent::Every {
                    interval,
                    from,
                    to,
                    body,
                } => {
                    // All three are evaluated before the schedule is built, so a range with two
                    // mistakes in it reports both rather than only the first.
                    let interval_value = self.constant_value_u32(interval);
                    let from_value = self.visible_scanline(from);
                    let to_value = self.visible_scanline(to);
                    let (Some(interval_value), Some(from), Some(to)) =
                        (interval_value, from_value, to_value)
                    else {
                        continue;
                    };
                    if interval_value == 0 {
                        // `analyze` has already refused this; a zero step here would not terminate.
                        continue;
                    }
                    // The body is lowered once and cloned per occurrence. Lowering it again for
                    // each would allocate a fresh temporary every time, and `every 1 scanlines
                    // from 0 to 239` would exhaust the zero page on a handler that needs one slot.
                    let body = self.lower_frame_body(body);
                    let mut scanline = Some(from);
                    while let Some(current) = scanline.filter(|&current| current <= to) {
                        events.push(FrameEvent {
                            scanline: current,
                            body: body.clone(),
                            span: event.span,
                        });
                        // An interval wider than the picture is one occurrence, not a wrap: `to` is
                        // bounded by the visible range but the interval is bounded by nothing, and
                        // adding it blind panics in a debug build and walks backwards in a release
                        // one, silently compiling a schedule the author never wrote.
                        scanline = current.checked_add(interval_value);
                    }
                }
            }
        }

        events.sort_by_key(|event| event.scanline);
        for pair in events.windows(2) {
            if pair[0].scanline == pair[1].scanline {
                self.error(
                    pair[1].span,
                    format!("two frame events share scanline {}", pair[1].scanline),
                );
            }
        }

        self.program.frame = Some(Frame {
            name: frame.name.value.clone(),
            strategy,
            events,
            span,
            strategy_span,
        });
    }

    /// A constant scanline that falls on the visible picture, or nothing and a diagnostic.
    fn visible_scanline(&mut self, value: &Spanned<SyntaxExpression>) -> Option<u32> {
        let scanline = self.constant_value_u32(value)?;
        if !(FIRST_VISIBLE_SCANLINE..=LAST_VISIBLE_SCANLINE).contains(&scanline) {
            self.error(
                value.span,
                format!(
                    "a frame event must fall on a visible scanline, \
                     {FIRST_VISIBLE_SCANLINE} to {LAST_VISIBLE_SCANLINE}"
                ),
            );
            return None;
        }
        Some(scanline)
    }

    /// Refuse a `frame ... using irq` the MMC3 would never clock.
    ///
    /// The checks themselves are `raster-timing`'s, which owns what the mapper needs; this supplies
    /// the PPU configuration the program set up and turns the verdict into a spanned diagnostic.
    /// Nothing is checked on a program that already failed: the configuration is read by walking
    /// calls, and a program whose recursion was just rejected has no walk that terminates.
    fn check_mmc3_irq_frame(&mut self) {
        if !self.errors.is_empty() {
            return;
        }
        let Some(frame) = &self.program.frame else {
            return;
        };
        if frame.strategy != FrameStrategy::Irq {
            return;
        }
        let span = frame.strategy_span;
        if let Err(error) = validate_mmc3_irq_frame(&self.ppu_configuration()) {
            self.error(span, mmc3_irq_message(error));
        }
    }

    /// What the program leaves in `ppu.ctrl` and `ppu.mask` by the time its frame runs.
    ///
    /// The walk is the program's own order — the global initializers, then `main`, stepping into
    /// each function where it is called — so a configuration written by a helper is read exactly
    /// as one written inline. A store under an `if` is taken at face value rather than treated as
    /// unproven: the alternative refuses correct programs, and the store is still the last thing
    /// the source says about the register.
    fn ppu_configuration(&self) -> PpuConfiguration {
        let functions: BTreeMap<Label, &Function> = self
            .program
            .functions
            .iter()
            .map(|function| (function.label, function))
            .collect();
        let mut configuration = PpuConfiguration {
            // What the reset runtime leaves behind: it stores zero into both before the program's
            // own code runs, so a program that writes neither has rendering off and one half of
            // pattern memory, which is exactly what the checks are about.
            ctrl: RegisterState::Known(0),
            mask: RegisterState::Known(0),
        };
        let mut visiting = Vec::new();
        collect_ppu_stores(
            &self.program.global_initializers,
            &functions,
            &mut visiting,
            &mut configuration,
            false,
        );
        if let Some(main) = &self.program.main {
            collect_ppu_stores(
                &main.statements,
                &functions,
                &mut visiting,
                &mut configuration,
                false,
            );
        }
        configuration
    }

    /// A handler body, in its own scope. Its cycle budget is codegen's to impose.
    ///
    /// A `return` never reaches here: `raster-sema` refuses one in a frame handler, which is what
    /// keeps an IRQ handler's `RTI` reachable — codegen compiles a `return` into a jump to the end
    /// of `main`, and an interrupt left that way keeps its three bytes of stack for ever.
    fn lower_frame_body(&mut self, body: &Block) -> Vec<Statement> {
        self.enter_scope();
        self.selection = BankSelection::Unknown(Unseen::FrameEntry);
        let (statements, _) = self.lower_statements(body);
        self.leave_scope();
        statements
    }

    fn lower_top_level_declaration(&mut self, declaration: &Declaration) {
        match declaration.kind {
            Keyword::Group => {
                self.not_in_this_release(
                    declaration_name_span(declaration),
                    "`group` storage is not supported yet",
                );
            }
            Keyword::Const => {
                let Some(name) = &declaration.name else {
                    return;
                };
                if !self.declaration_is_u8(declaration) {
                    return;
                }
                if !self.check_storage(declaration.storage.as_ref()) {
                    return;
                }
                let Some(initializer) = &declaration.initializer else {
                    self.error(name.span, "constants require an initializer");
                    return;
                };
                if let Some(value) = self.constant_value(initializer) {
                    self.scopes[0].insert(name.value.clone(), Binding::Constant(value));
                }
            }
            Keyword::Var => {
                let Some(name) = &declaration.name else {
                    return;
                };
                let Some(place) = self.global_places.get(&name.span.start).copied() else {
                    return;
                };
                if let Some(initializer) = &declaration.initializer {
                    let value = self.lower_value(initializer);
                    self.program.global_initializers.push(Statement::Assign {
                        destination: Destination::Place(place),
                        value,
                    });
                }
            }
            _ => self.error(
                declaration_name_span(declaration),
                "this declaration is not supported",
            ),
        }
    }

    fn lower_function(&mut self, function: &SyntaxFunction, span: Span) {
        let Some(signature) = self.functions.get(&function.name.value).copied() else {
            return;
        };
        if function.is_assembly {
            self.not_in_this_release(function.name.span, "`asm` functions are not supported yet");
            return;
        }
        if function.cycle_spec.is_some() {
            self.not_in_this_release(
                function.name.span,
                "function timing specifications are not supported",
            );
        }
        if function.storage.is_some() {
            self.not_in_this_release(function.name.span, "function storage is not supported");
        }
        if !function.employs.is_empty() {
            self.not_in_this_release(
                function.name.span,
                "function group employment is not supported",
            );
        }
        if !self.function_return_is_supported(function) {
            return;
        }
        self.enter_scope();
        let mut parameters = Vec::new();
        for parameter in &function.parameters {
            if !self.annotation_is_u8(&parameter.type_annotation) {
                continue;
            }
            let place = self.allocate_place(PlaceKind::Parameter, parameter.name.span, false);
            self.bind(&parameter.name.value, Binding::Place(place));
            parameters.push(place);
        }
        self.selection = BankSelection::Unknown(Unseen::FunctionEntry);
        let (statements, always_returns) = self.lower_statements(&function.body);
        if function_returns_u8(function) && !always_returns {
            self.error(
                function.name.span,
                "u8-returning functions cannot fall through without returning",
            );
        }
        self.leave_scope();
        self.program.functions.push(Function {
            name: function.name.value.clone(),
            label: signature.label,
            parameters,
            statements,
            span,
        });
    }

    fn lower_main(&mut self, block: &Block) {
        let Some((label, halt_label)) = self.main_label else {
            return;
        };
        self.enter_scope();
        self.selection = BankSelection::ResetLeftR7;
        let (statements, _) = self.lower_statements(block);
        self.leave_scope();
        self.program.main = Some(Main {
            label,
            halt_label,
            statements,
            span: block.span,
        });
    }

    fn lower_statements(&mut self, block: &Block) -> (Vec<Statement>, bool) {
        let mut statements = Vec::new();
        let mut always_returns = false;
        // Counted for the whole block up front: a statement's neighbour on the
        // right has not been lowered yet, so it cannot be observed while
        // lowering.
        let reads: Vec<usize> = block.statements.iter().map(ppu_data_reads).collect();
        let effects: Vec<LatchEffect> = block.statements.iter().map(latch_effect).collect();
        // A local, not a field on `Lowerer`: a nested block goes through its
        // own `lower_statements` call and therefore starts with the latch
        // closed for free, which is the agreed brace rule.
        let mut latch: Option<LatchPair> = None;
        let outer = self.ppu_data_read_has_neighbour;
        for (index, statement) in block.statements.iter().enumerate() {
            let effect = effects[index];
            // Pushed before the statement is lowered, so the warnings vector
            // stays in source order: a warning from inside a nested block would
            // otherwise print ahead of one about the statement containing it.
            if let Some(opener) = latch {
                if let Some(span) = effect.read {
                    self.warnings.push(half_written_pair(opener, span));
                }
            }
            // Then the state, in the order the hardware sees it: a statement's
            // reads happen before its write, because codegen emits the value
            // and then the store.
            if effect.read.is_some() {
                latch = None;
            }
            if let Some(pair) = effect.write {
                latch = if latch.is_some() { None } else { Some(pair) };
            }
            let before = index > 0 && reads[index - 1] > 0;
            let after = reads.get(index + 1).is_some_and(|count| *count > 0);
            self.ppu_data_read_has_neighbour = before || after || reads[index] >= 2;
            always_returns |= self.lower_statement(statement, &mut statements);
        }
        // Restored, not cleared, as the safe direction rather than an observed
        // one: no `lower_statement` arm today lowers a nested block and *then*
        // lowers an expression of its own — `lower_if`, `lower_while`,
        // `lower_for` and the `Cycles` arm all lower their condition or spec
        // first, and the loop above re-sets the field on its next iteration —
        // so nothing can currently tell this apart from clearing it. It guards
        // the arm that does it in the other order.
        self.ppu_data_read_has_neighbour = outer;
        (statements, always_returns)
    }

    fn lower_statement(
        &mut self,
        statement: &Spanned<SyntaxStatement>,
        output: &mut Vec<Statement>,
    ) -> bool {
        match &statement.value {
            SyntaxStatement::Declaration(declaration) => {
                self.lower_local_declaration(declaration, output);
                false
            }
            SyntaxStatement::Block(block) => {
                self.enter_scope();
                let (statements, always_returns) = self.lower_statements(block);
                output.extend(statements);
                self.leave_scope();
                always_returns
            }
            SyntaxStatement::If {
                condition,
                then_body,
                else_body,
            } => self.lower_if(condition, then_body, else_body.as_ref(), output),
            SyntaxStatement::While { condition, body } => {
                self.lower_while(condition, body, output);
                false
            }
            SyntaxStatement::For {
                binding,
                range,
                step,
                body,
            } => self.lower_for(binding, range, step.as_ref(), body, output),
            SyntaxStatement::Loop(block) => {
                self.not_in_this_release(statement.span, "`loop` is not supported yet");
                let entry_selection = self.selection;
                if block_selects_bank(block, &self.selects_bank) {
                    self.selection = BankSelection::Unknown(Unseen::InThisBody);
                }
                self.enter_scope();
                let _ = self.lower_statements(block);
                self.leave_scope();
                self.selection = entry_selection.join(self.selection);
                false
            }
            SyntaxStatement::Cycles { spec, label, body } => {
                let constraint = self.cycle_constraint(spec, label.as_ref());
                self.enter_scope();
                let (statements, always_returns) = self.lower_statements(body);
                self.leave_scope();
                if let Some(constraint) = constraint {
                    output.push(Statement::Timed {
                        constraint,
                        pad: spec.pad,
                        interruptible: spec.interruptible,
                        body: statements,
                        span: spec.span,
                    });
                }
                always_returns
            }
            SyntaxStatement::Wait(Wait::Cycles(value)) => {
                if let Some(cycles) = self.constant_value_u32(value) {
                    output.push(Statement::Delay {
                        cycles,
                        span: value.span,
                    });
                }
                false
            }
            SyntaxStatement::Wait(_) => {
                self.not_in_this_release(
                    statement.span,
                    "only `wait cycles` is supported yet; frame waits arrive with frame scheduling",
                );
                false
            }
            SyntaxStatement::Sync(_) => {
                output.push(Statement::SyncExact);
                false
            }
            SyntaxStatement::Break => {
                self.not_in_this_release(statement.span, "`break` is not supported yet");
                false
            }
            SyntaxStatement::Continue => {
                self.not_in_this_release(statement.span, "`continue` is not supported yet");
                false
            }
            SyntaxStatement::Return(value) => {
                output.push(Statement::Return(
                    value.as_ref().map(|value| self.lower_value(value)),
                ));
                true
            }
            SyntaxStatement::Expression(expression) => {
                self.lower_expression_statement(expression, output);
                false
            }
        }
    }

    fn lower_local_declaration(&mut self, declaration: &Declaration, output: &mut Vec<Statement>) {
        match declaration.kind {
            Keyword::Group => {
                self.not_in_this_release(
                    declaration_name_span(declaration),
                    "`group` storage is not supported yet",
                );
            }
            Keyword::Const => {
                let Some(name) = &declaration.name else {
                    return;
                };
                if !self.declaration_is_u8(declaration) {
                    return;
                }
                if !self.check_storage(declaration.storage.as_ref()) {
                    return;
                }
                let Some(initializer) = &declaration.initializer else {
                    self.error(name.span, "constants require an initializer");
                    return;
                };
                if let Some(value) = self.constant_value(initializer) {
                    self.bind(&name.value, Binding::Constant(value));
                }
            }
            Keyword::Var => {
                let Some(name) = &declaration.name else {
                    return;
                };
                if !self.declaration_is_u8(declaration) {
                    return;
                }
                let explicit_zero_page = self.check_storage(declaration.storage.as_ref());
                let place = self.allocate_place(PlaceKind::Local, name.span, explicit_zero_page);
                self.bind(&name.value, Binding::Place(place));
                output.push(Statement::Declare { place });
                if let Some(initializer) = &declaration.initializer {
                    let value = self.lower_value(initializer);
                    output.push(Statement::Assign {
                        destination: Destination::Place(place),
                        value,
                    });
                }
            }
            _ => self.error(
                declaration_name_span(declaration),
                "this declaration is not supported",
            ),
        }
    }

    fn lower_if(
        &mut self,
        condition: &Spanned<SyntaxExpression>,
        then_body: &Block,
        else_body: Option<&Block>,
        output: &mut Vec<Statement>,
    ) -> bool {
        let condition = self.lower_condition(condition);
        let otherwise = self.fresh_label();
        output.push(Statement::Branch {
            condition,
            if_false: otherwise,
        });
        let entry_selection = self.selection;
        self.enter_scope();
        let (then_statements, then_always_returns) = self.lower_statements(then_body);
        output.extend(then_statements);
        self.leave_scope();
        let then_selection = self.selection;
        // The else-arm starts where the then-arm did, and when there is no
        // `else` its exit *is* the entry selection: the path that skips the
        // branch changes nothing.
        self.selection = entry_selection;
        let returns = if let Some(else_body) = else_body {
            let end = self.fresh_label();
            output.push(Statement::Jump { target: end });
            output.push(Statement::Label(otherwise));
            self.enter_scope();
            let (else_statements, else_always_returns) = self.lower_statements(else_body);
            output.extend(else_statements);
            self.leave_scope();
            output.push(Statement::Label(end));
            then_always_returns && else_always_returns
        } else {
            output.push(Statement::Label(otherwise));
            false
        };
        self.selection = then_selection.join(self.selection);
        returns
    }

    fn lower_while(
        &mut self,
        condition: &Spanned<SyntaxExpression>,
        body: &Block,
        output: &mut Vec<Statement>,
    ) {
        let start = self.fresh_label();
        let end = self.fresh_label();
        output.push(Statement::Label(start));
        output.push(Statement::Branch {
            condition: self.lower_condition(condition),
            if_false: end,
        });
        let entry_selection = self.selection;
        if block_selects_bank(body, &self.selects_bank) {
            self.selection = BankSelection::Unknown(Unseen::InThisBody);
        }
        self.enter_scope();
        let (body_statements, _) = self.lower_statements(body);
        output.extend(body_statements);
        self.leave_scope();
        // The body may run zero times, so the exit must be true of both.
        self.selection = entry_selection.join(self.selection);
        output.push(Statement::Jump { target: start });
        output.push(Statement::Label(end));
    }

    fn lower_for(
        &mut self,
        binding: &Spanned<String>,
        range: &Spanned<SyntaxExpression>,
        step: Option<&Spanned<SyntaxExpression>>,
        body: &Block,
        output: &mut Vec<Statement>,
    ) -> bool {
        let SyntaxExpression::Range { start, end } = &range.value else {
            self.error(range.span, "for ranges must be compile-time constants");
            return false;
        };
        let Some(start) = self.constant_value(start) else {
            return false;
        };
        let Some(end) = self.constant_value(end) else {
            return false;
        };
        let step_span = step.map(|step| step.span).unwrap_or(range.span);
        let step = step.and_then(|step| self.constant_value(step)).unwrap_or(1);
        if start >= end || step == 0 {
            self.error(
                range.span,
                "for ranges must advance through a finite u8 range",
            );
            return false;
        }
        let last_counter = u16::from(start)
            + ((u16::from(end) - 1 - u16::from(start)) / u16::from(step)) * u16::from(step);
        if last_counter + u16::from(step) > u16::from(u8::MAX) {
            self.error(
                step_span,
                "for step would overflow before the range terminates",
            );
            return false;
        }

        self.enter_scope();
        let counter = self.allocate_place(PlaceKind::Counter, binding.span, false);
        self.bind(&binding.value, Binding::Place(counter));
        output.push(Statement::Declare { place: counter });
        output.push(Statement::Assign {
            destination: Destination::Place(counter),
            value: Value::Constant(start),
        });
        let loop_start = self.fresh_label();
        let loop_end = self.fresh_label();
        output.push(Statement::Label(loop_start));
        output.push(Statement::Branch {
            condition: self.condition(
                Value::Place(counter),
                Comparison::Less,
                Value::Constant(end),
                binding.span,
            ),
            if_false: loop_end,
        });
        if block_selects_bank(body, &self.selects_bank) {
            self.selection = BankSelection::Unknown(Unseen::InThisBody);
        }
        // No join afterwards: `lower_for` has already refused an empty range,
        // so the body always runs, and `break` and `continue` are refused too,
        // so it always finishes - the exit is the body's exit. Whoever lands
        // `break` inherits this: a body that selects and then breaks past a
        // later select would exit here with the later select's answer.
        let (body_statements, body_always_returns) = self.lower_statements(body);
        output.extend(body_statements);
        output.push(Statement::Assign {
            destination: Destination::Place(counter),
            value: self.binary(
                Value::Place(counter),
                BinaryOperator::Add,
                Value::Constant(step),
                binding.span,
            ),
        });
        output.push(Statement::Jump { target: loop_start });
        output.push(Statement::Label(loop_end));
        self.leave_scope();
        body_always_returns
    }

    fn lower_expression_statement(
        &mut self,
        expression: &Spanned<SyntaxExpression>,
        output: &mut Vec<Statement>,
    ) {
        if let SyntaxExpression::Infix {
            left,
            operator,
            right,
        } = &expression.value
        {
            if matches!(
                operator.value,
                Operator::Assign
                    | Operator::PlusEqual
                    | Operator::MinusEqual
                    | Operator::StarEqual
                    | Operator::SlashEqual
            ) {
                let Some(destination) = self.lower_destination(left) else {
                    return;
                };
                // The operator, decided once, because the write refusal below
                // quotes its spelling and the compound read further down quotes
                // it too. `None` is a plain `=`. The error arm is an operator
                // the `matches!` above admits and `compound_operator` does not
                // map, which is unreachable today and stays as a guard.
                let compound = if operator.value == Operator::Assign {
                    None
                } else {
                    match compound_operator(operator.value) {
                        Some(pair) => Some(pair),
                        None => {
                            self.error(operator.span, "this compound assignment is not supported");
                            return;
                        }
                    }
                };
                // `left.span` covers the destination and nothing else: the
                // label names the port, and the right-hand side is not part of
                // the port.
                let write_site = match compound {
                    Some((_, spelling)) => WriteSite::CompoundAssignment(spelling),
                    None => WriteSite::Named,
                };
                // The write check comes before the compound read below, so the
                // error a reader sees is always the one at the start of the
                // line.
                if !self.write_destination(destination, left.span, write_site) {
                    // The right-hand side is still lowered, so its own faults
                    // are reported in this run rather than the next one — Q6:
                    // both errors on `ppu.status = ppu.mask`. Nothing is pushed:
                    // the statement has been refused, so it has no `Assign` to
                    // emit, and returning here also keeps the warnings below
                    // from firing on a line already refused.
                    let _ = self.lower_value(right);
                    return;
                }
                // Q7: one fault, one message. If lowering this statement's own
                // value refused something — a read of a write-only register,
                // say — the statement is being rewritten, so what its value
                // would have been is moot and the bank-select warning below is
                // unactionable until the refusal is fixed. Counted rather than
                // returned, so it covers a plain assignment as well as the
                // compound arm's early `return`.
                let errors_before = self.errors.len();
                let value = match compound {
                    None => self.lower_value(right),
                    Some((binary, spelling)) => {
                        let Some(left_value) = self.destination_read(
                            destination,
                            left.span,
                            ReadSite::CompoundAssignment(spelling),
                        ) else {
                            // The right-hand side is still lowered, so its own
                            // faults are reported in this run rather than the
                            // next one. Nothing is pushed: the statement has
                            // been refused, so it has no `Assign` to emit — and
                            // returning here is also what keeps
                            // `bank_select_warning` below from firing on a line
                            // whose read has already been refused.
                            let _ = self.lower_value(right);
                            return;
                        };
                        let right = self.lower_value(right);
                        self.binary(left_value, binary, right, expression.span)
                    }
                };
                // Whether lowering this statement's own value refused something —
                // a read of a write-only register, say. Q7 of `raster-1t9`: one
                // fault, one message, so the bank-select warning below stands
                // aside, because it is about a value that is now a placeholder.
                // The bank-data warning does not, because it is about which
                // register is selected, which is a fact about the statements
                // before it and stays true once the read is fixed.
                let refused = self.errors.len() != errors_before;
                if destination == Destination::Register(Register::Mmc3BankSelect) {
                    if !refused {
                        if let Some(warning) = bank_select_warning(&value, expression.span) {
                            self.warnings.push(warning);
                        }
                    }
                    self.selection = match value {
                        // A refused value lowers to a `Constant(0)` placeholder
                        // rather than to a byte the author wrote, so it is not
                        // a selection rasterc has seen.
                        Value::Constant(bits) if !refused => BankSelection::known(bits),
                        _ => BankSelection::Unknown(Unseen::InThisBody),
                    };
                }
                if destination == Destination::Register(Register::Mmc3BankData) {
                    if let Some(warning) = bank_data_warning(self.selection, expression.span) {
                        self.warnings.push(warning);
                    }
                }
                output.push(Statement::Assign { destination, value });
                return;
            }
        }
        match self.lower_value(expression) {
            Value::Call {
                target,
                arguments,
                argument_temporaries,
            } => output.push(Statement::Call {
                target,
                arguments,
                argument_temporaries,
            }),
            _ => self.error(
                expression.span,
                "only direct calls and assignments are supported as statements",
            ),
        }
    }

    fn lower_destination(&mut self, expression: &Spanned<SyntaxExpression>) -> Option<Destination> {
        match &expression.value {
            SyntaxExpression::Name(name) => match self.lookup(&name.value) {
                Some(Binding::Place(place)) => Some(Destination::Place(place)),
                Some(Binding::Constant(_)) => {
                    self.error(name.span, "constants cannot be assigned");
                    None
                }
                None => {
                    self.error(name.span, format!("unknown place `{}`", name.value));
                    None
                }
            },
            SyntaxExpression::Member { base, member } => {
                self.register(base, member).map(Destination::Register)
            }
            SyntaxExpression::Index { base, index } => {
                self.not_in_this_release(expression.span, "arrays are not supported yet");
                let _ = self.lower_value(base);
                let _ = self.lower_value(index);
                None
            }
            _ => {
                self.error(expression.span, "assignment target is not supported");
                None
            }
        }
    }

    fn lower_condition(&mut self, expression: &Spanned<SyntaxExpression>) -> Condition {
        if let SyntaxExpression::Prefix { operator, operand } = &expression.value {
            if operator.value == Operator::Bang {
                let mut condition = self.lower_condition(operand);
                condition.comparison = condition.comparison.inverted();
                return condition;
            }
        }
        if let SyntaxExpression::Infix {
            left,
            operator,
            right,
        } = &expression.value
        {
            if let Some(comparison) = comparison_operator(operator.value) {
                let left = self.lower_value(left);
                let right = self.lower_value(right);
                return self.condition(left, comparison, right, expression.span);
            }
        }
        self.not_in_this_release(
            expression.span,
            "bool expressions are not supported; use a u8 comparison",
        );
        let _ = self.lower_value(expression);
        self.condition(
            Value::Constant(0),
            Comparison::NotEqual,
            Value::Constant(0),
            expression.span,
        )
    }

    fn condition(
        &mut self,
        left: Value,
        comparison: Comparison,
        right: Value,
        span: Span,
    ) -> Condition {
        Condition {
            left,
            comparison,
            right,
            left_temporary: self.allocate_place(PlaceKind::Temporary, span, false),
            right_temporary: self.allocate_place(PlaceKind::Temporary, span, false),
        }
    }

    fn lower_value(&mut self, expression: &Spanned<SyntaxExpression>) -> Value {
        match &expression.value {
            SyntaxExpression::Name(name) => match self.lookup(&name.value) {
                Some(Binding::Place(place)) => Value::Place(place),
                Some(Binding::Constant(value)) => Value::Constant(value),
                None => {
                    self.error(name.span, format!("unknown value `{}`", name.value));
                    Value::Constant(0)
                }
            },
            SyntaxExpression::Number(number) => match parse_number(number).and_then(to_u8) {
                Some(value) => Value::Constant(value),
                None => {
                    self.error(expression.span, "u8 literal is out of range");
                    Value::Constant(0)
                }
            },
            SyntaxExpression::String(_) => {
                self.not_in_this_release(expression.span, "string expressions are not supported");
                Value::Constant(0)
            }
            SyntaxExpression::Character(_) => {
                self.not_in_this_release(
                    expression.span,
                    "character expressions are not supported",
                );
                Value::Constant(0)
            }
            SyntaxExpression::Boolean(_) => {
                self.not_in_this_release(expression.span, "bool expressions are not supported");
                Value::Constant(0)
            }
            SyntaxExpression::Prefix { operator, operand } => match operator.value {
                Operator::Plus => self.lower_value(operand),
                Operator::Minus => Value::Unary {
                    operator: UnaryOperator::Negate,
                    operand: Box::new(self.lower_value(operand)),
                },
                Operator::Tilde => Value::Unary {
                    operator: UnaryOperator::Not,
                    operand: Box::new(self.lower_value(operand)),
                },
                Operator::Bang => {
                    self.not_in_this_release(operator.span, "bool expressions are not supported");
                    let _ = self.lower_value(operand);
                    Value::Constant(0)
                }
                _ => {
                    self.error(operator.span, "this prefix expression is not supported");
                    Value::Constant(0)
                }
            },
            SyntaxExpression::Infix {
                left,
                operator,
                right,
            } => {
                let Some(operator) = binary_operator(operator.value) else {
                    self.error(
                        operator.span,
                        "this expression is not supported for u8 code generation",
                    );
                    let _ = self.lower_value(left);
                    let _ = self.lower_value(right);
                    return Value::Constant(0);
                };
                let left = self.lower_value(left);
                let right = self.lower_value(right);
                self.binary(left, operator, right, expression.span)
            }
            SyntaxExpression::Call { callee, arguments } => {
                let SyntaxExpression::Name(name) = &callee.value else {
                    self.error(callee.span, "call target must be a direct function name");
                    return Value::Constant(0);
                };
                let Some(function) = self.functions.get(&name.value).copied() else {
                    self.error(name.span, format!("unknown function `{}`", name.value));
                    return Value::Constant(0);
                };
                // The one site every call passes through, statement or
                // expression: what the callee selected is not visible here.
                if self.selects_bank.contains(&name.value) {
                    self.selection = BankSelection::Unknown(Unseen::InThisBody);
                }
                let arguments: Vec<_> = arguments
                    .iter()
                    .map(|argument| self.lower_value(argument))
                    .collect();
                let argument_temporaries = (0..arguments.len())
                    .map(|_| self.allocate_place(PlaceKind::Temporary, expression.span, false))
                    .collect();
                Value::Call {
                    target: function.label,
                    arguments,
                    argument_temporaries,
                }
            }
            SyntaxExpression::Index { base, index } => {
                self.not_in_this_release(expression.span, "arrays are not supported yet");
                let _ = self.lower_value(base);
                let _ = self.lower_value(index);
                Value::Constant(0)
            }
            SyntaxExpression::Member { base, member } => match self.register(base, member) {
                // `expression.span` covers `ppu.mask` exactly: the parser joins
                // the namespace's span with the member's.
                Some(register) => self
                    .read_register(register, expression.span, ReadSite::Named)
                    .unwrap_or(Value::Constant(0)),
                None => Value::Constant(0),
            },
            SyntaxExpression::Range { start, end } => {
                self.error(
                    expression.span,
                    "ranges are only supported by finite for loops",
                );
                let _ = self.lower_value(start);
                let _ = self.lower_value(end);
                Value::Constant(0)
            }
        }
    }

    /// A read of a hardware register, or `None` once it has been refused.
    ///
    /// Both ways to read a register come here — naming it in an expression, and
    /// a compound assignment reading its own destination — so the two cannot
    /// drift apart. A write never comes here: every one of the sixteen may
    /// still be written, and this changes nothing about a write.
    fn read_register(&mut self, register: Register, span: Span, site: ReadSite) -> Option<Value> {
        if register == Register::PpuData {
            return self.read_ppu_data(span, site);
        }
        if !register.is_write_only() {
            return Some(Value::Register(register));
        }
        let mut notes = Vec::new();
        if let ReadSite::CompoundAssignment(spelling) = site {
            notes.push(format!(
                "`{spelling}` reads its destination before it writes, so this reads ${:04X}",
                register.address()
            ));
        }
        notes.push(dead_read_note(register));
        notes.push(WRITE_THE_WHOLE_VALUE.to_owned());
        self.refuse_with(
            span,
            format!("`{}` cannot be read", register.name()),
            format!("${:04X} is a write-only port", register.address()),
            notes,
        );
        None
    }

    /// A read of `$2007`, which reads back but not the byte you just addressed.
    ///
    /// Unlike the thirteen ports raster-1t9 refuses, this one has a sequence
    /// that is correct — read twice, keep the second — so a plain read is a
    /// warning rather than a refusal. A compound assignment is refused: its
    /// read happens inside the statement, where no second read can reach it.
    fn read_ppu_data(&mut self, span: Span, site: ReadSite) -> Option<Value> {
        if let ReadSite::CompoundAssignment(spelling) = site {
            self.refuse_with(
                span,
                "`ppu.data` cannot be the destination of a compound assignment",
                format!("`{spelling}` reads $2007 before it writes, and that read is buffered"),
                vec![
                    PPU_DATA_COMPOUND_NOTE.to_owned(),
                    PPU_DATA_KEEP_IT.to_owned(),
                ],
            );
            return None;
        }
        if !self.ppu_data_read_has_neighbour {
            self.warnings.push(LowerWarning {
                message: "this `ppu.data` read gives you the byte before the one you asked for"
                    .to_owned(),
                label: "nothing next to this read primes the PPU's read buffer".to_owned(),
                notes: vec![
                    PPU_DATA_BUFFER_NOTE.to_owned(),
                    PPU_DATA_READ_TWICE.to_owned(),
                ],
                span,
            });
        }
        Some(Value::Register(Register::PpuData))
    }

    /// The value a compound assignment reads out of its own destination, or
    /// `None` if that destination is a port that does not read. Replaces the
    /// free function `destination_value`, which had no way to refuse.
    fn destination_read(
        &mut self,
        destination: Destination,
        span: Span,
        site: ReadSite,
    ) -> Option<Value> {
        match destination {
            Destination::Place(place) => Some(Value::Place(place)),
            Destination::Register(register) => self.read_register(register, span, site),
        }
    }

    /// Whether this destination may be written, refusing it if it may not.
    ///
    /// Every write of a register comes here — a plain assignment and the write
    /// a compound assignment makes — so the two cannot drift apart. This is the
    /// mirror of `read_register`, and the two are deliberately separate
    /// functions: a register can be unreadable, unwritable, or neither, and
    /// nothing about one answers the other.
    ///
    /// Returns `true` when the write may proceed, so the caller reads as a
    /// guard. A `Place` is always writable.
    fn write_destination(&mut self, destination: Destination, span: Span, site: WriteSite) -> bool {
        let Destination::Register(register) = destination else {
            return true;
        };
        if !register.is_read_only() {
            return true;
        }
        let mut notes = Vec::new();
        if let WriteSite::CompoundAssignment(spelling) = site {
            notes.push(format!(
                "`{spelling}` writes its destination, so this writes ${:04X}",
                register.address()
            ));
        }
        notes.push(dead_write_note(register));
        notes.push(DELETE_THE_LINE.to_owned());
        self.refuse_with(
            span,
            format!("`{}` cannot be written", register.name()),
            format!("${:04X} is a read-only port", register.address()),
            notes,
        );
        false
    }

    fn binary(&mut self, left: Value, operator: BinaryOperator, right: Value, span: Span) -> Value {
        Value::Binary {
            left: Box::new(left),
            operator,
            right: Box::new(right),
            left_temporary: self.allocate_place(PlaceKind::Temporary, span, false),
            right_temporary: self.allocate_place(PlaceKind::Temporary, span, false),
        }
    }

    fn register(
        &mut self,
        base: &Spanned<SyntaxExpression>,
        member: &Spanned<String>,
    ) -> Option<Register> {
        let SyntaxExpression::Name(namespace) = &base.value else {
            self.error(base.span, "register access requires a namespace");
            return None;
        };
        let register = match (namespace.value.as_str(), member.value.as_str()) {
            ("ppu", "ctrl") => Register::PpuCtrl,
            ("ppu", "mask") => Register::PpuMask,
            ("ppu", "status") => Register::PpuStatus,
            ("ppu", "oam_addr") => Register::PpuOamAddr,
            ("ppu", "oam_data") => Register::PpuOamData,
            ("ppu", "scroll") => Register::PpuScroll,
            ("ppu", "addr") => Register::PpuAddr,
            ("ppu", "data") => Register::PpuData,
            ("mmc3", "bank_select") => Register::Mmc3BankSelect,
            ("mmc3", "bank_data") => Register::Mmc3BankData,
            ("mmc3", "mirroring") => Register::Mmc3Mirroring,
            ("mmc3", "ram_protect") => Register::Mmc3RamProtect,
            ("mmc3", "irq_latch") => Register::Mmc3IrqLatch,
            ("mmc3", "irq_reload") => Register::Mmc3IrqReload,
            ("mmc3", "irq_disable") => Register::Mmc3IrqDisable,
            ("mmc3", "irq_enable") => Register::Mmc3IrqEnable,
            _ => {
                self.not_in_this_release(member.span, "this byte register is not supported");
                return None;
            }
        };
        Some(register)
    }

    fn declaration_is_u8(&mut self, declaration: &Declaration) -> bool {
        declaration
            .type_annotation
            .as_ref()
            .map(|annotation| self.annotation_is_u8(annotation))
            .unwrap_or(true)
    }

    fn annotation_is_u8(&mut self, annotation: &Spanned<Type>) -> bool {
        match &annotation.value {
            Type::Name(name) if name.value == "u8" => true,
            Type::Name(name) if name.value == "u16" => {
                self.not_in_this_release(name.span, "`u16` values are not supported yet");
                false
            }
            Type::Name(name) if name.value == "bool" => {
                self.not_in_this_release(
                    name.span,
                    "bool storage and expressions are not supported",
                );
                false
            }
            Type::Name(name) if name.value == "void" => {
                self.error(name.span, "void storage is not supported");
                false
            }
            Type::Name(name) => {
                self.error(name.span, "this storage type is not supported");
                false
            }
            Type::Array { .. } => {
                self.not_in_this_release(annotation.span, "arrays are not supported yet");
                false
            }
        }
    }

    fn function_return_is_supported(&mut self, function: &SyntaxFunction) -> bool {
        let Some(return_type) = &function.return_type else {
            return true;
        };
        match &return_type.value {
            Type::Name(name) if name.value == "void" || name.value == "u8" => true,
            _ => self.annotation_is_u8(return_type),
        }
    }

    fn check_storage(&mut self, storage: Option<&Spanned<String>>) -> bool {
        match storage {
            None => true,
            Some(storage) if storage.value == "zp" => true,
            Some(storage) => {
                self.not_in_this_release(storage.span, "only `in zp` storage is supported");
                false
            }
        }
    }

    fn cycle_constraint(
        &mut self,
        spec: &raster_syntax::CycleSpec,
        label: Option<&Spanned<String>>,
    ) -> Option<CycleConstraint> {
        match &spec.bound {
            CycleBound::Exact(value) => self.constant_value_u32(value).map(CycleConstraint::Exact),
            CycleBound::AtMost(value) => {
                self.constant_value_u32(value).map(CycleConstraint::AtMost)
            }
            CycleBound::Inferred(_) => Some(CycleConstraint::Report {
                label: label.map(|label| label.value.clone()).unwrap_or_default(),
            }),
        }
    }

    fn constant_value(&mut self, expression: &Spanned<SyntaxExpression>) -> Option<u8> {
        let value = self.constant_value_u32(expression)?;
        match to_u8(value) {
            Some(value) => Some(value),
            None => {
                self.error(expression.span, "u8 constant is out of range");
                None
            }
        }
    }

    fn constant_value_u32(&mut self, expression: &Spanned<SyntaxExpression>) -> Option<u32> {
        match &expression.value {
            SyntaxExpression::Number(number) => parse_number(number).or_else(|| {
                self.error(expression.span, "invalid numeric constant");
                None
            }),
            SyntaxExpression::Name(name) => match self.lookup(&name.value) {
                Some(Binding::Constant(value)) => Some(u32::from(value)),
                _ => {
                    self.error(name.span, "constant expression is required");
                    None
                }
            },
            SyntaxExpression::Prefix { operator, operand } if operator.value == Operator::Tilde => {
                self.constant_value_u32(operand).map(|value| !value)
            }
            SyntaxExpression::Infix {
                left,
                operator,
                right,
            } => {
                let left = self.constant_value_u32(left)?;
                let right = self.constant_value_u32(right)?;
                match operator.value {
                    Operator::Plus => left.checked_add(right),
                    Operator::Minus => left.checked_sub(right),
                    Operator::Star => left.checked_mul(right),
                    Operator::Slash if right != 0 => left.checked_div(right),
                    Operator::Percent if right != 0 => left.checked_rem(right),
                    Operator::Ampersand => Some(left & right),
                    Operator::Pipe => Some(left | right),
                    Operator::Caret => Some(left ^ right),
                    Operator::ShiftLeft => left.checked_shl(right),
                    Operator::ShiftRight => left.checked_shr(right),
                    _ => None,
                }
                .or_else(|| {
                    self.error(expression.span, "constant expression is not supported");
                    None
                })
            }
            _ => {
                self.error(expression.span, "constant expression is required");
                None
            }
        }
    }

    fn enter_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    fn leave_scope(&mut self) {
        let _ = self.scopes.pop();
    }

    fn bind(&mut self, name: &str, binding: Binding) {
        self.scopes
            .last_mut()
            .expect("lowerer always has a scope")
            .insert(name.to_owned(), binding);
    }

    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }
}

#[derive(Clone, Debug)]
struct FunctionCall {
    target: String,
    span: Span,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Visited,
}

/// Every function that can reach a `mmc3.bank_select` write.
///
/// A fixed point rather than a recursive walk: `reject_recursive_calls` records
/// recursion as an *error* and `lower_program` runs regardless, so the call
/// graph reaching here may still contain a cycle. Each pass either adds a name
/// or stops, so this terminates in at most one pass per function.
fn functions_that_select(syntax: &SyntaxProgram) -> BTreeSet<String> {
    let bodies: Vec<(&str, &Block)> = syntax
        .items
        .iter()
        .filter_map(|item| match &item.value {
            Item::Function(function) => Some((function.name.value.as_str(), &function.body)),
            _ => None,
        })
        .collect();
    let mut selects = BTreeSet::new();
    loop {
        let mut grew = false;
        for (name, body) in &bodies {
            if !selects.contains(*name) && block_selects_bank(body, &selects) {
                selects.insert((*name).to_owned());
                grew = true;
            }
        }
        if !grew {
            return selects;
        }
    }
}

fn function_call_graph(syntax: &SyntaxProgram) -> BTreeMap<String, Vec<FunctionCall>> {
    syntax
        .items
        .iter()
        .filter_map(|item| match &item.value {
            Item::Function(function) => {
                let mut calls = Vec::new();
                collect_calls_in_block(&function.body, &mut calls);
                Some((function.name.value.clone(), calls))
            }
            _ => None,
        })
        .collect()
}

/// Whether this block can reach a write to `mmc3.bank_select` - directly, or
/// through a call to a function in `via`.
///
/// One walker serves two callers. It fills `Lowerer::selects_bank` at the fixed
/// point below, and it answers the loop-body question in `lower_while`,
/// `lower_for` and the `loop` arm: lowering walks a body once, but a loop runs
/// it many times, so a body that can select must be judged with the selection
/// already unknown or the second iteration is judged against the first
/// iteration's answer.
fn block_selects_bank(block: &Block, via: &BTreeSet<String>) -> bool {
    let mut calls = Vec::new();
    collect_calls_in_block(block, &mut calls);
    calls.iter().any(|call| via.contains(&call.target)) || block_assigns_bank_select(block)
}

/// Whether any statement in this block assigns to `mmc3.bank_select`.
///
/// It mirrors the statement shape of `collect_calls_in_statement` and is
/// deliberately narrower than it: an assignment is a statement, so this does
/// not descend into conditions, ranges, initializers or returned values, where
/// a call can hide but an assignment cannot. `raster-zbh` is the bead for
/// merging the two walkers - the shapes are not interchangeable as they stand.
fn block_assigns_bank_select(block: &Block) -> bool {
    block
        .statements
        .iter()
        .any(|statement| statement_assigns_bank_select(&statement.value))
}

fn statement_assigns_bank_select(statement: &SyntaxStatement) -> bool {
    match statement {
        // An initializer is an expression and cannot assign; only a
        // declaration's body can hold statements.
        SyntaxStatement::Declaration(declaration) => declaration
            .body
            .as_ref()
            .is_some_and(block_assigns_bank_select),
        SyntaxStatement::Block(block) | SyntaxStatement::Loop(block) => {
            block_assigns_bank_select(block)
        }
        SyntaxStatement::If {
            then_body,
            else_body,
            ..
        } => {
            block_assigns_bank_select(then_body)
                || else_body.as_ref().is_some_and(block_assigns_bank_select)
        }
        SyntaxStatement::While { body, .. }
        | SyntaxStatement::For { body, .. }
        | SyntaxStatement::Cycles { body, .. } => block_assigns_bank_select(body),
        SyntaxStatement::Expression(expression) => expression_assigns_bank_select(expression),
        SyntaxStatement::Wait(_)
        | SyntaxStatement::Sync(_)
        | SyntaxStatement::Return(_)
        | SyntaxStatement::Break
        | SyntaxStatement::Continue => false,
    }
}

fn expression_assigns_bank_select(expression: &Spanned<SyntaxExpression>) -> bool {
    let SyntaxExpression::Infix { left, operator, .. } = &expression.value else {
        return false;
    };
    if !matches!(
        operator.value,
        Operator::Assign
            | Operator::PlusEqual
            | Operator::MinusEqual
            | Operator::StarEqual
            | Operator::SlashEqual
    ) {
        return false;
    }
    let SyntaxExpression::Member { base, member } = &left.value else {
        return false;
    };
    let SyntaxExpression::Name(name) = &base.value else {
        return false;
    };
    name.value == "mmc3" && member.value == "bank_select"
}

fn collect_calls_in_block(block: &Block, calls: &mut Vec<FunctionCall>) {
    for statement in &block.statements {
        collect_calls_in_statement(&statement.value, calls);
    }
}

fn collect_calls_in_statement(statement: &SyntaxStatement, calls: &mut Vec<FunctionCall>) {
    match statement {
        SyntaxStatement::Declaration(declaration) => {
            if let Some(initializer) = &declaration.initializer {
                collect_calls_in_expression(initializer, calls);
            }
            if let Some(body) = &declaration.body {
                collect_calls_in_block(body, calls);
            }
        }
        SyntaxStatement::Block(block) | SyntaxStatement::Loop(block) => {
            collect_calls_in_block(block, calls);
        }
        SyntaxStatement::If {
            condition,
            then_body,
            else_body,
        } => {
            collect_calls_in_expression(condition, calls);
            collect_calls_in_block(then_body, calls);
            if let Some(else_body) = else_body {
                collect_calls_in_block(else_body, calls);
            }
        }
        SyntaxStatement::While { condition, body } => {
            collect_calls_in_expression(condition, calls);
            collect_calls_in_block(body, calls);
        }
        SyntaxStatement::For {
            range, step, body, ..
        } => {
            collect_calls_in_expression(range, calls);
            if let Some(step) = step {
                collect_calls_in_expression(step, calls);
            }
            collect_calls_in_block(body, calls);
        }
        SyntaxStatement::Cycles { spec, body, .. } => {
            collect_calls_in_cycle_bound(&spec.bound, calls);
            collect_calls_in_block(body, calls);
        }
        SyntaxStatement::Wait(Wait::Cycles(value) | Wait::Scanline(value)) => {
            collect_calls_in_expression(value, calls);
        }
        SyntaxStatement::Return(Some(value)) | SyntaxStatement::Expression(value) => {
            collect_calls_in_expression(value, calls);
        }
        SyntaxStatement::Wait(Wait::Vblank(_))
        | SyntaxStatement::Sync(_)
        | SyntaxStatement::Return(None)
        | SyntaxStatement::Break
        | SyntaxStatement::Continue => {}
    }
}

fn collect_calls_in_cycle_bound(bound: &CycleBound, calls: &mut Vec<FunctionCall>) {
    match bound {
        CycleBound::Exact(value) | CycleBound::AtMost(value) => {
            collect_calls_in_expression(value, calls);
        }
        CycleBound::Inferred(_) => {}
    }
}

fn collect_calls_in_expression(
    expression: &Spanned<SyntaxExpression>,
    calls: &mut Vec<FunctionCall>,
) {
    match &expression.value {
        SyntaxExpression::Prefix { operand, .. } => collect_calls_in_expression(operand, calls),
        SyntaxExpression::Infix { left, right, .. }
        | SyntaxExpression::Index {
            base: left,
            index: right,
        }
        | SyntaxExpression::Range {
            start: left,
            end: right,
        } => {
            collect_calls_in_expression(left, calls);
            collect_calls_in_expression(right, calls);
        }
        SyntaxExpression::Call { callee, arguments } => {
            if let SyntaxExpression::Name(name) = &callee.value {
                calls.push(FunctionCall {
                    target: name.value.clone(),
                    span: name.span,
                });
            }
            collect_calls_in_expression(callee, calls);
            for argument in arguments {
                collect_calls_in_expression(argument, calls);
            }
        }
        SyntaxExpression::Member { base, .. } => collect_calls_in_expression(base, calls),
        SyntaxExpression::Name(_)
        | SyntaxExpression::Number(_)
        | SyntaxExpression::String(_)
        | SyntaxExpression::Character(_)
        | SyntaxExpression::Boolean(_) => {}
    }
}

fn function_returns_u8(function: &SyntaxFunction) -> bool {
    matches!(
        function.return_type.as_ref().map(|return_type| &return_type.value),
        Some(Type::Name(name)) if name.value == "u8"
    )
}

fn declaration_name_span(declaration: &Declaration) -> Span {
    declaration
        .name
        .as_ref()
        .map(|name| name.span)
        .unwrap_or_default()
}

fn parse_number(number: &str) -> Option<u32> {
    number.strip_prefix('$').map_or_else(
        || number.parse().ok(),
        |hexadecimal| u32::from_str_radix(hexadecimal, 16).ok(),
    )
}

fn to_u8(value: u32) -> Option<u8> {
    u8::try_from(value).ok()
}

fn binary_operator(operator: Operator) -> Option<BinaryOperator> {
    match operator {
        Operator::Plus => Some(BinaryOperator::Add),
        Operator::Minus => Some(BinaryOperator::Subtract),
        Operator::Star => Some(BinaryOperator::Multiply),
        Operator::Slash => Some(BinaryOperator::Divide),
        Operator::Percent => Some(BinaryOperator::Remainder),
        Operator::Ampersand => Some(BinaryOperator::And),
        Operator::Pipe => Some(BinaryOperator::Or),
        Operator::Caret => Some(BinaryOperator::Xor),
        Operator::ShiftLeft => Some(BinaryOperator::ShiftLeft),
        Operator::ShiftRight => Some(BinaryOperator::ShiftRight),
        _ => None,
    }
}

/// The binary operator a compound assignment applies, and how it is written.
/// The spelling is what a refusal quotes when it explains where a read the
/// author did not write came from; keeping both in one match is what stops the
/// two lists drifting.
fn compound_operator(operator: Operator) -> Option<(BinaryOperator, &'static str)> {
    match operator {
        Operator::PlusEqual => Some((BinaryOperator::Add, "+=")),
        Operator::MinusEqual => Some((BinaryOperator::Subtract, "-=")),
        Operator::StarEqual => Some((BinaryOperator::Multiply, "*=")),
        Operator::SlashEqual => Some((BinaryOperator::Divide, "/=")),
        _ => None,
    }
}

fn comparison_operator(operator: Operator) -> Option<Comparison> {
    match operator {
        Operator::EqualEqual => Some(Comparison::Equal),
        Operator::BangEqual => Some(Comparison::NotEqual),
        Operator::Less => Some(Comparison::Less),
        Operator::LessEqual => Some(Comparison::LessEqual),
        Operator::Greater => Some(Comparison::Greater),
        Operator::GreaterEqual => Some(Comparison::GreaterEqual),
        _ => None,
    }
}

/// Record every store to `ppu.ctrl` and `ppu.mask` in `statements`, following calls.
///
/// `visiting` is the call stack: `reject_recursive_calls` has already refused a recursive program,
/// and this walk only runs on one with no errors, so the guard is what makes the recursion provably
/// finite rather than merely finite in practice.
///
/// `conditional` says whether these statements are themselves reached only on some paths — see
/// [`Conditionally`] for how a store under a branch is told from one after it. Taking the
/// *textually* last store instead made `if c { mask = $1e } else { mask = $00 }` and the same two
/// arms swapped disagree with each other, and let the second of them compile a ROM that turns
/// rendering off on a path the check never looked at.
fn collect_ppu_stores(
    statements: &[Statement],
    functions: &BTreeMap<Label, &Function>,
    visiting: &mut Vec<Label>,
    configuration: &mut PpuConfiguration,
    conditional: bool,
) {
    let mut reached = Conditionally::new(conditional);
    for statement in statements {
        reached.before(statement);
        let conditional = reached.here();
        match statement {
            Statement::Assign { destination, value } => {
                // The value runs before the store, calls in it included.
                collect_value_stores(value, functions, visiting, configuration, conditional);
                let Destination::Register(register) = destination else {
                    continue;
                };
                let state = match value {
                    _ if conditional => RegisterState::Conditional,
                    Value::Constant(value) => RegisterState::Known(*value),
                    _ => RegisterState::Unproven,
                };
                match register {
                    Register::PpuCtrl => configuration.ctrl = state,
                    Register::PpuMask => configuration.mask = state,
                    _ => {}
                }
            }
            Statement::Call {
                target, arguments, ..
            } => {
                for argument in arguments {
                    collect_value_stores(argument, functions, visiting, configuration, conditional);
                }
                enter_function(*target, functions, visiting, configuration, conditional);
            }
            Statement::Branch { condition, .. } => {
                collect_value_stores(
                    &condition.left,
                    functions,
                    visiting,
                    configuration,
                    conditional,
                );
                collect_value_stores(
                    &condition.right,
                    functions,
                    visiting,
                    configuration,
                    conditional,
                );
            }
            Statement::Return(Some(value)) => {
                collect_value_stores(value, functions, visiting, configuration, conditional)
            }
            Statement::Timed { body, .. } => {
                collect_ppu_stores(body, functions, visiting, configuration, conditional)
            }
            _ => {}
        }
    }
}

/// Which statements of a flattened body run every time it runs, and which run only on some paths.
///
/// The IR is flat, so an `if` is a `Branch` to a label further down and an `else` adds a `Jump` over
/// the first arm. A statement is conditional exactly while some *forward* jump is outstanding — one
/// whose label has not been passed yet, which is the jump that could have skipped over here. A
/// backward jump is a loop's own edge and skips nothing, so it is not counted; without that, every
/// statement after the first `while` would read as conditional and a configuration written after
/// the loop would be refused.
struct Conditionally {
    enclosing: bool,
    seen: BTreeSet<Label>,
    outstanding: BTreeSet<Label>,
}

impl Conditionally {
    fn new(enclosing: bool) -> Self {
        Self {
            enclosing,
            seen: BTreeSet::new(),
            outstanding: BTreeSet::new(),
        }
    }

    /// Take account of `statement` before it is walked.
    fn before(&mut self, statement: &Statement) {
        match statement {
            Statement::Label(label) => {
                self.seen.insert(*label);
                self.outstanding.remove(label);
            }
            Statement::Branch { if_false, .. } => self.forward(*if_false),
            Statement::Jump { target } => self.forward(*target),
            _ => {}
        }
    }

    fn forward(&mut self, target: Label) {
        if !self.seen.contains(&target) {
            self.outstanding.insert(target);
        }
    }

    fn here(&self) -> bool {
        self.enclosing || !self.outstanding.is_empty()
    }
}

/// Follow a value's own calls: a function reached through `x = configure()` or through an argument
/// writes the same registers a called statement does, and a walk that could not see it would report
/// a configuration the program never had — in either direction.
fn collect_value_stores(
    value: &Value,
    functions: &BTreeMap<Label, &Function>,
    visiting: &mut Vec<Label>,
    configuration: &mut PpuConfiguration,
    conditional: bool,
) {
    match value {
        Value::Constant(_) | Value::Place(_) | Value::Register(_) => {}
        Value::Unary { operand, .. } => {
            collect_value_stores(operand, functions, visiting, configuration, conditional)
        }
        Value::Binary { left, right, .. } => {
            collect_value_stores(left, functions, visiting, configuration, conditional);
            collect_value_stores(right, functions, visiting, configuration, conditional);
        }
        Value::Call {
            target, arguments, ..
        } => {
            for argument in arguments {
                collect_value_stores(argument, functions, visiting, configuration, conditional);
            }
            enter_function(*target, functions, visiting, configuration, conditional);
        }
    }
}

/// Walk a called function's body, unless it is already on the call stack.
fn enter_function(
    target: Label,
    functions: &BTreeMap<Label, &Function>,
    visiting: &mut Vec<Label>,
    configuration: &mut PpuConfiguration,
    conditional: bool,
) {
    let Some(function) = functions.get(&target) else {
        return;
    };
    if visiting.contains(&target) {
        return;
    }
    visiting.push(target);
    collect_ppu_stores(
        &function.statements,
        functions,
        visiting,
        configuration,
        conditional,
    );
    visiting.pop();
}

/// The diagnostic an MMC3 precondition failure reads as, naming the value that caused it.
///
/// Each of these is about a ROM that would assemble and never take an interrupt, so the message
/// says both what the strategy needs and what the program actually wrote.
fn mmc3_irq_message(error: Mmc3IrqError) -> String {
    match error {
        Mmc3IrqError::RenderingDisabled { mask } => format!(
            "`using irq` needs rendering enabled, and `ppu.mask = ${mask:02X}` enables neither \
             the background nor the sprites"
        ),
        Mmc3IrqError::TallSpritesUncheckable { ctrl } => format!(
            "`using irq` cannot check the A12 pattern for 8x16 sprites, and \
             `ppu.ctrl = ${ctrl:02X}` selects them"
        ),
        Mmc3IrqError::PatternTablesShareHalf { ctrl } => {
            let half = if ctrl & 0b0001_0000 != 0 {
                "$1000"
            } else {
                "$0000"
            };
            format!(
                "`using irq` needs the background and sprite pattern tables in opposite halves, \
                 and `ppu.ctrl = ${ctrl:02X}` puts both at {half}"
            )
        }
        Mmc3IrqError::UnprovenConfiguration { register } => format!(
            "`using irq` needs a constant `{register}` before the frame, and this program's last \
             write to it is not one"
        ),
        Mmc3IrqError::ConditionalConfiguration { register } => format!(
            "`using irq` needs a constant `{register}` before the frame, and this program writes \
             it on some paths and not others"
        ),
    }
}
