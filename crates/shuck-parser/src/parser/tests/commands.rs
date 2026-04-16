use super::*;

#[test]
fn test_parse_simple_command() {
    let input = "echo hello";
    let parser = Parser::new(input);
    let parsed = parser.parse().unwrap();
    assert_eq!(parsed.status, ParseStatus::Clean);
    assert!(parsed.diagnostics.is_empty());
    assert!(parsed.terminal_error.is_none());
    let script = parsed.file;

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
    let parsed = Parser::new("echo ok\n)\necho later\n").parse();
    assert_eq!(parsed.status, ParseStatus::Fatal);
    assert!(parsed.terminal_error.is_some());
    let error = parsed.unwrap_err();

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
    let recovered = Parser::new(input).parse();

    assert_eq!(recovered.status, ParseStatus::Fatal);
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
fn test_parse_reports_eof_only_missing_fi_as_recovered() {
    let input = "if true; then\n  :\n";
    let parsed = Parser::new(input).parse();

    assert_eq!(parsed.status, ParseStatus::Recovered);
    assert!(parsed.terminal_error.is_none());
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].message, "expected 'fi'");
}

#[test]
fn test_parse_collects_zsh_brace_if_fact_in_bash_mode() {
    let input = "if [[ -n $x ]] {\n  :\n}\n";
    let parsed = Parser::new(input).parse();

    assert_eq!(parsed.syntax_facts.zsh_brace_if_spans.len(), 1);
    assert_eq!(parsed.syntax_facts.zsh_brace_if_spans[0].slice(input), "{");
}

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
fn test_parse_collects_zsh_always_fact_in_posix_mode() {
    let input = "{ :; } always { :; }\n";
    let parsed = Parser::with_dialect(input, ShellDialect::Posix).parse();

    assert_eq!(parsed.syntax_facts.zsh_always_spans.len(), 1);
    assert_eq!(
        parsed.syntax_facts.zsh_always_spans[0].slice(input),
        "always"
    );
}

#[test]
fn test_parse_collects_zsh_case_group_facts_in_posix_mode() {
    let input = "case $x in\n  foo_(a|b)_*) echo ok ;;\nesac\n";
    let parsed = Parser::with_dialect(input, ShellDialect::Posix).parse();

    assert_eq!(parsed.syntax_facts.zsh_case_group_parts.len(), 1);
    assert_eq!(
        parsed.syntax_facts.zsh_case_group_parts[0].pattern_part_index,
        1
    );
    assert_eq!(
        parsed.syntax_facts.zsh_case_group_parts[0]
            .span
            .slice(input),
        "(a|b)"
    );
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
fn test_zsh_function_keyword_allows_line_continued_multi_name_brace_body() {
    let input = "function chruby_prompt_info \\\n  rbenv_prompt_info \\\n  hg_prompt_info \\\n  pyenv_prompt_info \\\n{\n  return 1\n}\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    assert!(function.uses_function_keyword());
    assert!(!function.has_trailing_parens());
    assert_eq!(function.header.entries.len(), 4);
    assert_eq!(
        function
            .header
            .static_names()
            .map(Name::as_str)
            .collect::<Vec<_>>(),
        vec![
            "chruby_prompt_info",
            "rbenv_prompt_info",
            "hg_prompt_info",
            "pyenv_prompt_info",
        ]
    );

    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);

    let AstCommand::Builtin(AstBuiltinCommand::Return(command)) = &body[0].command else {
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
fn test_zsh_function_keyword_allows_multiline_nameless_anonymous_function_command() {
    let input = "function {\n  local agents\n  local -a identities\n  return 0\n}\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_anonymous_function(&script.body[0]);
    assert!(function.uses_function_keyword());
    assert!(function.args.is_empty());

    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group anonymous function body");
    };
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 3);
    assert!(matches!(body[0].command, AstCommand::Decl(_)));
    assert!(matches!(body[1].command, AstCommand::Decl(_)));

    let AstCommand::Builtin(AstBuiltinCommand::Return(command)) = &body[2].command else {
        panic!("expected return body");
    };
    assert_eq!(
        command
            .code
            .as_ref()
            .expect("expected return code")
            .render(input),
        "0"
    );
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
fn test_zsh_top_level_anonymous_function_can_wrap_compact_helper_functions() {
    let input = "() {\n  local _sublime_linux_paths\n  _sublime_linux_paths=(\"$HOME/bin/sublime_merge\")\n  for _sublime_merge_path in $_sublime_linux_paths; do\n    if [[ -a $_sublime_merge_path ]]; then\n      sm_run() { $_sublime_merge_path \"$@\" >/dev/null 2>&1 &| }\n      ssm_run_sudo() {sudo $_sublime_merge_path \"$@\" >/dev/null 2>&1}\n      alias ssm=ssm_run_sudo\n      alias sm=sm_run\n      break\n    fi\n  done\n}\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_anonymous_function(&script.body[0]);
    assert!(!function.uses_function_keyword());
    assert!(function.args.is_empty());

    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group anonymous function body");
    };
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 3);

    let AstCommand::Compound(AstCompoundCommand::For(command)) = &body[2].command else {
        panic!("expected for loop in anonymous function body");
    };
    assert_eq!(command.body.len(), 1);

    let AstCommand::Compound(AstCompoundCommand::If(command)) = &command.body[0].command else {
        panic!("expected if statement in for loop body");
    };
    assert_eq!(command.then_branch.len(), 5);
    assert!(matches!(
        command.then_branch[0].command,
        AstCommand::Function(_)
    ));
    assert!(matches!(
        command.then_branch[1].command,
        AstCommand::Function(_)
    ));
    assert!(matches!(
        command.then_branch[2].command,
        AstCommand::Simple(_)
    ));
    assert!(matches!(
        command.then_branch[3].command,
        AstCommand::Simple(_)
    ));
    assert!(matches!(
        command.then_branch[4].command,
        AstCommand::Builtin(AstBuiltinCommand::Break(_))
    ));
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
fn test_zsh_function_keyword_preserves_multi_name_header_with_local_assignment_body() {
    let input = "function music itunes() {\n  local APP_NAME=Music sw_vers=$(sw_vers -productVersion 2>/dev/null)\n  print -- \"$APP_NAME $sw_vers\"\n}\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&script.body[0]);
    assert!(function.uses_function_keyword());
    assert!(function.has_trailing_parens());
    assert_eq!(
        function
            .header
            .static_names()
            .map(Name::as_str)
            .collect::<Vec<_>>(),
        vec!["music", "itunes"]
    );

    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 2);
    assert!(matches!(body[0].command, AstCommand::Decl(_)));

    let AstCommand::Simple(command) = &body[1].command else {
        panic!("expected print body");
    };
    assert_eq!(command.name.render(input), "print");
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
fn test_zsh_case_accepts_suffix_bare_group_pattern() {
    let input = concat!(
        "case \"$mode\" in\n",
        "  plugin::(disable|enable|load)) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    assert_eq!(
        pattern.render_syntax(input),
        "plugin::(disable|enable|load)"
    );
    assert!(matches!(&pattern.parts[0].kind, PatternPart::Literal(_)));
    let PatternPart::Group { kind, patterns } = &pattern.parts[1].kind else {
        panic!("expected zsh bare group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["disable", "enable", "load"]
    );
}

#[test]
fn test_zsh_case_accepts_numeric_range_pattern() {
    let input = concat!("case \"$jobspec\" in\n", "  <->) print ok ;;\n", "esac\n",);
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases[0].patterns[0].render_syntax(input), "<->");
}

