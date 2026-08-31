mod ast;
mod lexer;
mod parser;

pub use ast::*;
pub use lexer::{Token, TokenKind, lex};
pub use parser::{ParseError, parse};

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(source: &str, span: Span) -> &str {
        &source[span.start as usize..span.end as usize]
    }

    #[test]
    fn parses_timing_forms_with_spans() {
        let source = "fn shade() cycles(<= 12) pad {
}
main {
    sync exact
    cycles(114) pad {
        ppu.mask = 1
    }
    cycles(?) hblank {
        wait cycles(20)
    }
    cycles(28) interruptible {
        wait vblank
    }
}
";
        let program = parse(source).expect("the timing forms parse");

        let Item::Function(function) = &program.items[0].value else {
            panic!("the first item is a function")
        };
        let spec = function
            .cycle_spec
            .as_ref()
            .expect("an annotated function carries a cycle spec");
        assert_eq!(slice(source, spec.span), "cycles(<= 12) pad");
        assert!(spec.pad && !spec.interruptible);
        let CycleBound::AtMost(bound) = &spec.bound else {
            panic!("`<=` parses as an upper bound")
        };
        assert_eq!(slice(source, bound.span), "12");

        let Item::Main(block) = &program.items[1].value else {
            panic!("the second item is main")
        };
        let statements = &block.statements;

        let Statement::Sync(strategy) = &statements[0].value else {
            panic!("`sync exact` parses as a sync statement")
        };
        assert_eq!(strategy.value, "exact");

        let Statement::Cycles { spec, label, body } = &statements[1].value else {
            panic!("an exact budget parses as a timed region")
        };
        assert_eq!(slice(source, spec.span), "cycles(114) pad");
        assert!(spec.pad && !spec.interruptible);
        assert!(label.is_none());
        assert!(matches!(spec.bound, CycleBound::Exact(_)));
        assert_eq!(body.statements.len(), 1);

        let Statement::Cycles { spec, label, body } = &statements[2].value else {
            panic!("`cycles(?)` parses as a timed region")
        };
        assert_eq!(slice(source, spec.span), "cycles(?)");
        let CycleBound::Inferred(question) = spec.bound else {
            panic!("`?` parses as an inferred bound")
        };
        assert_eq!(slice(source, question), "?");
        assert_eq!(
            label.as_ref().map(|label| label.value.as_str()),
            Some("hblank")
        );
        assert!(matches!(
            body.statements[0].value,
            Statement::Wait(Wait::Cycles(_))
        ));

        let Statement::Cycles { spec, body, .. } = &statements[3].value else {
            panic!("`interruptible` parses as a timed region")
        };
        assert_eq!(slice(source, spec.span), "cycles(28) interruptible");
        assert!(!spec.pad && spec.interruptible);
        assert!(matches!(
            body.statements[0].value,
            Statement::Wait(Wait::Vblank(_))
        ));
    }

    #[test]
    fn lexer_recognizes_mvp_tokens_and_spans() {
        let source = "// setup\ncycles(<= 28) pad { ppu.mask = $1E | bars[line] << 1 >> 2 }";
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
                &TokenKind::Operator(Operator::ShiftLeft),
                &TokenKind::Number("1".into()),
                &TokenKind::Operator(Operator::ShiftRight),
                &TokenKind::Number("2".into()),
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

        let Item::Declaration(group) = &program.items[3].value else {
            panic!("expected the raster group");
        };
        assert_eq!(
            group.name.as_ref().map(|name| name.value.as_str()),
            Some("raster")
        );
        let group_body = group.body.as_ref().expect("group body");
        assert!(matches!(
            &group_body.statements[0].value,
            Statement::Declaration(Declaration {
                type_annotation: Some(annotation),
                storage: Some(storage),
                ..
            }) if matches!(&annotation.value, Type::Array { length, element }
                if matches!(&length.value, Expression::Number(value) if value == "240")
                    && matches!(&element.value, Type::Name(name) if name.value == "u8"))
                && storage.value == "bss"
        ));

        let Item::Function(init) = &program.items[4].value else {
            panic!("expected init function");
        };
        assert_eq!(init.name.value, "init");
        assert!(matches!(
            &init.body.statements[4].value,
            Statement::Expression(value)
                if matches!(&value.value, Expression::Call { callee, arguments }
                    if matches!(&callee.value, Expression::Name(name) if name.value == "load_palette")
                        && arguments.len() == 1)
        ));
        assert!(matches!(
            &init.body.statements[6].value,
            Statement::For { binding, range, body, .. }
                if binding.value == "i"
                    && matches!(&range.value, Expression::Range { start, end }
                        if matches!(&start.value, Expression::Number(value) if value == "0")
                            && matches!(&end.value, Expression::Number(value) if value == "240"))
                    && matches!(&body.statements[0].value, Statement::Expression(_))
        ));

        let Item::Frame(frame) = &program.items[5].value else {
            panic!("expected main frame");
        };
        assert_eq!(frame.name.value, "main");
        assert_eq!(
            frame
                .strategy
                .as_ref()
                .map(|strategy| strategy.value.as_str()),
            Some("irq")
        );
        assert!(matches!(
            &frame.events[0].value,
            FrameEvent::Every { interval, from, to, body }
                if matches!(&interval.value, Expression::Number(value) if value == "1")
                    && matches!(&from.value, Expression::Number(value) if value == "0")
                    && matches!(&to.value, Expression::Number(value) if value == "239")
                    && matches!(&body.statements[0].value, Statement::Cycles { spec, .. }
                        if spec.pad
                            && matches!(&spec.bound, CycleBound::AtMost(value)
                                if matches!(&value.value, Expression::Number(number) if number == "28")))
        ));

        let Item::Main(main) = &program.items[6].value else {
            panic!("expected main block");
        };
        assert!(matches!(
            &main.statements[1].value,
            Statement::Loop(body)
                if matches!(&body.statements[..], [Spanned { value: Statement::Wait(_), .. }, Spanned { value: Statement::Sync(name), .. }]
                    if name.value == "exact")
        ));
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

    #[test]
    fn parser_retains_declaration_and_function_signatures() {
        let source = r#"
            const LIMIT: u8 = 4
            var counter: [LIMIT]u8 in zp
            group state { var line: u8 }
            fn render(x: u8, y: u16) -> void cycles(<= 20) pad interruptible { return }
            unsafe asm fn upload(data: u8) -> void employs(state) cycles(?) {}
        "#;

        let program = parse(source).expect("the declarations and signatures should parse");

        let Item::Declaration(limit) = &program.items[0].value else {
            panic!("expected a declaration");
        };
        assert_eq!(limit.kind, Keyword::Const);
        let limit_name = limit.name.as_ref().expect("const name");
        assert_eq!(limit_name.value, "LIMIT");
        assert_eq!(
            limit_name.span,
            Span::new(
                source.find("LIMIT").unwrap() as u32,
                source.find("LIMIT").unwrap() as u32 + 5
            )
        );
        assert!(matches!(
            limit.type_annotation.as_ref().map(|annotation| &annotation.value),
            Some(Type::Name(name)) if name.value == "u8"
        ));
        assert!(matches!(
            limit.initializer.as_ref().map(|initializer| &initializer.value),
            Some(Expression::Number(value)) if value == "4"
        ));

        let Item::Declaration(counter) = &program.items[1].value else {
            panic!("expected a declaration");
        };
        assert!(matches!(
            counter.type_annotation.as_ref().map(|annotation| &annotation.value),
            Some(Type::Array { length, element })
                if matches!(&length.value, Expression::Name(name) if name.value == "LIMIT")
                    && matches!(&element.value, Type::Name(name) if name.value == "u8")
        ));
        assert_eq!(
            counter
                .storage
                .as_ref()
                .map(|storage| storage.value.as_str()),
            Some("zp")
        );

        let Item::Declaration(group) = &program.items[2].value else {
            panic!("expected a group declaration");
        };
        let group_body = group.body.as_ref().expect("group body");
        assert!(matches!(
            group_body.statements.as_slice(),
            [Spanned { value: Statement::Declaration(Declaration { name: Some(name), .. }), .. }]
                if name.value == "line"
        ));

        let Item::Function(render) = &program.items[3].value else {
            panic!("expected a function");
        };
        assert_eq!(render.name.value, "render");
        assert_eq!(
            render
                .parameters
                .iter()
                .map(|parameter| parameter.name.value.as_str())
                .collect::<Vec<_>>(),
            ["x", "y"]
        );
        assert!(matches!(
            render.return_type.as_ref().map(|annotation| &annotation.value),
            Some(Type::Name(name)) if name.value == "void"
        ));
        assert!(matches!(
            render.cycle_spec.as_ref().map(|spec| &spec.bound),
            Some(CycleBound::AtMost(value)) if matches!(&value.value, Expression::Number(number) if number == "20")
        ));
        assert!(
            render
                .cycle_spec
                .as_ref()
                .is_some_and(|spec| spec.pad && spec.interruptible)
        );

        let Item::Function(upload) = &program.items[4].value else {
            panic!("expected an assembly function");
        };
        assert_eq!(upload.name.value, "upload");
        assert!(upload.is_assembly);
        assert!(upload.is_unsafe);
        assert!(matches!(
            upload.cycle_spec.as_ref().map(|spec| &spec.bound),
            Some(CycleBound::Inferred(_))
        ));
    }

    #[test]
    fn parser_retains_expression_precedence_and_statement_payloads() {
        let source = r#"
            fn render() {
                ppu.mask = $1E | raster.bars[line]
                if ready && !done { return ppu.mask } else { while count < 3 { count += 1 } }
                for i in 0..240 step 4 { wait i + 1 }
                loop { sync exact }
                cycles(20) label { return }
                emit("text", 'x', true, false)
                result = value << 1 >> 2
            }
        "#;

        let program = parse(source).expect("the statement payloads should parse");
        let Item::Function(function) = &program.items[0].value else {
            panic!("expected a function");
        };
        let statements = &function.body.statements;

        let Statement::Expression(assignment) = &statements[0].value else {
            panic!("expected an expression statement");
        };
        let Expression::Infix {
            left,
            operator,
            right,
        } = &assignment.value
        else {
            panic!("expected an assignment");
        };
        assert_eq!(operator.value, Operator::Assign);
        assert!(matches!(&left.value, Expression::Member { member, .. } if member.value == "mask"));
        let Expression::Infix {
            operator: pipe,
            right: indexed,
            ..
        } = &right.value
        else {
            panic!("expected a bitwise-or expression");
        };
        assert_eq!(pipe.value, Operator::Pipe);
        assert!(matches!(
            &indexed.value,
            Expression::Index { base, index }
                if matches!(&base.value, Expression::Member { member, .. } if member.value == "bars")
                    && matches!(&index.value, Expression::Name(name) if name.value == "line")
        ));

        let Statement::If {
            condition,
            then_body,
            else_body,
        } = &statements[1].value
        else {
            panic!("expected an if statement");
        };
        assert!(matches!(
            &condition.value,
            Expression::Infix { operator, right, .. }
                if operator.value == Operator::AmpersandAmpersand
                    && matches!(&right.value, Expression::Prefix { operator, .. } if operator.value == Operator::Bang)
        ));
        assert!(matches!(
            &then_body.statements[0].value,
            Statement::Return(Some(value))
                if matches!(&value.value, Expression::Member { member, .. } if member.value == "mask")
        ));
        let else_body = else_body.as_ref().expect("else body");
        assert!(matches!(
            &else_body.statements[0].value,
            Statement::While { condition, body }
                if matches!(&condition.value, Expression::Infix { operator, .. } if operator.value == Operator::Less)
                    && matches!(&body.statements[0].value, Statement::Expression(_))
        ));

        let Statement::For {
            binding,
            range,
            step,
            body,
        } = &statements[2].value
        else {
            panic!("expected a for statement");
        };
        assert_eq!(binding.value, "i");
        assert!(matches!(&range.value, Expression::Range { .. }));
        assert!(matches!(
            step.as_ref().map(|step| &step.value),
            Some(Expression::Number(number)) if number == "4"
        ));
        assert!(matches!(
            &body.statements[0].value,
            Statement::Wait(Wait::Cycles(value))
                if matches!(&value.value, Expression::Infix { operator, .. } if operator.value == Operator::Plus)
        ));

        assert!(matches!(
            &statements[3].value,
            Statement::Loop(body)
                if matches!(&body.statements[0].value, Statement::Sync(name) if name.value == "exact")
        ));
        assert!(matches!(
            &statements[4].value,
            Statement::Cycles { spec, label: Some(label), body }
                if label.value == "label"
                    && matches!(&spec.bound, CycleBound::Exact(value) if matches!(&value.value, Expression::Number(number) if number == "20"))
                    && matches!(&body.statements[0].value, Statement::Return(None))
        ));
        assert!(matches!(
            &statements[5].value,
            Statement::Expression(value)
                if matches!(&value.value, Expression::Call { callee, arguments }
                    if matches!(&callee.value, Expression::Name(name) if name.value == "emit")
                        && matches!(arguments.as_slice(),
                            [Spanned { value: Expression::String(text), .. },
                             Spanned { value: Expression::Character(character), .. },
                             Spanned { value: Expression::Boolean(true), .. },
                             Spanned { value: Expression::Boolean(false), .. }]
                                if text == "text" && character == "x"))
        ));
        let Statement::Expression(value) = &statements[6].value else {
            panic!("expected a shift expression");
        };
        let Expression::Infix {
            operator: assign,
            right: shifted_right,
            ..
        } = &value.value
        else {
            panic!("expected an assignment");
        };
        assert_eq!(assign.value, Operator::Assign);
        let Expression::Infix {
            left: shifted_left,
            operator: right_shift,
            right: two,
        } = &shifted_right.value
        else {
            panic!("expected a right shift");
        };
        assert_eq!(right_shift.value, Operator::ShiftRight);
        assert!(matches!(&two.value, Expression::Number(number) if number == "2"));
        assert!(matches!(
            &shifted_left.value,
            Expression::Infix { operator, right, .. }
                if operator.value == Operator::ShiftLeft
                    && matches!(&right.value, Expression::Number(number) if number == "1")
        ));
    }

    #[test]
    fn parser_retains_frame_schedule_and_timing_bounds() {
        let source = r#"
            fn timed() cycles(42) pad interruptible {}
            fn inferred() cycles(?) {}
            main { cycles(<= LIMIT) label { wait cycles(3) } }
            frame display using irq {
                at scanline split + 1 { wait vblank }
                at vblank { sync exact }
                every interval scanlines from start to end {
                    cycles(<= 28) pad { ppu.mask = 1 }
                }
            }
        "#;

        let program = parse(source).expect("timing annotations and frames should parse");
        let Item::Function(timed) = &program.items[0].value else {
            panic!("expected a timed function");
        };
        assert!(matches!(
            timed.cycle_spec.as_ref().map(|spec| &spec.bound),
            Some(CycleBound::Exact(value)) if matches!(&value.value, Expression::Number(number) if number == "42")
        ));
        assert!(
            timed
                .cycle_spec
                .as_ref()
                .is_some_and(|spec| spec.pad && spec.interruptible)
        );

        let Item::Function(inferred) = &program.items[1].value else {
            panic!("expected an inferred function");
        };
        assert!(matches!(
            inferred.cycle_spec.as_ref().map(|spec| &spec.bound),
            Some(CycleBound::Inferred(span))
                if *span == Span::new(source.find('?').unwrap() as u32, source.find('?').unwrap() as u32 + 1)
        ));

        let Item::Main(main) = &program.items[2].value else {
            panic!("expected main");
        };
        assert!(matches!(
            &main.statements[0].value,
            Statement::Cycles { spec, label: Some(label), body }
                if label.value == "label"
                    && matches!(&spec.bound, CycleBound::AtMost(value) if matches!(&value.value, Expression::Name(name) if name.value == "LIMIT"))
                    && matches!(&body.statements[0].value, Statement::Wait(Wait::Cycles(value))
                        if matches!(&value.value, Expression::Number(number) if number == "3"))
        ));

        let Item::Frame(frame) = &program.items[3].value else {
            panic!("expected a frame");
        };
        assert_eq!(frame.name.value, "display");
        assert_eq!(
            frame
                .strategy
                .as_ref()
                .map(|strategy| strategy.value.as_str()),
            Some("irq")
        );
        assert!(matches!(
            &frame.events[0].value,
            FrameEvent::At { position: FramePosition::Scanline(position), body }
                if matches!(&position.value, Expression::Infix { operator, .. } if operator.value == Operator::Plus)
                    && matches!(&body.statements[0].value, Statement::Wait(Wait::Vblank(_)))
        ));
        assert!(matches!(
            &frame.events[1].value,
            FrameEvent::At { position: FramePosition::Vblank(span), body }
                if *span == Span::new(source.rfind("vblank").unwrap() as u32, source.rfind("vblank").unwrap() as u32 + 6)
                    && matches!(&body.statements[0].value, Statement::Sync(name) if name.value == "exact")
        ));
        assert!(matches!(
            &frame.events[2].value,
            FrameEvent::Every { interval, from, to, body }
                if matches!(&interval.value, Expression::Name(name) if name.value == "interval")
                    && matches!(&from.value, Expression::Name(name) if name.value == "start")
                    && matches!(&to.value, Expression::Name(name) if name.value == "end")
                    && matches!(&body.statements[0].value, Statement::Cycles { spec, .. }
                        if matches!(&spec.bound, CycleBound::AtMost(value) if matches!(&value.value, Expression::Number(number) if number == "28")))
        ));
    }

    #[test]
    fn parser_retains_wait_modes_and_assembly_employs() {
        let source = r#"
            unsafe asm fn upload() employs(vram_state, oam_state) employs(audio_state) cycles(<= 1400) {}
            main {
                wait vblank
                wait cycles(1234)
                wait scanline 96
            }
        "#;

        let program = parse(source).expect("wait modes and assembly employs should parse");
        let Item::Function(upload) = &program.items[0].value else {
            panic!("expected an assembly function");
        };
        assert_eq!(
            upload
                .employs
                .iter()
                .map(|group| group.value.as_str())
                .collect::<Vec<_>>(),
            ["vram_state", "oam_state", "audio_state"]
        );

        let Item::Main(main) = &program.items[1].value else {
            panic!("expected main");
        };
        assert!(matches!(
            &main.statements[0].value,
            Statement::Wait(Wait::Vblank(span)) if *span == Span::new(
                source.find("vblank").unwrap() as u32,
                source.find("vblank").unwrap() as u32 + 6
            )
        ));
        assert!(matches!(
            &main.statements[1].value,
            Statement::Wait(Wait::Cycles(value))
                if matches!(&value.value, Expression::Number(number) if number == "1234")
        ));
        assert!(matches!(
            &main.statements[2].value,
            Statement::Wait(Wait::Scanline(value))
                if matches!(&value.value, Expression::Number(number) if number == "96")
        ));
    }

    #[test]
    fn parser_reports_multiple_errors_after_structured_parse_failures() {
        let source = r#"
            const broken: [size u8 = 1
            fn bad(value: ) -> {
                ppu.mask = (1 +
                if { return )
            }
            frame broken using {
                at scanline { }
                every scanlines from to { }
            }
        "#;

        let errors = parse(source).expect_err("malformed structured input must fail");
        assert!(
            errors.len() >= 4,
            "expected independent structured errors, got {errors:#?}"
        );
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("type") || error.message.contains("expression"))
        );
        assert!(errors.iter().all(
            |error| error.span.start <= error.span.end && error.span.end <= source.len() as u32
        ));
    }
}
