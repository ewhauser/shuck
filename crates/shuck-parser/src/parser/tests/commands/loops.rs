use super::*;

#[test]
fn test_parse_does_not_collect_zsh_brace_if_fact_for_condition_brace_group() {
    let input = "if true\n{ echo ok; }\nthen\n  :\nfi\n";
    let parsed = Parser::new(input).parse().unwrap();

    assert!(parsed.syntax_facts.zsh_brace_if_spans.is_empty());
}

#[test]
fn test_parse_does_not_collect_zsh_brace_if_fact_for_later_then_after_brace_group_condition() {
    let input = "if true; { echo ok; }; echo more; then\n  :\nfi\n";
    let parsed = Parser::new(input).parse().unwrap();

    assert!(parsed.syntax_facts.zsh_brace_if_spans.is_empty());
}

#[test]
fn test_disabled_repeat_probe_restores_checkpoint_when_newline_skip_errors() {
    let input = "repeat 3\ndo echo hi; done\n";
    let mut parser = Parser::with_fuel(input, 0);
    let original_span = parser.current_span();

    assert_eq!(parser.current_keyword(), Some(Keyword::Repeat));

    let error = parser.looks_like_disabled_repeat_loop().unwrap_err();

    assert!(matches!(
        error,
        Error::Parse { ref message, .. } if message.contains("parser fuel exhausted")
    ));
    assert_eq!(parser.current_keyword(), Some(Keyword::Repeat));
    assert_eq!(parser.current_span(), original_span);
}

#[test]
fn test_disabled_foreach_probe_restores_checkpoint_when_newline_skip_errors() {
    let input = "foreach item in a\ndo echo hi; done\n";
    let mut parser = Parser::with_fuel(input, 0);
    let original_span = parser.current_span();

    assert_eq!(parser.current_keyword(), Some(Keyword::Foreach));

    let error = parser.looks_like_disabled_foreach_loop().unwrap_err();

    assert!(matches!(
        error,
        Error::Parse { ref message, .. } if message.contains("parser fuel exhausted")
    ));
    assert_eq!(parser.current_keyword(), Some(Keyword::Foreach));
    assert_eq!(parser.current_span(), original_span);
}

#[cfg(feature = "benchmarking")]
#[test]
fn test_parse_with_benchmark_counters_is_repeatable() {
    let input = "echo hello\nprintf '%s' \"$x\"\n";

    let (first, first_counters) = Parser::new(input).parse_with_benchmark_counters();
    let (second, second_counters) = Parser::new(input).parse_with_benchmark_counters();
    let first = first.unwrap();
    let second = second.unwrap();

    assert_eq!(first.file.body.len(), second.file.body.len());
    assert_eq!(first.file.span, second.file.span);
    assert_eq!(first_counters, second_counters);
    assert!(first_counters.lexer_current_position_calls > 0);
    assert!(first_counters.parser_set_current_spanned_calls > 0);
    assert!(first_counters.parser_advance_raw_calls > 0);
}

#[test]
fn test_empty_while_body_rejected() {
    let parser = Parser::new("while true; do\ndone");
    assert!(
        parser.parse().is_err(),
        "empty while body should be rejected"
    );
}

