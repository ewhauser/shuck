use super::*;

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
fn test_zsh_unbraced_array_access_parses_as_array_access() {
    let input = "print $Plugins[MY_PLUGIN_DIR]\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let reference = expect_array_access(&command.args[0]);
    assert_eq!(reference.name.as_str(), "Plugins");
    expect_subscript(reference, input, "MY_PLUGIN_DIR");
    assert_eq!(
        command.args[0].render_syntax(input),
        "$Plugins[MY_PLUGIN_DIR]"
    );
}

#[test]
fn test_zsh_unbraced_array_access_inside_double_quotes_stays_nested() {
    let source = "[[ -n \"$termcap[ku]\" ]]\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional command");
    };

    assert!(redirects.is_empty());
    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    let ConditionalExpr::Word(word) = unary.expr.as_ref() else {
        panic!("expected quoted word operand");
    };
    let [
        WordPartNode {
            kind: WordPart::DoubleQuoted { parts, .. },
            ..
        },
    ] = word.parts.as_slice()
    else {
        panic!("expected one double-quoted expansion, got {:?}", word.parts);
    };
    let [nested] = parts.as_slice() else {
        panic!("expected one nested part, got {parts:?}");
    };
    let reference = array_access_reference(&nested.kind).expect("expected nested array access");
    assert_eq!(reference.name.as_str(), "termcap");
    expect_subscript(reference, source, "ku");
    assert_eq!(word.render_syntax(source), "\"$termcap[ku]\"");
}

#[test]
fn test_zsh_unbraced_parameter_set_probe_array_access_parses_as_array_access() {
    let input = "print -r -- $+ice[extract]\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let reference = expect_array_access(&command.args[2]);
    assert_eq!(reference.name.as_str(), "+ice");
    expect_subscript(reference, input, "extract");
    assert_eq!(command.args[2].render_syntax(input), "$+ice[extract]");
}

#[test]
fn test_zsh_unbraced_parameter_set_probe_array_access_in_conditionals_stays_nested() {
    let source = "[[ $+ice[extract] -ne 0 ]]\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional command");
    };

    assert!(redirects.is_empty());
    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Word(left) = binary.left.as_ref() else {
        panic!("expected word lhs");
    };
    let reference = expect_array_access(left);
    assert_eq!(reference.name.as_str(), "+ice");
    expect_subscript(reference, source, "extract");
    assert_eq!(left.render_syntax(source), "$+ice[extract]");
}

#[test]
fn test_zsh_positional_parameter_subscripts_preserve_ordinary_subscripts() {
    let input = "print ${@[_i]} ${@[2,-1]:-fallback}\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let command = expect_simple(&script.body[0]);

    let reference = expect_array_access(&command.args[0]);
    let subscript = expect_subscript(reference, input, "_i");
    assert!(matches!(subscript.kind, SubscriptKind::Ordinary));
    assert_eq!(subscript.selector(), None);
    assert!(subscript.word_ast().is_some());

    let parameter = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        reference,
        operator,
        operand,
        colon_variant,
        ..
    }) = &parameter.syntax
    else {
        panic!("expected zsh positional parameter operation");
    };
    assert_eq!(reference.name.as_str(), "@");
    let subscript = reference
        .subscript
        .as_deref()
        .expect("expected positional subscript");
    assert!(matches!(subscript.kind, SubscriptKind::Ordinary));
    assert_eq!(subscript.selector(), None);
    assert_eq!(subscript.syntax_text(input), "2,-1");
    assert!(matches!(operator.as_ref(), ParameterOp::UseDefault));
    assert!(*colon_variant);
    assert_eq!(
        operand
            .as_ref()
            .expect("expected default operand")
            .slice(input),
        "fallback"
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
fn test_zsh_brace_ccl_marks_character_class_expansion_candidates() {
    let source = "print {ab} \"{ab}\" {a,b} {1..3}\n";
    let output = parse_zsh_with_options(source, |options| {
        options.brace_ccl = OptionValue::On;
    });

    let command = expect_simple(&output.file.body[0]);

    assert_eq!(brace_slices(&command.args[0], source), vec!["{ab}"]);
    assert_eq!(
        command.args[0].brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::CharacterClass)
    );
    assert!(command.args[0].has_active_brace_expansion());

    assert_eq!(brace_slices(&command.args[1], source), vec!["{ab}"]);
    assert_eq!(
        command.args[1].brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::CharacterClass)
    );
    assert_eq!(
        command.args[1].brace_syntax()[0].quote_context,
        BraceQuoteContext::DoubleQuoted
    );
    assert!(!command.args[1].has_active_brace_expansion());

    assert_eq!(
        command.args[2].brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList)
    );
    assert_eq!(
        command.args[3].brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::Sequence)
    );
}

