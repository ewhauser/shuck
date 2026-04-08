use super::*;

#[test]
fn test_parse_simple_command() {
    let input = "echo hello";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    assert_eq!(script.body.len(), 1);

    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.name.render(input), "echo");
        assert_eq!(cmd.args.len(), 1);
        assert_eq!(cmd.args[0].render(input), "hello");
    } else {
        panic!("expected simple command");
    }
}

#[test]
fn test_parse_break_as_typed_builtin() {
    let input = "break 2";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Builtin(AstBuiltinCommand::Break(command)) = &script.body[0].command else {
        panic!("expected break builtin");
    };

    assert_eq!(command.depth.as_ref().unwrap().render(input), "2");
    assert!(command.extra_args.is_empty());
}

#[test]
fn test_parse_continue_preserves_extra_args() {
    let input = "continue 1 extra";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Builtin(AstBuiltinCommand::Continue(command)) = &script.body[0].command else {
        panic!("expected continue builtin");
    };

    assert_eq!(command.depth.as_ref().unwrap().render(input), "1");
    assert_eq!(command.extra_args.len(), 1);
    assert_eq!(command.extra_args[0].render(input), "extra");
}

#[test]
fn test_parse_exit_as_typed_builtin() {
    let input = "exit 1";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Builtin(AstBuiltinCommand::Exit(command)) = &script.body[0].command else {
        panic!("expected exit builtin");
    };

    assert_eq!(command.code.as_ref().unwrap().render(input), "1");
    assert!(command.extra_args.is_empty());
}

#[test]
fn test_parse_multiple_args() {
    let input = "echo hello world";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.name.render(input), "echo");
        assert_eq!(cmd.args.len(), 2);
        assert_eq!(cmd.args[0].render(input), "hello");
        assert_eq!(cmd.args[1].render(input), "world");
    } else {
        panic!("expected simple command");
    }
}

#[test]
fn test_unexpected_top_level_token_errors_in_strict_mode() {
    let error = Parser::new("echo ok\n)\necho later\n").parse().unwrap_err();

    let Error::Parse {
        message,
        line,
        column,
    } = error;
    assert_eq!(message, "expected command");
    assert_eq!(line, 2);
    assert_eq!(column, 1);
}

#[test]
fn test_parse_recovered_skips_invalid_command_and_continues() {
    let input = "echo one\ncat >\necho two\n";
    let recovered = Parser::new(input).parse_recovered();

    assert_eq!(recovered.file.body.len(), 2);
    assert_eq!(recovered.diagnostics.len(), 1);
    assert_eq!(recovered.diagnostics[0].message, "expected word");
    assert_eq!(recovered.diagnostics[0].span.start.line, 2);

    let first = expect_simple(&recovered.file.body[0]);
    assert_eq!(first.name.render(input), "echo");
    assert_eq!(first.args[0].render(input), "one");

    let second = expect_simple(&recovered.file.body[1]);
    assert_eq!(second.name.render(input), "echo");
    assert_eq!(second.args[0].render(input), "two");
}

#[test]
fn test_parse_pipeline() {
    let parser = Parser::new("echo hello | cat");
    let script = parser.parse().unwrap().file;

    assert_eq!(script.body.len(), 1);
    let pipeline = expect_binary(&script.body[0]);
    assert_eq!(pipeline.op, BinaryOp::Pipe);
    assert_eq!(
        expect_simple(&pipeline.left)
            .name
            .render("echo hello | cat"),
        "echo"
    );
    assert_eq!(
        expect_simple(&pipeline.right)
            .name
            .render("echo hello | cat"),
        "cat"
    );
}

#[test]
fn test_parse_pipe_both_pipeline() {
    let input = "echo hello |& cat";
    let script = Parser::new(input).parse().unwrap().file;

    let pipeline = expect_binary(&script.body[0]);
    assert_eq!(pipeline.op, BinaryOp::PipeAll);
    assert_eq!(expect_simple(&pipeline.left).name.render(input), "echo");
    assert_eq!(expect_simple(&pipeline.right).name.render(input), "cat");
}

