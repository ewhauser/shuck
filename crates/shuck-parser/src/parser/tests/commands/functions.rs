use super::*;

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
fn test_alias_expansion_does_not_replace_posix_function_name() {
    let input = "\
shopt -s expand_aliases
alias wget='wget -V'
wget() { echo hi; }
";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[2]);

    assert_eq!(
        function.static_names().next().map(|name| name.as_str()),
        Some("wget")
    );
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
fn test_posix_function_allows_single_quoted_zsh_glob_control_text_in_body() {
    let input = "\
parse_hint(){
    printf '%s\\n' 'literal (# marker)' \"$@\"
}
";
    let parsed = Parser::new(input).parse().unwrap();

    assert_eq!(parsed.file.body.len(), 1);
}

#[test]
fn test_function_keyword_allows_plus_name_in_bash() {
    let input = "function run++ () {\n  ((run_count+=1))\n}\nrun++\n";
    let parsed = Parser::new(input).parse().unwrap();

    assert_eq!(parsed.file.body.len(), 2);
    let function = expect_function(&parsed.file.body[0]);
    assert_eq!(
        function.static_names().next().map(|name| name.as_str()),
        Some("run++")
    );
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
fn test_zsh_function_keyword_preserves_multi_name_backslash_newline_brace_body() {
    let input = "function foo \\\n    bar \\\n{ \n    echo hi\n}\n";
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

    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
    assert_eq!(expect_simple(&body[0]).name.render(input), "echo");
    assert_eq!(expect_simple(&body[0]).args[0].render(input), "hi");
}

#[test]
fn test_zsh_function_keyword_preserves_multi_name_compact_empty_brace_body() {
    let input = "function foo bar {}\n";
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

    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    assert!(body.is_empty());
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
fn test_nonempty_function_body_accepted() {
    let parser = Parser::new("f() { echo hi; }");
    assert!(
        parser.parse().is_ok(),
        "non-empty function body should be accepted"
    );
}

#[test]
fn test_parse_fixture_style_function_after_awk_dollar_program() {
    let input = r#"
parse_file_col_to_csv(){
    local field="$2"
    awk "{print \$$field}" "$data_file" |
    tr '\n' ',' |
    sed 's/,/, /g; s/, $//'
}

file_modified_in_last_days(){
    local file="$1"
    local days="$2"
    if ! is_int "$days"; then
        die "bad days"
    fi
    if ! [ -f "$file" ]; then
        return 1
    elif find "$file" -mtime -"$days" -print | grep -q .; then
        return 0
    else
        local days_ago_in_seconds
        days_ago_in_seconds="$(date -d "$days days ago" '+%s')"
        if is_mac; then
            if [ "$(stat -f '%m' "$file")" -ge "$days_ago_in_seconds" ]; then
                return 0
            else
                return 1
            fi
        elif [ "$(stat -c '%Y' "$file")" -ge "$days_ago_in_seconds" ]; then
            return 0
        else
            return 1
        fi
    fi
    }
"#;
    let parsed = Parser::with_dialect(input, ShellDialect::Bash).parse();
    assert_eq!(
        parsed.status,
        ParseStatus::Clean,
        "{}",
        parsed.strict_error()
    );
}

#[test]
fn test_parse_bash_function_body_with_literal_zsh_glob_marker() {
    let input = r#"
first(){
    printf '%s\n' 'payload (# marker'
}

second(){
    :
}
"#;
    let parsed = Parser::with_dialect(input, ShellDialect::Bash).parse();
    assert_eq!(
        parsed.status,
        ParseStatus::Clean,
        "{}",
        parsed.strict_error()
    );
}

#[test]
fn test_parse_arithmetic_for_with_line_continuations_inside_function_body() {
    let input = "\
subcommand()
{
    local args_index=$positional_start
    local usage_args_index
    for ((\\
    usage_args_index = 0;  \\
    usage_args_index < ${#args[@]};  \\
    args_index++, usage_args_index++)); do
        echo ok
    done
}
";
    let parsed = Parser::new(input).parse();

    assert_eq!(parsed.status, ParseStatus::Clean);
    assert!(parsed.diagnostics.is_empty());
    assert!(parsed.terminal_error.is_none());

    let function = expect_function(&parsed.file.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(redirects.is_empty());
    assert_eq!(body.len(), 3);
    let (compound, redirects) = expect_compound(&body[2]);
    let AstCompoundCommand::ArithmeticFor(command) = compound else {
        panic!("expected arithmetic for compound command");
    };
    assert!(redirects.is_empty());
    assert_eq!(
        command.init_span.unwrap().slice(input),
        "\\\n    usage_args_index = 0"
    );
    assert_eq!(
        command.condition_span.unwrap().slice(input),
        "  \\\n    usage_args_index < ${#args[@]}"
    );
    assert_eq!(
        command.step_span.unwrap().slice(input),
        "  \\\n    args_index++, usage_args_index++"
    );
}

#[test]
fn test_parse_zsh_function_with_char_literal_arithmetic_commands() {
    let input = "function -$0-rand() {\n  local c\n  while true; do\n    sysread -s1 c || return\n    (( #c < 256 / $1 * $1 )) && break\n  done\n  typeset -g REPLY=$((#c % $1 + 1))\n}\n";
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
fn test_parse_zsh_function_compound_array_assignment_with_nested_parameter_length_in_arithmetic() {
    let input = "f() {\n  [[ -n $foo ]] && region_highlight+=(\"$((buflen+7)) $((buflen+7+${#${(MS)POSTDISPLAY##<->##}})) fg=39,bold\")\n  return 0\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_function_with_multiline_and_list_after_alias_lookup() {
    let input = "zsh-z_plugin_unload() {\n  emulate -L zsh\n\n  add-zsh-hook -D precmd _zshz_precmd\n  add-zsh-hook -d chpwd _zshz_chpwd\n\n  local x\n  for x in ${=ZSHZ[FUNCTIONS]}; do\n    (( ${+functions[$x]} )) && unfunction $x\n  done\n\n  unset ZSHZ\n\n  fpath=( \"${(@)fpath:#${0:A:h}}\" )\n\n  (( ${+aliases[${ZSHZ_CMD:-${_Z_CMD:-z}}]} )) &&\n    unalias ${ZSHZ_CMD:-${_Z_CMD:-z}}\n\n  unfunction $0\n}\n";
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
