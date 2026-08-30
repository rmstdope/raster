#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub const fn join(self, other: Self) -> Self {
        Self::new(self.start, other.end)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub const fn new(value: T, span: Span) -> Self {
        Self { value, span }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Program {
    pub items: Vec<Spanned<Item>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Item {
    Target(Block),
    Import(String),
    Declaration(Declaration),
    Function(Function),
    Frame(Frame),
    Main(Block),
    Other(Block),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    pub statements: Vec<Spanned<Statement>>,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Declaration {
    pub kind: Keyword,
    pub name: Option<String>,
    pub body: Option<Block>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function {
    pub name: String,
    pub body: Block,
    pub is_assembly: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub name: String,
    pub events: Vec<Spanned<FrameEvent>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameEvent {
    At(Block),
    Every(Block),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Statement {
    Declaration(Declaration),
    Block(Block),
    If(Block),
    While(Block),
    For(Block),
    Loop(Block),
    Cycles(Block),
    Return,
    Break,
    Continue,
    Expression,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Keyword {
    Target,
    Import,
    Const,
    Var,
    Group,
    Fn,
    Asm,
    Main,
    Loop,
    While,
    For,
    If,
    Else,
    Match,
    Break,
    Continue,
    Return,
    Cycles,
    Wait,
    Frame,
    At,
    Every,
    From,
    To,
    Using,
    Nmi,
    Irq,
    Timeline,
    Part,
    On,
    Asset,
    Png,
    Fam,
    Bin,
    Chrrom,
    Charmap,
    Palette,
    Bank,
    In,
    Out,
    Employs,
    Pad,
    Unsafe,
    True,
    False,
    Sync,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Operator {
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Ampersand,
    Pipe,
    Caret,
    Tilde,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    EqualEqual,
    BangEqual,
    AmpersandAmpersand,
    PipePipe,
    Bang,
    PlusEqual,
    MinusEqual,
    StarEqual,
    SlashEqual,
    Arrow,
    FatArrow,
    DotDot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Punctuation {
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Colon,
    Semicolon,
    Dot,
    Question,
}
