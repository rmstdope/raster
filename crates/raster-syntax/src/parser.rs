use crate::{
    Block, CycleBound, CycleSpec, Declaration, Expression, Frame, FrameEvent, FramePosition,
    Function, Identifier, Item, Keyword, Parameter, Program, Punctuation, Span, Spanned, Statement,
    Token, TokenKind, Type, lex,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

pub fn parse(source: &str) -> Result<Program, Vec<ParseError>> {
    let mut parser = Parser::new(source);
    let mut items = Vec::new();
    while !parser.at_end() {
        let current = parser.current;
        match parser.item() {
            Some(item) => items.push(item),
            None => parser.recover_item(),
        }
        if parser.current == current {
            parser.advance();
        }
    }
    if parser.errors.is_empty() {
        Ok(Program { items })
    } else {
        Err(parser.errors)
    }
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    current: usize,
    errors: Vec<ParseError>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            tokens: lex(source),
            current: 0,
            errors: Vec::new(),
        }
    }

    fn item(&mut self) -> Option<Spanned<Item>> {
        let start = self.peek().span;
        let item = match self.peek().value {
            TokenKind::Keyword(Keyword::Target) => {
                self.advance();
                self.expect_name("nes", "expected `nes` after `target`");
                Item::Target(self.opaque_block("expected `{` after target"))
            }
            TokenKind::Keyword(Keyword::Import) => {
                self.advance();
                let path = self
                    .take_string("expected string path after `import`")
                    .unwrap_or_default();
                self.consume_statement_terminator();
                Item::Import(path)
            }
            TokenKind::Keyword(Keyword::Fn)
            | TokenKind::Keyword(Keyword::Asm)
            | TokenKind::Keyword(Keyword::Unsafe) => Item::Function(self.function()),
            TokenKind::Keyword(Keyword::Frame) => Item::Frame(self.frame()),
            TokenKind::Keyword(Keyword::Main) => {
                self.advance();
                Item::Main(self.required_block("expected `{` after `main`"))
            }
            TokenKind::Keyword(keyword @ (Keyword::Const | Keyword::Var | Keyword::Group)) => {
                self.advance();
                Item::Declaration(self.declaration(keyword))
            }
            TokenKind::Keyword(
                Keyword::Asset
                | Keyword::Chrrom
                | Keyword::Charmap
                | Keyword::Bank
                | Keyword::Timeline,
            ) => Item::Other(self.opaque_item()),
            TokenKind::End => return None,
            _ => {
                self.error_here("expected a top-level declaration");
                return None;
            }
        };
        Some(Spanned::new(item, start.join(self.previous().span)))
    }

    fn function(&mut self) -> Function {
        let is_unsafe = self.match_keyword(Keyword::Unsafe);
        let is_assembly = self.match_keyword(Keyword::Asm);
        if is_unsafe && !is_assembly {
            self.error_here("`unsafe` is only valid before `asm fn`");
        }
        self.expect_keyword(Keyword::Fn, "expected `fn`");
        let name = self.required_identifier("expected function name");
        let parameters = self.parameters();
        let return_type = if self.match_operator(crate::Operator::Arrow) {
            Some(self.required_type())
        } else {
            None
        };

        let mut cycle_spec = None;
        let mut storage = None;
        let mut employs = Vec::new();
        while !self.at_end() && !self.check_punctuation(Punctuation::LeftBrace) {
            if self.check_keyword(Keyword::Cycles) {
                if cycle_spec.is_some() {
                    self.error_here("duplicate cycle specification");
                }
                cycle_spec = Some(self.cycle_spec());
            } else if self.match_keyword(Keyword::In) {
                storage = Some(self.required_identifier("expected storage after `in`"));
            } else if self.match_keyword(Keyword::Employs) {
                if !is_assembly {
                    self.error_here("`employs` is only valid on `asm fn`");
                }
                employs = self.employs_list();
            } else {
                break;
            }
        }

        let body = if is_assembly {
            self.opaque_block("expected function body")
        } else {
            self.required_block("expected function body")
        };
        Function {
            name,
            parameters,
            return_type,
            cycle_spec,
            storage,
            employs,
            body,
            is_assembly,
            is_unsafe,
        }
    }

    fn parameters(&mut self) -> Vec<Parameter> {
        let mut parameters = Vec::new();
        if !self.expect_punctuation(Punctuation::LeftParen, "expected `(` after function name") {
            return parameters;
        }
        while !self.at_end() && !self.check_punctuation(Punctuation::RightParen) {
            let name = self.required_identifier("expected parameter name");
            self.expect_punctuation(Punctuation::Colon, "expected `:` after parameter name");
            let type_annotation = self.required_type();
            parameters.push(Parameter {
                name,
                type_annotation,
            });

            if !self.match_punctuation(Punctuation::Comma) {
                if !self.check_punctuation(Punctuation::RightParen) {
                    self.error_here("expected `,` or `)` after parameter");
                }
                break;
            }
        }
        self.expect_punctuation(Punctuation::RightParen, "expected `)` after parameters");
        parameters
    }

    fn employs_list(&mut self) -> Vec<Identifier> {
        let mut employs = Vec::new();
        if !self.expect_punctuation(Punctuation::LeftParen, "expected `(` after `employs`") {
            return employs;
        }
        while !self.at_end() && !self.check_punctuation(Punctuation::RightParen) {
            employs.push(self.required_identifier("expected group name in `employs`"));
            if !self.match_punctuation(Punctuation::Comma) {
                break;
            }
        }
        self.expect_punctuation(Punctuation::RightParen, "expected `)` after `employs`");
        employs
    }

    fn frame(&mut self) -> Frame {
        self.advance();
        let name = if self.check_keyword(Keyword::Main) {
            let token = self.advance();
            Spanned::new("main".into(), token.span)
        } else {
            self.required_identifier("expected frame name")
        };
        let strategy = if self.match_keyword(Keyword::Using) {
            self.required_strategy()
        } else {
            None
        };

        self.expect_punctuation(Punctuation::LeftBrace, "expected `{` after frame header");
        let mut events = Vec::new();
        while !self.at_end() && !self.check_punctuation(Punctuation::RightBrace) {
            let current = self.current;
            let start = self.peek().span;
            let event = match self.peek().value {
                TokenKind::Keyword(Keyword::At) => {
                    self.advance();
                    self.frame_at_event()
                }
                TokenKind::Keyword(Keyword::Every) => {
                    self.advance();
                    self.frame_every_event()
                }
                _ => {
                    self.error_here("expected `at` or `every` frame event");
                    self.recover_statement();
                    if self.current == current {
                        self.advance();
                    }
                    continue;
                }
            };
            events.push(Spanned::new(event, start.join(self.previous().span)));
        }
        self.expect_punctuation(Punctuation::RightBrace, "expected `}` after frame");
        Frame {
            name,
            strategy,
            events,
        }
    }

    fn required_strategy(&mut self) -> Option<Identifier> {
        match self.peek().value.clone() {
            TokenKind::Identifier(value) => {
                let token = self.advance();
                Some(Spanned::new(value, token.span))
            }
            TokenKind::Keyword(Keyword::Irq) => {
                let token = self.advance();
                Some(Spanned::new("irq".into(), token.span))
            }
            TokenKind::Keyword(Keyword::Nmi) => {
                let token = self.advance();
                Some(Spanned::new("nmi".into(), token.span))
            }
            _ => {
                self.error_here("expected frame strategy after `using`");
                None
            }
        }
    }

    fn frame_at_event(&mut self) -> FrameEvent {
        let position = if self.match_name("scanline") {
            FramePosition::Scanline(self.required_expression())
        } else if self.match_name("vblank") {
            FramePosition::Vblank(self.previous().span)
        } else {
            self.error_here("expected `scanline` or `vblank` after `at`");
            FramePosition::Vblank(self.peek().span)
        };
        let body = self.required_block("expected event body");
        FrameEvent::At { position, body }
    }

    fn frame_every_event(&mut self) -> FrameEvent {
        let interval = self.required_expression();
        self.expect_name("scanlines", "expected `scanlines` after frame interval");
        self.expect_keyword(Keyword::From, "expected `from` after frame interval");
        let from = self.required_expression();
        self.expect_keyword(Keyword::To, "expected `to` after frame event start");
        let to = self.required_expression();
        let body = self.required_block("expected event body");
        FrameEvent::Every {
            interval,
            from,
            to,
            body,
        }
    }

    fn declaration(&mut self, kind: Keyword) -> Declaration {
        let name = self.take_identifier();
        if name.is_none() {
            if self.check_keyword(Keyword::Main) {
                self.error_here("reserved keyword `main` cannot be a declaration name");
            } else {
                self.error_here("expected declaration name");
            }
        }
        let type_annotation = if self.match_punctuation(Punctuation::Colon) {
            Some(self.required_type())
        } else {
            None
        };
        let storage = if self.match_keyword(Keyword::In) {
            Some(self.required_identifier("expected storage after `in`"))
        } else {
            None
        };
        let initializer = if self.match_operator(crate::Operator::Assign) {
            Some(self.required_expression())
        } else {
            None
        };
        let body = if self.check_punctuation(Punctuation::LeftBrace) {
            Some(self.required_block("expected declaration body"))
        } else {
            self.consume_statement_terminator();
            None
        };
        Declaration {
            kind,
            name,
            type_annotation,
            storage,
            initializer,
            body,
        }
    }

    fn required_block(&mut self, message: &str) -> Block {
        let start = self.peek().span;
        if !self.expect_punctuation(Punctuation::LeftBrace, message) {
            return Block {
                statements: Vec::new(),
                span: start,
            };
        }
        let mut statements = Vec::new();
        while !self.at_end() && !self.check_punctuation(Punctuation::RightBrace) {
            let current = self.current;
            match self.statement() {
                Some(statement) => statements.push(statement),
                None => self.recover_statement(),
            }
            if self.current == current {
                self.advance();
            }
        }
        let end = self.peek().span;
        self.expect_punctuation(Punctuation::RightBrace, "expected `}` to close block");
        Block {
            statements,
            span: start.join(end),
        }
    }

    fn opaque_block(&mut self, message: &str) -> Block {
        let start = self.peek().span;
        if !self.expect_punctuation(Punctuation::LeftBrace, message) {
            return Block {
                statements: Vec::new(),
                span: start,
            };
        }

        let mut depth = 1usize;
        while !self.at_end() && depth > 0 {
            match self.advance().value {
                TokenKind::Punctuation(Punctuation::LeftBrace) => depth += 1,
                TokenKind::Punctuation(Punctuation::RightBrace) => depth -= 1,
                _ => {}
            }
        }
        if depth != 0 {
            self.error_here("expected `}` to close block");
        }
        Block {
            statements: Vec::new(),
            span: start.join(self.previous().span),
        }
    }

    fn opaque_item(&mut self) -> Block {
        let start = self.peek().span;
        self.advance();
        while !self.at_end()
            && !self.check_punctuation(Punctuation::LeftBrace)
            && !self.at_statement_end()
        {
            self.advance();
        }
        if self.check_punctuation(Punctuation::LeftBrace) {
            self.opaque_block("expected `{` after top-level item")
        } else {
            Block {
                statements: Vec::new(),
                span: start.join(self.previous().span),
            }
        }
    }

    fn statement(&mut self) -> Option<Spanned<Statement>> {
        let start = self.peek().span;
        let statement = match self.peek().value {
            TokenKind::Keyword(Keyword::Var | Keyword::Const | Keyword::Group) => {
                let kind = match self.advance().value {
                    TokenKind::Keyword(kind) => kind,
                    _ => unreachable!(),
                };
                Statement::Declaration(self.declaration(kind))
            }
            TokenKind::Keyword(Keyword::If) => {
                self.advance();
                self.if_statement()
            }
            TokenKind::Keyword(Keyword::While) => {
                self.advance();
                let condition = self.required_expression();
                let body = self.required_block("expected while body");
                Statement::While { condition, body }
            }
            TokenKind::Keyword(Keyword::For) => {
                self.advance();
                let binding = self.required_identifier("expected loop binding");
                self.expect_keyword(Keyword::In, "expected `in` after loop binding");
                let range = self.required_expression();
                let step = if self.match_name("step") {
                    Some(self.required_expression())
                } else {
                    None
                };
                let body = self.required_block("expected for body");
                Statement::For {
                    binding,
                    range,
                    step,
                    body,
                }
            }
            TokenKind::Keyword(Keyword::Loop) => {
                self.advance();
                Statement::Loop(self.required_block("expected loop body"))
            }
            TokenKind::Keyword(Keyword::Cycles) => {
                let spec = self.cycle_spec();
                let label = if self.check_punctuation(Punctuation::LeftBrace) {
                    None
                } else {
                    self.take_identifier()
                };
                let body = self.required_block("expected cycles body");
                Statement::Cycles { spec, label, body }
            }
            TokenKind::Keyword(Keyword::Wait) => {
                self.advance();
                let value = if self.match_name("vblank") {
                    crate::Wait::Vblank(self.previous().span)
                } else if self.match_name("scanline") {
                    crate::Wait::Scanline(self.required_expression())
                } else if self.match_keyword(Keyword::Cycles) {
                    self.expect_punctuation(Punctuation::LeftParen, "expected `(` after `cycles`");
                    let cycles = self.required_expression();
                    self.expect_punctuation(
                        Punctuation::RightParen,
                        "expected `)` after cycle delay",
                    );
                    crate::Wait::Cycles(cycles)
                } else {
                    crate::Wait::Cycles(self.required_expression())
                };
                self.consume_statement_terminator();
                Statement::Wait(value)
            }
            TokenKind::Keyword(Keyword::Sync) => {
                self.advance();
                let name = self.required_identifier("expected sync strategy");
                self.consume_statement_terminator();
                Statement::Sync(name)
            }
            TokenKind::Keyword(Keyword::Break) => {
                self.advance();
                self.consume_statement_terminator();
                Statement::Break
            }
            TokenKind::Keyword(Keyword::Continue) => {
                self.advance();
                self.consume_statement_terminator();
                Statement::Continue
            }
            TokenKind::Keyword(Keyword::Return) => {
                self.advance();
                let value = if self.at_statement_boundary() {
                    None
                } else {
                    Some(self.required_expression())
                };
                self.consume_statement_terminator();
                Statement::Return(value)
            }
            TokenKind::Punctuation(Punctuation::LeftBrace) => {
                Statement::Block(self.required_block("expected block"))
            }
            TokenKind::Invalid(_) => {
                self.error_here("invalid character");
                return None;
            }
            TokenKind::End | TokenKind::Punctuation(Punctuation::RightBrace) => return None,
            _ => {
                let expression = self.required_expression();
                self.consume_statement_terminator();
                Statement::Expression(expression)
            }
        };
        Some(Spanned::new(statement, start.join(self.previous().span)))
    }

    fn if_statement(&mut self) -> Statement {
        let condition = self.required_expression();
        let then_body = self.required_block("expected if body");
        let else_body = if self.match_keyword(Keyword::Else) {
            if self.match_keyword(Keyword::If) {
                let start = self.previous().span;
                let nested = self.if_statement();
                let span = start.join(self.previous().span);
                Some(Block {
                    statements: vec![Spanned::new(nested, span)],
                    span,
                })
            } else {
                Some(self.required_block("expected else body"))
            }
        } else {
            None
        };
        Statement::If {
            condition,
            then_body,
            else_body,
        }
    }

    fn cycle_spec(&mut self) -> CycleSpec {
        self.expect_keyword(Keyword::Cycles, "expected `cycles`");
        self.expect_punctuation(Punctuation::LeftParen, "expected `(` after `cycles`");
        let bound = if self.match_punctuation(Punctuation::Question) {
            CycleBound::Inferred(self.previous().span)
        } else if self.match_operator(crate::Operator::LessEqual) {
            CycleBound::AtMost(self.required_expression())
        } else {
            CycleBound::Exact(self.required_expression())
        };
        self.expect_punctuation(Punctuation::RightParen, "expected `)` after cycle bound");
        let pad = self.match_keyword(Keyword::Pad);
        let interruptible = self.match_name("interruptible");
        CycleSpec {
            bound,
            pad,
            interruptible,
        }
    }

    fn required_type(&mut self) -> Spanned<Type> {
        self.type_annotation().unwrap_or_else(|| {
            let span = self.peek().span;
            Spanned::new(Type::Name(Spanned::new(String::new(), span)), span)
        })
    }

    fn type_annotation(&mut self) -> Option<Spanned<Type>> {
        if self.match_punctuation(Punctuation::LeftBracket) {
            let start = self.previous().span;
            let length = self.required_expression();
            self.expect_punctuation(Punctuation::RightBracket, "expected `]` after array length");
            let element = self.required_type();
            let span = start.join(element.span);
            return Some(Spanned::new(
                Type::Array {
                    length: Box::new(length),
                    element: Box::new(element),
                },
                span,
            ));
        }
        self.take_identifier()
            .map(|name| Spanned::new(Type::Name(name.clone()), name.span))
            .or_else(|| {
                self.error_here("expected type");
                None
            })
    }

    fn required_expression(&mut self) -> Spanned<Expression> {
        self.expression(0).unwrap_or_else(|| {
            let span = self.peek().span;
            Spanned::new(Expression::Name(Spanned::new(String::new(), span)), span)
        })
    }

    fn expression(&mut self, minimum_precedence: u8) -> Option<Spanned<Expression>> {
        let mut left = self.prefix_expression()?;
        loop {
            left = match self.peek().value {
                TokenKind::Punctuation(Punctuation::LeftParen) => self.call_expression(left),
                TokenKind::Punctuation(Punctuation::LeftBracket) => self.index_expression(left),
                TokenKind::Punctuation(Punctuation::Dot) => self.member_expression(left),
                TokenKind::Operator(operator) => {
                    let Some(precedence) = infix_precedence(operator) else {
                        break;
                    };
                    if precedence < minimum_precedence {
                        break;
                    }
                    let operator = self.advance();
                    let TokenKind::Operator(operator_value) = operator.value else {
                        unreachable!();
                    };
                    let right_precedence = if is_right_associative(operator_value) {
                        precedence
                    } else {
                        precedence + 1
                    };
                    let right = self.required_expression_with_precedence(right_precedence);
                    let span = left.span.join(right.span);
                    if operator_value == crate::Operator::DotDot {
                        Spanned::new(
                            Expression::Range {
                                start: Box::new(left),
                                end: Box::new(right),
                            },
                            span,
                        )
                    } else {
                        Spanned::new(
                            Expression::Infix {
                                left: Box::new(left),
                                operator: Spanned::new(operator_value, operator.span),
                                right: Box::new(right),
                            },
                            span,
                        )
                    }
                }
                _ => break,
            };
        }
        Some(left)
    }

    fn required_expression_with_precedence(&mut self, precedence: u8) -> Spanned<Expression> {
        self.expression(precedence).unwrap_or_else(|| {
            let span = self.peek().span;
            Spanned::new(Expression::Name(Spanned::new(String::new(), span)), span)
        })
    }

    fn prefix_expression(&mut self) -> Option<Spanned<Expression>> {
        match self.peek().value.clone() {
            TokenKind::Identifier(value) => {
                let token = self.advance();
                Some(Spanned::new(
                    Expression::Name(Spanned::new(value, token.span)),
                    token.span,
                ))
            }
            TokenKind::Number(value) => {
                let token = self.advance();
                Some(Spanned::new(Expression::Number(value), token.span))
            }
            TokenKind::String(value) => {
                let token = self.advance();
                Some(Spanned::new(Expression::String(value), token.span))
            }
            TokenKind::Character(value) => {
                let token = self.advance();
                Some(Spanned::new(Expression::Character(value), token.span))
            }
            TokenKind::Keyword(Keyword::True) | TokenKind::Keyword(Keyword::False) => {
                let token = self.advance();
                let TokenKind::Keyword(keyword) = token.value else {
                    unreachable!();
                };
                Some(Spanned::new(
                    Expression::Boolean(keyword == Keyword::True),
                    token.span,
                ))
            }
            TokenKind::Keyword(Keyword::Cycles) => {
                let token = self.advance();
                Some(Spanned::new(
                    Expression::Name(Spanned::new("cycles".into(), token.span)),
                    token.span,
                ))
            }
            TokenKind::Operator(
                operator @ (crate::Operator::Plus
                | crate::Operator::Minus
                | crate::Operator::Bang
                | crate::Operator::Tilde),
            ) => {
                let token = self.advance();
                let operand = self.required_expression_with_precedence(PREFIX_PRECEDENCE);
                let span = token.span.join(operand.span);
                Some(Spanned::new(
                    Expression::Prefix {
                        operator: Spanned::new(operator, token.span),
                        operand: Box::new(operand),
                    },
                    span,
                ))
            }
            TokenKind::Punctuation(Punctuation::LeftParen) => {
                let start = self.advance().span;
                let mut expression = self.required_expression();
                let end = self.peek().span;
                self.expect_punctuation(Punctuation::RightParen, "expected `)` after expression");
                expression.span = start.join(end);
                Some(expression)
            }
            _ => {
                self.error_here("expected expression");
                None
            }
        }
    }

    fn call_expression(&mut self, callee: Spanned<Expression>) -> Spanned<Expression> {
        self.advance();
        let mut arguments = Vec::new();
        while !self.at_end() && !self.check_punctuation(Punctuation::RightParen) {
            arguments.push(self.required_expression());
            if !self.match_punctuation(Punctuation::Comma) {
                if !self.check_punctuation(Punctuation::RightParen) {
                    self.error_here("expected `,` or `)` after argument");
                }
                break;
            }
        }
        let end = self.peek().span;
        self.expect_punctuation(Punctuation::RightParen, "expected `)` after arguments");
        Spanned::new(
            Expression::Call {
                callee: Box::new(callee.clone()),
                arguments,
            },
            callee.span.join(end),
        )
    }

    fn index_expression(&mut self, base: Spanned<Expression>) -> Spanned<Expression> {
        self.advance();
        let index = self.required_expression();
        let end = self.peek().span;
        self.expect_punctuation(Punctuation::RightBracket, "expected `]` after index");
        Spanned::new(
            Expression::Index {
                base: Box::new(base.clone()),
                index: Box::new(index),
            },
            base.span.join(end),
        )
    }

    fn member_expression(&mut self, base: Spanned<Expression>) -> Spanned<Expression> {
        self.advance();
        let member = self.required_member_identifier();
        let span = base.span.join(member.span);
        Spanned::new(
            Expression::Member {
                base: Box::new(base),
                member,
            },
            span,
        )
    }

    fn consume_statement_terminator(&mut self) {
        if self.match_punctuation(Punctuation::Semicolon) || self.at_statement_boundary() {
            return;
        }
        self.error_here("expected end of statement");
    }

    fn at_statement_boundary(&self) -> bool {
        self.at_end()
            || self.check_punctuation(Punctuation::RightBrace)
            || self.check_punctuation(Punctuation::Semicolon)
            || self.at_statement_end()
    }

    fn at_statement_end(&self) -> bool {
        if self.current == 0 || self.at_end() {
            return false;
        }
        self.source[self.previous().span.end as usize..self.peek().span.start as usize]
            .contains('\n')
    }

    fn recover_item(&mut self) {
        while !self.at_end() {
            if self.current > 0
                && self.at_statement_end()
                && matches!(self.peek().value, TokenKind::Keyword(_))
            {
                return;
            }
            self.advance();
        }
    }

    fn recover_statement(&mut self) {
        if !self.at_end() && !self.check_punctuation(Punctuation::RightBrace) {
            self.advance();
        }
        while !self.at_end()
            && !self.check_punctuation(Punctuation::RightBrace)
            && !self.at_statement_end()
        {
            self.advance();
        }
    }

    fn take_identifier(&mut self) -> Option<Identifier> {
        match self.peek().value.clone() {
            TokenKind::Identifier(value) => {
                let token = self.advance();
                Some(Spanned::new(value, token.span))
            }
            _ => None,
        }
    }

    fn required_identifier(&mut self, message: &str) -> Identifier {
        self.take_identifier().unwrap_or_else(|| {
            let span = self.peek().span;
            self.error_here(message);
            Spanned::new(String::new(), span)
        })
    }

    fn required_member_identifier(&mut self) -> Identifier {
        if let TokenKind::Keyword(Keyword::Palette) = self.peek().value {
            let token = self.advance();
            return Spanned::new("palette".into(), token.span);
        }
        self.required_identifier("expected member name after `.`")
    }

    fn match_name(&mut self, expected: &str) -> bool {
        if matches!(&self.peek().value, TokenKind::Identifier(value) if value == expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_name(&mut self, expected: &str, message: &str) {
        if !self.match_name(expected) {
            self.error_here(message);
        }
    }

    fn take_string(&mut self, message: &str) -> Option<String> {
        match self.peek().value.clone() {
            TokenKind::String(value) => {
                self.advance();
                Some(value)
            }
            _ => {
                self.error_here(message);
                None
            }
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword, message: &str) {
        if !self.match_keyword(keyword) {
            self.error_here(message);
        }
    }

    fn check_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.peek().value, TokenKind::Keyword(value) if value == keyword)
    }

    fn match_keyword(&mut self, keyword: Keyword) -> bool {
        if self.check_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn match_operator(&mut self, wanted: crate::Operator) -> bool {
        if matches!(self.peek().value, TokenKind::Operator(operator) if operator == wanted) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn check_punctuation(&self, punctuation: Punctuation) -> bool {
        matches!(self.peek().value, TokenKind::Punctuation(value) if value == punctuation)
    }

    fn match_punctuation(&mut self, punctuation: Punctuation) -> bool {
        if self.check_punctuation(punctuation) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_punctuation(&mut self, punctuation: Punctuation, message: &str) -> bool {
        if self.match_punctuation(punctuation) {
            true
        } else {
            self.error_here(message);
            false
        }
    }

    fn error_here(&mut self, message: &str) {
        let span = self.peek().span;
        if self
            .errors
            .last()
            .is_none_or(|previous| previous.span != span || previous.message != message)
        {
            self.errors.push(ParseError {
                message: message.into(),
                span,
            });
        }
    }

    fn at_end(&self) -> bool {
        matches!(self.peek().value, TokenKind::End)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current.saturating_sub(1)]
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.current].clone();
        if !self.at_end() {
            self.current += 1;
        }
        token
    }
}

const PREFIX_PRECEDENCE: u8 = 12;

fn infix_precedence(operator: crate::Operator) -> Option<u8> {
    Some(match operator {
        crate::Operator::Assign
        | crate::Operator::PlusEqual
        | crate::Operator::MinusEqual
        | crate::Operator::StarEqual
        | crate::Operator::SlashEqual => 1,
        crate::Operator::DotDot => 2,
        crate::Operator::PipePipe => 3,
        crate::Operator::AmpersandAmpersand => 4,
        crate::Operator::Pipe => 5,
        crate::Operator::Caret => 6,
        crate::Operator::Ampersand => 7,
        crate::Operator::EqualEqual | crate::Operator::BangEqual => 8,
        crate::Operator::Less
        | crate::Operator::LessEqual
        | crate::Operator::Greater
        | crate::Operator::GreaterEqual => 9,
        crate::Operator::Plus | crate::Operator::Minus => 10,
        crate::Operator::Star | crate::Operator::Slash | crate::Operator::Percent => 11,
        crate::Operator::Tilde
        | crate::Operator::Bang
        | crate::Operator::Arrow
        | crate::Operator::FatArrow => {
            return None;
        }
    })
}

fn is_right_associative(operator: crate::Operator) -> bool {
    matches!(
        operator,
        crate::Operator::Assign
            | crate::Operator::PlusEqual
            | crate::Operator::MinusEqual
            | crate::Operator::StarEqual
            | crate::Operator::SlashEqual
    )
}
