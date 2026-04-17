use super::*;

fn word_part_tree_contains_variable(parts: &[WordPartNode], expected: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Variable(name) => name == expected,
        WordPart::DoubleQuoted { parts, .. } => word_part_tree_contains_variable(parts, expected),
        _ => false,
    })
}

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
fn test_parse_variable() {
    let parser = Parser::new("echo $HOME");
    let script = parser.parse().unwrap().file;

    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.args.len(), 1);
        assert_eq!(cmd.args[0].parts.len(), 1);
        assert!(matches!(&cmd.args[0].parts[0].kind, WordPart::Variable(v) if v == "HOME"));
    } else {
        panic!("expected simple command");
    }
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
fn test_parse_escaped_dollar_expansions_stay_literal_in_script_words() {
    let input = r#"echo \$HOME \${USER}"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 2);

    let expected = [("$HOME", r#"\$HOME"#), ("${USER}", r#"\${USER}"#)];
    for (word, (decoded, syntax)) in command.args.iter().zip(expected) {
        assert_eq!(word.render(input), decoded);
        assert_eq!(word.render_syntax(input), syntax);
        assert!(matches!(
            word.parts.as_slice(),
            [WordPartNode {
                kind: WordPart::Literal(text),
                ..
            }] if text.is_source_backed() && text.as_str(input, word.parts[0].span) == decoded
        ));
    }
}

#[test]
fn test_parse_escaped_braced_parameter_with_nested_default_stays_literal() {
    let input = r#"echo \${x:-$HOME}"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.render(input), "${x:-$HOME}");
    assert_eq!(word.render_syntax(input), r#"\${x:-$HOME}"#);
    assert!(matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Literal(text),
            ..
        }] if text.is_source_backed() && text.as_str(input, word.parts[0].span) == "${x:-$HOME}"
    ));
}

#[test]
fn test_parse_escaped_backslash_then_variable_keeps_variable_live() {
    let input = "echo \\\\$HOME\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];
    assert!(matches!(
        word.parts.as_slice(),
        [
            WordPartNode {
                kind: WordPart::Literal(text),
                ..
            },
            WordPartNode {
                kind: WordPart::Variable(name),
                ..
            }
        ] if text.as_str(input, word.parts[0].span) == "\\"
            && name.as_str() == "HOME"
    ));
}

#[test]
fn test_parse_mixed_quoted_and_cooked_plain_continuation_keeps_variable_live() {
    let input = "echo \"x\"\\\\$HOME\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(matches!(
        word.parts.as_slice(),
        [
            WordPartNode {
                kind: WordPart::DoubleQuoted { parts, .. },
                ..
            },
            WordPartNode {
                kind: WordPart::Literal(text),
                ..
            },
            WordPartNode {
                kind: WordPart::Variable(name),
                ..
            }
        ] if matches!(
            parts.as_slice(),
            [WordPartNode {
                kind: WordPart::Literal(inner),
                ..
            }] if inner.as_str(input, parts[0].span) == "x"
        ) && text.as_str(input, word.parts[1].span) == "\\"
            && name.as_str() == "HOME"
    ));
}

#[test]
fn test_parse_escaped_quote_before_command_substitution_keeps_substitution_live() {
    let input = "echo TERMUX_SUBPKG_INCLUDE=\\\"$(find ${_ADD_PREFIX}lib{,32})\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(
        word.parts
            .iter()
            .any(|part| !matches!(part.kind, WordPart::Literal(_))),
        "parts: {:?}",
        word.parts
    );
    assert_eq!(word.brace_syntax().len(), 0);
}

#[test]
fn test_parse_escaped_command_substitution_stays_literal_in_double_quotes() {
    let input = r#"echo "\$(pwd)""#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.render(input), "$(pwd)");
    assert_eq!(word.render_syntax(input), r#""\$(pwd)""#);
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert!(matches!(
        parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Literal(text),
            ..
        }] if text.is_source_backed() && text.as_str(input, parts[0].span) == "$(pwd)"
    ));
}

#[test]
fn test_parse_positional_parameters() {
    let parser = Parser::new("echo $@ $*");
    let script = parser.parse().unwrap().file;

    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.args.len(), 2);
        assert!(matches!(&cmd.args[0].parts[0].kind, WordPart::Variable(v) if v == "@"));
        assert!(matches!(&cmd.args[1].parts[0].kind, WordPart::Variable(v) if v == "*"));
    } else {
        panic!("expected simple command");
    }
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
fn test_zsh_function_keyword_allows_empty_compact_brace_body() {
    let input = "function quit() {}\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert!(redirects.is_empty());
    assert!(body.is_empty());
}

#[test]
fn test_non_zsh_dialects_reject_compact_function_keyword_brace_body() {
    for dialect in [ShellDialect::Posix, ShellDialect::Mksh, ShellDialect::Bash] {
        assert!(
            Parser::with_dialect("function quit() {}\n", dialect)
                .parse()
                .is_err(),
            "expected {dialect:?} to reject compact function-keyword brace body",
        );
    }
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
fn test_unterminated_single_quote_rejected() {
    let parser = Parser::new("echo 'unterminated");
    assert!(
        parser.parse().is_err(),
        "unterminated single quote should be rejected"
    );
}

#[test]
fn test_unterminated_double_quote_rejected() {
    let parser = Parser::new("echo \"unterminated");
    assert!(
        parser.parse().is_err(),
        "unterminated double quote should be rejected"
    );
}

#[test]
fn test_nested_expansion_in_array_subscript() {
    // ${arr[$RANDOM % ${#arr[@]}]} must parse without error.
    // The subscript contains ${#arr[@]} which has its own [ and ].
    let input = "echo ${arr[$RANDOM % ${#arr[@]}]}";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;
    assert_eq!(script.body.len(), 1);
    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.name.render(input), "echo");
        assert_eq!(cmd.args.len(), 1);
        // The arg should contain an ArrayAccess with the full nested index
        let arg = &cmd.args[0];
        let has_array_access = arg.parts.iter().any(|p| {
            array_access_reference(&p.kind).is_some_and(|reference| {
                reference.name == "arr"
                    && reference
                        .subscript
                        .as_ref()
                        .is_some_and(|subscript| subscript.text.slice(input).contains("${#arr[@]}"))
            })
        });
        assert!(
            has_array_access,
            "expected ArrayAccess with nested index, got: {:?}",
            arg.parts
        );
    } else {
        panic!("expected simple command");
    }
}

/// Assignment with nested subscript must parse (previously caused fuel exhaustion).

#[test]
fn test_assignment_nested_subscript_parses() {
    let parser = Parser::new("x=${arr[$RANDOM % ${#arr[@]}]}");
    assert!(
        parser.parse().is_ok(),
        "assignment with nested subscript should parse"
    );
}

#[test]
fn test_indexed_assignment_with_spaces_in_subscript_parses() {
    let input = "a[1 + 2]=3\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.assignments.len(), 1);
    assert_eq!(command.assignments[0].target.name, "a");
    expect_subscript(&command.assignments[0].target, input, "1 + 2");
    assert!(command.name.render(input).is_empty());
}

#[test]
fn test_parenthesized_indexed_assignment_is_not_function_definition() {
    let input = "a[(1+2)*3]=9\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.assignments.len(), 1);
    assert_eq!(command.assignments[0].target.name, "a");
    expect_subscript(&command.assignments[0].target, input, "(1+2)*3");
    assert!(command.name.render(input).is_empty());
}

#[test]
fn test_assignment_index_ast_tracks_arithmetic_subscripts() {
    let input = "a[i + 1]=x\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let assignment = &command.assignments[0];
    let subscript_ast = assignment
        .target
        .subscript
        .as_ref()
        .and_then(|subscript| subscript.arithmetic_ast.as_ref());
    let expr = subscript_ast.expect("expected arithmetic subscript AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected additive subscript");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_variable(left.as_ref(), "i");
    expect_number(right.as_ref(), input, "1");
}

#[test]
fn test_decl_name_and_array_access_attach_arithmetic_index_asts() {
    let input = "declare foo[1+2]\necho ${arr[i+1]}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration command");
    };
    let DeclOperand::Name(name) = &command.operands[0] else {
        panic!("expected declaration name operand");
    };
    let subscript_ast = name
        .subscript
        .as_ref()
        .and_then(|subscript| subscript.arithmetic_ast.as_ref());
    let expr = subscript_ast.expect("expected declaration index AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected additive expression in declaration index");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_number(left.as_ref(), input, "1");
    expect_number(right.as_ref(), input, "2");

    let AstCommand::Simple(command) = &script.body[1].command else {
        panic!("expected simple command");
    };
    let reference = expect_array_access(&command.args[0]);
    expect_subscript(reference, input, "i+1");
    let expr = reference
        .subscript
        .as_ref()
        .and_then(|subscript| subscript.arithmetic_ast.as_ref())
        .expect("expected array access index AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected additive array index");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_variable(left.as_ref(), "i");
    expect_number(right.as_ref(), input, "1");
}

#[test]
fn test_substring_and_array_slice_attach_arithmetic_companion_asts() {
    let input = "echo ${s:i+1:len*2} ${arr[@]:i:j}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let (_, offset_ast, length_ast) = expect_substring_part(&command.args[0].parts[0].kind);
    let offset_ast = offset_ast.as_ref().expect("expected substring offset AST");
    let ArithmeticExpr::Binary { left, op, right } = &offset_ast.kind else {
        panic!("expected additive substring offset");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_variable(left, "i");
    expect_number(right, input, "1");
    let length_ast = length_ast.as_ref().expect("expected substring length AST");
    let ArithmeticExpr::Binary {
        left: len_left,
        op: len_op,
        right: len_right,
    } = &length_ast.kind
    else {
        panic!("expected multiplicative substring length");
    };
    assert_eq!(*len_op, ArithmeticBinaryOp::Multiply);
    expect_variable(len_left, "len");
    expect_number(len_right, input, "2");

    let (_, offset_ast, length_ast) = expect_array_slice_part(&command.args[1].parts[0].kind);
    expect_variable(
        offset_ast
            .as_ref()
            .expect("expected array slice offset AST"),
        "i",
    );
    expect_variable(
        length_ast
            .as_ref()
            .expect("expected array slice length AST"),
        "j",
    );
}

#[test]
fn test_non_arithmetic_subscripts_leave_companion_ast_empty() {
    let input = "echo ${arr[@]} ${arr[*]} ${map[\"key\"]}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let reference = expect_array_access(&command.args[0]);
    let subscript = reference
        .subscript
        .as_ref()
        .expect("expected first array subscript");
    assert_eq!(subscript.selector(), Some(SubscriptSelector::At));
    assert!(subscript.arithmetic_ast.is_none());

    let reference = expect_array_access(&command.args[1]);
    let subscript = reference
        .subscript
        .as_ref()
        .expect("expected second array subscript");
    assert_eq!(subscript.selector(), Some(SubscriptSelector::Star));
    assert!(subscript.arithmetic_ast.is_none());

    let reference = expect_array_access(&command.args[2]);
    let subscript = reference
        .subscript
        .as_ref()
        .expect("expected third array subscript");
    assert_eq!(subscript.selector(), None);
    assert!(subscript.arithmetic_ast.is_none());
}

#[test]
fn test_parameter_forms_preserve_selector_kinds() {
    let input = "echo ${arr[@]} ${arr[*]} ${#arr[@]} ${!arr[*]} ${arr[@]:1:2}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let reference = expect_array_access(&command.args[0]);
    assert_eq!(
        reference.subscript.as_ref().and_then(Subscript::selector),
        Some(SubscriptSelector::At)
    );

    let reference = expect_array_access(&command.args[1]);
    assert_eq!(
        reference.subscript.as_ref().and_then(Subscript::selector),
        Some(SubscriptSelector::Star)
    );

    let reference = expect_array_length_part(&command.args[2].parts[0].kind);
    assert_eq!(
        reference.subscript.as_ref().and_then(Subscript::selector),
        Some(SubscriptSelector::At)
    );

    let reference = expect_array_indices_part(&command.args[3].parts[0].kind);
    assert_eq!(
        reference.subscript.as_ref().and_then(Subscript::selector),
        Some(SubscriptSelector::Star)
    );

    let (reference, _, _) = expect_array_slice_part(&command.args[4].parts[0].kind);
    assert_eq!(
        reference.subscript.as_ref().and_then(Subscript::selector),
        Some(SubscriptSelector::At)
    );
}

#[test]
fn test_braced_special_parameters_parse_as_parameter_accesses() {
    let input = "echo ${#} ${$}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);

    let hash = expect_array_access(&command.args[0]);
    assert_eq!(hash.name.as_str(), "#");
    assert_eq!(hash.name_span.slice(input), "#");

    let pid = expect_array_access(&command.args[1]);
    assert_eq!(pid.name.as_str(), "$");
    assert_eq!(pid.name_span.slice(input), "$");
}

#[test]
fn test_indirect_expansions_preserve_reference_structure() {
    let input = "echo ${!tools[$target]} ${!var//$'\\n'/' '}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);

    let (tools, operator, operand, colon_variant) =
        expect_indirect_expansion_part(&command.args[0].parts[0].kind);
    assert_eq!(tools.name.as_str(), "tools");
    assert!(!colon_variant);
    assert!(operator.is_none());
    assert!(operand.is_none());
    let subscript = expect_subscript(tools, input, "$target");
    assert_eq!(subscript.syntax_text(input), "$target");

    let (var, operator, operand, colon_variant) =
        expect_indirect_expansion_part(&command.args[1].parts[0].kind);
    assert_eq!(var.name.as_str(), "var");
    assert!(!colon_variant);
    assert!(operand.is_none());
    match operator {
        Some(ParameterOp::ReplaceAll {
            pattern,
            replacement,
            replacement_word_ast,
        }) => {
            assert_eq!(pattern.render(input), "\n");
            assert_eq!(replacement.slice(input), "' '");
            assert_eq!(replacement_word_ast.render_syntax(input), "' '");
        }
        other => panic!("expected replace-all indirect expansion, got {other:?}"),
    }
}

#[test]
fn test_parse_word_fragment_rebases_indirect_operator_spans() {
    let source = "echo ${!var//$'\\n'/' '}";
    let start = Position::new().advanced_by("echo ");
    let span = Span::from_positions(start, start.advanced_by("${!var//$'\\n'/' '}"));

    let word = Parser::parse_word_fragment(source, span.slice(source), span);
    let parameter = expect_parameter(&word);

    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indirect {
        operator:
            Some(ParameterOp::ReplaceAll {
                replacement,
                replacement_word_ast,
                ..
            }),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected indirect replacement operator");
    };

    assert_eq!(replacement.slice(source), "' '");
    assert_eq!(replacement_word_ast.render_syntax(source), "' '");
    assert_eq!(replacement_word_ast.span.slice(source), "' '");
}

#[test]
fn test_non_zsh_dialect_parses_zsh_modifier_forms_as_zsh_parameters() {
    let input = "print ${(%):-%x} ${(f)mapfile[$WD_CONFIG]//$HOME/~}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);

    let first = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(first.target, ZshExpansionTarget::Empty));
    assert!(matches!(
        first.operation,
        Some(ZshExpansionOperation::Defaulting {
            kind: ZshDefaultingOp::UseDefault,
            ref operand,
            colon_variant: true,
            ..
        }) if operand.slice(input) == "%x"
    ));
    let first_operation = first.operation.as_ref().expect("expected zsh operation");
    assert_eq!(
        first_operation
            .operand_word_ast()
            .expect("expected defaulting operand word")
            .render(input),
        "%x"
    );

    let second = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &second.target else {
        panic!("expected zsh reference target");
    };
    assert_eq!(reference.name.as_str(), "mapfile");
    let subscript = expect_subscript(reference, input, "$WD_CONFIG");
    assert_eq!(subscript.syntax_text(input), "$WD_CONFIG");
    assert!(matches!(
        second.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceAll,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(input) == "$HOME" && replacement.slice(input) == "~"
    ));
    let second_operation = second.operation.as_ref().expect("expected zsh operation");
    assert_eq!(
        second_operation
            .pattern_word_ast()
            .expect("expected replacement pattern word")
            .render(input),
        "$HOME"
    );
    assert_eq!(
        second_operation
            .replacement_word_ast()
            .expect("expected replacement word")
            .render(input),
        "~"
    );
}