#[test]
fn test_zsh_case_accepts_wrapper_alternatives_with_whitespace() {
    let input = concat!(
        "case $line in\n",
        "  (#* | <->..<->)\n",
        "    print -nP %F{blue}\n",
        "    ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(
        command.cases[0]
            .patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["#*", "<->..<->"]
    );
}

#[test]
fn test_zsh_case_accepts_start_group_with_suffix() {
    let input = concat!(
        "case \"$OSTYPE\" in\n",
        "  (darwin|freebsd)*) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    assert_eq!(pattern.render_syntax(input), "(darwin|freebsd)*");
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
fn test_zsh_case_accepts_optional_suffix_group_after_literal_url() {
    let input = concat!(
        "case \"$url\" in\n",
        "  https://github.com/ohmyzsh/ohmyzsh(|.git)) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    assert_eq!(
        pattern_part_slices(pattern, input),
        vec!["https://github.com/ohmyzsh/ohmyzsh", "(|.git)"]
    );
    let PatternPart::Group { kind, patterns } = &pattern.parts[1].kind else {
        panic!("expected optional suffix group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["", ".git"]
    );
}

#[test]
fn test_zsh_case_accepts_infix_group_with_trailing_wildcard() {
    let input = concat!(
        "case $widgets[$widget] in\n",
        "  user:_zsh_autosuggest_(bound|orig)_*) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    assert_eq!(
        pattern.render_syntax(input),
        "user:_zsh_autosuggest_(bound|orig)_*"
    );
    assert!(matches!(&pattern.parts[0].kind, PatternPart::Literal(_)));
    let PatternPart::Group { kind, patterns } = &pattern.parts[1].kind else {
        panic!("expected infix group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["bound", "orig"]
    );
    assert!(matches!(&pattern.parts[2].kind, PatternPart::Literal(_)));
    assert!(matches!(&pattern.parts[3].kind, PatternPart::AnyString));
}

#[test]
fn test_zsh_case_accepts_mixed_jobspec_patterns() {
    let input = concat!(
        "case \"$jobspec\" in\n",
        "  <->) print number ;;\n",
        "  \"\"|%|+) print current ;;\n",
        "  -) print previous ;;\n",
        "  [?]*) print contains ;;\n",
        "  *) print prefix ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases[0].patterns[0].render_syntax(input), "<->");
    assert_eq!(command.cases[1].patterns.len(), 3);
    assert!(matches!(
        &command.cases[1].patterns[0].parts[0].kind,
        PatternPart::Word(word) if is_fully_quoted(word)
    ));
    assert_eq!(command.cases[1].patterns[1].render_syntax(input), "%");
    assert_eq!(command.cases[1].patterns[2].render_syntax(input), "+");
    assert_eq!(command.cases[2].patterns[0].render_syntax(input), "-");
    assert_eq!(command.cases[3].patterns[0].render_syntax(input), "[?]*");
    assert_eq!(command.cases[4].patterns[0].render_syntax(input), "*");
}

#[test]
fn test_zsh_case_accepts_wrapped_wildcard_suffix_patterns() {
    let input = concat!(
        "case $line in\n",
        "  (*# SKIP*) print skip ;;\n",
        "  (ok*# TODO*) print xpass ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(
        command.cases[0].patterns[0].render_syntax(input),
        "*# SKIP*"
    );
    assert_eq!(
        command.cases[1].patterns[0].render_syntax(input),
        "ok*# TODO*"
    );
}

#[test]
fn test_zsh_case_accepts_wrapper_quoted_pattern_with_same_line_body() {
    let input = concat!("case $arg in\n", "  ($'\\n') print ok ;;\n", "esac\n",);
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(
        command.cases[0].patterns[0].render_syntax(input),
        concat!("$'", "\n", "'")
    );
}

#[test]
fn test_zsh_case_preserves_semipipe_terminator() {
    let input = concat!(
        "case $2 in\n",
        "  cygwin_nt-10.0-i686) bin='cygwin32/bin' ;|\n",
        "  *) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(
        command.cases[0].terminator,
        CaseTerminator::ContinueMatching
    );
    assert_eq!(command.cases[0].terminator_span.unwrap().slice(input), ";|");
}

#[test]
fn test_case_preserves_bash_fallthrough_terminator_spans() {
    let input = concat!(
        "case $mode in\n",
        "  start) printf '%s\\n' start ;&\n",
        "  stop) printf '%s\\n' stop ;;&\n",
        "esac\n",
    );
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases[0].terminator, CaseTerminator::FallThrough);
    assert_eq!(command.cases[0].terminator_span.unwrap().slice(input), ";&");
    assert_eq!(command.cases[1].terminator, CaseTerminator::Continue);
    assert_eq!(
        command.cases[1].terminator_span.unwrap().slice(input),
        ";;&"
    );
}

#[test]
fn test_zsh_case_preserves_semipipe_terminator_across_repeated_arms() {
    let input = concat!(
        "case $2 in\n",
        "  cygwin_nt-10.0-i686)   bin='cygwin32/bin'  ;|\n",
        "  cygwin_nt-10.0-x86_64) bin='cygwin64/bin'  ;|\n",
        "  msys_nt-10.0-i686)     bin='msys32/usr/bin';|\n",
        "  msys_nt-10.0-x86_64)   bin='msys64/usr/bin';|\n",
        "  cygwin_nt-10.0-*)\n",
        "    tmp='/cygdrive/c/tmp'\n",
        "  ;|\n",
        "  msys_nt-10.0-*)\n",
        "    tmp='/c/tmp'\n",
        "    env='MSYSTEM=MSYS'\n",
        "    intro+='PATH=\"$PATH:/usr/bin/site_perl:/usr/bin/vendor_perl:/usr/bin/core_perl\"'\n",
        "    ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases.len(), 6);
    assert_eq!(
        command.cases[..5]
            .iter()
            .map(|case| case.terminator)
            .collect::<Vec<_>>(),
        vec![CaseTerminator::ContinueMatching; 5]
    );
    assert_eq!(command.cases[5].terminator, CaseTerminator::Break);
}

#[test]
fn test_non_zsh_dialects_reject_zsh_case_group_and_semipipe_forms() {
    let group_case = concat!(
        "case \"$OSTYPE\" in\n",
        "  (darwin|freebsd)*) print ok ;;\n",
        "esac\n",
    );
    let semipipe_case = concat!(
        "case $2 in\n",
        "  cygwin*) bin='cygwin32/bin' ;|\n",
        "  *) print ok ;;\n",
        "esac\n",
    );

    for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
        assert!(
            Parser::with_dialect(group_case, dialect).parse().is_err(),
            "expected {dialect:?} to reject zsh bare case groups",
        );
        assert!(
            Parser::with_dialect(semipipe_case, dialect)
                .parse()
                .is_err(),
            "expected {dialect:?} to reject zsh ;| case terminators",
        );
    }
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
fn test_parse_zsh_conditional_unary_operand_with_subscripted_word() {
    let input = "[[ -z $opts[(r)-P] ]]\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    assert_eq!(unary.op, ConditionalUnaryOp::EmptyString);

    let ConditionalExpr::Word(word) = unary.expr.as_ref() else {
        panic!("expected word operand");
    };
    assert_eq!(word.render(input), "$opts[(r)-P]");
}

#[test]
fn test_parse_zsh_conditional_arithmetic_comparison_operand_with_subscripted_word() {
    let input = "[[ $GLOBALIAS_FILTER_VALUES[(Ie)$word] -eq 0 ]]\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::ArithmeticEq);

    let ConditionalExpr::Word(left) = binary.left.as_ref() else {
        panic!("expected word operand on left");
    };
    assert_eq!(left.render(input), "$GLOBALIAS_FILTER_VALUES[(Ie)$word]");
}

#[test]
fn test_parse_zsh_conditional_pattern_rhs_with_backrefs_and_parameter_expansion() {
    let input = "[[ \"$buf\" == (#b)(*)(${~pat})* ]]\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

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
    assert_eq!(pattern.render(input), "(#b)(*)(${~pat})*");
}

#[test]
fn test_parse_zsh_conditional_pattern_rhs_with_inline_anchors() {
    let input = "[[ $buffer != (#s)[$'\\t -~']#(#e) ]]\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::PatternNe);

    let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(pattern.render(input), "(#s)[\t -~]#(#e)");
}

#[test]
fn test_parse_zsh_conditional_pattern_rhs_accepts_bare_alternation_groups() {
    let input = "[[ $OPTARG != (|+|-)<->(|.<->)(|[eE](|-|+)<->) ]]\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::PatternNe);

    let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(pattern.render(input), "(|+|-)<->(|.<->)(|[eE](|-|+)<->)");
    assert!(!pattern.parts.is_empty());
}

#[test]
fn test_parse_zsh_if_with_pattern_capture_rhs() {
    let input = "if [[ \"$buf\" == (#b)(*)(${~pat})* ]]; then\n  print ok\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_if_else_with_inline_anchor_pattern_rhs() {
    let input =
        "if [[ $buffer != (#s)[$'\\t -~']#(#e) ]]; then\n  print ok\nelse\n  print alt\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_conditional_pattern_with_hash_repetition_after_char_class() {
    let input = "[[ $_p9k__ret == (#b)Python\\ ([[:digit:].]##)* ]]\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_conditional_pattern_with_numeric_range_prefix_and_and_rhs() {
    let input = "[[ $load == <->(|.<->) && $load != $_p9k__load_value ]]\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_if_with_defaulting_subscript_and_or_condition() {
    let input = "if [[ $zsyh_user_options[ignorebraces] == on || ${zsyh_user_options[ignoreclosebraces]:-off} == on ]]; then\n  print ok\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_command_substitution_with_comments_containing_apostrophes() {
    let input = "eval $(\n  exec 3>&1 >/dev/null\n  {\n    # won't break the command substitution scanner\n    print ok\n  } always {\n    :\n  }\n)\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
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
fn test_parse_zsh_repeat_body_with_bitwise_or_char_literal() {
    let input = "repeat 4; do\n  sysread -s1 c || return\n  (( rnd = (~(1 << 23) & rnd) << 8 | #c ))\n done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_function_with_char_literal_arithmetic_commands() {
    let input = "function -$0-rand() {\n  local c\n  while true; do\n    sysread -s1 c || return\n    (( #c < 256 / $1 * $1 )) && break\n  done\n  typeset -g REPLY=$((#c % $1 + 1))\n}\n";
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
fn test_parse_zsh_array_slice_assignment_to_empty_array() {
    let input = "if true; then\n  tokens[1,e]=()\nelse\n  tokens[1,e]=()\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_split_indexed_assignment_to_empty_array() {
    let input = "tokens[1,e]=()\n";
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
fn test_parse_zsh_if_with_comment_only_else_clause() {
    let input = "if [[ $this_word != *':start_of_pipeline:'* ]]; then\n  style=unknown-token\nelse\n  # '!' reserved word at start of pipeline; style already set above\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_if_with_empty_then_before_elif() {
    let input = "if false; then\nelif [[ $arg = $'\\x7d' ]]; then\n  print ok\nfi\n";
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
fn test_parse_zsh_conditional_group_with_arithmetic_subexpression() {
    let input = "until [[ $i -gt 99 || ( $i -ge $((length - ellen)) || $dir == $part ) && ( (( ${#expn} == 1 )) || $dir = $expn ) ]]; do\n  :\ndone\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_compact_helper_cluster_with_empty_middle_function() {
    let input = concat!(
        "function battery_pct_remaining() {\n",
        "  if ! battery_is_charging; then\n",
        "    battery_pct\n",
        "  else\n",
        "    echo External\n",
        "  fi\n",
        "}\n",
        "function battery_time_remaining() { }\n",
        "function battery_pct_prompt() {\n",
        "  local battery_pct color\n",
        "  battery_pct=$(battery_pct_remaining)\n",
        "  print -- $battery_pct\n",
        "}\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    assert_eq!(script.body.len(), 3);
    let middle = expect_function(&script.body[1]);
    let (compound, redirects) = expect_compound(middle.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group compact function body");
    };
    assert!(redirects.is_empty());
    assert!(body.is_empty());
}

#[test]
fn test_parse_zsh_compact_helper_functions_in_rbenv_fallback_branch() {
    let input = concat!(
        "if [[ $FOUND_RBENV -eq 1 ]]; then\n",
        "  function rbenv_prompt_info() {\n",
        "    print -- supported\n",
        "  }\n",
        "else\n",
        "  alias rubies='ruby -v'\n",
        "  function gemsets() { echo not-supported }\n",
        "  function current_ruby() { echo not-supported }\n",
        "  function current_gemset() { echo not-supported }\n",
        "  function gems() { echo not-supported }\n",
        "  function rbenv_prompt_info() {\n",
        "    print -- fallback\n",
        "  }\n",
        "fi\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert!(redirects.is_empty());
    let else_branch = command
        .else_branch
        .as_ref()
        .expect("expected fallback else branch");
    assert_eq!(else_branch.len(), 6);
    assert_eq!(expect_simple(&else_branch[0]).name.render(input), "alias");
    assert!(
        else_branch
            .iter()
            .skip(1)
            .all(|stmt| matches!(stmt.command, AstCommand::Function(_)))
    );
}

#[test]
fn test_parse_zsh_compact_xxd_helper_functions_in_elif_ladder() {
    let input = concat!(
        "if [[ $(whence python3) != \"\" ]]; then\n",
        "  alias urlencode='python3 encode'\n",
        "  alias urldecode='python3 decode'\n",
        "elif [[ $(whence xxd) != \"\" && ( \"x$URLTOOLS_METHOD\" = \"x\" || \"x$URLTOOLS_METHOD\" = \"xshell\" ) ]]; then\n",
        "  function urlencode() {echo $@ | tr -d \"\\n\" | xxd -plain | sed \"s/\\(..\\)/%\\1/g\"}\n",
        "  function urldecode() {printf $(echo -n $@ | sed 's/\\\\/\\\\\\\\/g;s/\\(%\\)\\([0-9a-fA-F][0-9a-fA-F]\\)/\\\\x\\2/g')\"\\n\"}\n",
        "elif [[ $(whence ruby) != \"\" ]]; then\n",
        "  alias urlencode='ruby encode'\n",
        "  alias urldecode='ruby decode'\n",
        "fi\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert_eq!(command.elif_branches.len(), 2);
    let xxd_branch = &command.elif_branches[0].1;
    assert_eq!(xxd_branch.len(), 2);
    assert!(matches!(xxd_branch[0].command, AstCommand::Function(_)));
    assert!(matches!(xxd_branch[1].command, AstCommand::Function(_)));
}

#[test]
fn test_parse_zsh_if_with_or_brace_condition_then_compact_prompt_helpers() {
    let input = concat!(
        "if zstyle -t ':omz:alpha:lib:git' async-prompt \\\n",
        "  || { is-at-least 5.0.6 && zstyle -T ':omz:alpha:lib:git' async-prompt }; then\n",
        "  function git_prompt_info() {\n",
        "    print -- info\n",
        "  }\n",
        "  function git_prompt_status() {\n",
        "    print -- status\n",
        "  }\n",
        "fi\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.condition.len(), 1);
    let condition = expect_binary(&command.condition[0]);
    assert_eq!(condition.op, BinaryOp::Or);

    let (group, group_redirects) = expect_compound(&condition.right);
    let AstCompoundCommand::BraceGroup(body) = group else {
        panic!("expected brace-group right-hand side");
    };
    assert!(group_redirects.is_empty());
    assert_eq!(body.len(), 1);
    assert_eq!(expect_binary(&body[0]).op, BinaryOp::And);

    assert_eq!(command.then_branch.len(), 2);
    assert!(matches!(
        command.then_branch[0].command,
        AstCommand::Function(_)
    ));
    assert!(matches!(
        command.then_branch[1].command,
        AstCommand::Function(_)
    ));
}

#[test]
fn test_parse_zsh_case_arm_with_multiline_or_brace_fallback_group() {
    let input = concat!(
        "case \"${file:l}\" in\n",
        "  (*.tar.gz|*.tgz)\n",
        "    (( $+commands[pigz] )) && { tar -I pigz -xvf \"$full_path\" } || tar zxvf \"$full_path\" ;;\n",
        "  (*.tar.bz2|*.tbz|*.tbz2)\n",
        "    (( $+commands[pbzip2] )) && { tar -I pbzip2 -xvf \"$full_path\" } || tar xvjf \"$full_path\" ;;\n",
        "  (*.tar.xz|*.txz)\n",
        "    (( $+commands[pixz] )) && { tar -I pixz -xvf \"$full_path\" } || {\n",
        "      tar --xz --help &> /dev/null \\\n",
        "      && tar --xz -xvf \"$full_path\" \\\n",
        "      || xzcat \"$full_path\" | tar xvf -\n",
        "    } ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.cases.len(), 3);

    let fallback = expect_binary(&command.cases[2].body[0]);
    assert_eq!(fallback.op, BinaryOp::Or);
    let (group, group_redirects) = expect_compound(&fallback.right);
    let AstCompoundCommand::BraceGroup(body) = group else {
        panic!("expected multiline fallback brace group");
    };
    assert!(group_redirects.is_empty());
    assert_eq!(body.len(), 1);
    assert_eq!(expect_binary(&body[0]).op, BinaryOp::Or);
}

#[test]
fn test_parse_zsh_helper_predicate_with_trailing_or_brace_group() {
    let input = concat!(
        "_rake_does_task_list_need_generating() {\n",
        "  _rake_tasks_missing || _rake_tasks_version_changed || _rakefile_has_changes || { _is_rails_app && _tasks_changed }\n",
        "}\n",
    );
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

    let outer = expect_binary(&body[0]);
    assert_eq!(outer.op, BinaryOp::Or);
    let (group, group_redirects) = expect_compound(&outer.right);
    let AstCompoundCommand::BraceGroup(fallback_body) = group else {
        panic!("expected trailing brace-group fallback");
    };
    assert!(group_redirects.is_empty());
    assert_eq!(fallback_body.len(), 1);
    assert_eq!(expect_binary(&fallback_body[0]).op, BinaryOp::And);
}

#[test]
fn test_parse_zsh_nested_empty_compact_quit_override_in_function() {
    let input = concat!(
        "function quit() {\n",
        "  consume_input\n",
        "  if [[ $1 == '-c' ]]; then\n",
        "    print -Pr -- ''\n",
        "    read -s\n",
        "  fi\n",
        "  function quit() {}\n",
        "  stty echo 2>/dev/null\n",
        "  show_cursor\n",
        "  exit 1\n",
        "}\n",
    );
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

    let nested_stmt = body
        .iter()
        .find(|stmt| matches!(stmt.command, AstCommand::Function(_)))
        .expect("expected nested quit override");
    let nested = expect_function(nested_stmt);
    let (nested_compound, nested_redirects) = expect_compound(nested.body.as_ref());
    let AstCompoundCommand::BraceGroup(nested_body) = nested_compound else {
        panic!("expected compact nested brace-group body");
    };
    assert!(nested_redirects.is_empty());
    assert!(nested_body.is_empty());
}

#[test]
fn test_parse_zsh_compact_bind_widgets_helper_before_hook_registration() {
    let input = concat!(
        "if is-at-least 5.9 && _zsh_highlight__function_callable_p add-zle-hook-widget\n",
        "then\n",
        "  _zsh_highlight__zle-line-pre-redraw() {\n",
        "    true && _zsh_highlight \"$@\"\n",
        "  }\n",
        "  _zsh_highlight_bind_widgets(){}\n",
        "  if [[ -o zle ]]; then\n",
        "    add-zle-hook-widget zle-line-pre-redraw _zsh_highlight__zle-line-pre-redraw\n",
        "    add-zle-hook-widget zle-line-finish _zsh_highlight__zle-line-finish\n",
        "  fi\n",
        "else\n",
        "  _zsh_highlight_bind_widgets() {\n",
        "    zmodload zsh/zleparameter 2>/dev/null || {\n",
        "      print -r -- >&2 failed\n",
        "      return 1\n",
        "    }\n",
        "  }\n",
        "fi\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.then_branch.len(), 3);

    let helper = expect_function(&command.then_branch[1]);
    let (helper_compound, helper_redirects) = expect_compound(helper.body.as_ref());
    let AstCompoundCommand::BraceGroup(helper_body) = helper_compound else {
        panic!("expected compact helper body");
    };
    assert!(helper_redirects.is_empty());
    assert!(helper_body.is_empty());
    assert!(matches!(
        command.then_branch[2].command,
        AstCommand::Compound(AstCompoundCommand::If(_))
    ));
    assert!(command.else_branch.is_some());
}

#[test]
fn test_parse_zsh_top_level_failure_blocks_after_or_lists() {
    let input = concat!(
        "_zsh_highlight_bind_widgets || {\n",
        "  print -r -- >&2 failed-binding\n",
        "  return 1\n",
        "}\n",
        "_zsh_highlight_load_highlighters \"$dir\" || {\n",
        "  print -r -- >&2 failed-loading\n",
        "  return 1\n",
        "}\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    assert_eq!(script.body.len(), 2);
    for stmt in script.body.iter() {
        let command = expect_binary(stmt);
        assert_eq!(command.op, BinaryOp::Or);

        let (group, redirects) = expect_compound(&command.right);
        let AstCompoundCommand::BraceGroup(body) = group else {
            panic!("expected top-level failure brace group");
        };
        assert!(redirects.is_empty());
        assert_eq!(body.len(), 2);
        assert!(matches!(
            body[1].command,
            AstCommand::Builtin(AstBuiltinCommand::Return(_))
        ));
    }
}

#[test]
fn test_parse_zsh_anonymous_function_after_and_list() {
    let input = "(( ${+ZSHZ_DEBUG} )) && () {\n  print ok\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_dynamic_posix_function_definition_in_if_branch() {
    let input = "if [[ $cur_widget == zle-* ]]; then\n  _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_case_arm_with_dynamic_function_definition() {
    let input = "case $widget_type in\n  *)\n    if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n      _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n      zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n    else\n      print -r -- >&2 unhandled\n    fi\n    ;;\nesac\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_case_statement_with_eval_wrapped_widget_rebindings() {
    let input = "case ${widgets[$cur_widget]:-\"\"} in\n  user:_zsh_highlight_widget_*);;\n  user:*) zle -N $prefix-$cur_widget ${widgets[$cur_widget]#*:}\n          eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget ${(q)prefix}-${(q)cur_widget} -- \\\"\\$@\\\" }\"\n          zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n  completion:*) zle -C $prefix-$cur_widget ${${(s.:.)widgets[$cur_widget]}[2,3]}\n                eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget ${(q)prefix}-${(q)cur_widget} -- \\\"\\$@\\\" }\"\n                zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n  builtin) eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget .${(q)cur_widget} -- \\\"\\$@\\\" }\"\n           zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n  *)\n     if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n       _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n       zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n     else\n       print -r -- >&2 \"zsh-syntax-highlighting: unhandled ZLE widget ${(qq)cur_widget}\"\n       print -r -- >&2 \"zsh-syntax-highlighting: (This is sometimes caused by doing \\`bindkey <keys> ${(q-)cur_widget}\\` without creating the ${(qq)cur_widget} widget with \\`zle -N\\` or \\`zle -C\\`.)\"\n     fi\nesac\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_if_with_anonymous_function_invocation_and_quoted_arg() {
    let input = "if [[ -t 1 ]]; then\n  if (( ${+__p9k_use_osc133_c_cmdline} )); then\n    () {\n      builtin printf '%s' \"$1\"\n    } \"$1\"\n  else\n    builtin print -n fallback\n  fi\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_top_level_anonymous_function_after_and_list_block() {
    let input = "(( ${+ZSHZ_DEBUG} )) && () {\n  if is-at-least 5.4.0; then\n    local x\n    for x in ${=ZSHZ[FUNCTIONS]}; do\n      functions -W $x\n    done\n  fi\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_anonymous_eval_callback_inside_worker_loop_with_always_followthrough() {
    let input = "{\n  while zselect -a ready 0 ${(k)_p9k_worker_fds}; do\n    [[ $ready[1] == -r ]] || return\n    for req in ${(ps:\\x1e:)buf}; do\n      _p9k_worker_request_id=${req%%$'\\x1f'*}\n      () { eval $req[$#_p9k_worker_request_id+2,-1] }\n      (( $+_p9k_worker_inflight[$_p9k_worker_request_id] )) && continue\n      print -rn -- d$_p9k_worker_request_id$'\\x1e' || return\n    done\n  done\n} always {\n  kill -- -$_p9k_worker_pgid\n}\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Always(command) = compound else {
        panic!("expected always command");
    };
    assert!(redirects.is_empty());
    assert_eq!(command.body.len(), 1);
    assert_eq!(command.always_body.len(), 1);

    let AstCommand::Compound(AstCompoundCommand::While(while_command)) = &command.body[0].command
    else {
        panic!("expected while loop in always body");
    };
    assert_eq!(while_command.body.len(), 2);

    let AstCommand::Compound(AstCompoundCommand::For(for_command)) = &while_command.body[1].command
    else {
        panic!("expected for loop in while body");
    };
    assert_eq!(for_command.body.len(), 4);

    let callback = expect_anonymous_function(&for_command.body[1]);
    assert!(!callback.uses_function_keyword());
    assert!(callback.args.is_empty());

    let (callback_compound, callback_redirects) = expect_compound(callback.body.as_ref());
    let AstCompoundCommand::BraceGroup(callback_body) = callback_compound else {
        panic!("expected brace-group callback body");
    };
    assert!(callback_redirects.is_empty());
    assert_eq!(callback_body.len(), 1);

    let AstCommand::Simple(command) = &callback_body[0].command else {
        panic!("expected eval body");
    };
    assert_eq!(command.name.render(input), "eval");
    assert_eq!(command.args.len(), 1);
    assert_eq!(
        command.args[0].render(input),
        "$req[$#_p9k_worker_request_id+2,-1]"
    );
}

#[test]
fn test_parse_zsh_function_with_multiline_and_list_after_alias_lookup() {
    let input = "zsh-z_plugin_unload() {\n  emulate -L zsh\n\n  add-zsh-hook -D precmd _zshz_precmd\n  add-zsh-hook -d chpwd _zshz_chpwd\n\n  local x\n  for x in ${=ZSHZ[FUNCTIONS]}; do\n    (( ${+functions[$x]} )) && unfunction $x\n  done\n\n  unset ZSHZ\n\n  fpath=( \"${(@)fpath:#${0:A:h}}\" )\n\n  (( ${+aliases[${ZSHZ_CMD:-${_Z_CMD:-z}}]} )) &&\n    unalias ${ZSHZ_CMD:-${_Z_CMD:-z}}\n\n  unfunction $0\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_precmd_function_with_inline_case_arm_and_subshell_background() {
    let input = "_zshz_precmd() {\n  setopt LOCAL_OPTIONS UNSET\n  [[ $PWD == \"$HOME\" ]] || (( ZSHZ[DIRECTORY_REMOVED] )) && return\n\n  local exclude\n  for exclude in ${(@)ZSHZ_EXCLUDE_DIRS:-${(@)_Z_EXCLUDE_DIRS}}; do\n    case $PWD in\n      ${exclude}|${exclude}/*) return ;;\n    esac\n  done\n\n  if [[ $OSTYPE == (cygwin|msys) ]]; then\n    zshz --add \"$PWD\"\n  else\n    (zshz --add \"$PWD\" &)\n  fi\n\n  : $RANDOM\n}\n";
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
fn test_parse_zsh_if_with_last_argument_slice_and_opt_subscripts() {
    let input = "f() {\n  local -A opts\n  if [[ ${@: -1} == /* ]] && (( ! $+opts[-e] && ! $+opts[-l] )); then\n    [[ -d ${@: -1} ]] && print ${@: -1} && return\n  fi\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_parameter_replacements_with_quoted_backslashes() {
    let input = "f() {\n  local path_field escaped_path_field\n  escaped_path_field=${path_field//'\\'/'\\\\'}\n  escaped_path_field=${escaped_path_field//'`'/'\\`'}\n  escaped_path_field=${escaped_path_field//'('/'\\('}\n  escaped_path_field=${escaped_path_field//')'/'\\)'}\n  escaped_path_field=${escaped_path_field//'['/'\\['}\n  escaped_path_field=${escaped_path_field//']'/'\\]'}\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_find_matches_helper_function() {
    let input = "_zshz_find_matches() {\n  setopt LOCAL_OPTIONS NO_EXTENDED_GLOB\n\n  local fnd=$1 method=$2 format=$3\n\n  local -a existing_paths\n  local line dir path_field rank_field time_field rank dx escaped_path_field\n  local -A matches imatches\n  local best_match ibest_match hi_rank=-9999999999 ihi_rank=-9999999999\n\n  for line in $lines; do\n    if [[ ! -d ${line%%\\|*} ]]; then\n      for dir in ${(@)ZSHZ_KEEP_DIRS}; do\n        if [[ ${line%%\\|*} == ${dir}/* ||\n              ${line%%\\|*} == $dir     ||\n              $dir == '/' ]]; then\n          existing_paths+=( $line )\n        fi\n      done\n    else\n      existing_paths+=( $line )\n    fi\n  done\n  lines=( $existing_paths )\n\n  for line in $lines; do\n    path_field=${line%%\\|*}\n    rank_field=${${line%\\|*}#*\\|}\n    time_field=${line##*\\|}\n\n    case $method in\n      rank) rank=$rank_field ;;\n      time) (( rank = time_field - EPOCHSECONDS )) ;;\n      *)\n        (( dx = EPOCHSECONDS - time_field ))\n        rank=$(( 10000 * rank_field * (3.75/( (0.0001 * dx + 1) + 0.25)) ))\n        ;;\n    esac\n\n    local q=${fnd//[[:space:]]/\\*}\n\n    local path_field_normalized=$path_field\n    if (( ZSHZ_TRAILING_SLASH )); then\n      path_field_normalized=${path_field%/}/\n    fi\n\n    if [[ $ZSHZ_CASE == 'smart' && ${1:l} == $1 &&\n          ${path_field_normalized:l} == ${~q:l} ]]; then\n      imatches[$path_field]=$rank\n    elif [[ $ZSHZ_CASE != 'ignore' && $path_field_normalized == ${~q} ]]; then\n      matches[$path_field]=$rank\n    elif [[ $ZSHZ_CASE != 'smart' && ${path_field_normalized:l} == ${~q:l} ]]; then\n      imatches[$path_field]=$rank\n    fi\n\n    escaped_path_field=${path_field//'\\\\'/'\\\\\\\\'}\n    escaped_path_field=${escaped_path_field//'`'/'\\`'}\n    escaped_path_field=${escaped_path_field//'('/'\\('}\n    escaped_path_field=${escaped_path_field//')'/'\\)'}\n    escaped_path_field=${escaped_path_field//'['/'\\['}\n    escaped_path_field=${escaped_path_field//']'/'\\]'}\n\n    if (( matches[$escaped_path_field] )) &&\n       (( matches[$escaped_path_field] > hi_rank )); then\n      best_match=$path_field\n      hi_rank=${matches[$escaped_path_field]}\n    elif (( imatches[$escaped_path_field] )) &&\n         (( imatches[$escaped_path_field] > ihi_rank )); then\n      ibest_match=$path_field\n      ihi_rank=${imatches[$escaped_path_field]}\n      ZSHZ[CASE_INSENSITIVE]=1\n    fi\n  done\n\n  [[ -z $best_match && -z $ibest_match ]] && return 1\n\n  if [[ -n $best_match ]]; then\n    _zshz_output matches best_match $format\n  elif [[ -n $ibest_match ]]; then\n    _zshz_output imatches ibest_match $format\n  fi\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_nested_find_matches_helper_function() {
    let input = "zshz() {\n  local -a lines\n  _zshz_find_matches() {\n    setopt LOCAL_OPTIONS NO_EXTENDED_GLOB\n\n    local fnd=$1 method=$2 format=$3\n    local -a existing_paths\n    local line dir path_field rank_field time_field rank dx escaped_path_field\n    local -A matches imatches\n    local best_match ibest_match hi_rank=-9999999999 ihi_rank=-9999999999\n\n    for line in $lines; do\n      if [[ ! -d ${line%%\\|*} ]]; then\n        for dir in ${(@)ZSHZ_KEEP_DIRS}; do\n          if [[ ${line%%\\|*} == ${dir}/* ||\n                ${line%%\\|*} == $dir     ||\n                $dir == '/' ]]; then\n            existing_paths+=( $line )\n          fi\n        done\n      else\n        existing_paths+=( $line )\n      fi\n    done\n    lines=( $existing_paths )\n\n    for line in $lines; do\n      path_field=${line%%\\|*}\n      rank_field=${${line%\\|*}#*\\|}\n      time_field=${line##*\\|}\n\n      case $method in\n        rank) rank=$rank_field ;;\n        time) (( rank = time_field - EPOCHSECONDS )) ;;\n        *)\n          (( dx = EPOCHSECONDS - time_field ))\n          rank=$(( 10000 * rank_field * (3.75/( (0.0001 * dx + 1) + 0.25)) ))\n          ;;\n      esac\n\n      local q=${fnd//[[:space:]]/\\*}\n\n      local path_field_normalized=$path_field\n      if (( ZSHZ_TRAILING_SLASH )); then\n        path_field_normalized=${path_field%/}/\n      fi\n\n      if [[ $ZSHZ_CASE == 'smart' && ${1:l} == $1 &&\n            ${path_field_normalized:l} == ${~q:l} ]]; then\n        imatches[$path_field]=$rank\n      elif [[ $ZSHZ_CASE != 'ignore' && $path_field_normalized == ${~q} ]]; then\n        matches[$path_field]=$rank\n      elif [[ $ZSHZ_CASE != 'smart' && ${path_field_normalized:l} == ${~q:l} ]]; then\n        imatches[$path_field]=$rank\n      fi\n\n      escaped_path_field=${path_field//'\\\\'/'\\\\\\\\'}\n      escaped_path_field=${escaped_path_field//'`'/'\\`'}\n      escaped_path_field=${escaped_path_field//'('/'\\('}\n      escaped_path_field=${escaped_path_field//')'/'\\)'}\n      escaped_path_field=${escaped_path_field//'['/'\\['}\n      escaped_path_field=${escaped_path_field//']'/'\\]'}\n\n      if (( matches[$escaped_path_field] )) &&\n         (( matches[$escaped_path_field] > hi_rank )); then\n        best_match=$path_field\n        hi_rank=${matches[$escaped_path_field]}\n      elif (( imatches[$escaped_path_field] )) &&\n           (( imatches[$escaped_path_field] > ihi_rank )); then\n        ibest_match=$path_field\n        ihi_rank=${imatches[$escaped_path_field]}\n        ZSHZ[CASE_INSENSITIVE]=1\n      fi\n    done\n\n    [[ -z $best_match && -z $ibest_match ]] && return 1\n\n    if [[ -n $best_match ]]; then\n      _zshz_output matches best_match $format\n    elif [[ -n $ibest_match ]]; then\n      _zshz_output imatches ibest_match $format\n    fi\n  }\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_nested_function_after_brace_fd_redirect_helper() {
    let input = "zshz() {\n  _zshz_add_or_remove_path() {\n    case $action in\n      --add)\n        exec {tmpfd}>|\"$tempfile\"\n        ;;\n    esac\n  }\n\n  _zshz_find_matches() {\n    for line in $lines; do\n      :\n    done\n  }\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_nested_function_after_full_brace_fd_helper() {
    let input = "zshz() {\n  _zshz_add_or_remove_path() {\n    local tempfile=\"${datafile}.${RANDOM}\"\n    integer tmpfd\n    case $action in\n      --add)\n        exec {tmpfd}>|\"$tempfile\"\n        _zshz_update_datafile $tmpfd \"$*\"\n        local ret=$?\n        ;;\n      --remove)\n        exec {tmpfd}>|\"$tempfile\"\n        print -u $tmpfd -l -- $lines\n        local ret=$?\n        ;;\n    esac\n\n    if (( tmpfd != 0 )); then\n      exec {tmpfd}>&-\n    fi\n  }\n\n  _zshz_find_matches() {\n    for line in $lines; do\n      :\n    done\n  }\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_nested_function_after_same_line_brace_group() {
    let input = "zshz() {\n  [[ -f $datafile ]] || { mkdir -p \"${datafile:h}\" && touch \"$datafile\" }\n\n  _zshz_find_matches() {\n    for line in $lines; do\n      :\n    done\n  }\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_nested_function_after_complex_array_assignments() {
    let input = "zshz() {\n  lines=( ${(f)\"$(< $datafile)\"} )\n  lines=( ${(M)lines:#/*\\|[[:digit:]]##[.,]#[[:digit:]]#\\|[[:digit:]]##} )\n\n  _zshz_find_matches() {\n    for line in $lines; do\n      :\n    done\n  }\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_anonymous_debug_block_followed_by_plugin_unload_function() {
    let input = "(( ${+ZSHZ_DEBUG} )) && () {\n  if is-at-least 5.4.0; then\n    local x\n    for x in ${=ZSHZ[FUNCTIONS]}; do\n      functions -W $x\n    done\n  fi\n}\n\nzsh-z_plugin_unload() {\n  emulate -L zsh\n\n  add-zsh-hook -D precmd _zshz_precmd\n  add-zsh-hook -d chpwd _zshz_chpwd\n\n  local x\n  for x in ${=ZSHZ[FUNCTIONS]}; do\n    (( ${+functions[$x]} )) && unfunction $x\n  done\n\n  unset ZSHZ\n\n  fpath=( \"${(@)fpath:#${0:A:h}}\" )\n\n  (( ${+aliases[${ZSHZ_CMD:-${_Z_CMD:-z}}]} )) &&\n    unalias ${ZSHZ_CMD:-${_Z_CMD:-z}}\n\n  unfunction $0\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_widget_binding_function_with_rebinding_loop() {
    let input = "_zsh_highlight_bind_widgets()\n{\n  setopt localoptions noksharrays\n  typeset -F SECONDS\n  local prefix=orig-s$SECONDS-r$RANDOM\n\n  zmodload zsh/zleparameter 2>/dev/null || {\n    print -r -- >&2 'zsh-syntax-highlighting: failed loading zsh/zleparameter.'\n    return 1\n  }\n\n  local -U widgets_to_bind\n  widgets_to_bind=(${${(k)widgets}:#(.*|run-help|which-command|beep|set-local-history|yank|yank-pop)})\n  widgets_to_bind+=(zle-line-finish)\n  widgets_to_bind+=(zle-isearch-update)\n\n  local cur_widget\n  for cur_widget in $widgets_to_bind; do\n    case ${widgets[$cur_widget]:-\"\"} in\n      user:_zsh_highlight_widget_*);;\n      user:*) zle -N $prefix-$cur_widget ${widgets[$cur_widget]#*:}\n              eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget ${(q)prefix}-${(q)cur_widget} -- \\\"\\$@\\\" }\"\n              zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n      completion:*) zle -C $prefix-$cur_widget ${${(s.:.)widgets[$cur_widget]}[2,3]}\n                    eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget ${(q)prefix}-${(q)cur_widget} -- \\\"\\$@\\\" }\"\n                    zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n      builtin) eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget .${(q)cur_widget} -- \\\"\\$@\\\" }\"\n               zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n      *)\n         if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n           _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n           zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n         else\n           print -r -- >&2 \"zsh-syntax-highlighting: unhandled ZLE widget ${(qq)cur_widget}\"\n           print -r -- >&2 \"zsh-syntax-highlighting: (This is sometimes caused by doing \\`bindkey <keys> ${(q-)cur_widget}\\` without creating the ${(qq)cur_widget} widget with \\`zle -N\\` or \\`zle -C\\`.)\"\n         fi\n    esac\n  done\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_widget_binding_loop_inside_function_without_prelude() {
    let input = "_zsh_highlight_bind_widgets()\n{\n  local cur_widget\n  for cur_widget in $widgets_to_bind; do\n    case ${widgets[$cur_widget]:-\"\"} in\n      user:_zsh_highlight_widget_*);;\n      user:*) zle -N $prefix-$cur_widget ${widgets[$cur_widget]#*:}\n              eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget ${(q)prefix}-${(q)cur_widget} -- \\\"\\$@\\\" }\"\n              zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n      completion:*) zle -C $prefix-$cur_widget ${${(s.:.)widgets[$cur_widget]}[2,3]}\n                    eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget ${(q)prefix}-${(q)cur_widget} -- \\\"\\$@\\\" }\"\n                    zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n      builtin) eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget .${(q)cur_widget} -- \\\"\\$@\\\" }\"\n               zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n      *)\n         if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n           _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n           zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n         else\n           print -r -- >&2 \"zsh-syntax-highlighting: unhandled ZLE widget ${(qq)cur_widget}\"\n         fi\n    esac\n  done\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_for_loop_case_arm_with_dynamic_function_definition() {
    let input = "for cur_widget in $widgets_to_bind; do\n  case ${widgets[$cur_widget]:-\"\"} in\n    *)\n       if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n         _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n         zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n       else\n         print -r -- >&2 \"zsh-syntax-highlighting: unhandled ZLE widget ${(qq)cur_widget}\"\n       fi\n  esac\n done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_function_case_with_builtin_eval_arm_before_dynamic_function_arm() {
    let input = "_zsh_highlight_bind_widgets()\n{\n  local cur_widget\n  for cur_widget in $widgets_to_bind; do\n    case ${widgets[$cur_widget]:-\"\"} in\n      builtin) eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget .${(q)cur_widget} -- \\\"\\$@\\\" }\"\n               zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n      *)\n         if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n           _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n           zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n         else\n           print -r -- >&2 \"zsh-syntax-highlighting: unhandled ZLE widget ${(qq)cur_widget}\"\n         fi\n    esac\n  done\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_function_body_with_eval_string_before_dynamic_function_definition() {
    let input = "_zsh_highlight_bind_widgets()\n{\n  eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget .${(q)cur_widget} -- \\\"\\$@\\\" }\"\n  zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget\n  _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n  zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_function_body_with_dynamic_function_definition_without_eval_prefix() {
    let input = "_zsh_highlight_bind_widgets()\n{\n  _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n  zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_top_level_dynamic_function_definition_followed_by_command() {
    let input = "_zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\nzle -N $cur_widget _zsh_highlight_widget_$cur_widget\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_nested_dynamic_function_leaves_following_command_and_outer_brace_visible() {
    let input = "_zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\nzle -N $cur_widget _zsh_highlight_widget_$cur_widget\n";
    let output = Parser::with_dialect(&format!("outer() {{\n{input}}}\n"), ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&output.body[0]);
    let (compound, _) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert_eq!(body.len(), 2);
    assert!(matches!(body[0].command, AstCommand::Function(_)));
    assert!(matches!(body[1].command, AstCommand::Simple(_)));
}

#[test]
fn test_parse_zsh_eval_string_before_dynamic_function_definition_at_top_level() {
    let input = "eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget .${(q)cur_widget} -- \\\"\\$@\\\" }\"\nzle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget\n_zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\nzle -N $cur_widget _zsh_highlight_widget_$cur_widget\n";
    let output = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();

    assert_eq!(output.file.body.len(), 4);
    match &output.file.body[2].command {
        AstCommand::Function(_) => {}
        _ => panic!("expected third statement to be a function definition"),
    };
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
fn test_parse_conditional_quoted_command_substitution_preserves_nested_quotes() {
    let input = "[[ \"$(get_permission \"$1\")\" != \"$(id -u)\" ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::PatternNe);

    let ConditionalExpr::Word(left) = binary.left.as_ref() else {
        panic!("expected left operand word");
    };
    let ConditionalExpr::Pattern(right) = binary.right.as_ref() else {
        panic!("expected right operand pattern");
    };
    assert_eq!(left.span.slice(input), "\"$(get_permission \"$1\")\"");
    assert_eq!(right.span.slice(input), "\"$(id -u)\"");

    let WordPart::DoubleQuoted { parts, .. } = &left.parts[0].kind else {
        panic!("expected double-quoted left operand");
    };
    let WordPart::CommandSubstitution { body, syntax } = &parts[0].kind else {
        panic!("expected left command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);

    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "get_permission");
    assert_eq!(inner.args[0].render_syntax(input), "\"$1\"");
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
    assert_eq!(word.render(input), "^\\\"\\-1[[:blank:]]((\\?[luds])+).*");
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

#[test]
fn test_parse_zsh_midfile_unsetopt_short_repeat_demotes_repeat_to_simple_command() {
    let source = "unsetopt short_repeat\nrepeat 2 echo hi\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[1]);

    assert_eq!(command.name.render(source), "repeat");
}

#[test]
fn test_parse_zsh_function_local_unsetopt_short_repeat_does_not_leak_to_top_level() {
    let source = "\
fn() {
  unsetopt short_repeat
  repeat 2 echo local
}
repeat 2 echo global
";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&output.body[0]);
    let (compound, _) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    let local_repeat = expect_simple(&body[1]);
    assert_eq!(local_repeat.name.render(source), "repeat");

    let (compound, _) = expect_compound(&output.body[1]);
    let AstCompoundCommand::Repeat(command) = compound else {
        panic!("expected top-level repeat command");
    };
    assert_eq!(command.count.render(source), "2");
    assert_eq!(command.body.len(), 1);
}

#[test]
fn test_parse_zsh_wrapped_unsetopt_short_repeat_demotes_repeat_to_simple_command() {
    let source = "command unsetopt short_repeat\nrepeat 2 echo hi\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let command = expect_simple(&output.body[1]);
    assert_eq!(command.name.render(source), "repeat");
}

#[test]
fn test_parse_zsh_plain_subshell_does_not_leak_short_repeat_prescan() {
    let source = "( unsetopt short_repeat )\nrepeat 2 echo global\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&output.body[0]);
    assert!(matches!(compound, AstCompoundCommand::Subshell(_)));

    let (compound, _) = expect_compound(&output.body[1]);
    let AstCompoundCommand::Repeat(command) = compound else {
        panic!("expected top-level repeat command");
    };
    assert_eq!(command.count.render(source), "2");
}

#[test]
fn test_parse_zsh_command_v_does_not_fake_short_repeat_effects() {
    let source = "command -v unsetopt short_repeat\nrepeat 2 echo global\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let command = expect_simple(&output.body[0]);
    assert_eq!(command.name.render(source), "command");

    let (compound, _) = expect_compound(&output.body[1]);
    let AstCompoundCommand::Repeat(repeat) = compound else {
        panic!("expected top-level repeat command");
    };
    assert_eq!(repeat.count.render(source), "2");
}

#[test]
fn test_parse_zsh_function_subshell_body_does_not_leak_short_repeat_prescan() {
    let source = "f() ( unsetopt short_repeat )\nrepeat 2 echo global\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&output.body[0]);
    let (compound, _) = expect_compound(function.body.as_ref());
    assert!(matches!(compound, AstCompoundCommand::Subshell(_)));

    let (compound, _) = expect_compound(&output.body[1]);
    let AstCompoundCommand::Repeat(command) = compound else {
        panic!("expected top-level repeat command");
    };
    assert_eq!(command.count.render(source), "2");
}

#[test]
fn test_parse_zsh_function_if_body_does_not_leak_short_repeat_prescan() {
    let source = "f() if true; then unsetopt short_repeat; fi\nrepeat 2 echo global\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let function = expect_function(&output.body[0]);
    let (compound, _) = expect_compound(function.body.as_ref());
    assert!(matches!(compound, AstCompoundCommand::If(_)));

    let (compound, _) = expect_compound(&output.body[1]);
    let AstCompoundCommand::Repeat(command) = compound else {
        panic!("expected top-level repeat command");
    };
    assert_eq!(command.count.render(source), "2");
}

#[test]
fn test_parse_zsh_midfile_unsetopt_short_loops_rejects_foreach_loop() {
    Parser::with_dialect("foreach x (a b c) { echo $x; }\n", ShellDialect::Zsh)
        .parse()
        .unwrap();
    let source = "unsetopt short_loops\nforeach x (a b c) { echo $x; }\n";
    let error = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap_err();

    assert!(matches!(
        error,
        Error::Parse { message, .. } if message.contains("foreach loops")
    ));
}

#[test]
fn test_parse_zsh_midfile_setopt_ignore_braces_treats_braces_as_words() {
    let source = "setopt ignore_braces\n{ echo hi }\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let command = expect_simple(&output.file.body[1]);
    assert_eq!(command.name.render(source), "{");
    assert_eq!(
        command
            .args
            .iter()
            .map(|word| word.render(source))
            .collect::<Vec<_>>(),
        vec!["echo", "hi", "}"]
    );
}

#[test]
fn test_parse_zsh_midfile_setopt_ignore_braces_disables_brace_syntax_collection() {
    let source = "setopt ignore_braces\nprint {a,b}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let command = expect_simple(&output.file.body[1]);
    assert!(command.args[0].brace_syntax.is_empty());
}