#[test]
fn test_parse_command_list_and() {
    let parser = Parser::new("true && echo success");
    let script = parser.parse().unwrap().file;

    assert_eq!(expect_binary(&script.body[0]).op, BinaryOp::And);
}

#[test]
fn test_parse_command_list_or() {
    let parser = Parser::new("false || echo fallback");
    let script = parser.parse().unwrap().file;

    assert_eq!(expect_binary(&script.body[0]).op, BinaryOp::Or);
}

#[test]
fn test_parse_command_list_preserves_operator_spans() {
    let input = "true && false || echo fallback";
    let script = Parser::new(input).parse().unwrap().file;

    let outer = expect_binary(&script.body[0]);
    assert_eq!(outer.op, BinaryOp::Or);
    assert_eq!(outer.op_span.slice(input), "||");
    let inner = expect_binary(&outer.left);
    assert_eq!(inner.op, BinaryOp::And);
    assert_eq!(inner.op_span.slice(input), "&&");
}

#[test]
fn test_posix_function_with_brace_group_preserves_surface_form() {
    let input = "inc() { :; }\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());

    assert!(!function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert_eq!(function.header.function_keyword_span, None);
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
fn test_posix_function_allows_subshell_body() {
    let input = "inc_subshell() ( j=$((j+5)); )\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::Subshell(body) = compound else {
        panic!("expected subshell function body");
    };
    assert!(!function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
}

#[test]
fn test_posix_function_allows_conditional_body() {
    let input = "f() [[ -n \"$x\" ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional function body");
    };

    assert!(!function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert!(redirects.is_empty());
    assert_eq!(command.span.slice(input), "[[ -n \"$x\" ]]");
}

#[test]
fn test_posix_function_allows_arithmetic_body() {
    let input = "f() (( x + 1 ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic function body");
    };

    assert!(!function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert!(redirects.is_empty());
    assert_eq!(command.span.slice(input), "(( x + 1 ))");
}