#[test]
fn test_zsh_empty_while_body_is_allowed() {
    Parser::with_dialect("while sleep 1; do; done\n", ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_empty_for_body_rejected() {
    let parser = Parser::new("for i in 1 2 3; do\ndone");
    assert!(parser.parse().is_err(), "empty for body should be rejected");
}

#[test]
fn test_nonempty_while_body_accepted() {
    let parser = Parser::new("while true; do echo hi; done");
    assert!(
        parser.parse().is_ok(),
        "non-empty while body should be accepted"
    );
}

#[test]
fn test_background_terminator_before_loop_done_is_preserved() {
    let input = "for item in a; do\n  echo \"$item\" &\ndone\n";
    let script = Parser::new(input).parse().unwrap().file;
    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command.body[0].terminator,
        Some(StmtTerminator::Background(BackgroundOperator::Plain))
    );
    assert_eq!(command.body[0].terminator_span.unwrap().slice(input), "&");
}

/// Issue #600: Subscript reader must handle nested ${...} containing brackets.

#[test]
fn test_parse_arithmetic_for_preserves_header_spans() {
    let input = "for (( i = 0 ; i < 10 ; i += 2 )); do echo \"$i\"; done\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.init_span.unwrap().slice(input), " i = 0 ");
    assert_eq!(command.first_semicolon_span.slice(input), ";");
    assert_eq!(command.condition_span.unwrap().slice(input), " i < 10 ");
    assert_eq!(command.second_semicolon_span.slice(input), ";");
    assert_eq!(command.step_span.unwrap().slice(input), " i += 2 ");
    assert_eq!(command.right_paren_span.slice(input), "))");
    let ArithmeticExpr::Assignment {
        target,
        op: init_op,
        value: init_value,
    } = &command
        .init_ast
        .as_ref()
        .expect("expected init arithmetic AST")
        .kind
    else {
        panic!("expected assignment init expression");
    };
    assert_eq!(*init_op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable init target");
    };
    assert_eq!(name, "i");
    expect_number(init_value, input, "0");

    let ArithmeticExpr::Binary {
        left: condition_left,
        op: condition_op,
        right: condition_right,
    } = &command
        .condition_ast
        .as_ref()
        .expect("expected condition arithmetic AST")
        .kind
    else {
        panic!("expected binary condition expression");
    };
    assert_eq!(*condition_op, ArithmeticBinaryOp::LessThan);
    expect_variable(condition_left, "i");
    expect_number(condition_right, input, "10");

    let ArithmeticExpr::Assignment {
        target,
        op: step_op,
        value: step_value,
    } = &command
        .step_ast
        .as_ref()
        .expect("expected step arithmetic AST")
        .kind
    else {
        panic!("expected assignment step expression");
    };
    assert_eq!(*step_op, ArithmeticAssignOp::AddAssign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable step target");
    };
    assert_eq!(name, "i");
    expect_number(step_value, input, "2");
}

#[test]
fn test_parse_arithmetic_for_with_nested_double_parens_in_segments() {
    let input = "for (( x = ((1 + 2) * (3 - 4)); y < ((5 + 6) * (7 - 8)); z = ((9 + 10) * (11 - 12)) )); do :; done\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(
        command.init_span.unwrap().slice(input),
        " x = ((1 + 2) * (3 - 4))"
    );
    assert_eq!(
        command.condition_span.unwrap().slice(input),
        " y < ((5 + 6) * (7 - 8))"
    );
    assert_eq!(
        command.step_span.unwrap().slice(input),
        " z = ((9 + 10) * (11 - 12)) "
    );

    let ArithmeticExpr::Assignment { target, op, value } = &command
        .init_ast
        .as_ref()
        .expect("expected init arithmetic AST")
        .kind
    else {
        panic!("expected assignment init expression");
    };
    assert_eq!(*op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable init target");
    };
    assert_eq!(name, "x");
    assert!(matches!(value.kind, ArithmeticExpr::Parenthesized { .. }));

    let ArithmeticExpr::Binary {
        left: condition_left,
        op: condition_op,
        right: condition_right,
    } = &command
        .condition_ast
        .as_ref()
        .expect("expected condition arithmetic AST")
        .kind
    else {
        panic!("expected binary condition expression");
    };
    assert_eq!(*condition_op, ArithmeticBinaryOp::LessThan);
    expect_variable(condition_left, "y");
    assert!(matches!(
        condition_right.kind,
        ArithmeticExpr::Parenthesized { .. }
    ));

    let ArithmeticExpr::Assignment {
        target,
        op: step_op,
        value: step_value,
    } = &command
        .step_ast
        .as_ref()
        .expect("expected step arithmetic AST")
        .kind
    else {
        panic!("expected assignment step expression");
    };
    assert_eq!(*step_op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable step target");
    };
    assert_eq!(name, "z");
    assert!(matches!(
        step_value.kind,
        ArithmeticExpr::Parenthesized { .. }
    ));
}