#[test]
fn test_non_zsh_dialect_treats_dot_prefixed_parameter_forms_as_non_references() {
    let input = "print ${.sh.file}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh-style fallback parameter syntax");
    };
    let ZshExpansionTarget::Word(word) = &parameter.target else {
        panic!("expected non-reference word target");
    };
    assert_eq!(word.render(input), ".sh.file");
    assert!(parameter.operation.is_none());
}

#[test]
fn test_compound_array_assignment_preserves_mixed_element_kinds() {
    let input = "arr=(one [two]=2 [three]+=3 four)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let assignment = &command.assignments[0];
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };

    assert_eq!(array.kind, ArrayKind::Contextual);
    assert_eq!(array.elements.len(), 4);

    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected first sequential element");
    };
    assert_eq!(first.span.slice(input), "one");

    let ArrayElem::Keyed { key, value } = &array.elements[1] else {
        panic!("expected keyed element");
    };
    assert_eq!(key.text.slice(input), "two");
    assert_eq!(key.interpretation, SubscriptInterpretation::Contextual);
    assert_eq!(value.span.slice(input), "2");

    let ArrayElem::KeyedAppend { key, value } = &array.elements[2] else {
        panic!("expected keyed append element");
    };
    assert_eq!(key.text.slice(input), "three");
    assert_eq!(key.interpretation, SubscriptInterpretation::Contextual);
    assert_eq!(value.span.slice(input), "3");

    let ArrayElem::Sequential(last) = &array.elements[3] else {
        panic!("expected trailing sequential element");
    };
    assert_eq!(last.span.slice(input), "four");
}

#[test]
fn test_assignment_append_and_keyed_append_stay_distinct() {
    let input = "arr+=one\nassoc=(one [key]+=value)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(first) = &script.body[0].command else {
        panic!("expected first simple command");
    };
    assert!(first.assignments[0].append);

    let AstCommand::Simple(second) = &script.body[1].command else {
        panic!("expected second simple command");
    };
    assert!(!second.assignments[0].append);
    let AssignmentValue::Compound(array) = &second.assignments[0].value else {
        panic!("expected compound assignment");
    };
    assert!(matches!(array.elements[1], ArrayElem::KeyedAppend { .. }));
}

#[test]
fn test_assignment_target_mixed_subscript_and_compound_value_stay_structured() {
    let input = "assoc[\"$key\"-suffix]=(\"$value\" plain)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let assignment = &command.assignments[0];
    assert_eq!(assignment.target.name.as_str(), "assoc");
    let subscript = assignment
        .target
        .subscript
        .as_ref()
        .expect("expected target subscript");
    assert_eq!(subscript.text.slice(input), "\"$key\"-suffix");

    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(array.elements.len(), 2);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected first sequential element");
    };
    assert_eq!(first.span.slice(input), "\"$value\"");
    let ArrayElem::Sequential(second) = &array.elements[1] else {
        panic!("expected second sequential element");
    };
    assert_eq!(second.span.slice(input), "plain");
}

#[test]
fn test_compound_array_value_words_track_top_level_unquoted_commas() {
    let input = "\
arr=(
  alpha,beta
  head,$tail
  [k]=v,
  \"alpha,beta\"
  $'alpha,beta'
  $(printf %s 1,2)
  <(printf %s 1,2)
  >(printf %s 3,4)
  ${x/a,b/c}
  ${x/`echo }`/a,b}
  ${x/<(echo })/foo,bar}
  $((1,2))
  foo,{x,y},bar
)
";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound array assignment");
    };

    assert_eq!(array.elements.len(), 13, "{:#?}", array.elements);

    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected first sequential element");
    };
    assert!(first.has_top_level_unquoted_comma());

    let ArrayElem::Sequential(second) = &array.elements[1] else {
        panic!("expected second sequential element");
    };
    assert!(second.has_top_level_unquoted_comma());

    let ArrayElem::Keyed { value, .. } = &array.elements[2] else {
        panic!("expected keyed element");
    };
    assert!(value.has_top_level_unquoted_comma());

    for (index, expected_span) in [
        (3usize, "\"alpha,beta\""),
        (4, "$'alpha,beta'"),
        (5, "$(printf %s 1,2)"),
        (6, "<(printf %s 1,2)"),
        (7, ">(printf %s 3,4)"),
        (8, "${x/a,b/c}"),
        (9, "${x/`echo }`/a,b}"),
        (10, "${x/<(echo })/foo,bar}"),
        (11, "$((1,2))"),
    ] {
        let ArrayElem::Sequential(value) = &array.elements[index] else {
            panic!("expected sequential element at index {index}");
        };
        assert_eq!(value.span.slice(input), expected_span);
        assert!(
            !value.has_top_level_unquoted_comma(),
            "unexpected comma flag for {}",
            value.span.slice(input)
        );
    }

    let ArrayElem::Sequential(last) = &array.elements[12] else {
        panic!("expected trailing sequential element");
    };
    assert_eq!(last.span.slice(input), "foo,{x,y},bar");
    assert!(last.has_top_level_unquoted_comma());
}

#[test]
fn test_compound_array_process_substitution_stays_typed_for_comma_detection() {
    let input = "arr=(<(printf %s 1,2) >(printf %s 3,4))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound array assignment");
    };

    for (index, is_input) in [(0usize, true), (1usize, false)] {
        let ArrayElem::Sequential(value) = &array.elements[index] else {
            panic!("expected sequential element at index {index}");
        };
        assert!(!value.has_top_level_unquoted_comma());
        assert!(matches!(
            &value.parts[0].kind,
            WordPart::ProcessSubstitution { is_input: actual, .. } if *actual == is_input
        ));
    }
}

#[test]
fn test_word_part_spans_track_mixed_expansions() {
    let input = "echo pre${name:-fallback}$(printf hi)$((1+2))post\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let slices = top_level_part_slices(word, input);

    assert_eq!(
        slices,
        vec![
            "pre",
            "${name:-fallback}",
            "$(printf hi)",
            "$((1+2))",
            "post"
        ]
    );
}

#[test]
fn test_word_part_spans_track_quoted_expansions() {
    let input = "echo \"x$HOME$(pwd)y\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(
        top_level_part_slices(word, input),
        vec!["\"x$HOME$(pwd)y\""]
    );
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
    assert_eq!(slices, vec!["x", "$HOME", "$(pwd)", "y"]);
}

#[test]
fn test_mixed_segment_word_preserves_expansion_boundaries() {
    let input = "echo foo\"$bar\"baz\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let slices = top_level_part_slices(word, input);

    assert_eq!(slices, vec!["foo", "\"$bar\"", "baz"]);
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[1].kind else {
        panic!("expected quoted middle segment");
    };
    assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
}

#[test]
fn test_escaped_quote_literal_does_not_truncate_following_variable_name() {
    let input = "echo \\\"$archname\\\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(
        top_level_part_slices(word, input),
        vec!["\\\"", "$archname", "\\\""]
    );
    assert!(word_part_tree_contains_variable(&word.parts, "archname"));
    assert!(!word_part_tree_contains_variable(&word.parts, "archnam"));
}

#[test]
fn test_assignment_value_preserves_mixed_quoted_boundaries() {
    let input = "foo=\"$bar\"baz echo\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let slices = top_level_part_slices(word, input);

    assert!(!is_fully_quoted(word));
    assert_eq!(slices, vec!["\"$bar\"", "baz"]);
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected quoted prefix");
    };
    assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
}

#[test]
fn test_assignment_value_stays_quoted_when_entire_value_is_quoted() {
    let input = "foo=\"$bar\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let slices = top_level_part_slices(word, input);

    assert!(is_fully_quoted(word));
    assert_eq!(slices, vec!["\"$bar\""]);
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected fully quoted value");
    };
    assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
}

#[test]
fn test_backtick_command_substitution_inside_double_quotes_preserves_syntax_form() {
    let input = "echo \"pre `printf hi` post\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(is_fully_quoted(word));
    assert_eq!(
        top_level_part_slices(word, input),
        vec!["\"pre `printf hi` post\""]
    );

    let WordPart::DoubleQuoted { parts, dollar } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert!(!dollar);

    let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
    assert_eq!(slices, vec!["pre ", "`printf hi`", " post"]);

    let WordPart::CommandSubstitution {
        body: commands,
        syntax,
    } = &parts[1].kind
    else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::Backtick);

    let inner = expect_simple(&commands[0]);
    assert_eq!(inner.name.render(input), "printf");
    assert_eq!(inner.args[0].render(input), "hi");
}

#[test]
fn test_dollar_paren_command_substitution_inside_double_quotes_preserves_nested_quoted_argument() {
    let input = "echo \"$(cmd \"$arg\")\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(is_fully_quoted(word));
    assert_eq!(
        top_level_part_slices(word, input),
        vec!["\"$(cmd \"$arg\")\""]
    );

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert_eq!(
        parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect::<Vec<_>>(),
        vec!["$(cmd \"$arg\")"]
    );

    let WordPart::CommandSubstitution { body, syntax } = &parts[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);

    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "cmd");
    assert_eq!(inner.args[0].render_syntax(input), "\"$arg\"");
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
fn test_process_substitution_like_regex_inside_nested_command_substitution_stays_literal() {
    let input = "value=$(printf '%s\\n' \"<record_id>([^<]*)</record_id>\")\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(value) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::CommandSubstitution { body, .. } = &value.parts[0].kind else {
        panic!("expected command substitution");
    };

    let inner = expect_simple(&body[0]);
    for word in &inner.args {
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
fn test_process_substitution_like_regex_inside_nested_pipeline_command_substitution_stays_literal()
{
    let input = "_record_id=$(echo \"$response\" | _egrep_o \"<record_id>([^<]*)</record_id><type>TXT</type><host>$fulldomain</host>\" | _egrep_o \"<record_id>([^<]*)</record_id>\" | sed -r \"s/<record_id>([^<]*)<\\/record_id>/\\1/\" | tail -n 1)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(value) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::CommandSubstitution { body, .. } = &value.parts[0].kind else {
        panic!("expected command substitution");
    };

    fn word_has_process_substitution(word: &Word) -> bool {
        word.parts.iter().any(|part| match &part.kind {
            WordPart::ProcessSubstitution { .. } => true,
            WordPart::DoubleQuoted { parts, .. } => parts
                .iter()
                .any(|part| matches!(part.kind, WordPart::ProcessSubstitution { .. })),
            _ => false,
        })
    }

    fn stmt_has_process_substitution(stmt: &Stmt) -> bool {
        command_has_process_substitution(&stmt.command)
    }

    fn command_has_process_substitution(command: &AstCommand) -> bool {
        match command {
            AstCommand::Simple(command) => command.args.iter().any(word_has_process_substitution),
            AstCommand::Binary(binary)
                if matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll) =>
            {
                stmt_has_process_substitution(&binary.left)
                    || stmt_has_process_substitution(&binary.right)
            }
            _ => false,
        }
    }

    for stmt in body.iter() {
        assert!(!stmt_has_process_substitution(stmt), "{:#?}", stmt);
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
fn test_compound_array_keeps_quoted_pipelined_heredoc_substitution_as_one_element() {
    let input = r#"# shellcheck shell=bash
project=owner/repo
graphql_request=(
  -X POST
  -d "$(
    cat <<-EOF | tr '\n' ' '
      {
        "query": "query {
          repository(owner: \"${project%/*}\", name: \"${project##*/}\") {
            refs(refPrefix: \"refs/tags/\")
          }
        }"
      }
EOF
  )"
)
"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[1].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment");
    };

    assert_eq!(array.elements.len(), 4);
    let rendered = array
        .elements
        .iter()
        .map(|element| match element {
            ArrayElem::Sequential(word) => word.span.slice(input).to_owned(),
            _ => panic!("expected sequential array element"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rendered,
        vec![
            "-X",
            "POST",
            "-d",
            "\"$(\n    cat <<-EOF | tr '\\n' ' '\n      {\n        \"query\": \"query {\n          repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\") {\n            refs(refPrefix: \\\"refs/tags/\\\")\n          }\n        }\"\n      }\nEOF\n  )\"",
        ]
    );
}

#[test]
fn test_brace_syntax_marks_unquoted_expansion_candidates() {
    let list_input = "{a,b}";
    let list = Parser::parse_word_string(list_input);
    assert_eq!(brace_slices(&list, list_input), vec!["{a,b}"]);
    assert_eq!(
        list.brace_syntax(),
        &[BraceSyntax {
            kind: BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList),
            span: list.span,
            quote_context: BraceQuoteContext::Unquoted,
        }]
    );
    assert!(list.has_active_brace_expansion());

    let sequence_input = "{1..3}";
    let sequence = Parser::parse_word_string(sequence_input);
    assert_eq!(brace_slices(&sequence, sequence_input), vec!["{1..3}"]);
    assert_eq!(
        sequence.brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::Sequence)
    );
    assert!(sequence.brace_syntax()[0].expands());
}

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
fn test_brace_syntax_preserves_brace_expansion_suffix_forms() {
    for input in [
        "{a,b}}",
        "{~,~root}/pwd",
        "\"\"{~,~root}/pwd",
        "\\{~,~root}/pwd",
    ] {
        let word = Parser::parse_word_string(input);
        assert_eq!(word.render_syntax(input), input);
        if input == "\\{~,~root}/pwd" {
            assert_eq!(brace_slices(&word, input), Vec::<&str>::new());
        } else {
            let expected = if input == "{a,b}}" {
                "{a,b}"
            } else {
                "{~,~root}"
            };
            assert_eq!(brace_slices(&word, input), vec![expected]);
        }
    }
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
fn test_brace_syntax_ignores_escaped_unquoted_braces() {
    let word = Parser::parse_word_string("\\{a,b\\}");
    assert!(word.brace_syntax().is_empty());
    assert!(!word.has_active_brace_expansion());
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
fn test_word_part_spans_track_nested_array_expansions() {
    let input = "echo ${arr[$RANDOM % ${#arr[@]}]}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.parts.len(), 1);
    assert_eq!(
        word.part_span(0).unwrap().slice(input),
        "${arr[$RANDOM % ${#arr[@]}]}"
    );

    let reference = array_access_reference(&word.parts[0].kind).expect("expected array access");
    let subscript = reference.subscript.as_ref().expect("expected subscript");
    assert!(subscript.is_source_backed());
    assert_eq!(subscript.text.slice(input), "$RANDOM % ${#arr[@]}");
}

#[test]
fn test_word_part_spans_track_parenthesized_arithmetic_expansion() {
    let input = "echo $((a <= (1 || 2)))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.parts.len(), 1);
    assert_eq!(
        word.part_span(0).unwrap().slice(input),
        "$((a <= (1 || 2)))"
    );

    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &word.parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "a <= (1 || 2)");
    let expr = expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::LessThanOrEqual);
    expect_variable(left, "a");
    let ArithmeticExpr::Parenthesized { expression } = &right.kind else {
        panic!("expected parenthesized right operand");
    };
    let ArithmeticExpr::Binary {
        left: inner_left,
        op: inner_op,
        right: inner_right,
    } = &expression.kind
    else {
        panic!("expected logical-or inside parentheses");
    };
    assert_eq!(*inner_op, ArithmeticBinaryOp::LogicalOr);
    expect_number(inner_left, input, "1");
    expect_number(inner_right, input, "2");
}

