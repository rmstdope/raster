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
    Target(Target),
    Import(String),
    Declaration(Declaration),
    Function(Function),
    Frame(Frame),
    Main(Block),
    Asset(Asset),
    Other(Block),
}

/// A `target nes { ... }` block: the console the program declares it is built for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Target {
    pub fields: Vec<TargetField>,
    pub span: Span,
}

/// One `name: value` line of a `target` block.
///
/// The value is kept exactly as the token spelled it — `mmc3`, `128K`, `ntsc` —
/// because *which* values this release can build is `raster-ir`'s question and
/// not the parser's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetField {
    pub name: Spanned<String>,
    pub value: Spanned<String>,
    pub span: Span,
}

/// An `asset image name = png("path") { ... }` item.
///
/// `kind` is the word after `asset` (`image`), and `loader` is the loader
/// keyword (`png`). Both are kept even though only one spelling of each is
/// supported, so the refusal can point at what the author actually wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Asset {
    pub kind: Spanned<String>,
    pub name: Spanned<String>,
    pub loader: Spanned<String>,
    pub path: Spanned<String>,
    pub fields: Vec<AssetField>,
    pub span: Span,
}

/// One `name: value` line of an asset's block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetField {
    pub name: Spanned<String>,
    pub value: Spanned<AssetValue>,
    pub span: Span,
}

/// What an asset field may be set to.
///
/// Three shapes cover the whole of the specification's asset blocks:
/// `kind: background` is a word, `dedup: true` is a word, `palette: auto(4)`
/// is a call, and a bare number is accepted so a wrong one can be refused with
/// a message rather than a parse error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetValue {
    Word(String),
    Number(String),
    Call {
        name: String,
        argument: String,
    },
    /// The specification's explicit-palette form, `palette: [ $0F, $30, ... ]`
    /// (§8.1). Parsed so it can be refused by name, not by token.
    List(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    pub statements: Vec<Spanned<Statement>>,
    pub span: Span,
}

pub type Identifier = Spanned<String>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Type {
    Name(Identifier),
    Array {
        length: Box<Spanned<Expression>>,
        element: Box<Spanned<Type>>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expression {
    Name(Identifier),
    Number(String),
    String(String),
    Character(String),
    Boolean(bool),
    Prefix {
        operator: Spanned<Operator>,
        operand: Box<Spanned<Expression>>,
    },
    Infix {
        left: Box<Spanned<Expression>>,
        operator: Spanned<Operator>,
        right: Box<Spanned<Expression>>,
    },
    Call {
        callee: Box<Spanned<Expression>>,
        arguments: Vec<Spanned<Expression>>,
    },
    Index {
        base: Box<Spanned<Expression>>,
        index: Box<Spanned<Expression>>,
    },
    Member {
        base: Box<Spanned<Expression>>,
        member: Identifier,
    },
    Range {
        start: Box<Spanned<Expression>>,
        end: Box<Spanned<Expression>>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Declaration {
    pub kind: Keyword,
    pub name: Option<Identifier>,
    pub type_annotation: Option<Spanned<Type>>,
    pub storage: Option<Identifier>,
    pub initializer: Option<Spanned<Expression>>,
    pub body: Option<Block>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    pub name: Identifier,
    pub type_annotation: Spanned<Type>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function {
    pub name: Identifier,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<Spanned<Type>>,
    pub cycle_spec: Option<CycleSpec>,
    pub storage: Option<Identifier>,
    pub employs: Vec<Identifier>,
    pub body: Block,
    pub is_assembly: bool,
    pub is_unsafe: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CycleSpec {
    pub bound: CycleBound,
    pub pad: bool,
    pub interruptible: bool,
    /// The header alone — `cycles(<= 28) pad` — which is what a budget diagnostic underlines.
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CycleBound {
    Exact(Spanned<Expression>),
    AtMost(Spanned<Expression>),
    Inferred(Span),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub name: Identifier,
    pub strategy: Option<Identifier>,
    pub events: Vec<Spanned<FrameEvent>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FramePosition {
    Vblank(Span),
    Scanline(Spanned<Expression>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameEvent {
    At {
        position: FramePosition,
        body: Block,
    },
    Every {
        interval: Spanned<Expression>,
        from: Spanned<Expression>,
        to: Spanned<Expression>,
        body: Block,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Wait {
    Vblank(Span),
    Cycles(Spanned<Expression>),
    Scanline(Spanned<Expression>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Statement {
    Declaration(Declaration),
    Block(Block),
    If {
        condition: Spanned<Expression>,
        then_body: Block,
        else_body: Option<Block>,
    },
    While {
        condition: Spanned<Expression>,
        body: Block,
    },
    For {
        binding: Identifier,
        range: Spanned<Expression>,
        step: Option<Spanned<Expression>>,
        body: Block,
    },
    Loop(Block),
    Cycles {
        spec: CycleSpec,
        label: Option<Identifier>,
        body: Block,
    },
    Wait(Wait),
    Sync(Identifier),
    Return(Option<Spanned<Expression>>),
    Break,
    Continue,
    Expression(Spanned<Expression>),
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
    ShiftLeft,
    ShiftRight,
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
