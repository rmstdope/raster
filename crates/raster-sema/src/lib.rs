use std::collections::{BTreeMap, HashMap};

use raster_syntax::{
    Block, CycleBound, Declaration, Expression, FrameEvent, FramePosition, Function, Item, Keyword,
    Operator, Program, Span, Spanned, Statement, Type, Wait,
};

#[derive(Debug)]
pub struct SemanticError {
    pub message: String,
    pub span: Span,
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
    /// Whether each enclosing timed region carries `pad`, innermost last.
    timed_regions: Vec<bool>,
    /// Which functions carry a cycle annotation, and so may be called from a timed region.
    annotated_functions: HashMap<String, bool>,
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
        for name in ["ppu", "mmc3"] {
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
            annotated_functions: HashMap::new(),
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push(SemanticError {
            message: message.into(),
            span,
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
        self.annotated_functions
            .insert(function.name.value.clone(), function.cycle_spec.is_some());
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
            self.timed_regions.push(spec.pad);
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

    fn check_frame_event(&mut self, event: &FrameEvent) {
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
                && writes_ppu_register(body)
                && !is_sync_exact(
                    index
                        .checked_sub(1)
                        .map(|previous| &block.statements[previous]),
                )
            {
                self.error(
                    spec.span,
                    "`sync exact` is required before a timed region that writes a PPU register, \
                     because NMI entry is not cycle-exact",
                );
            }
            self.check_statement(&statement.value, statement.span);
        }
    }

    fn in_timed_region(&self) -> bool {
        !self.timed_regions.is_empty()
    }

    /// Reject an expression a timed region cannot be charged for unless it is a known constant.
    fn require_static_in_timed_region(&mut self, expression: &Spanned<Expression>, message: &str) {
        if self.in_timed_region() && self.evaluate_constant(expression).is_none() {
            self.error(expression.span, message.to_owned());
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
                    self.error(
                        statement_span,
                        "an unbounded loop has no provable cycle cost inside a timed region",
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
                    self.error(
                        statement_span,
                        "branch arms inside a timed region cannot yet be balanced, because each \
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
                    self.error(
                        statement_span,
                        "a `while` loop's trip count cannot be proven inside a timed region; \
                         use `for` over a constant range",
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
                self.timed_regions.push(spec.pad);
                self.check_block(body);
                self.timed_regions.pop();
            }
            Statement::Wait(Wait::Scanline(value)) => {
                self.require_static(value, "wait bound");
                if self.in_timed_region() {
                    self.error(
                        statement_span,
                        "`wait scanline` has no provable cost inside a timed region",
                    );
                }
            }
            Statement::Wait(Wait::Cycles(value)) => {
                self.require_static(value, "wait bound");
            }
            Statement::Return(value) => match value {
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
            },
            Statement::Expression(expression) => {
                self.expression_type(expression);
            }
            Statement::Wait(Wait::Vblank(_)) => {
                if self.in_timed_region() {
                    self.error(
                        statement_span,
                        "`wait vblank` has no provable cost inside a timed region",
                    );
                }
            }
            Statement::Sync(strategy) => {
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
                let base_type = self.expression_type(base);
                if base_type != ValueType::Namespace {
                    self.error(base.span, "member access requires a register namespace");
                    return ValueType::Unknown;
                }
                let Expression::Name(root) = &base.value else {
                    return ValueType::Unknown;
                };
                if !register_member(&root.value, &member.value) {
                    self.error(
                        member.span,
                        format!("unknown {} register `{}`", root.value, member.value),
                    );
                    ValueType::Unknown
                } else {
                    ValueType::U8
                }
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
            Operator::Star => self.require_static_in_timed_region(
                right,
                "multiplication inside a timed region needs a compile-time constant multiplier",
            ),
            Operator::Slash | Operator::Percent => {
                self.require_static_in_timed_region(
                    left,
                    "division inside a timed region needs compile-time constant operands",
                );
                self.require_static_in_timed_region(
                    right,
                    "division inside a timed region needs compile-time constant operands",
                );
            }
            Operator::ShiftLeft | Operator::ShiftRight => self.require_static_in_timed_region(
                right,
                "a shift inside a timed region needs a compile-time constant shift count",
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
        if self.in_timed_region() && self.annotated_functions.get(&name.value) != Some(&true) {
            self.error(
                name.span,
                format!(
                    "`{}` must carry a cycle annotation to be called inside a timed region",
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
            Expression::Member { base, .. }
                if self.expression_type(base) == ValueType::Namespace => {}
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

fn is_sync_exact(statement: Option<&Spanned<Statement>>) -> bool {
    matches!(statement, Some(statement) if matches!(&statement.value, Statement::Sync(strategy) if strategy.value == "exact"))
}