#[test]
fn test_word_part_spans_track_nested_arithmetic_expansion() {
    let input = "echo $(((a) + ((b))))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.parts.len(), 1);
    assert_eq!(word.part_span(0).unwrap().slice(input), "$(((a) + ((b))))");

    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &word.parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "(a) + ((b))");
    let expr = expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    assert!(matches!(left.kind, ArithmeticExpr::Parenthesized { .. }));
    assert!(matches!(right.kind, ArithmeticExpr::Parenthesized { .. }));
}

#[test]
fn test_arithmetic_expansion_inside_double_quotes_preserves_legacy_and_modern_syntax() {
    let input = "echo \"$((1 + 2))\" \"$[3 + 4]\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 2);

    let modern = &command.args[0];
    assert!(is_fully_quoted(modern));
    let WordPart::DoubleQuoted { parts, dollar } = &modern.parts[0].kind else {
        panic!("expected double-quoted modern arithmetic");
    };
    assert!(!dollar);
    assert_eq!(parts[0].span.slice(input), "$((1 + 2))");
    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "1 + 2");
    let expr = expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_number(left, input, "1");
    expect_number(right, input, "2");

    let legacy = &command.args[1];
    assert!(is_fully_quoted(legacy));
    let WordPart::DoubleQuoted { parts, dollar } = &legacy.parts[0].kind else {
        panic!("expected double-quoted legacy arithmetic");
    };
    assert!(!dollar);
    assert_eq!(parts[0].span.slice(input), "$[3 + 4]");
    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::LegacyBracket);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "3 + 4");
    let expr = expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_number(left, input, "3");
    expect_number(right, input, "4");
}

#[test]
fn test_parameter_expansion_operand_stays_source_backed() {
    let input = "echo ${var:-$(pwd)}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, _, operand) = expect_parameter_operation_part(&word.parts[0].kind);
    let operand = operand.expect("expected operand");
    assert!(operand.is_source_backed());
    assert_eq!(operand.slice(input), "$(pwd)");

    let parameter = expect_parameter(word);
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        operand_word_ast: Some(operand_word_ast),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected unified bourne operation");
    };
    assert_eq!(operand_word_ast.render(input), "$(pwd)");
    assert_eq!(operand_word_ast.span.slice(input), "$(pwd)");
}

#[test]
fn test_array_target_parameter_operations_normalize_to_bourne_operations() {
    let input = "echo ${arr[0]//x/y} ${arr[@],,} ${arr[1]^^pattern}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let (replace_reference, replace_operator, _) =
        expect_parameter_operation_part(&command.args[0].parts[0].kind);
    expect_subscript(replace_reference, input, "0");
    assert!(matches!(replace_operator, ParameterOp::ReplaceAll { .. }));

    let (lower_reference, lower_operator, lower_operand) =
        expect_parameter_operation_part(&command.args[1].parts[0].kind);
    expect_subscript(lower_reference, input, "@");
    assert!(matches!(lower_operator, ParameterOp::LowerAll));
    assert!(lower_operand.is_none());

    let (upper_reference, upper_operator, upper_operand) =
        expect_parameter_operation_part(&command.args[2].parts[0].kind);
    expect_subscript(upper_reference, input, "1");
    assert!(matches!(upper_operator, ParameterOp::UpperAll));
    assert_eq!(
        upper_operand
            .expect("expected case-modification operand")
            .slice(input),
        "pattern"
    );
}

#[test]
fn test_case_modification_operands_consume_nested_parameter_expansions() {
    let input = "echo ${name^^${pat}} ${arr[1],,${pat}}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let (_, upper_operator, upper_operand) =
        expect_parameter_operation_part(&command.args[0].parts[0].kind);
    assert!(matches!(upper_operator, ParameterOp::UpperAll));
    assert_eq!(
        upper_operand
            .expect("expected upper case-modification operand")
            .slice(input),
        "${pat}"
    );

    let (lower_reference, lower_operator, lower_operand) =
        expect_parameter_operation_part(&command.args[1].parts[0].kind);
    expect_subscript(lower_reference, input, "1");
    assert!(matches!(lower_operator, ParameterOp::LowerAll));
    assert_eq!(
        lower_operand
            .expect("expected lower case-modification operand")
            .slice(input),
        "${pat}"
    );
}

#[test]
fn test_parameter_expansion_trim_operand_accepts_literal_left_brace_after_multiline_quote() {
    let input = "dns_servercow_info='ServerCow.de\nSite: ServerCow.de\n'\n\nf(){\n  if true; then\n    txtvalue_old=${response#*{\\\"name\\\":\\\"\"$_sub_domain\"\\\",\\\"ttl\\\":20,\\\"type\\\":\\\"TXT\\\",\\\"content\\\":\\\"}\n  fi\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[1].command else {
        panic!("expected function definition");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let (if_compound, redirects) = expect_compound(&body[0]);
    let AstCompoundCommand::If(if_command) = if_compound else {
        panic!("expected if command");
    };
    assert!(redirects.is_empty());
    let command = expect_simple(&if_command.then_branch[0]);
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::RemovePrefixShort { pattern } = operator else {
        panic!("expected short-prefix trim operator");
    };
    assert!(pattern.render(input).contains("$_sub_domain"));
    assert!(pattern.parts.iter().any(|part| {
        matches!(
            &part.kind,
            PatternPart::Word(word)
                if word_part_tree_contains_variable(&word.parts, "_sub_domain")
        )
    }));
}

#[test]
fn test_parameter_expansion_trim_operand_accepts_balanced_literal_braces() {
    let input = "echo ${var#foo{bar}}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::RemovePrefixShort { pattern } = operator else {
        panic!("expected short-prefix trim operator");
    };
    assert_eq!(pattern.render(input), "foo{bar}");
}

#[test]
fn test_parameter_expansion_trim_operand_tracks_nested_parameter_expansions() {
    let input = "echo ${var#${prefix:-fallback}}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::RemovePrefixShort { pattern } = operator else {
        panic!("expected short-prefix trim operator");
    };
    assert_eq!(pattern.render(input), "${prefix:-fallback}");
    assert!(matches!(
        &pattern.parts[..],
        [PatternPartNode {
            kind: PatternPart::Word(word),
            ..
        }] if matches!(
            &word.parts[..],
            [WordPartNode {
                kind: WordPart::Parameter(_) | WordPart::ParameterExpansion { .. },
                ..
            }]
        )
    ));
}

#[test]
fn test_parameter_replacement_pattern_stays_source_backed() {
    let input = "echo ${var/foo/bar}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceFirst {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-first operator");
    };

    assert_eq!(pattern.render(input), "foo");
    assert_eq!(pattern.parts.len(), 1);
    assert!(matches!(
        &pattern.parts[0].kind,
        PatternPart::Literal(text) if text.is_source_backed()
    ));
    assert!(replacement.is_source_backed());
    assert_eq!(replacement.slice(input), "bar");
    assert_eq!(replacement_word_ast.render(input), "bar");
    assert_eq!(replacement_word_ast.span.slice(input), "bar");
}

#[test]
fn test_parameter_trim_pattern_preserves_quoted_fragments_around_expansions() {
    let input = "echo ${var#\"pre\"$suffix'-'*}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::RemovePrefixShort { pattern } = operator else {
        panic!("expected short-prefix trim operator");
    };

    assert!(matches!(
        &pattern.parts[..],
        [
            PatternPartNode {
                kind: PatternPart::Word(first),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(second),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(third),
                ..
            },
            PatternPartNode {
                kind: PatternPart::AnyString,
                ..
            }
        ] if first.is_fully_quoted()
            && matches!(
                &second.parts[..],
                [WordPartNode {
                    kind: WordPart::Variable(name),
                    ..
                }] if name.as_str() == "suffix"
            )
            && third.is_fully_quoted()
    ));
}

#[test]
fn test_parameter_replacement_pattern_preserves_mixed_quote_fragments() {
    let input = "echo ${var//\"pre\"$suffix'-'/x}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceAll {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-all operator");
    };

    assert_eq!(
        pattern_part_slices(pattern, input),
        vec!["\"pre\"", "$suffix", "'-'"]
    );
    assert_eq!(replacement.slice(input), "x");
    assert_eq!(replacement_word_ast.render(input), "x");
    assert!(matches!(
        &pattern.parts[..],
        [
            PatternPartNode {
                kind: PatternPart::Word(first),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(second),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(third),
                ..
            }
        ] if first.is_fully_quoted()
            && matches!(
                &second.parts[..],
                [WordPartNode {
                    kind: WordPart::Variable(name),
                    ..
                }] if name.as_str() == "suffix"
            )
            && third.is_fully_quoted()
    ));
}

#[test]
fn test_parameter_replacement_pattern_cooks_escaped_slash() {
    let input = r#"echo ${var/foo\/bar/baz}"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceFirst {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-first operator");
    };

    assert_eq!(pattern.render(input), "foo/bar");
    assert_eq!(pattern.parts.len(), 1);
    assert!(matches!(
        &pattern.parts[0].kind,
        PatternPart::Literal(text) if !text.is_source_backed() && text == "foo/bar"
    ));
    assert!(replacement.is_source_backed());
    assert_eq!(replacement_word_ast.render(input), "baz");
    assert_eq!(replacement.slice(input), "baz");
}

#[test]
fn test_parameter_replacement_word_keeps_escaped_single_quotes_literal() {
    let input = r#"echo ${dest_dir//\'/\'\\\'\'}"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceAll {
        replacement: _,
        replacement_word_ast,
        ..
    } = operator
    else {
        panic!("expected replace-all operator");
    };

    assert!(!replacement_word_ast.parts.iter().any(|part| {
        matches!(part.kind, WordPart::SingleQuoted { .. })
            && part.span.slice(input).ends_with("\\'")
    }));
}

#[test]
fn test_parameter_replacement_spans_cover_complex_pattern_and_replacement_bodies() {
    let input = "\
echo ${dest_dir//\\'/\\'\\\\\\'\\'} ${TERMUX_PKG_VERSION_EDITED//${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}/${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}} ${GITHUB_GRAPHQL_QUERIES[$BATCH * $BATCH_SIZE]//\\\\/} ${run_depends/${i}/${dep}}\n\
";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(
        command.args[0]
            .parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect::<Vec<_>>(),
        vec![r#"${dest_dir//\'/\'\\\'\'}"#]
    );

    let spans = command
        .args
        .iter()
        .map(|word| word.parts[0].span.slice(input))
        .collect::<Vec<_>>();
    assert_eq!(
        spans,
        vec![
            "${dest_dir//\\'/\\'\\\\\\'\\'}",
            "${TERMUX_PKG_VERSION_EDITED//${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}/${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}}",
            "${GITHUB_GRAPHQL_QUERIES[$BATCH * $BATCH_SIZE]//\\\\/}",
            "${run_depends/${i}/${dep}}",
        ]
    );

    let (_, operator, _) = expect_parameter_operation_part(&command.args[1].parts[0].kind);
    let ParameterOp::ReplaceAll {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-all operator");
    };
    assert_eq!(
        pattern.render(input),
        "${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}"
    );
    assert_eq!(
        replacement.slice(input),
        "${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}"
    );
    assert_eq!(
        replacement_word_ast.render(input),
        "${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}"
    );

    let (_, operator, _) = expect_parameter_operation_part(&command.args[3].parts[0].kind);
    let ParameterOp::ReplaceFirst {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-first operator");
    };
    assert_eq!(pattern.render(input), "${i}");
    assert_eq!(replacement.slice(input), "${dep}");
    assert_eq!(replacement_word_ast.render(input), "${dep}");
}

#[test]
fn test_decode_cooked_word_keeps_variable_after_literal_backslash() {
    let cooked = r#"\$HOME"#;
    let span = Span::from_positions(Position::new(), Position::new().advanced_by(cooked));
    let word = Parser::new("").decode_word_text(cooked, span, span.start, false);

    assert_eq!(word.parts.len(), 2);
    let WordPart::Literal(text) = &word.parts[0].kind else {
        panic!("expected literal backslash prefix");
    };
    assert_eq!(text.as_str("", word.parts[0].span), "\\");
    assert!(matches!(
        &word.parts[1].kind,
        WordPart::Variable(name) if name.as_str() == "HOME"
    ));
}

#[test]
fn test_parse_arithmetic_command_with_command_substitution() {
    let input = "(($(date -u) > DATE))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(command.expr_span.unwrap().slice(input), "$(date -u) > DATE");
    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::GreaterThan);
    expect_shell_word(left, input, "$(date -u)");
    expect_variable(right, "DATE");
}

