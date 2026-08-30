mod ast;
mod lexer;
mod parser;

pub use ast::*;
pub use lexer::{Token, TokenKind, lex};
pub use parser::{ParseError, parse};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer_recognizes_mvp_tokens_and_spans() {
        let source = "// setup\ncycles(<= 28) pad { ppu.mask = $1E | bars[line] }";
        let tokens = lex(source);
        let kinds: Vec<_> = tokens.iter().map(|token| &token.value).collect();

        assert_eq!(
            kinds,
            vec![
                &TokenKind::Keyword(Keyword::Cycles),
                &TokenKind::Punctuation(Punctuation::LeftParen),
                &TokenKind::Operator(Operator::LessEqual),
                &TokenKind::Number("28".into()),
                &TokenKind::Punctuation(Punctuation::RightParen),
                &TokenKind::Keyword(Keyword::Pad),
                &TokenKind::Punctuation(Punctuation::LeftBrace),
                &TokenKind::Identifier("ppu".into()),
                &TokenKind::Punctuation(Punctuation::Dot),
                &TokenKind::Identifier("mask".into()),
                &TokenKind::Operator(Operator::Assign),
                &TokenKind::Number("$1E".into()),
                &TokenKind::Operator(Operator::Pipe),
                &TokenKind::Identifier("bars".into()),
                &TokenKind::Punctuation(Punctuation::LeftBracket),
                &TokenKind::Identifier("line".into()),
                &TokenKind::Punctuation(Punctuation::RightBracket),
                &TokenKind::Punctuation(Punctuation::RightBrace),
                &TokenKind::End,
            ]
        );
        assert_eq!(tokens[0].span, Span::new(9, 15));
        assert_eq!(tokens[5].span, Span::new(23, 26));
        assert_eq!(tokens[10].span, Span::new(38, 39));
    }

    #[test]
    fn parser_accepts_the_complete_mvp_example() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/raster-language-spec.md"
        ));
        let example = source
            .split("## 13. Complete example — the MVP demo")
            .nth(1)
            .unwrap()
            .split("```")
            .nth(1)
            .unwrap();
        let program = parse(example).expect("the MVP example should parse");
        assert_eq!(program.items.len(), 7);
    }

    #[test]
    fn parser_returns_multiple_source_spanned_errors() {
        let errors = parse("wat\nmain {\n  @\n  #\n}").expect_err("invalid source must fail");
        assert_eq!(errors.len(), 3);
        assert_eq!(errors[0].span, Span::new(0, 3));
        assert_eq!(errors[1].span, Span::new(13, 14));
        assert_eq!(errors[2].span, Span::new(17, 18));
    }

    #[test]
    fn lexer_recovers_from_nested_comments_and_unterminated_literals() {
        let tokens = lex("/* outer /* inner */ */ \"unterminated");
        assert_eq!(tokens[0].value, TokenKind::Invalid('"'));
        assert_eq!(tokens[0].span, Span::new(24, 37));
    }

    #[test]
    fn parser_accepts_each_mvp_top_level_production() {
        let source = r#"
            target nes { mapper: mmc3 }
            import "shared.raster"
            const LIMIT: u8 = 4
            var counter: u8 in zp
            group state { var line: u8 }
            fn render(x: u8) -> void cycles(<= 20) { if x { return } else { break } }
            unsafe asm fn upload() employs(state) cycles(<= 20) { lda #0 }
            frame display using irq { at vblank { wait vblank } every 8 scanlines from 0 to 239 { ppu.mask = $1e } }
            asset image picture = png("picture.png") { palette: auto(4) }
            chrrom bank 0 { picture.tiles }
            charmap ascii { 'A' => 0 }
            bank fixed { fn handler() {} }
            timeline demo { part intro from row 0 to end on beat { render(1) } }
            main { loop { sync exact } }
        "#;

        let result = parse(source);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn parser_enforces_reserved_target_and_unsafe_grammar() {
        assert!(parse("target famicom {}").is_err());
        assert!(parse("unsafe fn helper() {}").is_err());
        assert!(parse("frame main {}").is_ok());
        assert!(parse("const main: u8 = 1").is_err());
    }
}
