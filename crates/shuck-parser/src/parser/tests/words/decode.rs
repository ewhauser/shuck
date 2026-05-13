use super::*;

#[test]
fn test_current_word_cache_tracks_token_changes() {
    let input = "\"$foo\" bar\n";
    let mut parser = Parser::new(input);

    let first = parser.current_word().unwrap();
    assert_eq!(first.render(input), "$foo");
    assert!(is_fully_quoted(&first));
    let [quoted_part] = parser.current_word_cache.as_ref().unwrap().parts.as_slice() else {
        panic!("expected one quoted part");
    };
    let WordPart::DoubleQuoted { parts, .. } = &quoted_part.kind else {
        panic!("expected double-quoted word");
    };
    assert!(matches!(
        parts.as_slice(),
        [part] if matches!(&part.kind, WordPart::Variable(_))
    ));

    let repeated = parser.current_word().unwrap();
    assert_eq!(repeated.span, first.span);

    parser.advance();
    assert!(parser.current_word_cache.is_none());

    let next = parser.current_word().unwrap();
    assert_eq!(next.render(input), "bar");
    assert!(parser.current_word_cache.is_none());
}

#[test]
fn test_checkpoint_restore_rebuilds_current_word_cache() {
    let input = "\"$foo\" bar\n";
    let mut parser = Parser::new(input);

    let first = parser.current_word().unwrap();
    assert_eq!(first.render(input), "$foo");
    assert!(parser.current_word_cache.is_some());

    let checkpoint = parser.checkpoint();
    parser.advance();
    assert_eq!(parser.current_word().unwrap().render(input), "bar");

    parser.restore(checkpoint);
    assert!(parser.current_word_cache.is_none());
    let restored = parser.current_word().unwrap();
    assert_eq!(restored.render(input), "$foo");
    assert_eq!(restored.span, first.span);
    assert!(parser.current_word_cache.is_some());
}

#[test]
fn test_parse_word_fragment_preserves_original_span_for_cooked_text() {
    let source = r#"foo\/bar"#;
    let span = Span::from_positions(Position::new(), Position::new().advanced_by(source));

    let word = Parser::parse_word_fragment(source, "foo/bar", span);

    assert_eq!(word.render(source), "foo/bar");
    assert_eq!(word.span, span);
    assert_eq!(word.span.slice(source), source);
    assert!(matches!(
        &word.parts[..],
        [WordPartNode {
            kind: WordPart::Literal(text),
            ..
        }] if !text.is_source_backed() && text == "foo/bar"
    ));
}

#[test]
fn test_parse_quoted_flow_control_name_stays_simple_command() {
    let input = "'break' 2";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    assert!(is_fully_quoted(&command.name));
    assert_eq!(command.name.render(input), "break");
    assert_eq!(command.args[0].render(input), "2");
}

#[test]
fn test_parse_mixed_literal_word_consumes_segmented_token_directly() {
    let input = "printf foo\"bar\"'baz'";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let arg = &command.args[0];
    assert!(!is_fully_quoted(arg));
    assert_eq!(arg.render(input), "foobarbaz");
    assert_eq!(arg.parts.len(), 3);
    assert_eq!(arg.part_span(0).unwrap().slice(input), "foo");
    assert_eq!(arg.part_span(1).unwrap().slice(input), "\"bar\"");
    assert_eq!(arg.part_span(2).unwrap().slice(input), "'baz'");
    let WordPart::DoubleQuoted { parts, .. } = &arg.parts[1].kind else {
        panic!("expected double-quoted middle part");
    };
    assert_eq!(parts[0].span.slice(input), "bar");
    let WordPart::SingleQuoted { value, .. } = &arg.parts[2].kind else {
        panic!("expected single-quoted suffix part");
    };
    assert_eq!(value.slice(input), "baz");
}

#[test]
fn test_parse_single_quoted_prefix_word_consumes_segmented_token_directly() {
    let input = "printf 'foo'bar";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let arg = &command.args[0];
    assert!(!is_fully_quoted(arg));
    assert_eq!(arg.render(input), "foobar");
    assert_eq!(arg.parts.len(), 2);
    assert_eq!(arg.part_span(0).unwrap().slice(input), "'foo'");
    assert_eq!(arg.part_span(1).unwrap().slice(input), "bar");
}

#[test]
fn test_parse_word_string_keeps_escaped_dollar_literal() {
    let input = r#"\$HOME"#;
    let word = Parser::parse_word_string(input);

    assert_eq!(word.render(input), "$HOME");
    assert_eq!(word.render_syntax(input), input);
    assert!(matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Literal(text),
            ..
        }] if text.is_source_backed() && text.as_str(input, word.parts[0].span) == "$HOME"
    ));
}