#[test]
fn test_posix_function_allows_if_body() {
    let input = "f() if true; then :; fi\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    assert!(matches!(compound, AstCompoundCommand::If(_)));

    assert!(!function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert!(redirects.is_empty());
}

#[test]
fn test_function_body_rejects_simple_command() {
    let parser = Parser::new("f() echo hi\n");
    assert!(
        parser.parse().is_err(),
        "simple command should not be accepted as a function body"
    );
}

#[test]
fn test_function_body_rejects_time_command() {
    let parser = Parser::new("f() time { :; }\n");
    assert!(
        parser.parse().is_err(),
        "time command should not be accepted as a function body"
    );
}

#[test]
fn test_function_body_rejects_coproc_command() {
    let parser = Parser::new("f() coproc cat\n");
    assert!(
        parser.parse().is_err(),
        "coproc command should not be accepted as a function body"
    );
}

#[test]
fn test_empty_function_body_rejected() {
    let parser = Parser::new("f() { }");
    assert!(
        parser.parse().is_err(),
        "empty function body should be rejected"
    );
}

#[test]
fn test_zsh_posix_function_allows_empty_compact_brace_body() {
    let input = "f() {}\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(redirects.is_empty());
    assert!(body.is_empty());
}

#[test]
fn test_zsh_posix_function_allows_compact_same_line_brace_body() {
    let input = "f() { echo hi }\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
    let command = expect_simple(&body[0]);
    assert_eq!(command.name.render(input), "echo");
    assert_eq!(command.args[0].render(input), "hi");
}

#[test]
fn test_zsh_posix_function_allows_compact_brace_body_without_space_after_left_brace() {
    let input = "f() {echo hi }\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
    let command = expect_simple(&body[0]);
    assert_eq!(command.name.render(input), "echo");
    assert_eq!(command.args[0].render(input), "hi");
}

#[test]
fn test_non_zsh_dialects_reject_compact_posix_function_brace_bodies() {
    for dialect in [ShellDialect::Posix, ShellDialect::Mksh, ShellDialect::Bash] {
        assert!(
            Parser::with_dialect("f() { echo hi }\n", dialect)
                .parse()
                .is_err(),
            "expected {dialect:?} to reject compact same-line brace body",
        );
        assert!(
            Parser::with_dialect("f() {}\n", dialect).parse().is_err(),
            "expected {dialect:?} to reject compact empty brace body",
        );
    }
}

#[test]
fn test_zsh_function_keyword_allows_simple_body_on_following_line() {
    let input = "function a\nprint -- BODY\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    assert!(function.uses_function_keyword());
    assert!(!function.has_trailing_parens());
    assert_eq!(function.header.entries.len(), 1);
    assert_eq!(
        function
            .header
            .static_names()
            .map(Name::as_str)
            .collect::<Vec<_>>(),
        vec!["a"]
    );

    let AstCommand::Simple(command) = &function.body.command else {
        panic!("expected simple command body");
    };
    assert_eq!(command.name.render(input), "print");
    assert_eq!(
        command
            .args
            .iter()
            .map(|arg| arg.render(input))
            .collect::<Vec<_>>(),
        vec!["--", "BODY"]
    );
}

#[test]
fn test_zsh_posix_function_allows_simple_body_on_same_line() {
    let input = "a() print -- BODY\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    assert!(!function.uses_function_keyword());
    assert!(function.has_trailing_parens());
    assert_eq!(
        function
            .header
            .static_names()
            .map(Name::as_str)
            .collect::<Vec<_>>(),
        vec!["a"]
    );

    let AstCommand::Simple(command) = &function.body.command else {
        panic!("expected simple command body");
    };
    assert_eq!(command.name.render(input), "print");
    assert_eq!(
        command
            .args
            .iter()
            .map(|arg| arg.render(input))
            .collect::<Vec<_>>(),
        vec!["--", "BODY"]
    );
}

#[test]
fn test_zsh_function_keyword_preserves_multi_name_header_with_trailing_parens() {
    let input = "function music itunes() { print -- hi; }\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    assert!(function.uses_function_keyword());
    assert!(function.has_trailing_parens());
    assert_eq!(function.header.entries.len(), 2);
    assert_eq!(
        function
            .header
            .static_names()
            .map(Name::as_str)
            .collect::<Vec<_>>(),
        vec!["music", "itunes"]
    );
    assert_eq!(
        function
            .header
            .entries
            .iter()
            .map(|entry| entry.word.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["music", "itunes"]
    );

    let (compound, redirects) = expect_compound(function.body.as_ref());
    assert!(redirects.is_empty());
    assert!(matches!(compound, AstCompoundCommand::BraceGroup(_)));
}

#[test]
fn test_zsh_function_keyword_preserves_multi_name_stub_body() {
    let input = "function foo bar\nreturn 1\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    assert_eq!(
        function
            .header
            .static_names()
            .map(Name::as_str)
            .collect::<Vec<_>>(),
        vec!["foo", "bar"]
    );

    let AstCommand::Builtin(AstBuiltinCommand::Return(command)) = &function.body.command else {
        panic!("expected return body");
    };
    assert_eq!(
        command
            .code
            .as_ref()
            .expect("expected return code")
            .render(input),
        "1"
    );
}

#[test]
fn test_zsh_function_keyword_allows_nameless_anonymous_function_command() {
    let input = "function { local x=1; print -- anon:$#; } a b\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_anonymous_function(&script.body[0]);
    assert!(function.uses_function_keyword());
    assert_eq!(
        function
            .args
            .iter()
            .map(|arg| arg.render(input))
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );

    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group anonymous function body");
    };
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 2);
}