#[test]
fn test_parse_arithmetic_command_distinguishes_assignment_from_comparison() {
    let input = "(( a = b == c ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Assignment { target, op, value } = &expr.kind else {
        panic!("expected arithmetic assignment");
    };
    assert_eq!(*op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable assignment target");
    };
    assert_eq!(name, "a");

    let ArithmeticExpr::Binary {
        left,
        op: cmp_op,
        right,
    } = &value.kind
    else {
        panic!("expected comparison on assignment right-hand side");
    };
    assert_eq!(*cmp_op, ArithmeticBinaryOp::Equal);
    expect_variable(left, "b");
    expect_variable(right, "c");
}

#[test]
fn test_parse_arithmetic_command_accepts_command_substitutions_and_quoted_words() {
    let input = "(( \"$(date -u)\" + '3' ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    let ArithmeticExpr::ShellWord(left_word) = &left.kind else {
        panic!("expected quoted shell word on left");
    };
    assert_eq!(left_word.span.slice(input), "\"$(date -u)\"");
    let ArithmeticExpr::ShellWord(right_word) = &right.kind else {
        panic!("expected quoted shell word on right");
    };
    assert_eq!(right_word.span.slice(input), "'3'");
}

#[test]
fn test_parse_zsh_arithmetic_command_keeps_subscripted_shell_words_intact() {
    let input = "(( $+aliases[(e)$1] ))\n(( $cmdnames[(Ie)$point] ))\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (first, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(first) = first else {
        panic!("expected arithmetic compound command");
    };
    let first_expr = first.expr_ast.as_ref().expect("expected arithmetic AST");
    expect_shell_word(first_expr, input, "$+aliases[(e)$1]");

    let (second, _) = expect_compound(&script.body[1]);
    let AstCompoundCommand::Arithmetic(second) = second else {
        panic!("expected arithmetic compound command");
    };
    let second_expr = second.expr_ast.as_ref().expect("expected arithmetic AST");
    expect_shell_word(second_expr, input, "$cmdnames[(Ie)$point]");
}

#[test]
fn test_parse_zsh_arithmetic_command_supports_char_literal_numbers() {
    let input = "(( #c < 256 / $1 * $1 ))\n(( rnd = (~(1 << 23) & rnd) << 8 | #c ))\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (first, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(first) = first else {
        panic!("expected arithmetic compound command");
    };
    let first_expr = first.expr_ast.as_ref().expect("expected arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &first_expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::LessThan);
    expect_number(left, input, "#c");
    let ArithmeticExpr::Binary {
        left: mul_left,
        op: mul_op,
        right: mul_right,
    } = &right.kind
    else {
        panic!("expected multiplication on right-hand side");
    };
    assert_eq!(*mul_op, ArithmeticBinaryOp::Multiply);
    expect_shell_word(mul_right, input, "$1");
    let ArithmeticExpr::Binary {
        left: div_left,
        op: div_op,
        right: div_right,
    } = &mul_left.kind
    else {
        panic!("expected division on left-hand side");
    };
    assert_eq!(*div_op, ArithmeticBinaryOp::Divide);
    expect_number(div_left, input, "256");
    expect_shell_word(div_right, input, "$1");

    let (second, _) = expect_compound(&script.body[1]);
    let AstCompoundCommand::Arithmetic(second) = second else {
        panic!("expected arithmetic compound command");
    };
    let second_expr = second.expr_ast.as_ref().expect("expected arithmetic AST");
    let ArithmeticExpr::Assignment { target, op, value } = &second_expr.kind else {
        panic!("expected arithmetic assignment");
    };
    assert_eq!(*op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable assignment target");
    };
    assert_eq!(name, "rnd");
    let ArithmeticExpr::Binary {
        left: or_left,
        op: or_op,
        right: or_right,
    } = &value.kind
    else {
        panic!("expected bitwise or value");
    };
    assert_eq!(*or_op, ArithmeticBinaryOp::BitwiseOr);
    assert!(matches!(or_left.kind, ArithmeticExpr::Binary { .. }));
    expect_number(or_right, input, "#c");
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
fn test_parse_conditional_var_ref_operand_preserves_quoted_subscript_syntax() {
    let input = "[[ -v assoc[\"key\"] ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
        panic!("expected typed var-ref operand");
    };

    let subscript = expect_subscript_syntax(var_ref, input, "\"key\"", "key");
    assert!(matches!(subscript.kind, SubscriptKind::Ordinary));
}

#[test]
fn test_parse_conditional_var_ref_operand_preserves_spaced_zero_subscript() {
    let input = "[[ -v assoc[ 0 ] ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
        panic!("expected typed var-ref operand");
    };

    let subscript = expect_subscript(var_ref, input, " 0 ");
    assert!(matches!(
        subscript.arithmetic_ast.as_ref().map(|expr| &expr.kind),
        Some(ArithmeticExpr::Number(_))
    ));
}

#[test]
fn test_parse_conditional_var_ref_operand_preserves_nested_arithmetic_subscript() {
    let input = "[[ -v assoc[$((0))] ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
        panic!("expected typed var-ref operand");
    };

    let subscript = expect_subscript(var_ref, input, "$((0))");
    assert!(subscript.arithmetic_ast.is_some());
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

#[test]
fn test_parse_parameter_expansion_operands_allow_quoted_and_escaped_right_brace() {
    let input = r###"echo "${var#\}}"
echo "${var#'}'}"
echo "${var#"}"}"
echo "${var-\}}"
echo "${var-'}'}"
echo "${var-"}"}"
"###;

    let script = Parser::new(input).parse().unwrap().file;
    assert_eq!(script.body.len(), 6);
}

#[test]
fn test_parse_long_suffix_trim_operator_inside_double_quotes() {
    let input = "echo \"${1%%.*}\" \"${package_url%%#*}\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    for word in &command.args {
        let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
            panic!("expected double-quoted word");
        };
        let parameter = match &parts[0].kind {
            WordPart::Parameter(parameter) => parameter,
            _ => panic!("expected parameter expansion"),
        };
        let BourneParameterExpansion::Operation { operator, .. } =
            parameter.bourne().expect("expected Bourne syntax")
        else {
            panic!("expected parameter operation");
        };
        assert!(matches!(operator, ParameterOp::RemoveSuffixLong { .. }));
    }
}

#[test]
fn test_parse_parameter_slices_preserve_shell_style_offsets() {
    let input = "echo \"${arg:$index:1}\" \"${@:1:$package_type_nargs}\" \"${@:$(( $package_type_nargs + 1 ))}\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let WordPart::DoubleQuoted {
        parts: first_parts, ..
    } = &command.args[0].parts[0].kind
    else {
        panic!("expected first double-quoted word");
    };
    let (_, first_offset_ast, first_length_ast) = expect_substring_part(&first_parts[0].kind);
    let ArithmeticExpr::ShellWord(first_offset_word) =
        &first_offset_ast.as_ref().expect("expected offset AST").kind
    else {
        panic!("expected shell-word offset");
    };
    assert_eq!(first_offset_word.span.slice(input), "$index");
    assert_eq!(
        first_length_ast
            .as_ref()
            .expect("expected first length AST")
            .span
            .slice(input),
        "1"
    );

    let WordPart::DoubleQuoted {
        parts: second_parts,
        ..
    } = &command.args[1].parts[0].kind
    else {
        panic!("expected second double-quoted word");
    };
    let (_, second_offset_ast, second_length_ast) = expect_substring_part(&second_parts[0].kind);
    assert_eq!(
        second_offset_ast
            .as_ref()
            .expect("expected second offset AST")
            .span
            .slice(input),
        "1"
    );
    let ArithmeticExpr::ShellWord(second_length_word) = &second_length_ast
        .as_ref()
        .expect("expected second length AST")
        .kind
    else {
        panic!("expected shell-word length");
    };
    assert_eq!(second_length_word.span.slice(input), "$package_type_nargs");

    let WordPart::DoubleQuoted {
        parts: third_parts, ..
    } = &command.args[2].parts[0].kind
    else {
        panic!("expected third double-quoted word");
    };
    let (_, third_offset_ast, third_length_ast) = expect_substring_part(&third_parts[0].kind);
    assert!(third_length_ast.is_none());
    let ArithmeticExpr::ShellWord(third_offset_word) = &third_offset_ast
        .as_ref()
        .expect("expected third offset AST")
        .kind
    else {
        panic!("expected shell-word arithmetic expansion");
    };
    assert_eq!(
        third_offset_word.span.slice(input),
        "$(( $package_type_nargs + 1 ))"
    );
}

#[test]
fn test_command_substitution_spans_are_absolute() {
    let script = Parser::new("out=$(\n  printf '%s\\n' $x\n)\n")
        .parse()
        .unwrap()
        .file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::CommandSubstitution {
        body: commands,
        syntax,
    } = &word.parts[0].kind
    else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);
    let inner = expect_simple(&commands[0]);

    assert_eq!(inner.name.span.start.line, 2);
    assert_eq!(inner.name.span.start.column, 3);
    assert_eq!(inner.args[0].span.start.line, 2);
    assert_eq!(inner.args[1].span.start.column, 17);
}

#[test]
fn test_parse_command_substitution_with_open_paren_inside_double_quotes() {
    Parser::new("x=$(echo \"(\")\n").parse().unwrap();
}

#[test]
fn test_parse_command_substitution_with_case_pattern_right_paren() {
    let input = "echo $(foo=a; case $foo in [0-9]) echo number;; [a-z]) echo letter ;; esac)\n";
    Parser::new(input).parse().unwrap();
}

#[test]
fn test_process_substitution_spans_are_absolute() {
    let script = Parser::new("cat <(\n  printf '%s\\n' $x\n)\n")
        .parse()
        .unwrap()
        .file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let WordPart::ProcessSubstitution {
        body: commands,
        is_input,
    } = &command.args[0].parts[0].kind
    else {
        panic!("expected process substitution");
    };
    assert!(*is_input);

    let inner = expect_simple(&commands[0]);
    assert_eq!(inner.name.span.start.line, 2);
    assert_eq!(inner.name.span.start.column, 3);
    assert_eq!(inner.args[1].span.start.column, 17);
}

#[test]
fn test_parse_declare_clause_classifies_operands_and_prefix_assignments() {
    let input = "FOO=1 declare -a arr=(\"hello world\" two) bar >out\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    assert_eq!(command.variant, "declare");
    assert_eq!(command.variant_span.slice(input), "declare");
    assert_eq!(command.assignments.len(), 1);
    assert_eq!(command.assignments[0].target.name, "FOO");
    assert_eq!(script.body[0].redirects.len(), 1);
    assert_eq!(
        redirect_word_target(&script.body[0].redirects[0])
            .span
            .slice(input),
        "out"
    );
    assert_eq!(command.operands.len(), 3);

    let DeclOperand::Flag(flag) = &command.operands[0] else {
        panic!("expected flag operand");
    };
    assert_eq!(flag.span.slice(input), "-a");

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand");
    };
    assert_eq!(assignment.target.name, "arr");
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.kind, ArrayKind::Indexed);
    assert_eq!(array.elements.len(), 2);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected first sequential element");
    };
    assert!(is_fully_quoted(first));
    assert_eq!(first.span.slice(input), "\"hello world\"");
    let ArrayElem::Sequential(second) = &array.elements[1] else {
        panic!("expected second sequential element");
    };
    assert_eq!(second.span.slice(input), "two");

    let DeclOperand::Name(name) = &command.operands[2] else {
        panic!("expected bare name operand");
    };
    assert_eq!(name.name, "bar");
}

#[test]
fn test_parse_declare_a_threads_associative_kind_into_compound_array() {
    let input = "declare -A assoc=(one [foo]=bar [bar]+=baz two)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };

    assert_eq!(array.kind, ArrayKind::Associative);
    assert_eq!(array.elements.len(), 4);
    assert!(matches!(array.elements[0], ArrayElem::Sequential(_)));

    let ArrayElem::Keyed { key, .. } = &array.elements[1] else {
        panic!("expected keyed element");
    };
    assert_eq!(key.text.slice(input), "foo");
    assert_eq!(key.interpretation, SubscriptInterpretation::Associative);

    let ArrayElem::KeyedAppend { key, .. } = &array.elements[2] else {
        panic!("expected keyed append element");
    };
    assert_eq!(key.text.slice(input), "bar");
    assert_eq!(key.interpretation, SubscriptInterpretation::Associative);

    assert!(matches!(array.elements[3], ArrayElem::Sequential(_)));
}

#[test]
fn test_parse_declare_array_preserves_quoted_command_substitution_elements() {
    let input = "f() {\n\tlocal -a graphql_request=(\n\t\t-X POST\n\t\t-d \"$(\n\t\t\tcat <<-EOF | tr '\\n' ' '\n\t\t\t\t{\"query\":\"field, direction\"}\n\t\t\tEOF\n\t\t)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 4, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[3] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_separator_comment_in_quoted_command_substitution() {
    let input = "f() {\n\tlocal -a parts=(\n\t\t\"$(printf '%s' x;# comment with ) and ,\n\t\tprintf '%s' y\n\t\t)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_piped_heredoc_without_spacing_in_command_substitution() {
    let input = "f() {\n\tlocal -a graphql_request=(\n\t\t\"$(\ncat <<EOF|tr '\\n' ' '\n{\"query\":\"field, direction\"}\nEOF\n\t\t)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_parameter_expansion_with_right_paren_in_command_substitution()
{
    let input = "f() {\n\tlocal -a parts=(\n\t\t\"$(printf %s ${x//foo/)},1)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_plain_case_words_in_command_substitution() {
    let input = "f() {\n\tlocal -a parts=(\n\t\t$(printf %s 1,2; echo case in)\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| matches!(
            &part.kind,
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::DollarParen,
                ..
            }
        )),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_ansi_c_quotes_in_command_substitution() {
    let input = "f() {\n\tlocal -a parts=(\n\t\t$(printf %s $'a\\'b'; printf %s 1,2)\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| matches!(
            &part.kind,
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::DollarParen,
                ..
            }
        )),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_backticks_with_right_parens_in_command_substitution() {
    let input = "f() {\n\tlocal -a parts=(\n\t\t$(printf %s `echo foo)`; printf %s ok)\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| matches!(
            &part.kind,
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::DollarParen,
                ..
            }
        )),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_backticks_inside_parameter_expansions_in_command_substitution()
 {
    let input = "f() {\n\tlocal -a parts=(\n\t\t\"$(printf %s ${x/`echo }`/foo)},1)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_process_substitutions_inside_parameter_expansions_in_command_substitution()
 {
    let input = "f() {\n\tlocal -a parts=(\n\t\t\"$(printf %s ${x/<(echo })/foo)},1)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_parameter_expansion_preserves_quoted_associative_subscripts() {
    let input = "printf '%s\\n' ${assoc[\"key\"]} ${assoc['k']}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let first = expect_array_access(&command.args[1]);
    let second = expect_array_access(&command.args[2]);

    let first_subscript = expect_subscript_syntax(first, input, "\"key\"", "key");
    assert!(matches!(first_subscript.kind, SubscriptKind::Ordinary));
    assert_eq!(
        first_subscript
            .word_ast()
            .expect("expected subscript word AST")
            .span
            .slice(input),
        "\"key\""
    );
    assert_eq!(command.args[1].render_syntax(input), "${assoc[\"key\"]}");

    let second_subscript = expect_subscript_syntax(second, input, "'k'", "k");
    assert!(matches!(second_subscript.kind, SubscriptKind::Ordinary));
    assert_eq!(
        second_subscript
            .word_ast()
            .expect("expected subscript word AST")
            .render_syntax(input),
        "'k'"
    );
    assert_eq!(command.args[2].render_syntax(input), "${assoc['k']}");
}

#[test]
fn test_parse_declare_a_preserves_quoted_associative_keys() {
    let input = "declare -A assoc=([\"key\"]=bar ['alt']+=baz)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand");
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };

    let ArrayElem::Keyed { key, .. } = &array.elements[0] else {
        panic!("expected keyed element");
    };
    assert_eq!(key.text.slice(input), "key");
    assert_eq!(key.syntax_text(input), "\"key\"");

    let ArrayElem::KeyedAppend { key, .. } = &array.elements[1] else {
        panic!("expected keyed append element");
    };
    assert_eq!(key.text.slice(input), "alt");
    assert_eq!(key.syntax_text(input), "'alt'");
}

