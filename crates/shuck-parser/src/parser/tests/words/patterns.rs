use super::*;

#[test]
fn test_brace_syntax_marks_literal_and_quoted_brace_forms() {
    let literal_input = "HEAD@{1}";
    let literal = Parser::parse_word_string(literal_input);
    assert_eq!(brace_slices(&literal, literal_input), vec!["{1}"]);
    assert_eq!(literal.brace_syntax()[0].kind, BraceSyntaxKind::Literal);
    assert!(literal.brace_syntax()[0].treated_literally());
    assert!(!literal.has_active_brace_expansion());

    let quoted_input = "\"{a,b}\"";
    let quoted = Parser::parse_word_string(quoted_input);
    assert_eq!(brace_slices(&quoted, quoted_input), vec!["{a,b}"]);
    assert_eq!(
        quoted.brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList)
    );
    assert_eq!(
        quoted.brace_syntax()[0].quote_context,
        BraceQuoteContext::DoubleQuoted
    );
    assert!(quoted.brace_syntax()[0].treated_literally());
    assert!(!quoted.has_active_brace_expansion());
}

#[test]
fn test_brace_syntax_treats_whitespace_and_quoted_lists_as_literal() {
    for input in ["{443, 8443}", "{tcp, udp}"] {
        let word = Parser::parse_word_string(input);
        assert_eq!(brace_slices(&word, input), vec![input]);
        assert_eq!(word.brace_syntax()[0].kind, BraceSyntaxKind::Literal);
        assert!(word.brace_syntax()[0].treated_literally());
        assert!(!word.has_active_brace_expansion());
    }

    let quoted_assembly = Parser::parse_word_string(r#"{"$mix_port, $redir_port, $tproxy_port"}"#);
    assert!(quoted_assembly.brace_syntax().is_empty());
    assert!(!quoted_assembly.has_active_brace_expansion());
}

#[test]
fn test_brace_syntax_marks_template_placeholders_inside_quotes() {
    let input = "\"$root/pkg/{{name}}/bin/{{cmd}}\"";
    let word = Parser::parse_word_string(input);

    assert_eq!(brace_slices(&word, input), vec!["{{name}}", "{{cmd}}"]);
    assert_eq!(word.brace_syntax().len(), 2);
    assert!(
        word.brace_syntax()
            .iter()
            .all(|brace| brace.kind == BraceSyntaxKind::TemplatePlaceholder)
    );
    assert!(
        word.brace_syntax()
            .iter()
            .all(|brace| brace.quote_context == BraceQuoteContext::DoubleQuoted)
    );
}

#[test]
fn test_brace_syntax_marks_all_nested_comma_lists() {
    let input =
        "usr/include/{sys/{capability,shm,sem},{glob,iconv,spawn,zlib,zconf},KHR/khrplatform}.h";
    let word = Parser::parse_word_string(input);

    assert_eq!(
        brace_slices(&word, input),
        vec![
            "{sys/{capability,shm,sem},{glob,iconv,spawn,zlib,zconf},KHR/khrplatform}",
            "{capability,shm,sem}",
            "{glob,iconv,spawn,zlib,zconf}",
        ]
    );
}

#[test]
fn test_brace_syntax_spans_quoted_members_inside_unquoted_lists() {
    let input =
        "\"$TERMUX_GODIR\"/{bin,src,doc,lib,\"pkg/tool/$TERMUX_GOLANG_DIRNAME\",pkg/include}";
    let word = Parser::parse_word_string(input);

    assert_eq!(
        brace_slices(&word, input),
        vec!["{bin,src,doc,lib,\"pkg/tool/$TERMUX_GOLANG_DIRNAME\",pkg/include}"]
    );
    assert!(word.has_active_brace_expansion());
}

#[test]
fn test_brace_syntax_does_not_treat_double_open_braces_as_template_placeholders_when_they_expand() {
    let input = "lib{{pthread,resolv,ffi_pic}.a,rt.so}";
    let word = Parser::parse_word_string(input);

    assert_eq!(
        brace_slices(&word, input),
        vec![
            "{{pthread,resolv,ffi_pic}.a,rt.so}",
            "{pthread,resolv,ffi_pic}"
        ]
    );
    assert!(word.brace_syntax().iter().all(|brace| brace.expands()));
}

#[test]
fn test_brace_syntax_ignores_escaped_unquoted_braces() {
    let word = Parser::parse_word_string("\\{a,b\\}");
    assert!(word.brace_syntax().is_empty());
    assert!(!word.has_active_brace_expansion());
}

#[test]
fn test_brace_syntax_keeps_ansi_c_escaped_quotes_inside_single_quoted_regions() {
    let input = r#"$'foo\'{a,b}'"#;
    let word = Parser::parse_word_string(input);

    assert_eq!(brace_slices(&word, input), vec!["{a,b}"]);
    assert!(
        word.brace_syntax()
            .iter()
            .all(|brace| brace.treated_literally())
    );
    assert!(!word.has_active_brace_expansion());
}

#[test]
fn test_brace_syntax_ignores_quoted_closers_when_balancing_cross_part_lists() {
    let input = r#"{"}",a}"#;
    let word = Parser::parse_word_string(input);

    assert_eq!(brace_slices(&word, input), vec![input]);
    assert_eq!(
        word.brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList)
    );
    assert!(word.has_active_brace_expansion());
}