#[test]
fn test_zsh_paren_anonymous_function_command_keeps_invocation_args() {
    let input = "() { print -- anon:$#; } a b\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_anonymous_function(&script.body[0]);
    assert!(!function.uses_function_keyword());
    assert_eq!(
        function
            .args
            .iter()
            .map(|arg| arg.render(input))
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );

    let (compound, redirects) = expect_compound(function.body.as_ref());
    assert!(redirects.is_empty());
    assert!(matches!(compound, AstCompoundCommand::BraceGroup(_)));
}

#[test]
fn test_zsh_function_keyword_accepts_punctuated_literal_names() {
    let input = "function cfh.() { :; }\nfunction cfh~() { :; }\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let first = expect_function(&script.body[0]);
    let second = expect_function(&script.body[1]);
    assert_eq!(
        first
            .header
            .static_names()
            .map(Name::as_str)
            .collect::<Vec<_>>(),
        vec!["cfh."]
    );
    assert_eq!(
        second
            .header
            .static_names()
            .map(Name::as_str)
            .collect::<Vec<_>>(),
        vec!["cfh~"]
    );
}

#[test]
fn test_zsh_function_keyword_preserves_dynamic_header_word_without_static_name() {
    let input = "function $0_error() { :; }\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    assert!(function.uses_function_keyword());
    assert!(function.has_trailing_parens());
    assert!(function.static_names().next().is_none());
    assert_eq!(function.header.entries.len(), 1);
    assert_eq!(
        function.header.entries[0].word.render_syntax(input),
        "$0_error"
    );
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
fn test_empty_for_body_rejected() {
    let parser = Parser::new("for i in 1 2 3; do\ndone");
    assert!(parser.parse().is_err(), "empty for body should be rejected");
}

#[test]
fn test_empty_if_then_rejected() {
    let parser = Parser::new("if true; then\nfi");
    assert!(
        parser.parse().is_err(),
        "empty then clause should be rejected"
    );
}

#[test]
fn test_empty_else_rejected() {
    let parser = Parser::new("if false; then echo yes; else\nfi");
    assert!(
        parser.parse().is_err(),
        "empty else clause should be rejected"
    );
}

#[test]
fn test_nonempty_function_body_accepted() {
    let parser = Parser::new("f() { echo hi; }");
    assert!(
        parser.parse().is_ok(),
        "non-empty function body should be accepted"
    );
}

#[test]
fn test_zsh_and_or_brace_groups_allow_same_line_closing_brace() {
    let input = "true && { echo yes } || { echo no }\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let outer = expect_binary(&script.body[0]);
    assert_eq!(outer.op, BinaryOp::Or);
    let inner = expect_binary(&outer.left);
    assert_eq!(inner.op, BinaryOp::And);

    let (yes_group, yes_redirects) = expect_compound(&inner.right);
    let AstCompoundCommand::BraceGroup(yes_body) = yes_group else {
        panic!("expected brace group on right side of &&");
    };
    assert!(yes_redirects.is_empty());
    assert_eq!(expect_simple(&yes_body[0]).name.render(input), "echo");

    let (no_group, no_redirects) = expect_compound(&outer.right);
    let AstCompoundCommand::BraceGroup(no_body) = no_group else {
        panic!("expected brace group on right side of ||");
    };
    assert!(no_redirects.is_empty());
    assert_eq!(expect_simple(&no_body[0]).name.render(input), "echo");
}

#[test]
fn test_zsh_brace_group_command_can_use_right_brace_as_literal_argument_before_closer() {
    let source = "rbrace() { echo }; }; rbrace\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let function = expect_function(&output.file.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
    let command = expect_simple(&body[0]);
    assert_eq!(command.name.render(source), "echo");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(source), "}");
}

#[test]
fn test_nonempty_while_body_accepted() {
    let parser = Parser::new("while true; do echo hi; done");
    assert!(
        parser.parse().is_ok(),
        "non-empty while body should be accepted"
    );
}

/// Issue #600: Subscript reader must handle nested ${...} containing brackets.