#[test]
fn test_parse_arithmetic_for_preserves_compact_header_spans() {
    let input = "for ((i=0;i<10;i++)) do echo \"$i\"; done\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.init_span.unwrap().slice(input), "i=0");
    assert_eq!(command.first_semicolon_span.slice(input), ";");
    assert_eq!(command.condition_span.unwrap().slice(input), "i<10");
    assert_eq!(command.second_semicolon_span.slice(input), ";");
    assert_eq!(command.step_span.unwrap().slice(input), "i++");
    assert_eq!(command.right_paren_span.slice(input), "))");
}

#[test]
fn test_parse_arithmetic_for_allows_all_empty_segments() {
    let input = "for ((;;)); do foo; done\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert!(command.init_span.is_none());
    assert_eq!(command.first_semicolon_span.slice(input), ";");
    assert!(command.condition_span.is_none());
    assert_eq!(command.second_semicolon_span.slice(input), ";");
    assert!(command.step_span.is_none());
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert!(command.init_ast.is_none());
    assert!(command.condition_ast.is_none());
    assert!(command.step_ast.is_none());
}

#[test]
fn test_parse_arithmetic_for_allows_only_init_segment() {
    let input = "for ((i = 0;;)); do foo; done\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.init_span.unwrap().slice(input), "i = 0");
    assert_eq!(command.first_semicolon_span.slice(input), ";");
    assert!(command.condition_span.is_none());
    assert_eq!(command.second_semicolon_span.slice(input), ";");
    assert!(command.step_span.is_none());
    assert_eq!(command.right_paren_span.slice(input), "))");
}

#[test]
fn test_parse_arithmetic_for_with_nested_parens_before_outer_close() {
    let input = "for (( i = 0 ; i < 10 ; i += ($# - 1))); do echo \"$i\"; done\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.init_span.unwrap().slice(input), " i = 0 ");
    assert_eq!(command.first_semicolon_span.slice(input), ";");
    assert_eq!(command.condition_span.unwrap().slice(input), " i < 10 ");
    assert_eq!(command.second_semicolon_span.slice(input), ";");
    assert_eq!(command.step_span.unwrap().slice(input), " i += ($# - 1)");
    assert_eq!(command.right_paren_span.slice(input), "))");
}

#[test]
fn test_parse_arithmetic_for_treats_less_than_left_paren_as_arithmetic() {
    let input = "for (( n=0; n<(3-(1)); n++ )) ; do echo $n; done\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.condition_span.unwrap().slice(input), " n<(3-(1))");
}

#[test]
fn test_parse_arithmetic_for_treats_spaced_less_than_left_paren_as_arithmetic() {
    let input = "for (( n=0; n<(3- (1)); n++ )) ; do echo $n; done\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.condition_span.unwrap().slice(input), " n<(3- (1))");
}

#[test]
fn test_parse_arithmetic_for_accepts_brace_group_body() {
    let input = "for ((a=1; a <= 3; a++)) {\n  echo $a\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.body.len(), 1);

    let (body_compound, body_redirects) = expect_compound(&command.body[0]);
    let AstCompoundCommand::BraceGroup(body) = body_compound else {
        panic!("expected brace-group loop body");
    };
    assert!(body_redirects.is_empty());
    assert_eq!(body.len(), 1);
}

#[test]
fn test_parse_zsh_while_body_with_arithmetic_command_and_and_list() {
    let input =
        "while true; do\n  sysread -s1 c || return\n  (( #c < 256 / $1 * $1 )) && break\n done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_while_with_char_literal_arithmetic_and_following_command() {
    let input = "while true; do\n  sysread -s1 c || return\n  (( #c < 256 / $1 * $1 )) && break\n done\n typeset -g REPLY=$((#c % $1 + 1))\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_while_condition_keeps_brace_group_before_do_boundary() {
    let input = "while cmd1; { cmd2; }; do cmd3; done\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::While(command) = compound else {
        panic!("expected while loop");
    };
    assert!(redirects.is_empty());
    assert_eq!(command.condition.len(), 2);
    assert_eq!(command.body.len(), 1);

    assert_eq!(
        expect_simple(&command.condition[0]).name.render(input),
        "cmd1"
    );

    let (group, group_redirects) = expect_compound(&command.condition[1]);
    let AstCompoundCommand::BraceGroup(condition_body) = group else {
        panic!("expected brace group in while condition");
    };
    assert!(group_redirects.is_empty());
    assert_eq!(condition_body.len(), 1);
    assert_eq!(expect_simple(&condition_body[0]).name.render(input), "cmd2");

    assert_eq!(expect_simple(&command.body[0]).name.render(input), "cmd3");
}