#[test]
fn test_parse_export_uses_dynamic_operand_for_invalid_assignment() {
    let script = Parser::new("export foo-bar=(one two)\n")
        .parse()
        .unwrap()
        .file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    assert_eq!(command.variant, "export");
    assert_eq!(command.operands.len(), 1);
    let DeclOperand::Dynamic(word) = &command.operands[0] else {
        panic!("expected dynamic operand");
    };
    assert_eq!(
        word.span.slice("export foo-bar=(one two)\n"),
        "foo-bar=(one two)"
    );
}

#[test]
fn test_parse_typeset_clause_classifies_flags_and_assignments() {
    let input = "typeset -xr VAR=value other\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    assert_eq!(command.variant, "typeset");
    assert_eq!(command.variant_span.slice(input), "typeset");
    assert_eq!(command.operands.len(), 3);

    let DeclOperand::Flag(flag) = &command.operands[0] else {
        panic!("expected flag operand");
    };
    assert_eq!(flag.span.slice(input), "-xr");

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand");
    };
    assert_eq!(assignment.target.name, "VAR");
    assert!(
        matches!(&assignment.value, AssignmentValue::Scalar(value) if value.span.slice(input) == "value")
    );

    let DeclOperand::Name(name) = &command.operands[2] else {
        panic!("expected bare name operand");
    };
    assert_eq!(name.name, "other");
}

#[test]
fn test_parse_declaration_name_operand_preserves_nested_arithmetic_subscript() {
    let input = "declare assoc[$((0))]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    let DeclOperand::Name(name) = &command.operands[0] else {
        panic!("expected declaration name operand");
    };
    let subscript = expect_subscript(name, input, "$((0))");
    assert!(subscript.arithmetic_ast.is_some());
}

#[test]
fn test_alias_expansion_can_form_a_for_loop_header() {
    let input = "\
shopt -s expand_aliases
alias FOR1='for '
alias FOR2='FOR1 '
alias eye1='i '
alias eye2='eye1 '
alias IN='in '
alias onetwo='1 2 '
FOR2 eye2 IN onetwo 3; do echo $i; done
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(command) = script.body.last() else {
        panic!("expected final command to be a for loop");
    };
    let (compound, _) = expect_compound(command);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected final command to be a for loop");
    };
    assert_eq!(command.targets[0].name.as_deref(), Some("i"));
    assert_eq!(command.words.as_ref().map(Vec::len), Some(3));
}

#[test]
fn test_alias_expansion_can_open_a_brace_group() {
    let input = "\
shopt -s expand_aliases
alias LEFT='{'
LEFT echo one; echo two; }
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(command) = script.body.last() else {
        panic!("expected final command to be a brace group");
    };
    let (compound, _) = expect_compound(command);
    let AstCompoundCommand::BraceGroup(commands) = compound else {
        panic!("expected final command to be a brace group");
    };
    assert_eq!(commands.len(), 2);
    assert!(matches!(commands[0].command, AstCommand::Simple(_)));
    assert!(matches!(commands[1].command, AstCommand::Simple(_)));
}

#[test]
fn test_alias_expansion_can_open_a_subshell() {
    let input = "\
shopt -s expand_aliases
alias LEFT='('
LEFT echo one; echo two )
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(command) = script.body.last() else {
        panic!("expected final command to be a subshell");
    };
    let (compound, _) = expect_compound(command);
    let AstCompoundCommand::Subshell(commands) = compound else {
        panic!("expected final command to be a subshell");
    };
    assert_eq!(commands.len(), 2);
    assert!(matches!(commands[0].command, AstCommand::Simple(_)));
    assert!(matches!(commands[1].command, AstCommand::Simple(_)));
}

#[test]
fn test_alias_expansion_with_trailing_space_expands_next_word() {
    let input = "\
shopt -s expand_aliases
alias greet='echo '
alias subject='hello'
greet subject
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(stmt) = script.body.last() else {
        panic!("expected final command to be a simple command");
    };
    let command = expect_simple(stmt);

    assert_eq!(command.name.render(input), "echo");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(input), "hello");

    let WordPart::Literal(name_text) = &command.name.parts[0].kind else {
        panic!("expected alias-expanded command name to stay literal");
    };
    let WordPart::Literal(arg_text) = &command.args[0].parts[0].kind else {
        panic!("expected alias-expanded arg to stay literal");
    };

    assert!(!name_text.is_source_backed());
    assert!(!arg_text.is_source_backed());
}

#[test]
fn test_alias_expansion_with_trailing_space_waits_until_replay_finishes() {
    let input = "\
shopt -s expand_aliases
alias e_='for i in 1 2 3; do echo '
e_ $i; done
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(stmt) = script.body.last() else {
        panic!("expected final command to be a for loop");
    };
    let (compound, _) = expect_compound(stmt);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected final command to be a for loop");
    };

    assert_eq!(command.targets[0].name.as_deref(), Some("i"));
    assert_eq!(command.words.as_ref().map(Vec::len), Some(3));

    let Some(body_stmt) = command.body.first() else {
        panic!("expected loop body command");
    };
    let body_command = expect_simple(body_stmt);
    assert_eq!(body_command.name.render(input), "echo");
    assert_eq!(body_command.args.len(), 1);
    assert_eq!(body_command.args[0].render(input), "$i");
}

#[test]
fn test_alias_expansion_recursive_guard_stops_self_reference() {
    let input = "\
shopt -s expand_aliases
alias loop='loop echo'
loop
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(stmt) = script.body.last() else {
        panic!("expected final command to be a simple command");
    };
    let command = expect_simple(stmt);

    assert_eq!(command.name.render(input), "loop");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(input), "echo");
}

#[test]
fn test_comment_ranges_simple() {
    let source = "# head\necho hi # inline\n# tail\n";
    let output = Parser::new(source).parse().unwrap();
    assert_eq!(collect_file_comments(&output.file).len(), 3);
    assert_comment_ranges_valid(source, &output);
}

#[test]
fn test_comment_ranges_with_unicode() {
    let source = "# café résumé\necho ok\n# 你好世界\n";
    let output = Parser::new(source).parse().unwrap();
    assert_eq!(collect_file_comments(&output.file).len(), 2);
    assert_comment_ranges_valid(source, &output);
}

#[test]
fn test_if_condition_semicolon_probe_does_not_duplicate_comments() {
    let source = "\
if foo; # keep this once
bar; then
  baz
fi
";
    let output = Parser::new(source).parse().unwrap();
    let comments = collect_file_comments(&output.file);
    let texts: Vec<&str> = comments
        .iter()
        .map(|comment| {
            let start = usize::from(comment.range.start());
            let end = usize::from(comment.range.end());
            &source[start..end]
        })
        .collect();

    assert_eq!(texts, vec!["# keep this once"]);

    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };
    assert_eq!(command.condition.len(), 2);
}

#[test]
fn test_zsh_dialect_accepts_c_style_for_loops() {
    Parser::with_dialect(
        "for ((i=0; i<2; i++)); do echo hi; done\n",
        ShellDialect::Zsh,
    )
    .parse()
    .unwrap();
}

#[test]
fn test_for_loop_preserves_single_target_and_in_do_done_syntax() {
    let source = "for item in a b; do echo \"$item\"; done\n";
    let output = Parser::new(source).parse().unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .targets
            .iter()
            .map(|target| target.name.as_deref().expect("expected normalized target"))
            .collect::<Vec<_>>(),
        vec!["item"]
    );
    assert_eq!(command.targets[0].word.render(source), "item");
    assert_eq!(command.targets[0].span.slice(source), "item");
    assert_eq!(
        command
            .words
            .as_ref()
            .expect("expected explicit word list")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    match command.syntax {
        ForSyntax::InDoDone {
            in_span: Some(in_span),
            do_span,
            done_span,
        } => {
            assert_eq!(in_span.slice(source), "in");
            assert_eq!(do_span.slice(source), "do");
            assert_eq!(done_span.slice(source), "done");
        }
        other => panic!("expected in/do/done syntax, got {other:?}"),
    }
}

#[test]
fn test_for_loop_preserves_non_identifier_target_surface() {
    for source in [
        "for - in a b c; do echo hi; done\n",
        "for i.j in a b c; do echo hi; done\n",
    ] {
        let output = Parser::new(source).parse().unwrap();
        let (compound, redirects) = expect_compound(&output.file.body[0]);
        let AstCompoundCommand::For(command) = compound else {
            panic!("expected for loop");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.targets.len(), 1);
        assert_eq!(
            command.targets[0].span.slice(source),
            command.targets[0].word.render(source)
        );
        assert!(command.targets[0].name.is_none());
    }
}

#[test]
fn test_zsh_for_loop_preserves_multiple_targets() {
    let source = "for k v in a b c d; do echo \"$k:$v\"; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .targets
            .iter()
            .map(|target| target.name.as_deref().expect("expected normalized target"))
            .collect::<Vec<_>>(),
        vec!["k", "v"]
    );
    assert_eq!(
        command
            .words
            .as_ref()
            .expect("expected explicit word list")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["a", "b", "c", "d"]
    );
    assert!(matches!(
        command.syntax,
        ForSyntax::InDoDone {
            in_span: Some(_),
            ..
        }
    ));
}

#[test]
fn test_zsh_for_loop_preserves_digit_targets() {
    let source = "for 1 2 3; do echo hi; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .targets
            .iter()
            .map(|target| target.name.as_deref().expect("expected normalized target"))
            .collect::<Vec<_>>(),
        vec!["1", "2", "3"]
    );
    assert!(command.words.is_none());
    assert!(matches!(
        command.syntax,
        ForSyntax::InDoDone { in_span: None, .. }
    ));
}

#[test]
fn test_zsh_for_loop_preserves_paren_do_done_syntax() {
    let source = "for version ($versions); do echo $version; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .targets
            .iter()
            .map(|target| target.name.as_deref().expect("expected normalized target"))
            .collect::<Vec<_>>(),
        vec!["version"]
    );
    assert_eq!(
        command
            .words
            .as_ref()
            .expect("expected parenthesized word list")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["$versions"]
    );
    match command.syntax {
        ForSyntax::ParenDoDone {
            left_paren_span,
            right_paren_span,
            do_span,
            done_span,
        } => {
            assert_eq!(left_paren_span.slice(source), "(");
            assert_eq!(right_paren_span.slice(source), ")");
            assert_eq!(do_span.slice(source), "do");
            assert_eq!(done_span.slice(source), "done");
        }
        other => panic!("expected paren/do/done syntax, got {other:?}"),
    }
}

#[test]
fn test_zsh_for_loop_paren_word_list_allows_newlines() {
    let source = "for file (\n  one\n  two\n); do echo $file; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .words
            .as_ref()
            .expect("expected parenthesized word list")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert!(matches!(command.syntax, ForSyntax::ParenDoDone { .. }));
}

#[test]
fn test_zsh_for_loop_paren_word_list_ignores_comments() {
    let source =
        "for file (\n  # first path\n  one\n  # second path\n  two\n); do echo $file; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .words
            .as_ref()
            .expect("expected parenthesized word list")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert!(matches!(command.syntax, ForSyntax::ParenDoDone { .. }));
}

#[test]
fn test_zsh_for_loop_preserves_multi_target_paren_do_done_syntax() {
    let source = "for old_name new_name (\n  current_branch git_current_branch\n); do aliases[$old_name]=$new_name; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .targets
            .iter()
            .map(|target| target.name.as_deref().expect("expected normalized target"))
            .collect::<Vec<_>>(),
        vec!["old_name", "new_name"]
    );
    assert_eq!(
        command
            .words
            .as_ref()
            .expect("expected parenthesized word list")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["current_branch", "git_current_branch"]
    );
    assert!(matches!(command.syntax, ForSyntax::ParenDoDone { .. }));
}

#[test]
fn test_zsh_for_loop_preserves_paren_brace_syntax() {
    let source = "for version ($versions); { echo $version; }\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    match command.syntax {
        ForSyntax::ParenBrace {
            left_paren_span,
            right_paren_span,
            left_brace_span,
            right_brace_span,
        } => {
            assert_eq!(left_paren_span.slice(source), "(");
            assert_eq!(right_paren_span.slice(source), ")");
            assert_eq!(left_brace_span.slice(source), "{");
            assert_eq!(right_brace_span.slice(source), "}");
        }
        other => panic!("expected paren/brace syntax, got {other:?}"),
    }
}

#[test]
fn test_zsh_for_loop_preserves_in_brace_syntax() {
    let source = "for part in a b; { echo $part; }\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    match command.syntax {
        ForSyntax::InBrace {
            in_span: Some(in_span),
            left_brace_span,
            right_brace_span,
        } => {
            assert_eq!(in_span.slice(source), "in");
            assert_eq!(left_brace_span.slice(source), "{");
            assert_eq!(right_brace_span.slice(source), "}");
        }
        other => panic!("expected in/brace syntax, got {other:?}"),
    }
}