#[test]
fn test_parse_arithmetic_command_preserves_exact_spans() {
    let input = "(( 1 +\n 2 <= 3 ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(command.expr_span.unwrap().slice(input), " 1 +\n 2 <= 3 ");
    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::LessThanOrEqual);
    let ArithmeticExpr::Binary {
        left: add_left,
        op: add_op,
        right: add_right,
    } = &left.kind
    else {
        panic!("expected additive left operand");
    };
    assert_eq!(*add_op, ArithmeticBinaryOp::Add);
    expect_number(add_left, input, "1");
    expect_number(add_right, input, "2");
    expect_number(right, input, "3");
}

#[test]
fn test_parse_empty_arithmetic_command_keeps_span_without_typed_ast() {
    let input = "((   ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.expr_span.unwrap().slice(input), "   ");
    assert!(command.expr_ast.is_none());
}

#[test]
fn test_parse_arithmetic_command_with_nested_parens_and_double_right_paren() {
    let input = "(( (previous_pipe_index > 0) && (previous_pipe_index == ($# - 1)) ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(
        command.expr_span.unwrap().slice(input),
        " (previous_pipe_index > 0) && (previous_pipe_index == ($# - 1)) "
    );
}

#[test]
fn test_parse_arithmetic_command_with_nested_parens_before_outer_close() {
    let input = "(( a <= (1 || 2)))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(command.expr_span.unwrap().slice(input), " a <= (1 || 2)");
}

#[test]
fn test_parse_arithmetic_command_with_nested_double_parens_and_grouping() {
    let input = "(( x = ((1 + 2) * (3 - 4)) ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(
        command.expr_span.unwrap().slice(input),
        " x = ((1 + 2) * (3 - 4)) "
    );

    let ArithmeticExpr::Assignment { target, op, value } = &command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST")
        .kind
    else {
        panic!("expected arithmetic assignment");
    };
    assert_eq!(*op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable assignment target");
    };
    assert_eq!(name, "x");
    assert!(matches!(value.kind, ArithmeticExpr::Parenthesized { .. }));
}

#[test]
fn test_parse_arithmetic_command_respects_precedence_and_associativity() {
    let input = "(( a + b * c ** d ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary {
        left,
        op: add_op,
        right,
    } = &expr.kind
    else {
        panic!("expected additive expression");
    };
    assert_eq!(*add_op, ArithmeticBinaryOp::Add);
    expect_variable(left, "a");

    let ArithmeticExpr::Binary {
        left: mul_left,
        op: mul_op,
        right: mul_right,
    } = &right.kind
    else {
        panic!("expected multiplicative expression");
    };
    assert_eq!(*mul_op, ArithmeticBinaryOp::Multiply);
    expect_variable(mul_left, "b");

    let ArithmeticExpr::Binary {
        left: pow_left,
        op: pow_op,
        right: pow_right,
    } = &mul_right.kind
    else {
        panic!("expected power expression");
    };
    assert_eq!(*pow_op, ArithmeticBinaryOp::Power);
    expect_variable(pow_left, "c");
    expect_variable(pow_right, "d");
}

#[test]
fn test_parse_arithmetic_command_parses_updates_ternary_and_comma() {
    let input = "(( ++i ? j-- : (k = 1), m ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary {
        left,
        op: comma_op,
        right,
    } = &expr.kind
    else {
        panic!("expected comma expression");
    };
    assert_eq!(*comma_op, ArithmeticBinaryOp::Comma);
    expect_variable(right, "m");

    let ArithmeticExpr::Conditional {
        condition,
        then_expr,
        else_expr,
    } = &left.kind
    else {
        panic!("expected conditional expression");
    };

    let ArithmeticExpr::Unary { op: unary_op, expr } = &condition.kind else {
        panic!("expected prefix update condition");
    };
    assert_eq!(*unary_op, ArithmeticUnaryOp::PreIncrement);
    expect_variable(expr, "i");

    let ArithmeticExpr::Postfix {
        expr,
        op: postfix_op,
    } = &then_expr.kind
    else {
        panic!("expected postfix update in then branch");
    };
    assert_eq!(*postfix_op, ArithmeticPostfixOp::Decrement);
    expect_variable(expr, "j");

    let ArithmeticExpr::Parenthesized { expression } = &else_expr.kind else {
        panic!("expected parenthesized else branch");
    };
    let ArithmeticExpr::Assignment { target, op, value } = &expression.kind else {
        panic!("expected assignment inside else branch");
    };
    assert_eq!(*op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable else target");
    };
    assert_eq!(name, "k");
    expect_number(value, input, "1");
}

