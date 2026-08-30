use crate::{Keyword, Operator, Punctuation, Span, Spanned};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Number(String),
    String(String),
    Character(String),
    Keyword(Keyword),
    Operator(Operator),
    Punctuation(Punctuation),
    Invalid(char),
    End,
}

pub type Token = Spanned<TokenKind>;

pub fn lex(source: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token() {
        tokens.push(token);
    }
    tokens.push(Spanned::new(
        TokenKind::End,
        Span::new(source.len() as u32, source.len() as u32),
    ));
    tokens
}

struct Lexer<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, offset: 0 }
    }

    fn next_token(&mut self) -> Option<Token> {
        self.skip_trivia();
        let start = self.offset;
        let character = self.bump()?;
        let kind = match character {
            'A'..='Z' | 'a'..='z' | '_' => self.identifier_or_keyword(start),
            '0'..='9' => self.number(start),
            '$' => self.prefixed_number(start, |character| {
                character.is_ascii_hexdigit() || character == '_'
            }),
            '"' => self.quoted(start, '"', TokenKind::String),
            '\'' => self.quoted(start, '\'', TokenKind::Character),
            '(' => TokenKind::Punctuation(Punctuation::LeftParen),
            ')' => TokenKind::Punctuation(Punctuation::RightParen),
            '{' => TokenKind::Punctuation(Punctuation::LeftBrace),
            '}' => TokenKind::Punctuation(Punctuation::RightBrace),
            '[' => TokenKind::Punctuation(Punctuation::LeftBracket),
            ']' => TokenKind::Punctuation(Punctuation::RightBracket),
            ',' => TokenKind::Punctuation(Punctuation::Comma),
            ':' => TokenKind::Punctuation(Punctuation::Colon),
            ';' => TokenKind::Punctuation(Punctuation::Semicolon),
            '?' => TokenKind::Punctuation(Punctuation::Question),
            '.' if self.consume('.') => TokenKind::Operator(Operator::DotDot),
            '.' => TokenKind::Punctuation(Punctuation::Dot),
            '+' if self.consume('=') => TokenKind::Operator(Operator::PlusEqual),
            '+' => TokenKind::Operator(Operator::Plus),
            '-' if self.consume('>') => TokenKind::Operator(Operator::Arrow),
            '-' if self.consume('=') => TokenKind::Operator(Operator::MinusEqual),
            '-' => TokenKind::Operator(Operator::Minus),
            '*' if self.consume('=') => TokenKind::Operator(Operator::StarEqual),
            '*' => TokenKind::Operator(Operator::Star),
            '/' if self.consume('=') => TokenKind::Operator(Operator::SlashEqual),
            '/' => TokenKind::Operator(Operator::Slash),
            '%' => TokenKind::Operator(Operator::Percent),
            '&' if self.consume('&') => TokenKind::Operator(Operator::AmpersandAmpersand),
            '&' => TokenKind::Operator(Operator::Ampersand),
            '|' if self.consume('|') => TokenKind::Operator(Operator::PipePipe),
            '|' => TokenKind::Operator(Operator::Pipe),
            '^' => TokenKind::Operator(Operator::Caret),
            '~' => TokenKind::Operator(Operator::Tilde),
            '<' if self.consume('=') => TokenKind::Operator(Operator::LessEqual),
            '<' => TokenKind::Operator(Operator::Less),
            '>' if self.consume('=') => TokenKind::Operator(Operator::GreaterEqual),
            '>' => TokenKind::Operator(Operator::Greater),
            '=' if self.consume('=') => TokenKind::Operator(Operator::EqualEqual),
            '=' if self.consume('>') => TokenKind::Operator(Operator::FatArrow),
            '=' => TokenKind::Operator(Operator::Assign),
            '!' if self.consume('=') => TokenKind::Operator(Operator::BangEqual),
            '!' => TokenKind::Operator(Operator::Bang),
            other => TokenKind::Invalid(other),
        };
        Some(Spanned::new(
            kind,
            Span::new(start as u32, self.offset as u32),
        ))
    }

    fn skip_trivia(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
            if self.source[self.offset..].starts_with("//") {
                while self.bump().is_some_and(|character| character != '\n') {}
            } else if self.source[self.offset..].starts_with("/*") {
                let comment_start = self.offset;
                self.offset += 2;
                let mut depth = 1;
                while depth > 0 && self.offset < self.source.len() {
                    if self.source[self.offset..].starts_with("/*") {
                        self.offset += 2;
                        depth += 1;
                    } else if self.source[self.offset..].starts_with("*/") {
                        self.offset += 2;
                        depth -= 1;
                    } else {
                        self.bump();
                    }
                }
                if depth != 0 {
                    // Leave the opening delimiter for the next token so callers can
                    // diagnose the unterminated comment at the useful location.
                    self.offset = comment_start;
                    return;
                }
            } else {
                return;
            }
        }
    }

    fn identifier_or_keyword(&mut self, start: usize) -> TokenKind {
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            self.bump();
        }
        let text = &self.source[start..self.offset];
        keyword(text)
            .map(TokenKind::Keyword)
            .unwrap_or_else(|| TokenKind::Identifier(text.into()))
    }

    fn number(&mut self, start: usize) -> TokenKind {
        if self.source[start..].starts_with("0x") || self.source[start..].starts_with("0b") {
            self.bump();
        }
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            self.bump();
        }
        TokenKind::Number(self.source[start..self.offset].into())
    }

    fn prefixed_number(&mut self, start: usize, valid: impl Fn(char) -> bool) -> TokenKind {
        while self.peek().is_some_and(&valid) {
            self.bump();
        }
        TokenKind::Number(self.source[start..self.offset].into())
    }

    fn quoted(
        &mut self,
        start: usize,
        quote: char,
        make: impl FnOnce(String) -> TokenKind,
    ) -> TokenKind {
        let content_start = self.offset;
        let mut escaped = false;
        while let Some(character) = self.bump() {
            if character == quote && !escaped {
                return make(self.source[content_start..self.offset - quote.len_utf8()].into());
            }
            escaped = character == '\\' && !escaped;
            if character != '\\' {
                escaped = false;
            }
        }
        TokenKind::Invalid(self.source[start..].chars().next().unwrap_or(quote))
    }

    fn consume(&mut self, wanted: char) -> bool {
        if self.peek() == Some(wanted) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.offset += character.len_utf8();
        Some(character)
    }
}