#[test]
fn test_function_keyword_without_parens_preserves_surface_form() {
    let input = "function inc { :; }\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(function.uses_function_keyword());
    assert!(!function.has_name_parens());
    assert_eq!(
        function
            .header
            .function_keyword_span
            .map(|span| span.slice(input)),
        Some("function")
    );
    assert_eq!(function.header.trailing_parens_span, None);
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
}

#[test]
fn test_posix_function_keyword_without_parens_preserves_surface_form() {
    let input = "function inc { :; }\n";
    let script = Parser::with_dialect(input, ShellDialect::Posix)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(function.uses_function_keyword());
    assert!(!function.has_name_parens());
    assert_eq!(
        function
            .header
            .function_keyword_span
            .map(|span| span.slice(input)),
        Some("function")
    );
    assert_eq!(function.header.trailing_parens_span, None);
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
}

#[test]
fn test_function_keyword_with_parens_preserves_surface_form() {
    let input = "function inc() { :; }\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());

    assert!(function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert_eq!(
        function
            .header
            .function_keyword_span
            .map(|span| span.slice(input)),
        Some("function")
    );
    assert_eq!(
        function
            .header
            .trailing_parens_span
            .map(|span| span.slice(input)),
        Some("()")
    );
    assert!(matches!(compound, AstCompoundCommand::BraceGroup(_)));
    assert!(redirects.is_empty());
}

#[test]
fn test_function_keyword_allows_subshell_body() {
    let input = "function inc_subshell() ( j=$((j+5)); )\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::Subshell(body) = compound else {
        panic!("expected subshell function body");
    };
    assert!(function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
}

#[test]
fn test_function_keyword_allows_newline_conditional_body() {
    let input = "function f()\n[[ -n x ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional function body");
    };

    assert!(function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert!(redirects.is_empty());
    assert_eq!(command.span.slice(input), "[[ -n x ]]");
}

#[test]
fn test_function_keyword_rejects_same_line_conditional_body() {
    let parser = Parser::new("function f() [[ -n x ]]\n");
    assert!(
        parser.parse().is_err(),
        "same-line conditional body should be rejected for function keyword definitions"
    );
}

#[test]
fn test_function_keyword_accepts_bash_reserved_name_tokens() {
    let input = "\
function [[ { :; }
function ]] { :; }
function { { :; }
function } { :; }
";
    let script = Parser::new(input).parse().unwrap().file;

    let names = script
        .body
        .iter()
        .map(expect_function)
        .map(|function| function.header.entries[0].word.span.slice(input))
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["[[", "]]", "{", "}"]);
}

#[test]
fn test_adjacent_left_paren_after_command_word_is_a_parse_error() {
    let parser = Parser::new("foo$identity('z')\n");
    assert!(
        parser.parse().is_err(),
        "a command word followed immediately by '(' should be rejected"
    );
}

#[test]
fn test_parse_word_fragment_rebases_indirect_operator_spans() {
    let source = "echo ${!var//$'\\n'/' '}";
    let start = Position::new().advanced_by("echo ");
    let span = Span::from_positions(start, start.advanced_by("${!var//$'\\n'/' '}"));

    let word = Parser::parse_word_fragment(source, span.slice(source), span);
    let parameter = expect_parameter(&word);

    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indirect {
        operator: Some(operator),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected indirect replacement operator");
    };
    let ParameterOp::ReplaceAll {
        replacement,
        replacement_word_ast,
        ..
    } = operator.as_ref()
    else {
        panic!("expected indirect replacement operator");
    };

    assert_eq!(replacement.slice(source), "' '");
    assert_eq!(replacement_word_ast.render_syntax(source), "' '");
    assert_eq!(replacement_word_ast.span.slice(source), "' '");
}

#[test]
fn test_escaped_backticks_inside_double_quotes_stay_literal() {
    let input = "echo \"pre \\`pwd\\` post\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.render(input), "pre `pwd` post");

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert!(
        !parts
            .iter()
            .any(|part| matches!(part.kind, WordPart::CommandSubstitution { .. }))
    );
}