#[test]
fn test_double_left_paren_command_closed_with_spaced_right_parens_parses_as_subshells() {
    let input = "(( echo 1\necho 2\n(( x ))\n: $(( x ))\necho 3\n) )\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Subshell(commands) = compound else {
        panic!("expected outer subshell");
    };
    assert!(redirects.is_empty());
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0].command,
        AstCommand::Compound(AstCompoundCommand::Subshell(_))
    ));
}

#[test]
fn test_double_left_paren_test_clause_parses_as_command() {
    let input =
        "if ! ((test x\\\"$i\\\" = x-g) || (test x\\\"$i\\\" = x-O2)); then\n  echo bye\nfi\n";
    Parser::new(input).parse().unwrap();
}

#[test]
fn test_double_left_paren_pipeline_parses_as_command() {
    let input = "((cat </dev/zero; echo $? >&7) | true) 7>&1\n";
    Parser::new(input).parse().unwrap();
}

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
fn test_case_patterns_consume_segmented_tokens_directly() {
    let input = "case $x in foo\"bar\"|'baz'qux) echo hi ;; esac";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let patterns = &command.cases[0].patterns;
    assert_eq!(patterns.len(), 2);

    assert_eq!(patterns[0].render(input), "foobar");
    assert_eq!(patterns[0].parts.len(), 2);
    assert_eq!(
        pattern_part_slices(&patterns[0], input),
        vec!["foo", "\"bar\""]
    );
    assert!(matches!(
        &patterns[0].parts[1].kind,
        PatternPart::Word(word) if is_fully_quoted(word)
    ));

    assert_eq!(patterns[1].render(input), "bazqux");
    assert_eq!(patterns[1].parts.len(), 2);
    assert_eq!(
        pattern_part_slices(&patterns[1], input),
        vec!["'baz'", "qux"]
    );
    assert!(matches!(
        &patterns[1].parts[0].kind,
        PatternPart::Word(word) if is_fully_quoted(word)
    ));
}

#[test]
fn test_parse_conditional_builds_structured_logical_ast() {
    let script = Parser::new("[[ ! (foo && bar) ]]\n").parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    assert_eq!(unary.op, ConditionalUnaryOp::Not);

    let ConditionalExpr::Parenthesized(paren) = unary.expr.as_ref() else {
        panic!("expected parenthesized conditional");
    };
    let ConditionalExpr::Binary(binary) = paren.expr.as_ref() else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::And);
    assert!(matches!(binary.left.as_ref(), ConditionalExpr::Word(_)));
    assert!(matches!(binary.right.as_ref(), ConditionalExpr::Word(_)));
    assert_eq!(command.left_bracket_span.start.column, 1);
    assert_eq!(command.right_bracket_span.start.column, 19);
}

#[test]
fn test_parse_conditional_accepts_nested_grouping_with_double_parens() {
    let input = "[[ ! -e \"$cache\" && (( -e \"$prefix/n\" && ! -w \"$prefix/n\" ) || ( ! -e \"$prefix/n\" && ! -w \"$prefix\" )) ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::And);

    let ConditionalExpr::Parenthesized(paren) = binary.right.as_ref() else {
        panic!("expected parenthesized conditional term");
    };
    assert_eq!(
        paren.span().slice(input),
        "(( -e \"$prefix/n\" && ! -w \"$prefix/n\" ) || ( ! -e \"$prefix/n\" && ! -w \"$prefix\" ))"
    );

    let ConditionalExpr::Binary(inner) = paren.expr.as_ref() else {
        panic!("expected grouped binary conditional");
    };
    assert_eq!(inner.op, ConditionalBinaryOp::Or);
    assert!(matches!(
        inner.left.as_ref(),
        ConditionalExpr::Parenthesized(_)
    ));
    assert!(matches!(
        inner.right.as_ref(),
        ConditionalExpr::Parenthesized(_)
    ));
}

