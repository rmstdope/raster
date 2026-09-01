use std::collections::{BTreeMap, HashMap};

use raster_diag::Refusal;
use raster_syntax::{
    Block, CycleBound, Declaration, Expression, FrameEvent, FramePosition, Function, Item, Keyword,
    Operator, Program, Span, Spanned, Statement, Type, Wait,
};

#[derive(Debug)]
pub struct SemanticError {
    pub message: String,
    /// What is drawn under the carets, when it differs from the message.
    /// `None` means the label mirrors the message, which is what every
    /// diagnostic in this compiler did before raster-3o3 and what most still do.
    pub label: Option<String>,
    pub span: Span,
    pub refusal: Refusal,
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
    Unknown,
}

#[derive(Clone)]
enum SymbolKind {
    Constant(Option<u32>),
    Variable,
    Group,
    Function(Vec<ValueType>, ValueType),
    Namespace,
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
        Self {
            scopes: vec![root],
            errors: Vec::new(),
            constants: BTreeMap::new(),
            return_type: ValueType::Void,
            timed_regions: Vec::new(),
            synchronized: false,
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

    fn refuse(&mut self, span: Span, message: impl Into<String>, refusal: Refusal) {
        self.errors.push(SemanticError {
            message: message.into(),
            label: None,
            span,
            refusal,
        });
    }

    /// Refuse with a label of its own: the message says what is wrong, and the
    /// label under the carets says what to write instead.
    fn refuse_with(
        &mut self,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
        refusal: Refusal,
    ) {
        self.errors.push(SemanticError {
            message: message.into(),
            label: Some(label.into()),
            span,
            refusal,
        });
    }

    fn error_with(&mut self, span: Span, message: impl Into<String>, label: impl Into<String>) {
        self.refuse_with(span, message, label, Refusal::Rejected);
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
        self.refuse_with(span, message, label, Refusal::NotInThisRelease);
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
        if let Some(previous_span) = previous_span {
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

    fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
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
        }
    }

    fn collect_top_level(&mut self, program: &Program) {
        for item in &program.items {
            match &item.value {
                Item::Declaration(declaration) => self.declare_declaration(declaration),
                Item::Function(function) => self.declare_function(function),
                _ => {}
            }
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
                for event in &frame.events {
                    self.check_frame_event(&event.value);
                }
                self.leave_scope();
            }
            Item::Main(block) => self.check_block(block),
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
    fn check_frame_event(&mut self, event: &FrameEvent) {
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
                && writes_ppu_register(body)
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

    /// Refuse something whose cost a straight-line region cannot charge.
    fn reject_in_timed_region(&mut self, span: Span, message: &str) {
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
                // The two sentences differ by one clause because an `interruptible` block emits no
                // `PHP`/`SEI`/`PLP`, so there is no interrupt flag left masked when the `return`
                // jumps out. `reject_in_timed_region` takes one fixed message and cannot say both.
                if let Some(block) = self.innermost_cycles_block() {
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
                let value_type = self.expression_type(expression);
                // A bare register read — `ppu.status` on a line of its own,
                // which §9.1 showed until raster-3o3. It reads $2002 for its
                // side effect and throws the byte away, and this release has no
                // such statement. `raster-ir` refuses it too, with a message
                // about statements in general; said here, where the shape is
                // known, it can name the fix.
                //
                // A non-`Unknown` type means `member_type` found a real
                // register, so the base is the namespace name.
                if value_type != ValueType::Unknown
                    && let Expression::Member { base, member } = &expression.value
                    && let Expression::Name(root) = &base.value
                {
                    self.error_with(
                        expression.span,
                        "a register read cannot stand on its own as a statement",
                        format!(
                            "assign it to a variable: `var s: u8 = {}.{}`",
                            root.value, member.value
                        ),
                    );
                }
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
            Expression::Member { base, member } => self.member_type(expression, base, member),
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
            Operator::Star | Operator::Slash | Operator::Percent => self.reject_in_timed_region(
                operator.span,
                "multiplication, division and remainder inside a timed block compile to loops \
                 whose cost is not yet proven",
            ),
            Operator::ShiftLeft | Operator::ShiftRight => self.reject_in_timed_region(
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

        // Hardware the specification names and this release cannot reach.
        if let Some((_, _, message, label)) = NOT_YET.iter().find(|(namespace, name, _, _)| {
            *namespace == root.value
                && match name {
                    None => true,
                    Some(name) => Some(*name) == member,
                }
        }) {
            self.not_in_this_release_with(whole.span, *message, *label);
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
            // returned a register or raised its one error. Adding "assignment
            // target must be ..." on top was the second half of every cascade,
            // and re-evaluating the base to decide was the duplicate `unknown
            // name` in the middle of it.
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

fn parse_number(value: &str) -> Option<u32> {
    if let Some(hexadecimal) = value.strip_prefix('$') {
        u32::from_str_radix(hexadecimal, 16).ok()
    } else {
        value.parse().ok()
    }
}

/// The register namespaces this release has. Seeds the root scope in
/// `Analyzer::new` and is what `NAMESPACE_LIST` promises, so a namespace cannot
/// be added to one and missed by the other.
const NAMESPACES: [&str; 2] = ["ppu", "mmc3"];

/// The label under the carets when a namespace does not exist and rasterc has
/// nothing more specific to say.
///
/// Written out rather than joined from `NAMESPACES`: two names read as "`ppu`
/// and `mmc3`" and three would not, and this module's own
/// `the_namespace_list_names_every_namespace` goes red the moment a third
/// arrives, which is when somebody has to write the prose anyway.
const NAMESPACE_LIST: &str = "rasterc has `ppu` and `mmc3`";

/// Register names §9.1 of the specification used until raster-3o3, and the
/// spelling that reaches the port today. An author reading an older draft types
/// `oam.addr`, and so does one who knows $2003 as OAMADDR.
///
/// Each row is (namespace, member, the port, the name that works).
const RENAMED: [(&str, &str, &str, &str); 2] = [
    ("oam", "addr", "$2003", "ppu.oam_addr"),
    ("oam", "data", "$2004", "ppu.oam_data"),
];

/// Hardware the specification names as intended and this release cannot reach.
///
/// The message names the *feature*, never a spelling. §9.1 no longer shows
/// `oam.dma`, and nothing in this project has chosen between `oam.dma` and
/// `ppu.oam_dma` — naming one here would settle a language question in a
/// terminal message. Decided with the navigator; see the bead's plan.
///
/// Each row is (namespace, the member it is about or `None` for the whole
/// namespace, the message, the label).
const NOT_YET: [(&str, Option<&str>, &str, &str); 2] = [
    (
        "oam",
        Some("dma"),
        "OAM DMA is not supported yet",
        "$4014 stalls the CPU for 513 or 514 cycles",
    ),
    (
        "apu",
        None,
        "the `apu` registers are not supported yet",
        "the sound driver owns the APU today (§8.3)",
    ),
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
            "ctrl" | "mask" | "status" | "oam_addr" | "oam_data" | "scroll" | "addr" | "data"
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
fn writes_ppu_register(block: &Block) -> bool {
    block
        .statements
        .iter()
        .any(|statement| match &statement.value {
            Statement::Expression(expression) => is_ppu_register_write(expression),
            Statement::Block(body) | Statement::Loop(body) => writes_ppu_register(body),
            Statement::If {
                then_body,
                else_body,
                ..
            } => {
                writes_ppu_register(then_body)
                    || else_body.as_ref().is_some_and(writes_ppu_register)
            }
            Statement::While { body, .. }
            | Statement::For { body, .. }
            | Statement::Cycles { body, .. } => writes_ppu_register(body),
            _ => false,
        })
}

fn is_ppu_register_write(expression: &Spanned<Expression>) -> bool {
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