#[test]
fn test_zsh_for_loop_allows_in_as_first_target_name() {
    let source = "for in other in a b; do echo \"$in:$other\"; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .targets
            .iter()
            .map(|target| target.name.as_deref().expect("expected normalized target"))
            .collect::<Vec<_>>(),
        vec!["in", "other"]
    );
    assert_eq!(
        command
            .words
            .as_ref()
            .expect("expected explicit word list")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn test_non_zsh_dialects_reject_zsh_for_loop_forms() {
    for source in [
        "for k v in a b c; do echo hi; done\n",
        "for 1 2 3; do echo hi; done\n",
        "for version ($versions); do echo $version; done\n",
        "for part in a b; { echo $part; }\n",
    ] {
        for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
            let error = Parser::with_dialect(source, dialect).parse();
            assert!(
                error.is_err(),
                "expected parse error for {dialect:?} on {source:?}",
            );
        }
    }
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_star_dot() {
    let source = "print *(.)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "*(.)");
    assert_eq!(command.args[0].span.slice(source), "*(.)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(.)");
    assert!(matches!(
        qualifiers.fragments.as_slice(),
        [ZshGlobQualifier::Flag { name: '.', span }] if span.slice(source) == "."
    ));
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_star_slash() {
    let source = "print *(/)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "*(/)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(/)");
    assert!(matches!(
        qualifiers.fragments.as_slice(),
        [ZshGlobQualifier::Flag { name: '/', span }] if span.slice(source) == "/"
    ));
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_star_n() {
    let source = "print *(N)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "*(N)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(N)");
    assert!(matches!(
        qualifiers.fragments.as_slice(),
        [ZshGlobQualifier::Flag { name: 'N', span }] if span.slice(source) == "N"
    ));
}

#[test]
fn test_zsh_trailing_glob_qualifier_after_quoted_prefix_stays_single_argument() {
    let source = "print \"$plugin_dir\"/*(:t)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].span.slice(source), "\"$plugin_dir\"/*(:t)");
}

#[test]
fn test_zsh_trailing_glob_qualifier_inside_compound_array_stays_single_element() {
    let source = "plugins=( \"$plugin_dir\"/*(:t) )\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.assignments.len(), 1);
    let assignment = &command.assignments[0];
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(first.span.slice(source), "\"$plugin_dir\"/*(:t)");
}

#[test]
fn test_zsh_quoted_variable_qualifier_inside_compound_array_stays_single_element() {
    let source = "__GREP_ALIAS_CACHES=( \"$__GREP_CACHE_FILE\"(Nm-1) )\nif true; then :; fi\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.assignments.len(), 1);
    let assignment = &command.assignments[0];
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(first.span.slice(source), "\"$__GREP_CACHE_FILE\"(Nm-1)");
    assert!(matches!(
        output.file.body[1].command,
        AstCommand::Compound(_)
    ));
}

#[test]
fn test_zsh_parameter_expansion_qualifier_inside_compound_array_stays_single_element() {
    let source = "files=( $dir/${~pats}(N) )\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.assignments.len(), 1);
    let assignment = &command.assignments[0];
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(first.span.slice(source), "$dir/${~pats}(N)");
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_recursive_pattern_with_letter_sequence_and_range() {
    let source = "print **/*(.om[1,3])\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "**/*(.om[1,3])");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "**/*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(.om[1,3])");

    let [
        ZshGlobQualifier::Flag {
            name: '.',
            span: dot_span,
        },
        ZshGlobQualifier::LetterSequence {
            text,
            span: letters_span,
        },
        ZshGlobQualifier::NumericArgument {
            span: range_span,
            start,
            end: Some(end),
        },
    ] = qualifiers.fragments.as_slice()
    else {
        panic!("expected dot, letter sequence, and numeric range qualifiers");
    };

    assert_eq!(dot_span.slice(source), ".");
    assert_eq!(letters_span.slice(source), "om");
    assert_eq!(text.slice(source), "om");
    assert_eq!(range_span.slice(source), "[1,3]");
    assert_eq!(start.slice(source), "1");
    assert_eq!(end.slice(source), "3");
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_prefixed_glob_with_negation() {
    let source = "print foo*(^-)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "foo*(^-)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "foo*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(^-)");
    let [
        ZshGlobQualifier::Negation {
            span: negation_span,
        },
        ZshGlobQualifier::Flag {
            name: '-',
            span: flag_span,
        },
    ] = qualifiers.fragments.as_slice()
    else {
        panic!("expected negation and dash flag qualifiers");
    };
    assert_eq!(negation_span.slice(source), "^");
    assert_eq!(flag_span.slice(source), "-");
}

#[test]
fn test_zsh_inline_glob_case_insensitive_control_preserves_segments() {
    let source = "print (#i)*.jpg\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);

    let [control, segment] = glob.segments.as_slice() else {
        panic!("expected inline control followed by pattern segment");
    };
    let ZshGlobSegment::InlineControl(ZshInlineGlobControl::CaseInsensitive { span }) = control
    else {
        panic!("expected case-insensitive inline control");
    };

    assert_eq!(glob.span.slice(source), "(#i)*.jpg");
    assert_eq!(span.slice(source), "(#i)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*.jpg"
    );
    assert!(glob.qualifiers.is_none());
}

#[test]
fn test_zsh_inline_glob_backreference_control_preserves_segments() {
    let source = "print (#b)(*)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);

    let [control, segment] = glob.segments.as_slice() else {
        panic!("expected inline control followed by pattern segment");
    };
    let ZshGlobSegment::InlineControl(ZshInlineGlobControl::Backreferences { span }) = control
    else {
        panic!("expected backreference inline control");
    };

    assert_eq!(glob.span.slice(source), "(#b)(*)");
    assert_eq!(span.slice(source), "(#b)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "(*)"
    );
    assert!(glob.qualifiers.is_none());
}

#[test]
fn test_zsh_inline_glob_anchor_controls_preserve_segments() {
    let parser = Parser::with_dialect("", ShellDialect::Zsh);

    let (start_len, start) = parser
        .parse_zsh_inline_glob_control("(#s)", Position::new(), 0)
        .expect("expected start-anchor control");
    let (end_len, end) = parser
        .parse_zsh_inline_glob_control("(#e)", Position::new(), 0)
        .expect("expected end-anchor control");

    assert_eq!(start_len, "(#s)".len());
    assert_eq!(end_len, "(#e)".len());
    assert!(matches!(
        start,
        ZshInlineGlobControl::StartAnchor { span } if span.slice("(#s)") == "(#s)"
    ));
    assert!(matches!(
        end,
        ZshInlineGlobControl::EndAnchor { span } if span.slice("(#e)") == "(#e)"
    ));
}

#[test]
fn test_zsh_hash_q_glob_qualifier_parses_terminal_flag_group() {
    let source = "print *.log(#qN)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "*.log(#qN)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*.log"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::HashQ);
    assert_eq!(qualifiers.span.slice(source), "(#qN)");
    assert!(matches!(
        qualifiers.fragments.as_slice(),
        [ZshGlobQualifier::Flag { name: 'N', span }] if span.slice(source) == "N"
    ));
}

#[test]
fn test_zsh_hash_q_glob_qualifier_parses_recursive_pattern_with_letter_sequence_and_range() {
    let source = "print **/*(#q.om[1,3])\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "**/*(#q.om[1,3])");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "**/*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::HashQ);
    assert_eq!(qualifiers.span.slice(source), "(#q.om[1,3])");

    let [
        ZshGlobQualifier::Flag {
            name: '.',
            span: dot_span,
        },
        ZshGlobQualifier::LetterSequence {
            text,
            span: letters_span,
        },
        ZshGlobQualifier::NumericArgument {
            span: range_span,
            start,
            end: Some(end),
        },
    ] = qualifiers.fragments.as_slice()
    else {
        panic!("expected dot, letter sequence, and numeric range qualifiers");
    };

    assert_eq!(dot_span.slice(source), ".");
    assert_eq!(letters_span.slice(source), "om");
    assert_eq!(text.slice(source), "om");
    assert_eq!(range_span.slice(source), "[1,3]");
    assert_eq!(start.slice(source), "1");
    assert_eq!(end.slice(source), "3");
}

#[test]
fn test_zsh_glob_falls_back_for_unsupported_hash_control_group() {
    let source = "print *(#a)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.args[0].span.slice(source), "*(#a)");
    assert!(!matches!(
        command.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));
}

#[test]
fn test_non_zsh_dialects_do_not_special_case_trailing_glob_qualifiers() {
    for syntax in [
        "*(.)",
        "*(/)",
        "*(N)",
        "**/*(.om[1,3])",
        "foo*(^-)",
        "(#i)*.jpg",
        "(#b)(*)",
        "*.log(#qN)",
        "**/*(#q.om[1,3])",
    ] {
        let source = format!("print {syntax}\n");

        for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
            let output = Parser::with_dialect(&source, dialect).parse().unwrap();
            let AstCommand::Simple(command) = &output.file.body[0].command else {
                panic!("expected simple command");
            };

            assert_eq!(
                command.args[0].span.slice(&source),
                syntax,
                "expected non-zsh dialect {dialect:?} to preserve {syntax:?} as a plain word",
            );
            assert!(
                !matches!(
                    command.args[0].parts.as_slice(),
                    [WordPartNode {
                        kind: WordPart::ZshQualifiedGlob(_),
                        ..
                    }]
                ),
                "unexpected zsh-qualified glob node for {syntax:?} in {dialect:?}",
            );
        }
    }
}

#[test]
fn test_zsh_repeat_do_done_preserves_structure_and_spans() {
    let source = "repeat 3; do echo hi; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Repeat(command) = compound else {
        panic!("expected repeat command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.span.slice(source), "repeat 3; do echo hi; done");
    assert_eq!(command.count.span.slice(source), "3");
    assert_eq!(command.body.len(), 1);
    assert_eq!(command.body.span.slice(source), "echo hi; ");

    match command.syntax {
        RepeatSyntax::DoDone { do_span, done_span } => {
            assert_eq!(do_span.slice(source), "do");
            assert_eq!(done_span.slice(source), "done");
        }
        RepeatSyntax::Direct => panic!("expected do/done repeat syntax"),
        RepeatSyntax::Brace { .. } => panic!("expected do/done repeat syntax"),
    }

    let body_command = expect_simple(&command.body[0]);
    assert_eq!(body_command.name.render(source), "echo");
    assert_eq!(body_command.args[0].render(source), "hi");
}

#[test]
fn test_zsh_repeat_brace_preserves_structure_and_spans() {
    let source = "repeat 3 { echo hi; }\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Repeat(command) = compound else {
        panic!("expected repeat command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.span.slice(source), "repeat 3 { echo hi; }");
    assert_eq!(command.count.span.slice(source), "3");
    assert_eq!(command.body.len(), 1);
    assert_eq!(command.body.span.slice(source), "echo hi; ");

    match command.syntax {
        RepeatSyntax::Brace {
            left_brace_span,
            right_brace_span,
        } => {
            assert_eq!(left_brace_span.slice(source), "{");
            assert_eq!(right_brace_span.slice(source), "}");
        }
        RepeatSyntax::Direct => panic!("expected brace repeat syntax"),
        RepeatSyntax::DoDone { .. } => panic!("expected brace repeat syntax"),
    }

    let body_command = expect_simple(&command.body[0]);
    assert_eq!(body_command.name.render(source), "echo");
    assert_eq!(body_command.args[0].render(source), "hi");
}

#[test]
fn test_zsh_foreach_paren_brace_preserves_structure_and_spans() {
    let source = "foreach x (a b c) { echo $x; }\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Foreach(command) = compound else {
        panic!("expected foreach command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.span.slice(source), "foreach x (a b c) { echo $x; }");
    assert_eq!(command.variable.as_str(), "x");
    assert_eq!(command.variable_span.slice(source), "x");
    assert_eq!(
        command
            .words
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    assert_eq!(command.body.len(), 1);
    assert_eq!(command.body.span.slice(source), "echo $x; ");

    match command.syntax {
        ForeachSyntax::ParenBrace {
            left_paren_span,
            right_paren_span,
            left_brace_span,
            right_brace_span,
        } => {
            assert_eq!(left_paren_span.slice(source), "(");
            assert_eq!(right_paren_span.slice(source), ")");
            assert_eq!(left_brace_span.slice(source), "{");
            assert_eq!(right_brace_span.slice(source), "}");
        }
        ForeachSyntax::InDoDone { .. } => panic!("expected paren/brace foreach syntax"),
    }

    let body_command = expect_simple(&command.body[0]);
    assert_eq!(body_command.name.render(source), "echo");
    assert_eq!(body_command.args[0].render(source), "$x");
}

#[test]
fn test_zsh_foreach_in_do_done_preserves_structure_and_spans() {
    let source = "foreach x in a b c; do echo $x; done\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Foreach(command) = compound else {
        panic!("expected foreach command");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command.span.slice(source),
        "foreach x in a b c; do echo $x; done"
    );
    assert_eq!(command.variable.as_str(), "x");
    assert_eq!(command.variable_span.slice(source), "x");
    assert_eq!(
        command
            .words
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    assert_eq!(command.body.len(), 1);
    assert_eq!(command.body.span.slice(source), "echo $x; ");

    match command.syntax {
        ForeachSyntax::InDoDone {
            in_span,
            do_span,
            done_span,
        } => {
            assert_eq!(in_span.slice(source), "in");
            assert_eq!(do_span.slice(source), "do");
            assert_eq!(done_span.slice(source), "done");
        }
        ForeachSyntax::ParenBrace { .. } => panic!("expected in/do/done foreach syntax"),
    }

    let body_command = expect_simple(&command.body[0]);
    assert_eq!(body_command.name.render(source), "echo");
    assert_eq!(body_command.args[0].render(source), "$x");
}

#[test]
fn test_non_zsh_dialects_reject_repeat_and_foreach_forms() {
    for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
        for source in [
            "repeat 3; do echo hi; done\n",
            "repeat 3 { echo hi; }\n",
            "foreach x (a b c) { echo $x; }\n",
            "foreach x in a b c; do echo $x; done\n",
        ] {
            let error = Parser::with_dialect(source, dialect).parse().unwrap_err();
            assert!(
                matches!(error, Error::Parse { .. }),
                "expected parse error for {dialect:?} on {source:?}, got {error:?}"
            );
        }
    }
}

#[test]
fn test_zsh_parameter_modifier_records_modifier_and_target() {
    let source = "print ${(m)foo} ${(%):-%x}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    assert_eq!(first.raw_body.slice(source), "(m)foo");
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        first
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['m']
    );
    let ZshExpansionTarget::Reference(reference) = &first.target else {
        panic!("expected direct zsh reference target");
    };
    assert_eq!(reference.name.as_str(), "foo");
    assert!(first.operation.is_none());

    let second = expect_parameter(&command.args[1]);
    assert_eq!(second.raw_body.slice(source), "(%):-%x");
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        second
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['%']
    );
    assert!(matches!(second.target, ZshExpansionTarget::Empty));
    assert!(matches!(
        second.operation,
        Some(ZshExpansionOperation::Defaulting {
            kind: ZshDefaultingOp::UseDefault,
            ref operand,
            colon_variant: true,
            ..
        }) if operand.slice(source) == "%x"
    ));
    let defaulting = second
        .operation
        .as_ref()
        .expect("expected defaulting operation");
    assert_eq!(
        defaulting
            .operand_word_ast()
            .expect("expected defaulting operand word")
            .span
            .slice(source),
        "%x"
    );
}

#[test]
fn test_zsh_parameter_modifier_groups_split_flags_and_preserve_delimited_args() {
    let source = "print ${(Az)LBUFFER} ${(s./.)_p9k__cwd} ${(pj./.)parts[1,MATCH]}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        first
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['A', 'z']
    );
    assert!(
        first
            .modifiers
            .iter()
            .all(|modifier| modifier.span == first.modifiers[0].span)
    );
    let ZshExpansionTarget::Reference(reference) = &first.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "LBUFFER");

    let second = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        second
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['s']
    );
    assert_eq!(second.modifiers[0].argument_delimiter, Some('.'));
    assert_eq!(
        second.modifiers[0]
            .argument
            .as_ref()
            .expect("expected modifier argument")
            .slice(source),
        "/"
    );
    assert_eq!(
        second.modifiers[0]
            .argument_word_ast()
            .expect("expected modifier argument word")
            .span
            .slice(source),
        "/"
    );
    let ZshExpansionTarget::Reference(reference) = &second.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "_p9k__cwd");

    let third = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(third) = &third.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        third
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['p', 'j']
    );
    assert!(
        third
            .modifiers
            .iter()
            .all(|modifier| modifier.span == third.modifiers[0].span)
    );
    assert_eq!(third.modifiers[1].argument_delimiter, Some('.'));
    assert_eq!(
        third.modifiers[1]
            .argument
            .as_ref()
            .expect("expected modifier argument")
            .slice(source),
        "/"
    );
    assert_eq!(
        third.modifiers[1]
            .argument_word_ast()
            .expect("expected modifier argument word")
            .render(source),
        "/"
    );
    let ZshExpansionTarget::Reference(reference) = &third.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "parts");
    let subscript = expect_subscript(reference, source, "1,MATCH");
    assert_eq!(subscript.syntax_text(source), "1,MATCH");
}

