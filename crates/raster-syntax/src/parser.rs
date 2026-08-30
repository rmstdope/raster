use crate::{
    Block, Declaration, Frame, FrameEvent, Function, Item, Keyword, Program, Span, Spanned,
    Statement, Token, TokenKind, lex,
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
                Item::Target(self.required_block("expected `{` after target"))
            }
            TokenKind::Keyword(Keyword::Import) => {
                self.advance();
                let path = self
                    .take_string("expected string path after `import`")
                    .unwrap_or_default();
                self.finish_statement();
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
            TokenKind::Keyword(
                keyword @ (Keyword::Const
                | Keyword::Var
                | Keyword::Group
                | Keyword::Asset
                | Keyword::Chrrom
                | Keyword::Charmap
                | Keyword::Bank
                | Keyword::Timeline),
            ) => {
                self.advance();
                Item::Declaration(self.declaration(keyword))
            }
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
        let name = self
            .expect_identifier("expected function name")
            .unwrap_or_default();
        self.skip_to_block();
        Function {
            name,
            body: self.required_block("expected function body"),
            is_assembly,
            is_unsafe,
        }
    }

    fn frame(&mut self) -> Frame {
        self.advance();
        let name = match self.peek().value.clone() {
            TokenKind::Keyword(Keyword::Main) => {
                self.advance();
                "main".into()
            }
            _ => self
                .expect_identifier("expected frame name")
                .unwrap_or_default(),
        };
        self.skip_to_block();
        self.expect_left_brace("expected `{` after frame header");
        let mut events = Vec::new();
        while !self.at_end() && !self.check_right_brace() {
            let current = self.current;
            let start = self.peek().span;
            let event = match self.peek().value {
                TokenKind::Keyword(Keyword::At) => {
                    self.advance();
                    self.skip_to_block();
                    FrameEvent::At(self.required_block("expected event body"))
                }
                TokenKind::Keyword(Keyword::Every) => {
                    self.advance();
                    self.skip_to_block();
                    FrameEvent::Every(self.required_block("expected event body"))
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
        self.expect_right_brace("expected `}` after frame");
        Frame { name, events }
    }

    fn declaration(&mut self, kind: Keyword) -> Declaration {
        let name = self.take_identifier();
        if name.is_none() && matches!(self.peek().value, TokenKind::Keyword(Keyword::Main)) {
            self.error_here("reserved keyword `main` cannot be a declaration name");
        }
        let body = if self.skip_to_block_or_statement_end() {
            Some(self.required_block("expected declaration body"))
        } else {
            self.finish_statement();
            None
        };
        Declaration { kind, name, body }
    }

    fn required_block(&mut self, message: &str) -> Block {
        let start = self.peek().span;
        if !self.expect_left_brace(message) {
            // An expectation failure must consume a token (unless at EOF), otherwise
            // recovery can retry the same malformed token indefinitely.
            if !self.at_end() {
                self.advance();
            }
            return Block {
                statements: Vec::new(),
                span: start,
            };
        }
        let mut statements = Vec::new();
        while !self.at_end() && !self.check_right_brace() {
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
        self.expect_right_brace("expected `}` to close block");
        Block {
            statements,
            span: start.join(end),
        }
    }

    fn statement(&mut self) -> Option<Spanned<Statement>> {
        let start = self.peek().span;
        let statement = match self.peek().value {
            TokenKind::Keyword(Keyword::Var | Keyword::Const) => {
                let kind = match self.advance().value {
                    TokenKind::Keyword(kind) => kind,
                    _ => unreachable!(),
                };
                Statement::Declaration(self.declaration(kind))
            }
            TokenKind::Keyword(Keyword::If) => {
                self.advance();
                self.skip_to_block();
                let body = self.required_block("expected if body");
                if self.match_keyword(Keyword::Else) {
                    if self.match_keyword(Keyword::If) {
                        self.skip_to_block();
                    }
                    self.required_block("expected else body");
                }
                Statement::If(body)
            }
            TokenKind::Keyword(Keyword::While) => {
                self.advance();
                self.skip_to_block();
                Statement::While(self.required_block("expected while body"))
            }
            TokenKind::Keyword(Keyword::For) => {
                self.advance();
                self.skip_to_block();
                Statement::For(self.required_block("expected for body"))
            }
            TokenKind::Keyword(Keyword::Loop) => {
                self.advance();
                Statement::Loop(self.required_block("expected loop body"))
            }
            TokenKind::Keyword(Keyword::Cycles) => {
                self.advance();
                self.skip_to_block();
                Statement::Cycles(self.required_block("expected cycles body"))
            }
            TokenKind::Keyword(Keyword::Break) => {
                self.advance();
                self.finish_statement();
                Statement::Break
            }
            TokenKind::Keyword(Keyword::Continue) => {
                self.advance();
                self.finish_statement();
                Statement::Continue
            }
            TokenKind::Keyword(Keyword::Return) => {
                self.advance();
                self.finish_statement();
                Statement::Return
            }
            TokenKind::Punctuation(crate::Punctuation::LeftBrace) => {
                Statement::Block(self.required_block("expected block"))
            }
            TokenKind::Invalid(_) => {
                self.error_here("invalid character");
                return None;
            }
            TokenKind::End | TokenKind::Punctuation(crate::Punctuation::RightBrace) => return None,
            _ => {
                self.finish_statement();
                Statement::Expression
            }
        };
        Some(Spanned::new(statement, start.join(self.previous().span)))
    }

    fn skip_to_block(&mut self) {
        let mut depth = 0usize;
        while !self.at_end() {
            match self.peek().value {
                TokenKind::Punctuation(
                    crate::Punctuation::LeftParen | crate::Punctuation::LeftBracket,
                ) => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::Punctuation(
                    crate::Punctuation::RightParen | crate::Punctuation::RightBracket,
                ) if depth > 0 => {
                    depth -= 1;
                    self.advance();
                }
                TokenKind::Punctuation(crate::Punctuation::LeftBrace) if depth == 0 => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn skip_to_block_or_statement_end(&mut self) -> bool {
        while !self.at_end() && !self.check_right_brace() {
            if matches!(
                self.peek().value,
                TokenKind::Punctuation(crate::Punctuation::LeftBrace)
            ) {
                return true;
            }
            if self.at_statement_end() {
                return false;
            }
            self.advance();
        }
        false
    }

    fn finish_statement(&mut self) {
        let mut depth = 0usize;
        let mut braces = 0usize;
        while !self.at_end() {
            if self.check_right_brace() && braces == 0 {
                return;
            }
            match self.peek().value {
                TokenKind::Punctuation(
                    crate::Punctuation::LeftParen | crate::Punctuation::LeftBracket,
                ) => depth += 1,
                TokenKind::Punctuation(
                    crate::Punctuation::RightParen | crate::Punctuation::RightBracket,
                ) if depth > 0 => depth -= 1,
                TokenKind::Punctuation(crate::Punctuation::LeftBrace) => braces += 1,
                TokenKind::Punctuation(crate::Punctuation::RightBrace) if braces > 0 => braces -= 1,
                TokenKind::Punctuation(crate::Punctuation::Semicolon) if depth == 0 => {
                    self.advance();
                    return;
                }
                _ if depth == 0 && braces == 0 && self.at_statement_end() => return,
                _ => {}
            }
            self.advance();
        }
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
        if !self.at_end() && !self.check_right_brace() {
            self.advance();
        }
        while !self.at_end() && !self.check_right_brace() && !self.at_statement_end() {
            self.advance();
        }
    }

    fn take_identifier(&mut self) -> Option<String> {
        match self.peek().value.clone() {
            TokenKind::Identifier(value) => {
                self.advance();
                Some(value)
            }
            _ => None,
        }
    }

    fn expect_identifier(&mut self, message: &str) -> Option<String> {
        let value = self.take_identifier();
        if value.is_none() {
            self.error_here(message);
        }
        value
    }

    fn expect_name(&mut self, expected: &str, message: &str) {
        if matches!(&self.peek().value, TokenKind::Identifier(value) if value == expected) {
            self.advance();
        } else {
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

    fn match_keyword(&mut self, keyword: Keyword) -> bool {
        if matches!(self.peek().value, TokenKind::Keyword(value) if value == keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_left_brace(&mut self, message: &str) -> bool {
        if matches!(
            self.peek().value,
            TokenKind::Punctuation(crate::Punctuation::LeftBrace)
        ) {
            self.advance();
            true
        } else {
            self.error_here(message);
            false
        }
    }

    fn expect_right_brace(&mut self, message: &str) {
        if self.check_right_brace() {
            self.advance();
        } else {
            self.error_here(message);
        }
    }

    fn check_right_brace(&self) -> bool {
        matches!(
            self.peek().value,
            TokenKind::Punctuation(crate::Punctuation::RightBrace)
        )
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