#[test]
fn test_parse_conditional_pattern_rhs_preserves_structure() {
    let input = "[[ foo == @(bar|baz)* ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::PatternEq);

    let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(pattern.render(input), "@(bar|baz)*");
    assert!(matches!(
        &pattern.parts[0].kind,
        PatternPart::Group {
            kind: PatternGroupKind::ExactlyOne,
            ..
        }
    ));
    assert!(matches!(&pattern.parts[1].kind, PatternPart::AnyString));
}

#[test]
fn test_parse_conditional_var_ref_operand() {
    let input = "[[ -v assoc[$key] ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    assert_eq!(unary.op, ConditionalUnaryOp::VariableSet);

    let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
        panic!("expected typed var-ref operand");
    };
    assert_eq!(var_ref.name.as_str(), "assoc");
    assert_eq!(var_ref.name_span.slice(input), "assoc");
    expect_subscript(var_ref, input, "$key");
}

#[test]
fn test_parse_conditional_regex_rhs_preserves_structure() {
    let input = "[[ foo =~ [ab](c|d) ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::RegexMatch);

    let ConditionalExpr::Regex(word) = binary.right.as_ref() else {
        panic!("expected regex rhs");
    };
    assert_eq!(word.render(input), "[ab](c|d)");
}

#[test]
fn test_parse_conditional_regex_rhs_with_double_left_paren_groups() {
    let input = "[[ x =~ ^\\\"\\-1[[:blank:]]((\\?[luds])+).* ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::RegexMatch);

    let ConditionalExpr::Regex(word) = binary.right.as_ref() else {
        panic!("expected regex rhs");
    };
    assert_eq!(word.render(input), "^\"-1[[:blank:]]((\\?[luds])+).*");
}

#[test]
fn test_parse_conditional_regex_allows_left_brace_operand() {
    let input = "[[ { =~ \"{\" ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::RegexMatch);

    let ConditionalExpr::Word(left) = binary.left.as_ref() else {
        panic!("expected literal left operand");
    };
    assert_eq!(left.span.slice(input), "{");

    let ConditionalExpr::Regex(right) = binary.right.as_ref() else {
        panic!("expected regex rhs");
    };
    assert_eq!(right.render(input), "{");
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
fn test_posix_dialect_rejects_double_bracket_conditionals() {
    let error = Parser::with_dialect("[[ foo == bar ]]\n", ShellDialect::Posix)
        .parse()
        .unwrap_err();

    assert!(matches!(
        error,
        Error::Parse { message, .. } if message.contains("[[ ]] conditionals")
    ));
}

#[test]
fn test_bash_and_mksh_dialects_accept_double_bracket_conditionals() {
    Parser::with_dialect("[[ foo == bar ]]\n", ShellDialect::Bash)
        .parse()
        .unwrap();
    Parser::with_dialect("[[ foo == bar ]]\n", ShellDialect::Mksh)
        .parse()
        .unwrap();
    Parser::with_dialect("[[ foo == bar ]]\n", ShellDialect::Zsh)
        .parse()
        .unwrap();
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

#[test]
fn test_brace_group_command_can_use_right_brace_as_literal_argument() {
    let source = "rbrace() { echo }; }; rbrace\n";
    let output = Parser::new(source).parse().unwrap();

    let function = expect_function(&output.file.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
    let command = expect_simple(&body[0]);
    assert_eq!(command.name.render(source), "echo");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(source), "}");
}