#[test]
fn test_parse_zsh_repeat_body_with_bitwise_or_char_literal() {
    let input = "repeat 4; do\n  sysread -s1 c || return\n  (( rnd = (~(1 << 23) & rnd) << 8 | #c ))\n done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_nested_repeat_with_bitwise_or_char_literal() {
    let input = "while true; do\n  local -i rnd=0\n  repeat 4; do\n    sysread -s1 c || return\n    (( rnd = (~(1 << 23) & rnd) << 8 | #c ))\n  done\n  break\ndone\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_for_loop_with_empty_body_and_expansion_word_list() {
    let input = "for foo bar baz in \"${(@)resp[3,29]}\"; do\n done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_for_loop_with_newline_before_in_clause() {
    let input = "for foo bar baz\nin \"${(@0)1}\"; do done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_for_loop_with_glob_qualified_word_list_item() {
    let input =
        "for plugin in $root/plugins/[^[:space:]]##(/N); do\n  print -r -- $plugin\n done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_redirect_prefixed_while_loop() {
    let input = "<$SCD_IGNORE while read p; do\n  print -r -- $p\n done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_while_loop_with_brace_body() {
    let input = "while (( -- count + 1 )) {\n  echo hi\n}\n";
    let output = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::While(command) = compound else {
        panic!("expected while loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.body.len(), 1);
    let (body_compound, body_redirects) = expect_compound(&command.body[0]);
    let AstCompoundCommand::BraceGroup(body) = body_compound else {
        panic!("expected brace-group loop body");
    };
    assert!(body_redirects.is_empty());
    assert_eq!(body.len(), 1);
}

#[test]
fn test_parse_zsh_while_loop_with_brace_body_after_setopt_noshortloops() {
    let input = "f() {\n  setopt noshortloops\n  while (( count )) {\n    break\n  }\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_for_loop_over_parameter_keys() {
    let input = "f() {\n  local -A opts\n  for opt in ${(k)opts}; do\n    case $opt in\n      -l) output_format='list' ;;\n    esac\n  done\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_until_loop_with_nested_parameter_lengths() {
    let input = "f() {\n  local cd=/foo/bar/foo/bar q='bar' q_chars=1\n  until (( ( ${#cd:h} - ${#${${cd:h}//${~q}/}} ) != q_chars )); do\n    cd=${cd:h}\n  done\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_prefix_match_preserves_selector_kind() {
    let input = "printf '%s\\n' \"${!prefix@}\" \"${!prefix*}\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let first = &command.args[1];
    let second = &command.args[2];

    let [first_part] = first.parts.as_slice() else {
        panic!("expected quoted prefix match");
    };
    let WordPart::DoubleQuoted {
        parts: first_inner, ..
    } = &first_part.kind
    else {
        panic!("expected double-quoted prefix match");
    };
    let (prefix, kind) = expect_prefix_match_part(&first_inner[0].kind);
    assert_eq!(prefix.as_str(), "prefix");
    assert_eq!(kind, PrefixMatchKind::At);

    let [second_part] = second.parts.as_slice() else {
        panic!("expected quoted prefix match");
    };
    let WordPart::DoubleQuoted {
        parts: second_inner,
        ..
    } = &second_part.kind
    else {
        panic!("expected double-quoted prefix match");
    };
    let (prefix, kind) = expect_prefix_match_part(&second_inner[0].kind);
    assert_eq!(prefix.as_str(), "prefix");
    assert_eq!(kind, PrefixMatchKind::Star);
    assert_eq!(first.render_syntax(input), "\"${!prefix@}\"");
    assert_eq!(second.render_syntax(input), "\"${!prefix*}\"");
}

#[test]
fn test_non_bash_dialects_reject_c_style_for_loops() {
    let error = Parser::with_dialect(
        "for ((i=0; i<2; i++)); do echo hi; done\n",
        ShellDialect::Mksh,
    )
    .parse()
    .unwrap_err();

    assert!(matches!(
        error,
        Error::Parse { message, .. } if message.contains("c-style for loops")
    ));
}