#[test]
fn test_zsh_brace_ccl_ignores_quote_only_character_classes() {
    let source = "print {\"\"} {''} {$''} {\"a\"} {\\\"}\n";
    let output = parse_zsh_with_options(source, |options| {
        options.brace_ccl = OptionValue::On;
    });

    let command = expect_simple(&output.file.body[0]);

    for word in &command.args[..3] {
        assert!(!word.has_active_brace_expansion());
        assert!(word.brace_syntax().iter().all(|brace| {
            !matches!(
                brace.kind,
                BraceSyntaxKind::Expansion(BraceExpansionKind::CharacterClass)
            )
        }));
    }

    for word in &command.args[3..] {
        assert_eq!(
            word.brace_syntax()[0].kind,
            BraceSyntaxKind::Expansion(BraceExpansionKind::CharacterClass)
        );
        assert!(word.has_active_brace_expansion());
    }
}

#[test]
fn test_zsh_midfile_brace_ccl_toggles_brace_syntax_collection() {
    let source = "setopt brace_ccl\nprint {ab}\nunsetopt brace_ccl\nprint {cd}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let first = expect_simple(&output.file.body[1]);
    assert_eq!(brace_slices(&first.args[0], source), vec!["{ab}"]);
    assert_eq!(
        first.args[0].brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::CharacterClass)
    );
    assert!(first.args[0].has_active_brace_expansion());

    let second = expect_simple(&output.file.body[3]);
    assert_eq!(brace_slices(&second.args[0], source), vec!["{cd}"]);
    assert_eq!(
        second.args[0].brace_syntax()[0].kind,
        BraceSyntaxKind::Literal
    );
    assert!(!second.args[0].has_active_brace_expansion());
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
fn test_parse_zsh_arithmetic_command_keeps_parameter_set_probe_shell_words_intact() {
    let input = "(( $+ice[extract] ))\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };
    let expr = command.expr_ast.as_ref().expect("expected arithmetic AST");
    expect_shell_word(expr, input, "$+ice[extract]");
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
fn test_parse_zsh_integer_clause_classifies_assignment() {
    let input = "integer -g count=0 other\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    assert_eq!(command.variant, "integer");
    assert_eq!(command.operands.len(), 3);
    assert!(matches!(command.operands[0], DeclOperand::Flag(_)));

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand");
    };
    assert_eq!(assignment.target.name, "count");

    let DeclOperand::Name(name) = &command.operands[2] else {
        panic!("expected bare name operand");
    };
    assert_eq!(name.name, "other");
}