fn keyword(text: &str) -> Option<Keyword> {
    Some(match text {
        "target" => Keyword::Target,
        "import" => Keyword::Import,
        "const" => Keyword::Const,
        "var" => Keyword::Var,
        "group" => Keyword::Group,
        "fn" => Keyword::Fn,
        "asm" => Keyword::Asm,
        "main" => Keyword::Main,
        "loop" => Keyword::Loop,
        "while" => Keyword::While,
        "for" => Keyword::For,
        "if" => Keyword::If,
        "else" => Keyword::Else,
        "match" => Keyword::Match,
        "break" => Keyword::Break,
        "continue" => Keyword::Continue,
        "return" => Keyword::Return,
        "cycles" => Keyword::Cycles,
        "wait" => Keyword::Wait,
        "frame" => Keyword::Frame,
        "at" => Keyword::At,
        "every" => Keyword::Every,
        "from" => Keyword::From,
        "to" => Keyword::To,
        "using" => Keyword::Using,
        "nmi" => Keyword::Nmi,
        "irq" => Keyword::Irq,
        "timeline" => Keyword::Timeline,
        "part" => Keyword::Part,
        "on" => Keyword::On,
        "asset" => Keyword::Asset,
        "png" => Keyword::Png,
        "fam" => Keyword::Fam,
        "bin" => Keyword::Bin,
        "chrrom" => Keyword::Chrrom,
        "charmap" => Keyword::Charmap,
        "palette" => Keyword::Palette,
        "bank" => Keyword::Bank,
        "in" => Keyword::In,
        "out" => Keyword::Out,
        "employs" => Keyword::Employs,
        "pad" => Keyword::Pad,
        "unsafe" => Keyword::Unsafe,
        "true" => Keyword::True,
        "false" => Keyword::False,
        "sync" => Keyword::Sync,
        _ => return None,
    })
}