#[test]
fn test_zsh_parameter_word_target_preserves_non_reference_target_text() {
    let source = "print ${^$(pidof zsh):#$$}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['^']
    );
    let ZshExpansionTarget::Word(word) = &parameter.target else {
        panic!("expected word target");
    };
    assert_eq!(word.render(source), "$(pidof zsh)");
    assert!(matches!(
        parameter.operation,
        Some(ZshExpansionOperation::PatternOperation {
            kind: ZshPatternOp::Filter,
            ref operand,
            ..
        }) if operand.slice(source) == "$$"
    ));
    let operation = parameter
        .operation
        .as_ref()
        .expect("expected pattern operation");
    assert_eq!(
        operation
            .operand_word_ast()
            .expect("expected pattern operand word")
            .render(source),
        "$$"
    );
}

#[test]
fn test_zsh_parameter_bare_prefix_flags_record_modifier_sequence() {
    let source = "print ${=name} ${~foo} ${^^bar} ${~~baz}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let split = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(split) = &split.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        split
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['=']
    );
    let ZshExpansionTarget::Reference(reference) = &split.target else {
        panic!("expected split target reference");
    };
    assert_eq!(reference.name.as_str(), "name");

    let glob = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(glob) = &glob.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        glob.modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['~']
    );
    let ZshExpansionTarget::Reference(reference) = &glob.target else {
        panic!("expected glob target reference");
    };
    assert_eq!(reference.name.as_str(), "foo");

    let rc_expand = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(rc_expand) = &rc_expand.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        rc_expand
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['^', '^']
    );
    let ZshExpansionTarget::Reference(reference) = &rc_expand.target else {
        panic!("expected rc-expand target reference");
    };
    assert_eq!(reference.name.as_str(), "bar");

    let glob_off = expect_parameter(&command.args[3]);
    let ParameterExpansionSyntax::Zsh(glob_off) = &glob_off.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        glob_off
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['~', '~']
    );
    let ZshExpansionTarget::Reference(reference) = &glob_off.target else {
        panic!("expected glob-off target reference");
    };
    assert_eq!(reference.name.as_str(), "baz");
}

#[test]
fn test_zsh_parameter_word_target_accepts_quoted_command_substitution_text() {
    let source = "print ${\"$(xcode-select -p)\"%%/Contents/Developer*}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(parameter.length_prefix.is_none());
    let ZshExpansionTarget::Word(word) = &parameter.target else {
        panic!("expected quoted word target");
    };
    assert_eq!(word.span.slice(source), "\"$(xcode-select -p)\"");
    assert!(matches!(
        parameter.operation,
        Some(ZshExpansionOperation::TrimOperation {
            kind: ZshTrimOp::RemoveSuffixLong,
            ref operand,
            ..
        }) if operand.slice(source) == "/Contents/Developer*"
    ));
    let operation = parameter
        .operation
        .as_ref()
        .expect("expected trim operation");
    assert_eq!(
        operation
            .operand_word_ast()
            .expect("expected trim operand word")
            .span
            .slice(source),
        "/Contents/Developer*"
    );
}

#[test]
fn test_zsh_parameter_length_prefix_preserves_nested_replacement_target() {
    let source = "print ${#${cd//${~q}/}}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .length_prefix
            .expect("expected zsh length prefix")
            .slice(source),
        "#"
    );
    let ZshExpansionTarget::Nested(inner) = &parameter.target else {
        panic!("expected nested zsh target");
    };
    assert_eq!(inner.raw_body.slice(source), "cd//${~q}/");
    let ParameterExpansionSyntax::Zsh(inner) = &inner.syntax else {
        panic!("expected nested zsh syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &inner.target else {
        panic!("expected nested replacement target reference");
    };
    assert_eq!(reference.name.as_str(), "cd");
    assert!(matches!(
        inner.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceAll,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(source) == "${~q}" && replacement.slice(source).is_empty()
    ));
    let operation = inner
        .operation
        .as_ref()
        .expect("expected replacement operation");
    assert_eq!(
        operation
            .pattern_word_ast()
            .expect("expected replacement pattern word")
            .span
            .slice(source),
        "${~q}"
    );
}

#[test]
fn test_zsh_parameter_colon_modifiers_preserve_targets_without_bourne_slice_offsets() {
    let source = "print ${REPLY:l} ${1:t} ${0:h}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let reply = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(reply) = &reply.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &reply.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "REPLY");
    assert!(matches!(
        reply.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":l"
    ));

    let first = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &first.target else {
        panic!("expected positional reference target");
    };
    assert_eq!(reference.name.as_str(), "1");
    assert!(matches!(
        first.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":t"
    ));

    let zero = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(zero) = &zero.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &zero.target else {
        panic!("expected script-name reference target");
    };
    assert_eq!(reference.name.as_str(), "0");
    assert!(matches!(
        zero.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":h"
    ));
    let operation = zero.operation.as_ref().expect("expected unknown operation");
    assert_eq!(
        operation
            .operand_word_ast()
            .expect("expected unknown operation word")
            .render(source),
        ":h"
    );
}

#[test]
fn test_zsh_parameter_colon_modifiers_with_digits_preserve_targets() {
    let source = "print ${path:A:h3} ${path:t2}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &first.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "path");
    assert!(matches!(
        first.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":A:h3"
    ));

    let second = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &second.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "path");
    assert!(matches!(
        second.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":t2"
    ));
}

#[test]
fn test_zsh_plain_positional_parameters_preserve_bourne_references() {
    let source = "print ${1} ${10} ${#1} ${#10}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    assert!(matches!(
        &first.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "1"
    ));

    let second = expect_parameter(&command.args[1]);
    assert!(matches!(
        &second.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "10"
    ));

    let third = expect_parameter(&command.args[2]);
    assert!(matches!(
        &third.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { reference })
            if reference.name.as_str() == "1"
    ));

    let fourth = expect_parameter(&command.args[3]);
    assert!(matches!(
        &fourth.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { reference })
            if reference.name.as_str() == "10"
    ));
}

#[test]
fn test_parse_zsh_array_assignment_with_word_target_and_glob_qualifier() {
    let source = "dirs=( /proc/${^$(pidof zsh):#$$}/cwd(N:A) )\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(
        array.span.slice(source),
        "( /proc/${^$(pidof zsh):#$$}/cwd(N:A) )"
    );
}

#[test]
fn test_parse_zsh_assignment_with_nested_subscript_pattern_range() {
    let source = "in_alias=($in_alias[$in_alias[(i)<1->],-1])\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(
        array.span.slice(source),
        "($in_alias[$in_alias[(i)<1->],-1])"
    );
}

#[test]
fn test_parse_zsh_nested_join_modifier_inside_replacement_word() {
    let source =
        "_p9k__parent_dirs=(${(@)${:-{$#parts..1}}/(#m)*/$parent${(pj./.)parts[1,MATCH]}})\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(
        array.span.slice(source),
        "(${(@)${:-{$#parts..1}}/(#m)*/$parent${(pj./.)parts[1,MATCH]}})"
    );
}

#[test]
fn test_parse_zsh_compound_array_ignores_trailing_comments() {
    let source = "opts=(\n  'grc' :se # grc - a \"generic colouriser\" (that\\'s their spelling, not mine)\n  'cpulimit' elp:ivz # cpulimit 0.2\n)\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_associative_array_literal_with_blank_lines_and_comments() {
    let source = "local -A precommand_options\nprecommand_options=(\n  # Precommand modifiers as of zsh 5.6.2 cf. zshmisc(1).\n  '-' ''\n  'builtin' ''\n  'command' :pvV\n  'exec' a:cl\n  'noglob' ''\n  # 'time' and 'nocorrect' shouldn't be added here; they're reserved words, not precommands.\n\n  # miscellaneous commands\n  'doas' aCu:Lns # as of OpenBSD's doas(1) dated September 4, 2016\n  'nice' n: # as of current POSIX spec\n  'pkexec' '' # doesn't take short options; immune to #121 because it's usually not passed --option flags\n  # Not listed: -h, which has two different meanings.\n  'sudo' Cgprtu:AEHPSbilns:eKkVv # as of sudo 1.8.21p2\n  'stdbuf' ioe:\n  'eatmydata' ''\n  'catchsegv' ''\n  'nohup' ''\n  'setsid' :wc\n  'env' u:i\n  'ionice' cn:t:pPu # util-linux 2.33.1-0.1\n  'strace' IbeaosXPpEuOS:ACdfhikqrtTvVxyDc # strace 4.26-0.2\n  'proxychains' f:q # proxychains 4.4.0\n  'torsocks' idq:upaP # Torsocks 2.3.0\n  'torify' idq:upaP # Torsocks 2.3.0\n  'ssh-agent' aEPt:csDd:k # As of OpenSSH 8.1p1\n  'tabbed' gnprtTuU:cdfhs:v # suckless-tools v44\n  'chronic' :ev # moreutils 0.62-1\n  'ifne' :n # moreutils 0.62-1\n  'grc' :se # grc - a \"generic colouriser\" (that's their spelling, not mine)\n  'cpulimit' elp:ivz # cpulimit 0.2\n  'ktrace' fgpt:aBCcdiT\n  'caffeinate' tw:dimsu # as of macOS's caffeinate(8) dated November 9, 2012\n)\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_compound_array_with_nested_groups_and_qualifiers() {
    let source = "local -a bats=( /sys/class/power_supply/(CMB*|BAT*|*battery)/(FN) )\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_arithmetic_shell_word_lookup_with_nested_modifier() {
    let source = "(( e = ${tokens[(i)${(Q)token}]} ))\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_arithmetic_shell_word_preserves_nested_length_target() {
    let source = "(( q_chars = ${#${cd//${~q}/}} ))\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic command");
    };
    let expr = command.expr_ast.as_ref().expect("expected arithmetic AST");
    let ArithmeticExpr::Assignment { value, .. } = &expr.kind else {
        panic!("expected arithmetic assignment");
    };
    let ArithmeticExpr::ShellWord(word) = &value.kind else {
        panic!("expected arithmetic shell word");
    };
    let parameter = expect_parameter(word);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .length_prefix
            .expect("expected zsh length prefix")
            .slice(source),
        "#"
    );
    let ZshExpansionTarget::Nested(inner) = &parameter.target else {
        panic!("expected nested zsh target");
    };
    let ParameterExpansionSyntax::Zsh(inner) = &inner.syntax else {
        panic!("expected nested zsh syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &inner.target else {
        panic!("expected nested reference target");
    };
    assert_eq!(reference.name.as_str(), "cd");
    assert!(matches!(
        inner.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceAll,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(source) == "${~q}" && replacement.slice(source).is_empty()
    ));
}

#[test]
fn test_zsh_parameter_identifier_slices_preserve_legacy_slice_parts() {
    let source = "print ${foo:i} ${foo:i:j}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let (first_reference, first_offset_ast, first_length_ast) =
        expect_substring_part(&command.args[0].parts[0].kind);
    assert_eq!(first_reference.name.as_str(), "foo");
    assert_eq!(
        first_offset_ast
            .as_ref()
            .expect("expected first offset AST")
            .span
            .slice(source),
        "i"
    );
    assert!(first_length_ast.is_none());

    let (second_reference, second_offset_ast, second_length_ast) =
        expect_substring_part(&command.args[1].parts[0].kind);
    assert_eq!(second_reference.name.as_str(), "foo");
    assert_eq!(
        second_offset_ast
            .as_ref()
            .expect("expected second offset AST")
            .span
            .slice(source),
        "i"
    );
    assert_eq!(
        second_length_ast
            .as_ref()
            .expect("expected second length AST")
            .span
            .slice(source),
        "j"
    );
}

#[test]
fn test_zsh_parameter_identifier_slices_stay_typed_in_zsh_parameter_nodes() {
    let source = "print ${(m)foo:i:j}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let parameter = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['m']
    );
    let ZshExpansionTarget::Reference(reference) = &parameter.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "foo");
    assert!(matches!(
        parameter.operation,
        Some(ZshExpansionOperation::Slice {
            ref offset,
            length: Some(ref length),
            ..
        }) if offset.slice(source) == "i" && length.slice(source) == "j"
    ));
    let operation = parameter
        .operation
        .as_ref()
        .expect("expected slice operation");
    assert_eq!(
        operation
            .offset_word_ast()
            .expect("expected slice offset word")
            .render(source),
        "i"
    );
    assert_eq!(
        operation
            .length_word_ast()
            .expect("expected slice length word")
            .render(source),
        "j"
    );
}

#[test]
fn test_zsh_nested_parameter_modifier_records_nested_target_and_pattern_operation() {
    let source = "print ${(M)${(k)parameters[@]}:#__gitcomp_builtin_*}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['M']
    );
    let ZshExpansionTarget::Nested(inner) = &parameter.target else {
        panic!("expected nested zsh parameter target");
    };
    let ParameterExpansionSyntax::Zsh(inner) = &inner.syntax else {
        panic!("expected nested zsh syntax");
    };
    assert_eq!(
        inner
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['k']
    );
    let ZshExpansionTarget::Reference(reference) = &inner.target else {
        panic!("expected nested reference target");
    };
    assert_eq!(reference.name.as_str(), "parameters");
    assert!(reference.has_array_selector());
    assert!(matches!(
        parameter.operation,
        Some(ZshExpansionOperation::PatternOperation {
            kind: ZshPatternOp::Filter,
            ref operand,
            ..
        }) if operand.slice(source) == "__gitcomp_builtin_*"
    ));
}

