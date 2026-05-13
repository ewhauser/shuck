use super::*;

#[test]
fn test_parse_collects_zsh_brace_if_fact_in_bash_mode() {
    let input = "if [[ -n $x ]] {\n  :\n}\n";
    let parsed = Parser::new(input).parse();

    assert_eq!(parsed.syntax_facts.zsh_brace_if_spans.len(), 1);
    assert_eq!(parsed.syntax_facts.zsh_brace_if_spans[0].slice(input), "{");
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
fn test_parse_zsh_if_with_pattern_capture_rhs() {
    let input = "if [[ \"$buf\" == (#b)(*)(${~pat})* ]]; then\n  print ok\nfi\n";
    parse_zsh_with_options(input, |options| {
        options.extended_glob = OptionValue::On;
    });
}

#[test]
fn test_parse_zsh_if_else_with_inline_anchor_pattern_rhs() {
    let input =
        "if [[ $buffer != (#s)[$'\\t -~']#(#e) ]]; then\n  print ok\nelse\n  print alt\nfi\n";
    parse_zsh_with_options(input, |options| {
        options.extended_glob = OptionValue::On;
    });
}

#[test]
fn test_parse_zsh_remaining_upstream_advanced_command_examples() {
    for input in [
        "cat file.txt | grep pattern | sort | uniq\n",
        "sleep 10 &\n",
        "sleep 10 &!\n",
        "sleep 10 &|\n",
    ] {
        Parser::with_dialect(input, ShellDialect::Zsh)
            .parse()
            .unwrap();
    }
}

#[test]
fn test_parse_zsh_ansi_c_string_regex_match_expression_from_upstream() {
    let input = "[[ \"$x\" =~ \"^foo\"$'\\t'\"bar\" ]]\n";
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
    assert_eq!(binary.op, ConditionalBinaryOp::RegexMatch);

    let ConditionalExpr::Regex(word) = binary.right.as_ref() else {
        panic!("expected regex rhs");
    };
    assert_eq!(word.span.slice(input), "\"^foo\"$'\\t'\"bar\"");
}

#[test]
fn test_parse_zsh_if_with_defaulting_subscript_and_or_condition() {
    let input = "if [[ $zsyh_user_options[ignorebraces] == on || ${zsyh_user_options[ignoreclosebraces]:-off} == on ]]; then\n  print ok\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_job_spec_commands_preserve_percent_arguments() {
    for input in ["fg %1\n", "bg %2\n"] {
        let script = Parser::with_dialect(input, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;
        let command = expect_simple(&script.body[0]);

        assert_eq!(command.args.len(), 1);
        assert!(command.args[0].span.slice(input).starts_with('%'));
    }
}

#[test]
fn test_parse_zsh_command_substitution_with_comments_containing_apostrophes() {
    let input = "eval $(\n  exec 3>&1 >/dev/null\n  {\n    # won't break the command substitution scanner\n    print ok\n  } always {\n    :\n  }\n)\n";
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
fn test_parse_zsh_if_compact_brace_bodies_without_space_after_left_brace() {
    let input = "f() { if (($4==0)){c=1;} elif (($4==1)){c=2;} else {c=3;}; echo ok; }\n";
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