#[test]
fn test_parse_integer_stays_simple_command_outside_zsh() {
    let input = "integer count=0\n";
    let script = Parser::with_dialect(input, ShellDialect::Bash)
        .parse()
        .unwrap()
        .file;

    assert!(matches!(script.body[0].command, AstCommand::Simple(_)));
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
fn test_zsh_for_loop_preserves_paren_direct_syntax() {
    let source = "for topic_folder ($ZSH/*) if [ -d $topic_folder ]; then fpath=($topic_folder $fpath); fi\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert!(matches!(
        command.syntax,
        ForSyntax::ParenDirect {
            left_paren_span,
            right_paren_span,
        } if left_paren_span.slice(source) == "(" && right_paren_span.slice(source) == ")"
    ));
    assert_eq!(command.body.len(), 1);
    assert!(matches!(command.body[0].command, AstCommand::Compound(_)));
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
fn test_zsh_for_loop_preserves_in_direct_syntax() {
    let source = "for key in \"$key_info[Escape]\"{B,b} \"$key_info[Left]\"\n  bindkey -M emacs \"$key\" emacs-backward-word\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert!(matches!(
        command.syntax,
        ForSyntax::InDirect {
            in_span: Some(in_span),
        } if in_span.slice(source) == "in"
    ));
    assert_eq!(command.body.len(), 1);
    assert_eq!(
        expect_simple(&command.body[0]).name.render(source),
        "bindkey"
    );
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
fn test_zsh_upstream_word_surface_examples_parse() {
    for source in ["echo \\(foo\\)\n", "find . \\( -name \"*.zsh\" \\)\n"] {
        Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
    }

    let source = "diff =(sort file1) =(sort file2)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    assert_eq!(command.name.render(source), "diff");
    assert_eq!(command.args.len(), 2);
    assert_eq!(command.args[0].span.slice(source), "=(sort file1)");
    assert_eq!(command.args[1].span.slice(source), "=(sort file2)");
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
fn test_zsh_repeat_direct_preserves_structure_and_spans() {
    let source = "repeat 3 echo test\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::Repeat(command) = compound else {
        panic!("expected repeat command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.span.slice(source), source);
    assert_eq!(command.count.span.slice(source), "3");
    assert_eq!(command.body.len(), 1);

    match command.syntax {
        RepeatSyntax::Direct => {}
        RepeatSyntax::Brace { .. } => panic!("expected direct repeat syntax"),
        RepeatSyntax::DoDone { .. } => panic!("expected direct repeat syntax"),
    }

    let body_command = expect_simple(&command.body[0]);
    assert_eq!(body_command.name.render(source), "echo");
    assert_eq!(body_command.args[0].render(source), "test");
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
fn test_zsh_additional_upstream_declaration_examples_parse() {
    for source in [
        "array=(one two three)\n",
        "integer count=42\n",
        "float pi=3.14\n",
        "typeset -A hash\nhash=(key1 value1 key2 value2)\n",
        "local -a array\n",
        "0=${(%):-%N}\n",
    ] {
        Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
    }
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
fn test_zsh_setopt_and_unsetopt_change_following_word_parse() {
    let source = concat!(
        "print foo~bar\n",
        "setopt extended_glob\n",
        "print foo~bar\n",
        "unsetopt extended_glob\n",
        "print foo~bar\n",
    );
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let first = expect_simple(&output.file.body[0]);
    assert!(!matches!(
        first.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let second = expect_simple(&output.file.body[2]);
    assert!(matches!(
        second.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let third = expect_simple(&output.file.body[4]);
    assert!(!matches!(
        third.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));
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
fn test_zsh_brace_if_allows_brace_group_elif_conditions() {
    let source = "if [[ $profile == ./* || $profile == /* ]] {\n  local localpkg=1\n} elif { ! .zinit-download-file-stdout $URL 0 1 2>/dev/null > $tmpfile } {\n  command rm -f $tmpfile\n}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, _) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert_eq!(command.elif_branches.len(), 1);
    assert_eq!(command.elif_branches[0].0.len(), 1);
    assert!(matches!(
        command.elif_branches[0].0[0].command,
        AstCommand::Compound(_)
    ));
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
fn test_parse_zsh_deep_parameter_array_comma_stays_nested() {
    let source = "values=(${a:-${b:-${c:-${d:-${e:-x},y}}}})\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(array.elements.len(), 1);

    let ArrayElem::Sequential(value) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(
        value.span.slice(source),
        "${a:-${b:-${c:-${d:-${e:-x},y}}}}"
    );
}

#[test]
fn test_parse_zsh_deep_arithmetic_array_comma_stays_nested() {
    let mut nested = "$((a, b))".to_string();
    for _ in 0..5 {
        nested = format!("$(({nested}))");
    }
    let source = format!("values=({nested})\n");
    let output = Parser::with_dialect(&source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(array.elements.len(), 1);

    let ArrayElem::Sequential(value) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(value.span.slice(&source), nested);
}

#[test]
fn test_parse_zsh_deep_arithmetic_skips_nested_parameter_parens() {
    let mut nested = "$(( ${name:-1),2} ))".to_string();
    for _ in 0..5 {
        nested = format!("$(({nested}))");
    }
    let source = format!("values=({nested})\n");
    let output = Parser::with_dialect(&source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(array.elements.len(), 1);

    let ArrayElem::Sequential(value) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(value.span.slice(&source), nested);
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