#[test]
fn test_zsh_nested_plain_access_targets_preserve_bourne_refs_without_modifier_regression() {
    let source = "print ${${10}} ${#${10}} ${${#}} ${${$}} ${${1:t}}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let nested = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(nested) = &nested.syntax else {
        panic!("expected outer zsh syntax");
    };
    let ZshExpansionTarget::Nested(inner) = &nested.target else {
        panic!("expected nested target");
    };
    assert!(matches!(
        &inner.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "10"
    ));

    let length = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(length) = &length.syntax else {
        panic!("expected outer zsh syntax");
    };
    assert_eq!(
        length
            .length_prefix
            .expect("expected zsh length prefix")
            .slice(source),
        "#"
    );
    let ZshExpansionTarget::Nested(inner) = &length.target else {
        panic!("expected nested length target");
    };
    assert!(matches!(
        &inner.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "10"
    ));

    let count = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(count) = &count.syntax else {
        panic!("expected outer zsh syntax");
    };
    let ZshExpansionTarget::Nested(inner) = &count.target else {
        panic!("expected nested count target");
    };
    assert!(matches!(
        &inner.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "#"
    ));

    let pid = expect_parameter(&command.args[3]);
    let ParameterExpansionSyntax::Zsh(pid) = &pid.syntax else {
        panic!("expected outer zsh syntax");
    };
    let ZshExpansionTarget::Nested(inner) = &pid.target else {
        panic!("expected nested pid target");
    };
    assert!(matches!(
        &inner.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "$"
    ));

    let modifier = expect_parameter(&command.args[4]);
    let ParameterExpansionSyntax::Zsh(modifier) = &modifier.syntax else {
        panic!("expected outer zsh syntax");
    };
    let ZshExpansionTarget::Nested(inner) = &modifier.target else {
        panic!("expected nested modifier target");
    };
    let ParameterExpansionSyntax::Zsh(inner) = &inner.syntax else {
        panic!("expected inner zsh syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &inner.target else {
        panic!("expected positional reference target");
    };
    assert_eq!(reference.name.as_str(), "1");
    assert!(matches!(
        inner.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":t"
    ));
}

#[test]
fn test_zsh_parameter_supported_operations_are_typed_and_preserve_source_spans() {
    let source = "print ${(m)foo#${needle}} ${(S)foo//\"pre\"$suffix/$replacement} ${(m)foo:$offset:${length}} ${(m)foo:^other}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let trim = expect_parameter(&command.args[0]);
    assert_eq!(trim.raw_body.slice(source), "(m)foo#${needle}");
    let ParameterExpansionSyntax::Zsh(trim) = &trim.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        trim.operation,
        Some(ZshExpansionOperation::TrimOperation {
            kind: ZshTrimOp::RemovePrefixShort,
            ref operand,
            ..
        }) if operand.is_source_backed() && operand.slice(source) == "${needle}"
    ));
    let trim_operation = trim.operation.as_ref().expect("expected trim operation");
    assert_eq!(
        trim_operation
            .operand_word_ast()
            .expect("expected trim operand word")
            .span
            .slice(source),
        "${needle}"
    );

    let replacement = expect_parameter(&command.args[1]);
    assert_eq!(
        replacement.raw_body.slice(source),
        "(S)foo//\"pre\"$suffix/$replacement"
    );
    let ParameterExpansionSyntax::Zsh(replacement) = &replacement.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        replacement.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceAll,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.is_source_backed()
            && pattern.slice(source) == "\"pre\"$suffix"
            && replacement.is_source_backed()
            && replacement.slice(source) == "$replacement"
    ));
    let replacement_operation = replacement
        .operation
        .as_ref()
        .expect("expected replacement operation");
    assert_eq!(
        replacement_operation
            .pattern_word_ast()
            .expect("expected replacement pattern word")
            .span
            .slice(source),
        "\"pre\"$suffix"
    );
    assert_eq!(
        replacement_operation
            .replacement_word_ast()
            .expect("expected replacement word")
            .span
            .slice(source),
        "$replacement"
    );

    let slice = expect_parameter(&command.args[2]);
    assert_eq!(slice.raw_body.slice(source), "(m)foo:$offset:${length}");
    let ParameterExpansionSyntax::Zsh(slice) = &slice.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        slice.operation,
        Some(ZshExpansionOperation::Slice {
            ref offset,
            length: Some(ref length),
            ..
        }) if offset.is_source_backed()
            && offset.slice(source) == "$offset"
            && length.is_source_backed()
            && length.slice(source) == "${length}"
    ));
    let slice_operation = slice.operation.as_ref().expect("expected slice operation");
    assert_eq!(
        slice_operation
            .offset_word_ast()
            .expect("expected slice offset word")
            .span
            .slice(source),
        "$offset"
    );
    assert_eq!(
        slice_operation
            .length_word_ast()
            .expect("expected slice length word")
            .span
            .slice(source),
        "${length}"
    );

    let unknown = expect_parameter(&command.args[3]);
    assert_eq!(unknown.raw_body.slice(source), "(m)foo:^other");
    let ParameterExpansionSyntax::Zsh(unknown) = &unknown.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        unknown.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. })
            if text.is_source_backed() && text.slice(source) == ":^other"
    ));
    let unknown_operation = unknown
        .operation
        .as_ref()
        .expect("expected unknown operation");
    assert_eq!(
        unknown_operation
            .operand_word_ast()
            .expect("expected unknown operation word")
            .span
            .slice(source),
        ":^other"
    );
}

#[test]
fn test_zsh_parameter_operation_kinds_cover_long_trim_and_anchored_replacement() {
    let source = "print ${(m)foo##pre*} ${(m)foo%%post*} ${(S)foo/#$prefix/$replacement} ${(S)foo/%$suffix/$replacement}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        first.operation,
        Some(ZshExpansionOperation::TrimOperation {
            kind: ZshTrimOp::RemovePrefixLong,
            ref operand,
            ..
        }) if operand.slice(source) == "pre*"
    ));

    let second = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        second.operation,
        Some(ZshExpansionOperation::TrimOperation {
            kind: ZshTrimOp::RemoveSuffixLong,
            ref operand,
            ..
        }) if operand.slice(source) == "post*"
    ));

    let third = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(third) = &third.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        third.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplacePrefix,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(source) == "$prefix" && replacement.slice(source) == "$replacement"
    ));

    let fourth = expect_parameter(&command.args[3]);
    let ParameterExpansionSyntax::Zsh(fourth) = &fourth.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        fourth.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceSuffix,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(source) == "$suffix" && replacement.slice(source) == "$replacement"
    ));
}

#[test]
fn test_zsh_brace_if_records_brace_syntax() {
    let source =
        "if [[ -n $foo ]] { print foo; } elif [[ -n $bar ]] { print bar; } else { print baz; }\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert!(matches!(
        command.syntax,
        IfSyntax::Brace {
            left_brace_span,
            right_brace_span,
        } if left_brace_span.slice(source) == "{" && right_brace_span.slice(source) == "}"
    ));
    assert_eq!(command.elif_branches.len(), 1);
    assert!(command.else_branch.is_some());
}

#[test]
fn test_zsh_brace_if_allows_same_line_closing_braces_without_semicolons() {
    let source =
        "if [[ -n $foo ]] { print foo } elif [[ -n $bar ]] { print bar } else { print baz }\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert_eq!(command.then_branch.len(), 1);
    assert_eq!(command.elif_branches.len(), 1);
    assert!(command.else_branch.is_some());
    assert_eq!(
        expect_simple(&command.then_branch[0]).name.render(source),
        "print"
    );
    assert_eq!(
        expect_simple(&command.elif_branches[0].1[0])
            .name
            .render(source),
        "print"
    );
    assert_eq!(
        expect_simple(&command.else_branch.as_ref().unwrap()[0])
            .name
            .render(source),
        "print"
    );
}

#[test]
fn test_zsh_if_condition_allows_compact_brace_group_before_then_separator() {
    let source = "\
if zstyle -t ':omz:alpha:lib:git' async-prompt \\
  || { is-at-least 5.0.6 && zstyle -T ':omz:alpha:lib:git' async-prompt }; then
  print ok
fi
";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert_eq!(command.then_branch.len(), 1);
    assert_eq!(
        expect_simple(&command.then_branch[0]).name.render(source),
        "print"
    );
}

#[test]
fn test_zsh_if_condition_can_start_with_brace_group() {
    let source = "\
if { ! . \"$srcdir\"/\"$ARG\" } || (( $#fail_test )); then
  print ok
fi
";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert_eq!(command.condition.len(), 1);
    assert_eq!(command.then_branch.len(), 1);
    assert_eq!(
        expect_simple(&command.then_branch[0]).name.render(source),
        "print"
    );
}

#[test]
fn test_zsh_comment_only_elif_body_is_preserved_on_branch() {
    let source = "\
if true; then
  print ok
elif false; then
  # keep this branch for later
fi
";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert_eq!(command.elif_branches.len(), 1);
    let elif_body = &command.elif_branches[0].1;
    assert!(elif_body.is_empty());
    assert_eq!(elif_body.trailing_comments.len(), 1);

    let comment = elif_body.trailing_comments[0];
    let start = usize::from(comment.range.start());
    let end = usize::from(comment.range.end());
    assert_eq!(&source[start..end], "# keep this branch for later");
}

#[test]
fn test_parse_zsh_repeat_with_inline_simple_command_body() {
    let source = "repeat $difference do left_column+=(.); done\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_repeat_with_direct_assignment_body() {
    let source =
        "repeat $((num_right_lines - num_left_lines)) left_segments=(newline $left_segments)\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_function_keyword_with_spaced_empty_brace_body() {
    let source = "function battery_time_remaining() { } # Not available on android\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_compact_function_body_with_background_pipe_and_trailing_semicolon() {
    let source = "function clipcopy() { cat \"${1:-/dev/stdin}\" | wl-copy &>/dev/null &|; }\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_git_extras_style_quoted_continuations_inside_assignment() {
    let source = "tag_names=(${${(f)\"$(_call_program tags git for-each-ref --format='\"%(refname)\"' refs/tags 2>/dev/null)\"}#refs/tags/})\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    assert!(command.args.is_empty());

    let assignment = &command.assignments[0];
    assert_eq!(assignment.target.name, "tag_names");

    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(array.elements.len(), 1);

    let ArrayElem::Sequential(value) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(
        value.span.slice(source),
        "${${(f)\"$(_call_program tags git for-each-ref --format='\"%(refname)\"' refs/tags 2>/dev/null)\"}#refs/tags/}"
    );
}

#[test]
fn test_parse_zsh_parameter_default_with_prompt_escape_text() {
    let source = "color_green=${BATTERY_COLOR_GREEN:-%F{green}}\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_force_append_redirect() {
    let source = "print -lr -- ${p}${^*} >>| $SCD_HISTFILE\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_assignment_with_escaped_literal_parameter_template_in_double_quotes() {
    let source = "IFS=$'\\1' _p9k__param_pat+=\"${(@)${(@o)parameters[(I)POWERLEVEL9K_*]}:/(#m)*/\\${${(q)MATCH}-$IFS\\}}\"\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_anonymous_function_invocation_with_nested_replacement_word() {
    let source = r#"if [[ -t 1 ]]; then
  if (( ${+__p9k_use_osc133_c_cmdline} )); then
    () {
      emulate -L zsh -o extended_glob -o no_multibyte
      local MATCH MBEGIN MEND
      builtin printf '\e]133;C;cmdline_url=%s\a' "${1//(#m)[^a-zA-Z0-9"\/:_.-!'()~"]/%${(l:2::0:)$(([##16]#MATCH))}}"
    } "$1"
  else
    builtin print -n '\e]133;C;\a'
  fi
fi
"#;
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_standalone_anonymous_function_invocation_with_nested_replacement_word() {
    let source = r#"() {
  emulate -L zsh -o extended_glob -o no_multibyte
  local MATCH MBEGIN MEND
  builtin printf '\e]133;C;cmdline_url=%s\a' "${1//(#m)[^a-zA-Z0-9"\/:_.-!'()~"]/%${(l:2::0:)$(([##16]#MATCH))}}"
} "$1"
"#;
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_zsh_truly_empty_elif_body_is_still_rejected() {
    let source = "\
if true; then
  print ok
elif false; then
fi
";
    assert!(
        Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .is_err(),
        "expected zsh empty elif without comments to stay rejected",
    );
}

#[test]
fn test_zsh_if_condition_brace_group_keeps_closing_brace_out_of_arguments() {
    let source =
        "if (( fd != -1 && pid != -1 )) && { true <&$fd } 2>/dev/null; then\n  echo ok\nfi\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.condition.len(), 1);

    let condition = expect_binary(&command.condition[0]);
    assert_eq!(condition.op, BinaryOp::And);

    let (body_compound, body_redirects) = expect_compound(&condition.right);
    let AstCompoundCommand::BraceGroup(body) = body_compound else {
        panic!("expected brace group on right-hand side of &&");
    };

    assert_eq!(body.len(), 1);
    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(source), "true");
    assert!(inner.args.is_empty());
    assert_eq!(body[0].redirects.len(), 1);
    assert_eq!(body[0].redirects[0].kind, RedirectKind::DupInput);
    assert_eq!(
        redirect_word_target(&body[0].redirects[0]).render(source),
        "$fd"
    );

    assert_eq!(body_redirects.len(), 1);
    assert_eq!(body_redirects[0].fd, Some(2));
    assert_eq!(body_redirects[0].kind, RedirectKind::Output);
    assert_eq!(
        redirect_word_target(&body_redirects[0]).render(source),
        "/dev/null"
    );

    assert_eq!(command.then_branch.len(), 1);
    assert_eq!(
        expect_simple(&command.then_branch[0]).name.render(source),
        "echo"
    );
}

#[test]
fn test_zsh_always_and_background_operators_preserve_surface_forms() {
    let source = "\
{ print body; } always { print cleanup; }
print quiet &|
print hidden &!
";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Always(command) = compound else {
        panic!("expected always compound command");
    };
    assert_eq!(command.body.len(), 1);
    assert_eq!(command.always_body.len(), 1);

    assert_eq!(
        output.file.body[1].terminator,
        Some(StmtTerminator::Background(BackgroundOperator::Pipe))
    );
    assert_eq!(
        output.file.body[2].terminator,
        Some(StmtTerminator::Background(BackgroundOperator::Bang))
    );
}
