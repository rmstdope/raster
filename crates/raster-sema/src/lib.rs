use std::collections::{BTreeMap, BTreeSet, HashMap};

mod apu;

use raster_diag::Refusal;
use raster_syntax::{
    AssetValue, Block, CycleBound, Declaration, Expression, FrameEvent, FramePosition, Function,
    Item, Keyword, Operator, Program, Span, Spanned, Statement, Type, Wait,
};

#[derive(Debug)]
pub struct SemanticError {
    pub message: String,
    pub span: Span,
    pub refusal: Refusal,
    /// What to print under the carets, when the refusal has words of its own; otherwise the
    /// message is repeated there.
    pub label: Option<String>,
    /// Notes that belong to this refusal alone.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TypedProgram {
    pub program: Program,
    pub constants: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ValueType {
    U8,
    U16,
    Bool,
    Array(Box<ValueType>, u32),
    Void,
    Namespace,
    /// An `asset` item's name. It is not a value: nothing may be added to it or
    /// stored in it, and the only thing it admits is a member access naming one
    /// of the four blocks it exposes.
    Asset,
    Unknown,
}

#[derive(Clone)]
enum SymbolKind {
    Constant(Option<u32>),
    Variable,
    Group,
    Function(Vec<ValueType>, ValueType),
    Namespace,
    Asset,
}

#[derive(Clone)]
struct Symbol {
    kind: SymbolKind,
    value_type: ValueType,
    span: Span,
}

struct Analyzer {
    scopes: Vec<HashMap<String, Symbol>>,
    errors: Vec<SemanticError>,
    constants: BTreeMap<String, u32>,
    return_type: ValueType,
    /// Each enclosing timed region, innermost last.
    timed_regions: Vec<TimedBlock>,
    /// Whether the statements being checked run from a position the compiler has synchronized —
    /// which is what a `frame` handler does, and nothing else yet.
    synchronized: bool,
    /// The `chrrom bank N` numbers seen so far.
    chr_banks: BTreeSet<u32>,
    /// The assets whose tiles a `chrrom` bank has already placed.
    placed_tiles: BTreeSet<String>,
}

/// One enclosing timed region.
struct TimedBlock {
    /// Whether this is a `cycles(...) { }` block statement rather than a function's own annotation.
    ///
    /// Both put their body under the same restrictions, but only one of them is a block. `lower`
    /// refuses a function carrying a cycle annotation outright, and a refusal whose advice is
    /// "put it after the block" has nothing to name there.
    block: bool,
    /// Whether the region carries `interruptible`, and so emits no `PHP`/`SEI`/`PLP`.
    interruptible: bool,
    /// Whether this is a `using irq` handler, which is costed like a timed block but is an
    /// interrupt: it saves and restores no flag, and ends in `RTI`.
    irq_handler: bool,
}

pub fn analyze(program: &Program) -> Result<TypedProgram, Vec<SemanticError>> {
    let mut analyzer = Analyzer::new();
    analyzer.collect_top_level(program);
    for item in &program.items {
        analyzer.check_item(&item.value);
    }

    if analyzer.errors.is_empty() {
        Ok(TypedProgram {
            program: program.clone(),
            constants: analyzer.constants,
        })
    } else {
        Err(analyzer.errors)
    }
}

impl Analyzer {
    fn new() -> Self {
        let mut root = HashMap::new();
        for name in NAMESPACES {
            root.insert(
                name.into(),
                Symbol {
                    kind: SymbolKind::Namespace,
                    value_type: ValueType::Namespace,
                    span: Span::default(),
                },
            );
        }
        // The virtual register of spec §9.3. It takes any value here: whether the
        // address is constant and in range is lowering's to judge.
        root.insert(
            VRAM_ADDR.into(),
            Symbol {
                kind: SymbolKind::Variable,
                value_type: ValueType::Unknown,
                span: Span::default(),
            },
        );
        Self {
            scopes: vec![root],
            errors: Vec::new(),
            constants: BTreeMap::new(),
            return_type: ValueType::Void,
            timed_regions: Vec::new(),
            synchronized: false,
            chr_banks: BTreeSet::new(),
            placed_tiles: BTreeSet::new(),
        }
    }

    /// Refuse the program. The default kind is `Rejected`: a mistake, or
    /// something Raster does not intend to do.
    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.refuse(span, message, Refusal::Rejected);
    }

    /// Refuse a construct a timed region cannot charge, because the region is
    /// costed as straight-line code. Not every refusal that mentions a timed
    /// region belongs here: a hardware wait has no cost to measure at all, and
    /// where `sync exact` may stand is a placement rule. Both of those are
    /// ordinary rejections.
    fn cannot_be_costed(&mut self, span: Span, message: impl Into<String>) {
        self.refuse(span, message, Refusal::TimedRegionCost);
    }

    /// Refuse an `asset` construct this release does not build. These carry the
    /// note about what the release reads for assets.
    fn asset_not_in_this_release(&mut self, span: Span, message: impl Into<String>) {
        self.refuse(span, message, Refusal::AssetNotInThisRelease);
    }

    fn refuse(&mut self, span: Span, message: impl Into<String>, refusal: Refusal) {
        self.errors.push(SemanticError {
            message: message.into(),
            span,
            refusal,
            label: None,
            notes: Vec::new(),
        });
    }

    /// Refuse with a label and notes of the refusal's own.
    fn refuse_with(
        &mut self,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
        notes: Vec<String>,
    ) {
        self.refuse_labelled(span, message, label, notes, Refusal::Rejected);
    }

    /// Refuse with a label of the refusal's own and no notes.
    fn error_with(&mut self, span: Span, message: impl Into<String>, label: impl Into<String>) {
        self.refuse_labelled(span, message, label, Vec::new(), Refusal::Rejected);
    }

    /// Refuse a construct the specification names and this release does not
    /// build. `rasterc` attaches the "this release compiles ..." note from the
    /// `Refusal`, so nothing here carries it.
    fn not_in_this_release_with(
        &mut self,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
    ) {
        self.refuse_labelled(span, message, label, Vec::new(), Refusal::NotInThisRelease);
    }

    fn refuse_labelled(
        &mut self,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
        notes: Vec<String>,
        refusal: Refusal,
    ) {
        self.errors.push(SemanticError {
            message: message.into(),
            span,
            refusal,
            label: Some(label.into()),
            notes,
        });
    }

    fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn leave_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &Spanned<String>, symbol: Symbol) {
        let previous_span = self
            .scopes
            .last()
            .expect("analyzer always has a root scope")
            .get(&name.value)
            .map(|previous| previous.span);
        if name.value == VRAM_ADDR && self.vram_addr_is_register() && self.scopes.len() == 1 {
            self.error(
                name.span,
                "`vram_addr` is a built-in virtual register and cannot be declared",
            );
        } else if let Some(previous_span) = previous_span {
            self.error(
                name.span,
                format!(
                    "duplicate declaration `{}` (previous declaration at {}..{})",
                    name.value, previous_span.start, previous_span.end
                ),
            );
        } else {
            self.scopes
                .last_mut()
                .expect("analyzer always has a root scope")
                .insert(name.value.clone(), symbol);
        }
    }

    /// Whether `vram_addr` at this point is the virtual register and not
    /// something the author declared under that name in an inner scope.
    fn vram_addr_is_register(&self) -> bool {
        self.lookup(VRAM_ADDR)
            .is_some_and(|symbol| symbol.span == Span::default())
    }

    fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    /// Whether this expression is the name of an `asset` item.
    fn is_asset_name(&self, expression: &Spanned<Expression>) -> bool {
        matches!(
            &expression.value,
            Expression::Name(name)
                if matches!(self.lookup(&name.value), Some(Symbol { kind: SymbolKind::Asset, .. }))
        )
    }

    /// Whether this name is a register namespace, reporting nothing.
    ///
    /// The erroring `expression_type` was used to answer this — once from the
    /// `Member` arm and once again from `ensure_assignable` — and that second
    /// call is the duplicate `unknown name` in the middle of every cascade
    /// raster-3o3 removes.
    fn is_namespace(&self, name: &str) -> bool {
        matches!(
            self.lookup(name),
            Some(symbol) if matches!(symbol.kind, SymbolKind::Namespace)
        )
    }

    /// What a bound name is, in the words a label uses: "`x` is **a variable**,
    /// not a register namespace".
    ///
    /// `None` when the name is not bound at all — that is the "there is no `x`
    /// namespace" case, which says something different.
    fn root_kind(&self, name: &str) -> Option<&'static str> {
        match &self.lookup(name)?.kind {
            SymbolKind::Namespace => None,
            SymbolKind::Variable => Some("a variable"),
            SymbolKind::Constant(_) => Some("a constant"),
            SymbolKind::Function(_, _) => Some("a function"),
            SymbolKind::Group => Some("a group"),
            SymbolKind::Asset => Some("an asset"),
        }
    }

    fn collect_top_level(&mut self, program: &Program) {
        for item in &program.items {
            match &item.value {
                Item::Declaration(declaration) => self.declare_declaration(declaration),
                Item::Function(function) => self.declare_function(function),
                Item::Asset(asset) => self.declare_asset(asset),
                _ => {}
            }
        }
    }

    fn declare_asset(&mut self, asset: &raster_syntax::Asset) {
        self.declare(
            &asset.name,
            Symbol {
                kind: SymbolKind::Asset,
                value_type: ValueType::Asset,
                span: asset.name.span,
            },
        );
    }

    /// What an asset asks for, against what `raster-assets` actually does.
    ///
    /// Every accepted value here is a fact about the encoder rather than a
    /// preference: `BACKGROUND_SUBPALETTE_COUNT` is 4, `encode_background`
    /// deduplicates unconditionally, and the only encoder there is is a
    /// background one. A field this release cannot honour is refused rather
    /// than ignored, because a compiler that silently drops a setting is worse
    /// than one that says it cannot build it.
    fn check_asset(&mut self, asset: &raster_syntax::Asset) {
        if asset.kind.value != "image" {
            self.asset_not_in_this_release(asset.kind.span, "only `asset image` is supported yet");
        }
        if asset.loader.value != "png" {
            self.asset_not_in_this_release(asset.loader.span, "only `png` is supported yet");
        }

        let mut seen: Vec<&str> = Vec::new();
        for field in &asset.fields {
            let name = field.name.value.as_str();
            if seen.contains(&name) {
                self.error(field.name.span, "this asset field is set twice");
                continue;
            }
            seen.push(name);
            match name {
                "kind" => self.require_asset_value(
                    field,
                    &AssetValue::Word("background".into()),
                    "only `kind: background` is supported yet",
                ),
                "palette" => self.require_asset_value(
                    field,
                    &AssetValue::Call {
                        name: "auto".into(),
                        argument: "4".into(),
                    },
                    "only `palette: auto(4)` is supported yet",
                ),
                "dedup" => self.require_asset_value(
                    field,
                    &AssetValue::Word("true".into()),
                    "only `dedup: true` is supported yet",
                ),
                // §8.1 of the specification defines this and this release does
                // not build it. An author copying that block verbatim must not
                // be told they invented it.
                "compress" => self.asset_not_in_this_release(
                    field.name.span,
                    "only `kind`, `palette` and `dedup` are supported yet",
                ),
                _ => self.error(
                    field.name.span,
                    "unknown asset field; this release knows `kind`, `palette` and `dedup`",
                ),
            }
        }
    }

    /// A `chrrom bank N { ... }`: the bank exists once, the MMC3 can address it,
    /// and everything placed in it is some declared asset's tiles, placed once.
    ///
    /// How many tiles that is, and whether they fit, needs the decoded pictures
    /// and is `rasterc`'s to judge once they exist.
    fn check_chrrom(&mut self, bank: &raster_syntax::ChrRomBank) {
        match parse_number(&bank.number.value) {
            None => self.error(bank.number.span, "invalid numeric literal"),
            Some(number) if number > MAX_CHR_BANK => self.error(
                bank.number.span,
                format!("an MMC3 cartridge holds banks 0 to {MAX_CHR_BANK}"),
            ),
            Some(number) => {
                if !self.chr_banks.insert(number) {
                    self.error(
                        bank.number.span,
                        format!("`chrrom bank {number}` is declared twice"),
                    );
                }
            }
        }
        for entry in &bank.entries {
            let Expression::Member { base, member } = &entry.value else {
                self.not_tiles(entry.span);
                continue;
            };
            let Expression::Name(name) = &base.value else {
                self.not_tiles(entry.span);
                continue;
            };
            match self.lookup(&name.value).map(|symbol| symbol.kind.clone()) {
                None => self.error(name.span, format!("unknown name `{}`", name.value)),
                Some(SymbolKind::Asset) if member.value == "tiles" => {
                    if !self.placed_tiles.insert(name.value.clone()) {
                        self.error(
                            entry.span,
                            format!(
                                "`{}.tiles` is already placed in a `chrrom` bank",
                                name.value
                            ),
                        );
                    }
                }
                Some(_) => self.not_tiles(entry.span),
            }
        }
    }

    fn not_tiles(&mut self, span: Span) {
        self.error(span, "a `chrrom` bank holds tiles, such as `picture.tiles`");
    }

    /// The caret is on the value, which is the part the author must change.
    fn require_asset_value(
        &mut self,
        field: &raster_syntax::AssetField,
        wanted: &AssetValue,
        message: &str,
    ) {
        if &field.value.value != wanted {
            self.asset_not_in_this_release(field.value.span, message);
        }
    }

    fn declare_declaration(&mut self, declaration: &Declaration) {
        let Some(name) = &declaration.name else {
            return;
        };
        let value_type = declaration
            .type_annotation
            .as_ref()
            .map(|annotation| self.resolve_type_without_errors(annotation))
            .unwrap_or(ValueType::Unknown);
        let kind = match declaration.kind {
            Keyword::Const => SymbolKind::Constant(None),
            Keyword::Group => SymbolKind::Group,
            _ => SymbolKind::Variable,
        };
        self.declare(
            name,
            Symbol {
                kind,
                value_type,
                span: name.span,
            },
        );
    }

    fn declare_function(&mut self, function: &Function) {
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| self.resolve_type(&parameter.type_annotation))
            .collect();
        let return_type = function
            .return_type
            .as_ref()
            .map(|value| self.resolve_type(value))
            .unwrap_or(ValueType::Void);
        self.declare(
            &function.name,
            Symbol {
                kind: SymbolKind::Function(parameters, return_type.clone()),
                value_type: return_type,
                span: function.name.span,
            },
        );
    }

    fn check_item(&mut self, item: &Item) {
        match item {
            Item::Declaration(declaration) => self.check_declaration(declaration),
            Item::Function(function) => self.check_function(function),
            Item::Frame(frame) => {
                self.enter_scope();
                let irq = frame
                    .strategy
                    .as_ref()
                    .is_some_and(|strategy| strategy.value == "irq");
                for event in &frame.events {
                    self.check_frame_event(&event.value, irq);
                }
                self.leave_scope();
            }
            Item::Main(block) => self.check_block(block),
            Item::Asset(asset) => self.check_asset(asset),
            Item::ChrRom(bank) => self.check_chrrom(bank),
            _ => {}
        }
    }

    fn check_function(&mut self, function: &Function) {
        if let Some(spec) = &function.cycle_spec {
            self.check_cycle_bound(&spec.bound);
        }
        for group in &function.employs {
            match self.lookup(&group.value).map(|symbol| symbol.kind.clone()) {
                Some(SymbolKind::Group) => {}
                Some(_) => self.error(group.span, format!("`{}` is not a group", group.value)),
                None => self.error(group.span, format!("unknown name `{}`", group.value)),
            }
        }
        self.enter_scope();
        for parameter in &function.parameters {
            let value_type = self.resolve_type_without_errors(&parameter.type_annotation);
            self.declare(
                &parameter.name,
                Symbol {
                    kind: SymbolKind::Variable,
                    value_type,
                    span: parameter.name.span,
                },
            );
        }
        let function_return_type = self.lookup_function_return(function);
        let previous_return_type = std::mem::replace(&mut self.return_type, function_return_type);
        if let Some(spec) = &function.cycle_spec {
            self.timed_regions.push(TimedBlock {
                block: false,
                interruptible: spec.interruptible,
                irq_handler: false,
            });
        }
        self.check_block_statements(&function.body);
        if function.cycle_spec.is_some() {
            self.timed_regions.pop();
        }
        self.return_type = previous_return_type;
        self.leave_scope();
    }

    fn lookup_function_return(&self, function: &Function) -> ValueType {
        function
            .return_type
            .as_ref()
            .map(|value| self.resolve_type_without_errors(value))
            .unwrap_or(ValueType::Void)
    }

    /// Check one scheduled handler.
    ///
    /// A handler is a timed region, and is checked as one. Lowering pads it to the scanline it is
    /// scheduled on, so its cost has to be provable for exactly the reason a `cycles` block's does
    /// — a `wait` spending its cycles in a loop the region is not costed for, or a branch whose
    /// arms are not balanced, would put every handler after it in the wrong place and the effect on
    /// the screen somewhere the source never asked for. The restrictions of spec section 6.3 are
    /// what make a handler's window in section 7.2 mean anything.
    ///
    /// It is a *synchronized* one, which is what section 6.6 asks for: a `frame` says where its
    /// handlers run and lowering emits the synchronization that makes it true, so demanding a
    /// `sync exact` inside every handler would be asking the author to do by hand the one thing
    /// the construct exists to do for them.
    fn check_frame_event(&mut self, event: &FrameEvent, irq_handler: bool) {
        let outer = std::mem::replace(&mut self.synchronized, true);
        // A `using timed` handler is emitted through the same `timed_region` a `cycles(...) { }`
        // block is, with `pad` set and `interruptible` clear, so it carries the same restrictions
        // and the same `PHP`/`SEI`/`PLP` — see `Generator::timed_frame`. A `using irq` handler is
        // an interrupt rather than a region and masks nothing itself, but it is held to the same
        // restrictions here: a `return` out of one would jump past its `RTI` and keep the three
        // bytes the interrupt pushed.
        self.timed_regions.push(TimedBlock {
            block: true,
            interruptible: false,
            irq_handler,
        });
        self.check_frame_event_body(event);
        self.timed_regions.pop();
        self.synchronized = outer;
    }

    fn check_frame_event_body(&mut self, event: &FrameEvent) {
        match event {
            FrameEvent::At { position, body } => {
                if let FramePosition::Scanline(value) = position {
                    self.require_static(value, "frame scanline");
                }
                self.check_block(body);
            }
            FrameEvent::Every {
                interval,
                from,
                to,
                body,
            } => {
                let interval_value = self.require_static(interval, "frame interval");
                let from_value = self.require_static(from, "frame range start");
                let to_value = self.require_static(to, "frame range end");
                if interval_value == Some(0) {
                    self.error(interval.span, "frame interval must be greater than zero");
                }
                if matches!((from_value, to_value), (Some(from), Some(to)) if from >= to) {
                    self.error(to.span, "frame range start must be less than its end");
                }
                self.check_block(body);
            }
        }
    }

    fn check_block(&mut self, block: &Block) {
        self.enter_scope();
        self.check_block_statements(block);
        self.leave_scope();
    }

    fn check_block_statements(&mut self, block: &Block) {
        for (index, statement) in block.statements.iter().enumerate() {
            if let Statement::Cycles { spec, body, .. } = &statement.value
                && !self.synchronized
                && writes_ppu_register(body, self.vram_addr_is_register())
                && !block.statements[..index].iter().any(is_sync_exact)
            {
                self.error(
                    spec.span,
                    "`sync exact` is required before a timed block that writes a PPU register, \
                     because NMI entry is not cycle-exact",
                );
            }
            self.check_statement(&statement.value, statement.span);
        }
    }

    fn in_timed_region(&self) -> bool {
        !self.timed_regions.is_empty()
    }

    /// The innermost enclosing `cycles(...) { }` block statement, if any.
    ///
    /// A function's own cycle annotation is not one, and is always outermost — so where the
    /// innermost region is that annotation, there is no block between it and the statement.
    fn innermost_cycles_block(&self) -> Option<&TimedBlock> {
        self.timed_regions.last().filter(|region| region.block)
    }

    /// Refuse something whose cost a straight-line timed block cannot charge.
    fn reject_in_timed_block(&mut self, span: Span, message: &str) {
        if self.in_timed_region() {
            self.cannot_be_costed(span, message.to_owned());
        }
    }

    fn check_statement(&mut self, statement: &Statement, statement_span: Span) {
        match statement {
            Statement::Declaration(declaration) => {
                self.declare_declaration(declaration);
                self.check_declaration(declaration);
            }
            Statement::Block(block) => self.check_block(block),
            Statement::Loop(block) => {
                if self.in_timed_region() {
                    self.cannot_be_costed(
                        statement_span,
                        "an unbounded loop has no provable cycle cost inside a timed block",
                    );
                }
                self.check_block(block);
            }
            Statement::If {
                condition,
                then_body,
                else_body,
            } => {
                if self.in_timed_region() {
                    self.cannot_be_costed(
                        statement_span,
                        "branch arms inside a timed block cannot yet be balanced, because each \
                         path through them costs a different number of cycles",
                    );
                }
                self.require_type(condition, ValueType::Bool, "condition must have type bool");
                self.check_block(then_body);
                if let Some(body) = else_body {
                    self.check_block(body);
                }
            }
            Statement::While { condition, body } => {
                if self.in_timed_region() {
                    self.cannot_be_costed(
                        statement_span,
                        "a `while` loop's trip count cannot be proven inside a timed block",
                    );
                }
                self.require_type(condition, ValueType::Bool, "condition must have type bool");
                self.check_block(body);
            }
            Statement::For {
                binding,
                range,
                step,
                body,
            } => {
                if self.in_timed_region() {
                    self.cannot_be_costed(
                        statement_span,
                        "a `for` loop inside a timed block compiles to a loop whose cost is not \
                         yet proven, because the block is costed as straight-line code",
                    );
                }
                self.require_range(range, "for range");
                if let Some(step) = step
                    && self.require_static(step, "for step") == Some(0)
                {
                    self.error(step.span, "for step must be greater than zero");
                }
                self.enter_scope();
                self.declare(
                    binding,
                    Symbol {
                        kind: SymbolKind::Variable,
                        value_type: ValueType::U16,
                        span: binding.span,
                    },
                );
                self.check_block_statements(body);
                self.leave_scope();
            }
            Statement::Cycles { spec, body, label } => {
                if matches!(spec.bound, CycleBound::Inferred(_)) && label.is_none() {
                    self.error(
                        spec.span,
                        "`cycles(?)` needs a label to report its measured cost under",
                    );
                }
                self.check_cycle_bound(&spec.bound);
                self.timed_regions.push(TimedBlock {
                    block: true,
                    interruptible: spec.interruptible,
                    irq_handler: false,
                });
                self.check_block(body);
                self.timed_regions.pop();
            }
            Statement::Wait(Wait::Scanline(value)) => {
                self.require_static(value, "wait bound");
                if self.in_timed_region() {
                    self.error(
                        statement_span,
                        "`wait scanline` has no provable cost inside a timed block",
                    );
                }
            }
            Statement::Wait(Wait::Cycles(value)) => {
                self.require_static(value, "wait bound");
                if self.in_timed_region() {
                    self.cannot_be_costed(
                        statement_span,
                        "`wait cycles` inside a timed block spends its cycles in a loop, which \
                         the block's straight-line cost model cannot yet charge; widen the \
                         budget and let `pad` fill it instead",
                    );
                }
            }
            Statement::Return(value) => {
                if self
                    .innermost_cycles_block()
                    .is_some_and(|block| block.irq_handler)
                {
                    self.refuse_with(
                        statement_span,
                        "an `irq` handler cannot `return`",
                        "`return` jumps past the handler's `RTI`",
                        vec![
                            "a handler ends by acknowledging the interrupt, arming the next\n\
                             event and running `RTI`; a `return` skips all three, so no later\n\
                             event in the schedule runs"
                                .to_owned(),
                            "let the handler run to its last statement instead".to_owned(),
                        ],
                    );
                    // The value makes no difference to the refusal, so it is not also judged
                    // against a return type the handler does not have; it is still read, so a
                    // misspelt name in it is still reported.
                    if let Some(value) = value {
                        self.expression_type(value);
                    }
                    return;
                } else if let Some(block) = self.innermost_cycles_block() {
                    // The two sentences differ by one clause because an `interruptible` block emits no
                    // `PHP`/`SEI`/`PLP`, so there is no interrupt flag left masked when the `return`
                    // jumps out. `reject_in_timed_block` takes one fixed message and cannot say both.
                    let message = if block.interruptible {
                        "`return` inside a timed block jumps out before the block has spent its \
                         budget, so it belongs after the block rather than inside one"
                    } else {
                        "`return` inside a timed block jumps out before the block has spent its \
                         budget and before the interrupt flag is restored, so it belongs after the \
                         block rather than inside one"
                    };
                    self.error(statement_span, message.to_owned());
                }
                match value {
                    Some(value) => self.require_type(
                        value,
                        self.return_type.clone(),
                        "return expression does not match function return type",
                    ),
                    None if self.return_type != ValueType::Void => {
                        self.error(
                            statement_span,
                            "return expression is required for this function",
                        );
                    }
                    None => {}
                }
            }
            Statement::Expression(expression) => {
                self.expression_type(expression);
            }
            Statement::Wait(Wait::Vblank(_)) => {
                if self.in_timed_region() {
                    self.error(
                        statement_span,
                        "`wait vblank` has no provable cost inside a timed block",
                    );
                }
            }
            Statement::Sync(strategy) => {
                if self.in_timed_region() {
                    self.error(
                        statement_span,
                        "`sync exact` waits an unpredictable number of cycles, so it belongs \
                         before a timed block rather than inside one",
                    );
                }
                if strategy.value != "exact" {
                    self.error(
                        strategy.span,
                        format!("unknown sync strategy `{}`", strategy.value),
                    );
                }
            }
            Statement::Break | Statement::Continue => {}
        }
    }

    fn check_declaration(&mut self, declaration: &Declaration) {
        if declaration.kind == Keyword::Group {
            if let Some(body) = &declaration.body {
                self.check_block(body);
            }
            return;
        }
        let declared_type = declaration
            .type_annotation
            .as_ref()
            .map(|annotation| self.resolve_type(annotation));
        if let Some(initializer) = &declaration.initializer {
            let value_type = self.expression_type(initializer);
            if let Some(declared_type) = &declared_type {
                self.ensure_expression_compatible(
                    initializer,
                    declared_type,
                    &value_type,
                    "initializer type does not match declaration type",
                );
            }
        }
        if declaration.kind == Keyword::Const {
            let Some(name) = &declaration.name else {
                return;
            };
            let Some(initializer) = &declaration.initializer else {
                self.error(name.span, "constant declarations require an initializer");
                return;
            };
            if let Some(value) = self.evaluate_constant(initializer) {
                if let Some(value_type) = declared_type {
                    self.check_value_fits(value, &value_type, initializer.span);
                }
                self.constants.insert(name.value.clone(), value);
                if let Some(symbol) = self
                    .scopes
                    .last_mut()
                    .and_then(|scope| scope.get_mut(&name.value))
                {
                    symbol.kind = SymbolKind::Constant(Some(value));
                }
            } else {
                self.error(
                    initializer.span,
                    "constant initializer must be a compile-time constant",
                );
            }
        }
    }

    fn resolve_type(&mut self, annotation: &Spanned<Type>) -> ValueType {
        match &annotation.value {
            Type::Name(name) => match name.value.as_str() {
                "u8" => ValueType::U8,
                "u16" => ValueType::U16,
                "bool" => ValueType::Bool,
                "void" => ValueType::Void,
                _ => {
                    self.error(name.span, format!("unknown type `{}`", name.value));
                    ValueType::Unknown
                }
            },
            Type::Array { length, element } => {
                let length_value = self.require_static(length, "array length");
                if length_value == Some(0) {
                    self.error(length.span, "array length must be greater than zero");
                }
                ValueType::Array(
                    Box::new(self.resolve_type(element)),
                    length_value.unwrap_or(0),
                )
            }
        }
    }

    fn resolve_type_without_errors(&self, annotation: &Spanned<Type>) -> ValueType {
        match &annotation.value {
            Type::Name(name) => match name.value.as_str() {
                "u8" => ValueType::U8,
                "u16" => ValueType::U16,
                "bool" => ValueType::Bool,
                "void" => ValueType::Void,
                _ => ValueType::Unknown,
            },
            Type::Array { length, element } => ValueType::Array(
                Box::new(self.resolve_type_without_errors(element)),
                self.evaluate_constant(length).unwrap_or(0),
            ),
        }
    }

    fn expression_type(&mut self, expression: &Spanned<Expression>) -> ValueType {
        match &expression.value {
            Expression::Name(name) => match self.lookup(&name.value) {
                Some(symbol) => symbol.value_type.clone(),
                None => {
                    self.error(name.span, format!("unknown name `{}`", name.value));
                    ValueType::Unknown
                }
            },
            // Integer literals acquire their concrete type from the surrounding
            // declaration, parameter, or operation.
            Expression::Number(value) => {
                if parse_number(value).is_none() {
                    self.error(expression.span, "invalid numeric literal");
                }
                ValueType::Unknown
            }
            Expression::Character(_) => ValueType::Unknown,
            Expression::Boolean(_) => ValueType::Bool,
            Expression::String(_) => ValueType::Unknown,
            Expression::Prefix { operator, operand } => {
                let operand_type = self.expression_type(operand);
                match operator.value {
                    Operator::Bang => {
                        self.ensure_compatible(
                            operator.span,
                            &ValueType::Bool,
                            &operand_type,
                            "`!` requires a bool operand",
                        );
                        ValueType::Bool
                    }
                    Operator::Tilde => {
                        self.require_integer(
                            operator.span,
                            &operand_type,
                            "`~` requires an integer operand",
                        );
                        operand_type
                    }
                    _ => ValueType::Unknown,
                }
            }
            Expression::Infix {
                left,
                operator,
                right,
            } => self.infix_type(left, operator, right),
            Expression::Call { callee, arguments } => self.call_type(callee, arguments),
            Expression::Index { base, index } => {
                let base_type = self.expression_type(base);
                let index_type = self.expression_type(index);
                self.require_integer(index.span, &index_type, "array index must be an integer");
                match base_type {
                    ValueType::Array(element, _) => *element,
                    ValueType::Unknown => ValueType::Unknown,
                    _ => {
                        self.error(base.span, "indexing requires an array");
                        ValueType::Unknown
                    }
                }
            }
            Expression::Member { base, member } => {
                if !self.is_asset_name(base) {
                    return self.member_type(expression, base, member);
                }
                if spec_only_asset_member(&member.value) {
                    // Defined by §8.1 and not built yet, which is a
                    // different thing from a member nobody has heard of.
                    self.asset_not_in_this_release(
                        member.span,
                        "only `tiles`, `nametable`, `attributes` and `palette` \
                         are supported yet",
                    );
                } else if !asset_member(&member.value) {
                    self.error(
                        member.span,
                        format!(
                            "unknown asset member `{}`; an image asset has `tiles`, \
                             `nametable`, `attributes` and `palette`",
                            member.value
                        ),
                    );
                }
                // A block of ROM data rather than a value. `Unknown` is the
                // language's "do not build on this" type: it suppresses the
                // cascade a wrong member would otherwise cause. It does not
                // by itself forbid arithmetic — `require_integer` and
                // `ensure_compatible` both accept `Unknown` deliberately —
                // so `picture.tiles + 1` analyzes clean today and is caught
                // only by lowering's blanket refusal of the item — which
                // prints "this byte register is not supported" under the
                // member, beside the correct refusal. Giving these members a
                // type that holds once that refusal is lifted, and so
                // retires that second message, belongs to the bead that
                // gives them a consumer.
                ValueType::Unknown
            }
            Expression::Range { start, end } => {
                let start_type = self.expression_type(start);
                let end_type = self.expression_type(end);
                self.ensure_compatible(
                    expression.span,
                    &start_type,
                    &end_type,
                    "range bounds must have compatible types",
                );
                ValueType::Unknown
            }
        }
    }

    /// `ppu.addr = $3F00`: a number that does not fit the byte port. The generic
    /// "integer literal overflows u8" would be true and no help; this points at
    /// the one-line form. Returns whether it refused.
    fn refuse_wide_literal_on_ppu_addr(
        &mut self,
        left: &Spanned<Expression>,
        right: &Spanned<Expression>,
    ) -> bool {
        let Expression::Member { base, member } = &left.value else {
            return false;
        };
        let Expression::Number(written) = &right.value else {
            return false;
        };
        let is_ppu_addr = matches!(&base.value, Expression::Name(name) if name.value == "ppu")
            && member.value == "addr";
        if !is_ppu_addr || !parse_number(written).is_some_and(|value| value > u8::MAX as u32) {
            return false;
        }
        self.refuse_with(
            right.span,
            "ppu.addr is written one byte at a time",
            format!("{written} does not fit in a byte"),
            vec![format!(
                "{}to set the whole address in one line, write `vram_addr = {written}`",
                raster_diag::HELP_PREFIX
            )],
        );
        true
    }

    fn infix_type(
        &mut self,
        left: &Spanned<Expression>,
        operator: &Spanned<Operator>,
        right: &Spanned<Expression>,
    ) -> ValueType {
        if matches!(
            operator.value,
            Operator::Assign
                | Operator::PlusEqual
                | Operator::MinusEqual
                | Operator::StarEqual
                | Operator::SlashEqual
        ) {
            let left_type = self.expression_type(left);
            let right_type = self.expression_type(right);
            self.ensure_assignable(left);
            if operator.value == Operator::Assign
                && self.refuse_wide_literal_on_ppu_addr(left, right)
            {
                return left_type;
            }
            self.ensure_expression_compatible(
                right,
                &left_type,
                &right_type,
                "assignment operands must have compatible types",
            );
            return left_type;
        }
        let left_type = self.expression_type(left);
        let right_type = self.expression_type(right);
        match operator.value {
            // Each of these lowers to a `DEX`/`BNE` loop over one of its operands, so its cost
            // is a loop's cost however constant the operands are. A straight-line region would
            // charge a single pass: `v * 3` was predicted at 38 cycles and spends 69.
            Operator::Star | Operator::Slash | Operator::Percent => self.reject_in_timed_block(
                operator.span,
                "multiplication, division and remainder inside a timed block compile to loops \
                 whose cost is not yet proven",
            ),
            Operator::ShiftLeft | Operator::ShiftRight => self.reject_in_timed_block(
                operator.span,
                "a shift inside a timed block compiles to a loop whose cost is not yet proven",
            ),
            _ => {}
        }
        match operator.value {
            Operator::AmpersandAmpersand | Operator::PipePipe => {
                self.ensure_compatible(
                    operator.span,
                    &ValueType::Bool,
                    &left_type,
                    "logical operators require bool operands",
                );
                self.ensure_compatible(
                    operator.span,
                    &ValueType::Bool,
                    &right_type,
                    "logical operators require bool operands",
                );
                ValueType::Bool
            }
            Operator::EqualEqual
            | Operator::BangEqual
            | Operator::Less
            | Operator::LessEqual
            | Operator::Greater
            | Operator::GreaterEqual => {
                self.ensure_compatible(
                    operator.span,
                    &left_type,
                    &right_type,
                    "comparison operands must have compatible types",
                );
                ValueType::Bool
            }
            _ => {
                self.require_integer(
                    operator.span,
                    &left_type,
                    "arithmetic and bitwise operators require integer operands",
                );
                self.require_integer(
                    operator.span,
                    &right_type,
                    "arithmetic and bitwise operators require integer operands",
                );
                self.ensure_compatible(
                    operator.span,
                    &left_type,
                    &right_type,
                    "arithmetic operands must have compatible types",
                );
                left_type
            }
        }
    }

    fn call_type(
        &mut self,
        callee: &Spanned<Expression>,
        arguments: &[Spanned<Expression>],
    ) -> ValueType {
        let Expression::Name(name) = &callee.value else {
            self.error(callee.span, "call target must be a function name");
            return ValueType::Unknown;
        };
        let Some(symbol) = self.lookup(&name.value).cloned() else {
            if let Some(parameters) = load_builtin_arity(&name.value) {
                return self.load_call_type(name, parameters, arguments);
            }
            self.error(name.span, format!("unknown name `{}`", name.value));
            return ValueType::Unknown;
        };
        let SymbolKind::Function(parameters, return_type) = symbol.kind else {
            self.error(callee.span, format!("`{}` is not a function", name.value));
            return ValueType::Unknown;
        };
        // `lower` refuses a function that carries a cycle annotation, so demanding one here would
        // ask for the very thing the compiler then rejects. A call's cost is the callee's, and
        // nothing measures a callee yet.
        if self.in_timed_region() {
            self.cannot_be_costed(
                name.span,
                format!(
                    "`{}` cannot be called inside a timed block yet: a call's cost is the \
                     callee's, and no function's cost is measured yet",
                    name.value
                ),
            );
        }
        if parameters.len() != arguments.len() {
            self.error(
                callee.span,
                format!(
                    "function `{}` expects {} arguments but received {}",
                    name.value,
                    parameters.len(),
                    arguments.len()
                ),
            );
        }
        for (argument, parameter) in arguments.iter().zip(parameters.iter()) {
            let argument_type = self.expression_type(argument);
            self.ensure_expression_compatible(
                argument,
                parameter,
                &argument_type,
                "call argument type does not match parameter type",
            );
        }
        return_type
    }

    /// The type of a member expression, and the single error it raises when
    /// there is not one.
    ///
    /// This never calls `expression_type` on the base. That call is what
    /// produced the cascade it replaces: `oam.addr = 0` reported ``unknown name
    /// `oam` `` from the base, `member access requires a register namespace`
    /// from here, and then both again from `ensure_assignable` — four errors
    /// for one line, two of them byte for byte identical.
    fn member_type(
        &mut self,
        whole: &Spanned<Expression>,
        base: &Spanned<Expression>,
        member: &Spanned<String>,
    ) -> ValueType {
        let Some(root) = member_root(whole) else {
            // `f().x`, `a[0].y`: the chain does not root at a name at all.
            //
            // The base is still analysed for its own errors — `t[nope].y` must
            // say that `nope` is unknown as well as that the member access is
            // wrong. Collapsing the cascade was about the *duplicate* report,
            // which came from `ensure_assignable` evaluating the base a second
            // time and no longer happens. Review finding 2 on PR #47.
            self.expression_type(base);
            self.error(base.span, "member access requires a register namespace");
            return ValueType::Unknown;
        };
        if !self.is_namespace(&root.value) {
            return self.unknown_namespace(whole, root);
        }
        match &base.value {
            // One level on a real namespace: the check this release has always
            // made, with the message it has always used.
            Expression::Name(_) if root.value == "apu" => self.apu_direct_type(whole, member),
            Expression::Name(_) => {
                if register_member(&root.value, &member.value) {
                    ValueType::U8
                } else {
                    self.error(
                        member.span,
                        format!("unknown {} register `{}`", root.value, member.value),
                    );
                    ValueType::Unknown
                }
            }
            // `ppu.oam.addr`: report the deepest wrong step and nothing here.
            Expression::Member {
                base: inner,
                member: inner_member,
            } if root.value == "apu" && matches!(inner.value, Expression::Name(_)) => {
                self.apu_channel_type(whole, inner_member, member)
            }
            Expression::Member {
                base: inner,
                member: inner_member,
            } => {
                if self.member_type(base, inner, inner_member) != ValueType::Unknown {
                    // `ppu.ctrl.x`: the inner level is a real register, and a
                    // register has no members. Same message and same span as
                    // before raster-3o3.
                    self.error(base.span, "member access requires a register namespace");
                }
                ValueType::Unknown
            }
            // `member_root` found a name, so nothing else can be here. No error:
            // the arm above has already said whatever there is to say.
            _ => ValueType::Unknown,
        }
    }

    /// The one error a member chain rooted at something that is not a namespace
    /// raises. Exactly one, whatever the chain's depth.
    ///
    /// The carets sit under `whole` for every case but one, because the label is
    /// a substitution the author can perform — "$2003 is spelled
    /// `ppu.oam_addr`" does not read as one if the carets cover `oam` alone.
    /// The exception is a root bound to something ordinary, which is also the
    /// case checked first: the label is about that name, the carets stay on it
    /// as they do today, and what a name the author declared *is* outranks what
    /// the specification once called it.
    fn unknown_namespace(
        &mut self,
        whole: &Spanned<Expression>,
        root: &Spanned<String>,
    ) -> ValueType {
        // The root is bound, to something that is not a namespace. First,
        // because `oam` and `apu` are matched by name below and an author may
        // have declared either — "there is no `oam` namespace" is false when
        // they have just written `var oam`. The carets stay on that name, as
        // they do today, because the label is about it. Review finding 4 on
        // PR #47.
        if let Some(kind) = self.root_kind(&root.value) {
            self.error_with(
                root.span,
                "member access requires a register namespace",
                format!("`{}` is {kind}, not a register namespace", root.value),
            );
            return ValueType::Unknown;
        }

        let member = root_member(whole).map(|member| member.value.as_str());

        // The specification's own name for a port this release has.
        if let Some((_, _, port, spelling)) = RENAMED
            .iter()
            .find(|(namespace, name, _, _)| *namespace == root.value && Some(*name) == member)
        {
            self.error_with(
                whole.span,
                format!("there is no `{}` namespace", root.value),
                format!("{port} is spelled `{spelling}`"),
            );
            return ValueType::Unknown;
        }

        // Nothing of that name at all.
        self.error_with(
            whole.span,
            format!("there is no `{}` namespace", root.value),
            NAMESPACE_LIST,
        );
        ValueType::Unknown
    }

    /// `load_palette` and `load_nametable`, which upload an asset's blocks and
    /// are the language's only builtin calls. Whether each argument is the
    /// right *block* is `raster-ir`'s question, where a diagnostic can carry a
    /// label and a help line; here they are only checked to be names that
    /// exist and to be as many as the call takes.
    fn load_call_type(
        &mut self,
        name: &Spanned<String>,
        parameters: usize,
        arguments: &[Spanned<Expression>],
    ) -> ValueType {
        if self.in_timed_region() {
            self.cannot_be_costed(
                name.span,
                format!(
                    "`{}` cannot be called inside a timed block: it copies its data in a \
                     loop, and a loop cannot be costed",
                    name.value
                ),
            );
        }
        if parameters != arguments.len() {
            self.error(
                name.span,
                format!(
                    "function `{}` expects {parameters} arguments but received {}",
                    name.value,
                    arguments.len()
                ),
            );
        }
        for argument in arguments {
            self.expression_type(argument);
        }
        ValueType::Void
    }

    fn ensure_assignable(&mut self, expression: &Spanned<Expression>) {
        match &expression.value {
            Expression::Name(name) => {
                match self.lookup(&name.value).map(|symbol| symbol.kind.clone()) {
                    Some(SymbolKind::Variable) => {}
                    Some(SymbolKind::Constant(_)) => {
                        self.error(name.span, "constants are not assignable")
                    }
                    Some(SymbolKind::Function(_, _)) => {
                        self.error(name.span, "function names are not assignable")
                    }
                    _ => self.error(name.span, "assignment target is not mutable"),
                }
            }
            Expression::Index { .. } => {}
            // A member expression has already been judged: `expression_type` is
            // called on an assignment's left side before this, and it either
            // returned a register or raised its one error. Re-evaluating the base
            // here was the duplicate `unknown name` in the middle of every cascade.
            // An asset's member is a block of ROM data, not somewhere to store.
            Expression::Member { base, .. } if self.is_asset_name(base) => self.error(
                expression.span,
                "assignment target must be a mutable variable, array element, or register member",
            ),
            Expression::Member { .. } => {}
            _ => self.error(
                expression.span,
                "assignment target must be a mutable variable, array element, or register member",
            ),
        }
    }

    fn require_type(
        &mut self,
        expression: &Spanned<Expression>,
        expected: ValueType,
        message: &str,
    ) {
        let actual = self.expression_type(expression);
        self.ensure_expression_compatible(expression, &expected, &actual, message);
    }

    fn require_integer(&mut self, span: Span, value_type: &ValueType, message: &str) {
        if !matches!(
            value_type,
            ValueType::U8 | ValueType::U16 | ValueType::Unknown
        ) {
            self.error(span, message);
        }
    }

    fn ensure_compatible(
        &mut self,
        span: Span,
        expected: &ValueType,
        actual: &ValueType,
        message: &str,
    ) {
        if *expected != ValueType::Unknown && *actual != ValueType::Unknown && expected != actual {
            self.error(span, message);
        }
    }

    fn ensure_expression_compatible(
        &mut self,
        expression: &Spanned<Expression>,
        expected: &ValueType,
        actual: &ValueType,
        message: &str,
    ) {
        if let Expression::Number(value) = &expression.value {
            match (parse_number(value), expected) {
                (Some(value), ValueType::U8) if value > u8::MAX as u32 => {
                    self.error(expression.span, "integer literal overflows u8");
                    return;
                }
                (Some(value), ValueType::U16) if value > u16::MAX as u32 => {
                    self.error(expression.span, "integer literal overflows u16");
                    return;
                }
                (_, ValueType::Bool | ValueType::Void | ValueType::Array(_, _)) => {
                    self.error(expression.span, message);
                    return;
                }
                _ => {}
            }
        }
        self.ensure_compatible(expression.span, expected, actual, message);
    }

    fn check_cycle_bound(&mut self, bound: &CycleBound) {
        match bound {
            CycleBound::Exact(value) | CycleBound::AtMost(value) => {
                if self.require_static(value, "cycle bound") == Some(0) {
                    self.error(value.span, "cycle bound must be greater than zero");
                }
            }
            CycleBound::Inferred(_) => {}
        }
    }

    fn require_range(&mut self, expression: &Spanned<Expression>, context: &str) {
        if let Expression::Range { start, end } = &expression.value {
            let start_value = self.require_static(start, context);
            let end_value = self.require_static(end, context);
            if matches!((start_value, end_value), (Some(start), Some(end)) if start >= end) {
                self.error(
                    end.span,
                    format!("{context} start must be less than its end"),
                );
            }
        } else {
            self.error(
                expression.span,
                format!("{context} must be a compile-time range"),
            );
        }
    }

    fn require_static(&mut self, expression: &Spanned<Expression>, context: &str) -> Option<u32> {
        let value = self.evaluate_constant(expression);
        if value.is_none() {
            self.error(
                expression.span,
                format!("{context} must be a compile-time constant"),
            );
        }
        value
    }

    fn evaluate_constant(&self, expression: &Spanned<Expression>) -> Option<u32> {
        match &expression.value {
            Expression::Number(value) => parse_number(value),
            Expression::Boolean(value) => Some(u32::from(*value)),
            Expression::Name(name) => match self.lookup(&name.value)?.kind {
                SymbolKind::Constant(value) => value,
                _ => None,
            },
            Expression::Prefix { operator, operand } => {
                let value = self.evaluate_constant(operand)?;
                match operator.value {
                    Operator::Tilde => Some(!value),
                    Operator::Bang => Some(u32::from(value == 0)),
                    _ => None,
                }
            }
            Expression::Infix {
                left,
                operator,
                right,
            } => {
                let left = self.evaluate_constant(left)?;
                let right = self.evaluate_constant(right)?;
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
                    Operator::EqualEqual => Some(u32::from(left == right)),
                    Operator::BangEqual => Some(u32::from(left != right)),
                    Operator::Less => Some(u32::from(left < right)),
                    Operator::LessEqual => Some(u32::from(left <= right)),
                    Operator::Greater => Some(u32::from(left > right)),
                    Operator::GreaterEqual => Some(u32::from(left >= right)),
                    Operator::AmpersandAmpersand => Some(u32::from(left != 0 && right != 0)),
                    Operator::PipePipe => Some(u32::from(left != 0 || right != 0)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn check_value_fits(&mut self, value: u32, value_type: &ValueType, span: Span) {
        let maximum = match value_type {
            ValueType::U8 => Some(u8::MAX as u32),
            ValueType::U16 => Some(u16::MAX as u32),
            _ => None,
        };
        if maximum.is_some_and(|maximum| value > maximum) {
            self.error(span, "constant value overflows its declared type");
        }
    }
}

/// The highest `chrrom bank` number: an MMC3 addresses 256 KiB of CHR, which is
/// 32 banks of 8 KiB.
const MAX_CHR_BANK: u32 = 31;

/// How many arguments a builtin `load_*` call takes, or `None` for any other name.
fn load_builtin_arity(name: &str) -> Option<usize> {
    match name {
        "load_palette" => Some(1),
        "load_nametable" => Some(2),
        _ => None,
    }
}

fn parse_number(value: &str) -> Option<u32> {
    if let Some(hexadecimal) = value.strip_prefix('$') {
        u32::from_str_radix(hexadecimal, 16).ok()
    } else {
        value.parse().ok()
    }
}

/// The members §8.1 of the specification defines that this release does not
/// build. `raster_assets` counts its tiles, so `tile_count` is a value rather
/// than a block, and giving it a type is the job of the bead that gives these
/// members a consumer.
fn spec_only_asset_member(member: &str) -> bool {
    member == "tile_count"
}

/// The four blocks an image asset exposes, which are exactly the four accessors
/// of `raster_assets::NesBackground`.
fn asset_member(member: &str) -> bool {
    matches!(member, "tiles" | "nametable" | "attributes" | "palette")
}

/// The one virtual register this release has (spec §9.3). Reserved at the root.
const VRAM_ADDR: &str = "vram_addr";

/// The register namespaces this release has. Seeds the root scope in
/// `Analyzer::new` and is what `NAMESPACE_LIST` promises, so a namespace cannot
/// be added to one and missed by the other.
const NAMESPACES: [&str; 3] = ["ppu", "mmc3", "apu"];

/// The label under the carets when a namespace does not exist and rasterc has
/// nothing more specific to say.
///
/// Written out rather than joined from `NAMESPACES`: three names read as
/// "`ppu`, `mmc3` and `apu`" and a fourth would not, and this module's own
/// `the_namespace_list_names_every_namespace` goes red the moment a third
/// arrives, which is when somebody has to write the prose anyway.
const NAMESPACE_LIST: &str = "rasterc has `ppu`, `mmc3` and `apu`";

/// Register names §9.1 of the specification used until raster-3o3, and the
/// spelling that reaches the port today. An author reading an older draft types
/// `oam.addr`, and so does one who knows $2003 as OAMADDR.
///
/// Each row is (namespace, member, the port, the name that works).
const RENAMED: [(&str, &str, &str, &str); 3] = [
    ("oam", "addr", "$2003", "ppu.oam_addr"),
    ("oam", "data", "$2004", "ppu.oam_data"),
    ("oam", "dma", "$4014", "ppu.oam_dma"),
];

/// The leftmost `Name` of a member chain: `apu.pulse1.volume` and `apu.pulse1`
/// both root at `apu`.
///
/// `None` when the chain roots at something that is not a name — `f().x`,
/// `a[0].y` — which no register access can be.
fn member_root(expression: &Spanned<Expression>) -> Option<&Spanned<String>> {
    match &expression.value {
        Expression::Name(name) => Some(name),
        Expression::Member { base, .. } => member_root(base),
        _ => None,
    }
}

/// The member immediately after the root of a chain: `oam.addr` gives `addr`
/// and `apu.pulse1.volume` gives `pulse1`.
///
/// This, and not the outermost member, is what `RENAMED` and `NOT_YET` are
/// keyed on: `oam.addr` is the name the specification used, and `apu.pulse1` is
/// where an `apu` namespace would begin.
fn root_member(expression: &Spanned<Expression>) -> Option<&Spanned<String>> {
    let Expression::Member { base, member } = &expression.value else {
        return None;
    };
    match &base.value {
        Expression::Name(_) => Some(member),
        Expression::Member { .. } => root_member(base),
        _ => None,
    }
}

fn register_member(namespace: &str, member: &str) -> bool {
    match namespace {
        "ppu" => matches!(
            member,
            "ctrl"
                | "mask"
                | "status"
                | "oam_addr"
                | "oam_data"
                | "scroll"
                | "addr"
                | "data"
                | "oam_dma"
        ),
        "mmc3" => matches!(
            member,
            "bank_select"
                | "bank_data"
                | "mirroring"
                | "ram_protect"
                | "irq_latch"
                | "irq_reload"
                | "irq_disable"
                | "irq_enable"
        ),
        _ => false,
    }
}

/// Whether a block assigns to a PPU register, at any depth.
fn writes_ppu_register(block: &Block, vram_addr: bool) -> bool {
    block
        .statements
        .iter()
        .any(|statement| match &statement.value {
            Statement::Expression(expression) => is_ppu_register_write(expression, vram_addr),
            Statement::Block(body) | Statement::Loop(body) => writes_ppu_register(body, vram_addr),
            Statement::If {
                then_body,
                else_body,
                ..
            } => {
                writes_ppu_register(then_body, vram_addr)
                    || else_body
                        .as_ref()
                        .is_some_and(|body| writes_ppu_register(body, vram_addr))
            }
            Statement::While { body, .. }
            | Statement::For { body, .. }
            | Statement::Cycles { body, .. } => writes_ppu_register(body, vram_addr),
            _ => false,
        })
}

fn is_ppu_register_write(expression: &Spanned<Expression>, vram_addr: bool) -> bool {
    let Expression::Infix { left, operator, .. } = &expression.value else {
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
    if vram_addr && matches!(&left.value, Expression::Name(name) if name.value == VRAM_ADDR) {
        return true;
    }
    let Expression::Member { base, .. } = &left.value else {
        return false;
    };
    matches!(&base.value, Expression::Name(name) if name.value == "ppu")
}

/// Whether a statement is `sync exact`.
///
/// The guard looks for one anywhere earlier in the same block rather than immediately before the
/// region: `sync exact` followed by a variable assignment and then the region is the same
/// de-jitter, and refusing it would send the author looking for a rule that is not there.
fn is_sync_exact(statement: &Spanned<Statement>) -> bool {
    matches!(&statement.value, Statement::Sync(strategy) if strategy.value == "exact")
}

#[cfg(test)]
mod tests {
    use super::{NAMESPACE_LIST, NAMESPACES};

    /// `NAMESPACE_LIST` is prose, written out rather than joined, so it can
    /// drift from the namespaces it promises. Only a test inside this crate can
    /// see both — which is why the integration test that used to claim this
    /// guard could not actually make it. Review finding 3 on PR #47.
    #[test]
    fn the_namespace_list_names_every_namespace() {
        for namespace in NAMESPACES {
            assert!(
                NAMESPACE_LIST.contains(&format!("`{namespace}`")),
                "`{NAMESPACE_LIST}` should name `{namespace}`"
            );
        }
    }

    /// And names nothing else: a namespace removed from `NAMESPACES` and left
    /// in the prose promises an author something rasterc does not have.
    #[test]
    fn the_namespace_list_names_nothing_that_is_not_a_namespace() {
        assert_eq!(
            NAMESPACE_LIST.matches('`').count(),
            NAMESPACES.len() * 2,
            "`{NAMESPACE_LIST}` should quote exactly the {} namespaces",
            NAMESPACES.len()
        );
    }
}