#[test]
fn test_escaped_backticks_after_escaped_backslashes_inside_double_quotes_stay_literal() {
    let input = "echo \"  echo Remember to run \\\\\\`updatedb\\\\'.\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.render(input), "  echo Remember to run \\`updatedb\\'.");

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert!(
        !parts
            .iter()
            .any(|part| matches!(part.kind, WordPart::CommandSubstitution { .. }))
    );
}

#[test]
fn test_process_substitution_like_text_inside_double_quotes_stays_literal() {
    let input = "echo \"<(printf hi)\" \" >(printf bye)\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    for word in &command.args {
        let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
            panic!("expected double-quoted word");
        };
        assert!(
            !parts
                .iter()
                .any(|part| matches!(part.kind, WordPart::ProcessSubstitution { .. })),
            "{:#?}",
            parts
        );
    }
}

#[test]
fn test_escaped_process_substitution_like_text_stays_literal() {
    let input = "echo \\<(printf hi) \\>(printf bye)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    for word in &command.args {
        assert!(
            !word
                .parts
                .iter()
                .any(|part| matches!(part.kind, WordPart::ProcessSubstitution { .. })),
            "{:#?}",
            word.parts
        );
    }
}

#[test]
fn test_escaped_backticks_stay_literal_unquoted() {
    let input = "echo \\`pwd\\`\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.render(input), "`pwd`");
    assert_eq!(word.render_syntax(input), "\\`pwd\\`");
    assert!(matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Literal(text),
            ..
        }] if text.is_source_backed() && text.as_str(input, word.parts[0].span) == "`pwd`"
    ));
}

#[test]
fn test_unquoted_backtick_substitution_can_contain_spaces() {
    let input = "commands=(`pyenv-commands --sh`)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment");
    };

    assert_eq!(array.elements.len(), 1);
    let ArrayElem::Sequential(word) = &array.elements[0] else {
        panic!("expected sequential element");
    };

    assert_eq!(word.render(input), "`pyenv-commands --sh`");
    let WordPart::CommandSubstitution { body, syntax } = &word.parts[0].kind else {
        panic!("expected backtick substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::Backtick);
    assert_eq!(body.len(), 1);
    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "pyenv-commands");
    assert_eq!(inner.args[0].render(input), "--sh");
}

#[test]
fn test_dollar_quoted_words_preserve_quote_variants() {
    let input = "printf $'line\\n' $\"prefix $HOME\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 2);

    let ansi = &command.args[0];
    assert!(is_fully_quoted(ansi));
    assert_eq!(top_level_part_slices(ansi, input), vec!["$'line\\n'"]);
    let WordPart::SingleQuoted { value, dollar } = &ansi.parts[0].kind else {
        panic!("expected single-quoted word");
    };
    assert!(*dollar);
    assert_eq!(value.slice(input), "line\n");

    let translated = &command.args[1];
    assert!(is_fully_quoted(translated));
    assert_eq!(
        top_level_part_slices(translated, input),
        vec!["$\"prefix $HOME\""]
    );
    let WordPart::DoubleQuoted { parts, dollar } = &translated.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert!(*dollar);
    let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
    assert_eq!(slices, vec!["prefix ", "$HOME"]);
    assert!(matches!(parts[1].kind, WordPart::Variable(ref name) if name == "HOME"));
}

#[test]
fn test_dollar_quotes_stay_literal_inside_double_quotes() {
    let input = "printf \"%s\" \"$'inner'\" \"$\\\"inner\\\"\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 3);

    for arg in &command.args[1..] {
        let WordPart::DoubleQuoted { parts, .. } = &arg.parts[0].kind else {
            panic!("expected double-quoted word");
        };
        assert_eq!(arg.render_syntax(input), arg.span.slice(input));
        assert!(
            !parts.iter().any(|part| matches!(
                part.kind,
                WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { dollar: true, .. }
            )),
            "double-quoted contents should keep nested dollar-quote syntax literal: {parts:#?}"
        );
    }
}

#[test]
fn test_for_loop_words_consume_segmented_tokens_directly() {
    let input = "for item in foo\"bar\" 'baz'qux; do echo \"$item\"; done";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    let words = command.words.as_ref().expect("expected explicit for words");
    assert_eq!(words.len(), 2);
    assert_eq!(words[0].render(input), "foobar");
    assert_eq!(words[0].parts.len(), 2);
    assert_eq!(words[0].part_span(0).unwrap().slice(input), "foo");
    assert_eq!(words[0].part_span(1).unwrap().slice(input), "\"bar\"");

    assert_eq!(words[1].render(input), "bazqux");
    assert!(!is_fully_quoted(&words[1]));
    assert_eq!(words[1].parts.len(), 2);
    assert_eq!(words[1].part_span(0).unwrap().slice(input), "'baz'");
    assert_eq!(words[1].part_span(1).unwrap().slice(input), "qux");
}

#[test]
fn test_parse_conditional_non_direct_var_ref_falls_back_to_word() {
    let input = "[[ -v prefix$var ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    let ConditionalExpr::Word(word) = unary.expr.as_ref() else {
        panic!("expected word fallback");
    };
    assert_eq!(word.render(input), "prefix$var");
}