#[test]
fn test_parse_word_with_mid_word_brace_segment_ignores_quoted_closers() {
    let input = "echo {\"}\",a}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].span.slice(input), r#"{"}",a}"#);
    assert_eq!(brace_slices(&command.args[0], input), vec![r#"{"}",a}"#]);
    assert!(command.args[0].has_active_brace_expansion());
}

#[test]
fn test_brace_syntax_handles_deeply_nested_braces_without_recursion() {
    let depth = 8192usize;
    let mut input = String::with_capacity(depth * 3 + 2);
    for _ in 0..depth {
        input.push('{');
        input.push('a');
    }
    input.push(',');
    input.push('b');
    for _ in 0..depth {
        input.push('}');
    }

    let word = Parser::parse_word_string(&input);

    assert_eq!(word.brace_syntax().len(), depth);
    assert_eq!(
        word.brace_syntax()
            .iter()
            .filter(|brace| brace.expands())
            .count(),
        1
    );
    assert_eq!(brace_slices(&word, &input).last().copied(), Some("{a,b}"));
    assert!(word.has_active_brace_expansion());
}

#[test]
fn test_nested_default_operand_keeps_quoted_right_brace_literal() {
    let input = "echo \"${outer:-${inner:-\"}\"}}\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let [
        WordPartNode {
            kind: WordPart::DoubleQuoted { parts, .. },
            ..
        },
    ] = word.parts.as_slice()
    else {
        panic!("expected one double-quoted argument");
    };

    let [
        WordPartNode {
            kind: WordPart::Parameter(parameter),
            ..
        },
    ] = parts.as_slice()
    else {
        panic!("expected one parameter expansion in outer quotes");
    };
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        operand: Some(operand),
        operand_word_ast: Some(operand_word_ast),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected outer parameter operation with parsed operand");
    };
    assert_eq!(operand.slice(input), "${inner:-\"}\"}");

    let [
        WordPartNode {
            kind: WordPart::Parameter(parameter),
            ..
        },
    ] = operand_word_ast.parts.as_slice()
    else {
        panic!("expected nested parameter expansion in outer operand");
    };
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        operand: Some(operand),
        operand_word_ast: Some(operand_word_ast),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected inner parameter operation with parsed operand");
    };
    assert_eq!(operand.slice(input), "\"}\"");

    let [
        WordPartNode {
            kind:
                WordPart::DoubleQuoted {
                    parts: quoted_parts,
                    ..
                },
            ..
        },
    ] = operand_word_ast.parts.as_slice()
    else {
        panic!("expected quoted inner operand word");
    };
    let [literal] = quoted_parts.as_slice() else {
        panic!("expected quoted operand to contain a single literal brace");
    };
    assert_eq!(literal.span.slice(input), "}");
}

#[test]
fn test_parse_pattern_preserves_dynamic_fragments_inside_extglob() {
    let input = "[[ value == --@($choice|$prefix-'x') ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };

    assert_eq!(pattern.render(input), "--@($choice|$prefix-x)");
    let PatternPart::Group { patterns, .. } = &pattern.parts[1].kind else {
        panic!("expected extglob group");
    };
    assert!(matches!(
        &patterns[0].parts[..],
        [PatternPartNode {
            kind: PatternPart::Word(word),
            ..
        }] if matches!(
            &word.parts[..],
            [WordPartNode {
                kind: WordPart::Variable(name),
                ..
            }]
            if name == "choice"
        )
    ));
    assert!(matches!(
        &patterns[1].parts[..],
        [
            PatternPartNode {
                kind: PatternPart::Word(variable),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Literal(text),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(quoted),
                ..
            }
        ] if matches!(
            &variable.parts[..],
            [WordPartNode {
                kind: WordPart::Variable(name),
                ..
            }]
            if name == "prefix"
        ) && text.as_str(input, patterns[1].parts[1].span) == "-" && is_fully_quoted(quoted)
    ));
}

#[test]
fn test_parse_pattern_preserves_delimiters_past_group_depth_limit() {
    let mut nested = "?(a|b)".to_string();
    for suffix in ["c", "d", "e", "f", "g", "h", "i", "j", "k"] {
        nested = format!("@({nested}|{suffix})");
    }
    let input = format!("[[ value == {nested} ]]\n");
    let script = Parser::new(&input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    let mut pattern = pattern;

    for suffix in ["k", "j", "i", "h", "g", "f", "e", "d"] {
        let [
            PatternPartNode {
                kind: PatternPart::Group { patterns, .. },
                ..
            },
        ] = pattern.parts.as_slice()
        else {
            panic!("expected nested group");
        };
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[1].render(&input), suffix);
        pattern = &patterns[0];
    }
    assert_eq!(pattern.render(&input), "@(?(a|b)|c)");
}

#[test]
fn test_parse_conditional_regex_rejects_unquoted_right_brace_operand() {
    let input = "[[ { =~ { ]]\n";
    assert!(Parser::new(input).parse().is_err());
}

#[test]
fn test_parse_glob_word_with_embedded_quote_stays_single_arg() {
    let input = "echo [hello\"]\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].span.slice(input), "[hello\"]\"");
}

#[test]
fn test_parse_glob_word_with_command_sub_in_bracket_expression_stays_single_arg() {
    let input = "echo [$(echo abc)]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].span.slice(input), "[$(echo abc)]");
}

#[test]
fn test_parse_glob_word_with_extglob_chars_stays_single_arg() {
    let input = "echo [+()]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].span.slice(input), "[+()]");
}

#[test]
fn test_parse_glob_word_with_trailing_literal_right_paren_stays_single_arg() {
    let input = "echo [+(])\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].span.slice(input), "[+(])");
}

#[test]
fn test_parse_glob_of_unescaped_double_left_bracket_stays_word() {
    let input = "echo [[z] []z]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 2);
    assert_eq!(command.args[0].span.slice(input), "[[z]");
    assert_eq!(command.args[1].span.slice(input), "[]z]");
}
