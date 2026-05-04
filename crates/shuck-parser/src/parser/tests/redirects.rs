use super::*;

#[test]
fn test_parse_return_preserves_assignments_and_redirects() {
    let input = "FOO=bar return 42 > out.txt";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Builtin(AstBuiltinCommand::Return(command)) = &script.body[0].command else {
        panic!("expected return builtin");
    };

    assert_eq!(command.code.as_ref().unwrap().render(input), "42");
    assert_eq!(command.assignments.len(), 1);
    assert_eq!(command.assignments[0].target.name, "FOO");
    assert_eq!(script.body[0].redirects.len(), 1);
    assert_eq!(
        redirect_word_target(&script.body[0].redirects[0]).render(input),
        "out.txt"
    );
}

#[test]
fn test_parse_redirect_out() {
    let input = "echo hello > /tmp/out";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;
    let stmt = &script.body[0];
    let cmd = expect_simple(stmt);

    assert_eq!(cmd.name.render(input), "echo");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::Output);
    assert_eq!(
        redirect_word_target(&stmt.redirects[0]).render(input),
        "/tmp/out"
    );
}

#[test]
fn test_parse_redirect_both_append() {
    let input = "echo hello &>> /tmp/out";
    let script = Parser::new(input).parse().unwrap().file;
    let stmt = &script.body[0];
    let cmd = expect_simple(stmt);

    assert_eq!(cmd.name.render(input), "echo");
    assert_eq!(stmt.redirects.len(), 2);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::Append);
    assert_eq!(
        redirect_word_target(&stmt.redirects[0]).render(input),
        "/tmp/out"
    );
    assert_eq!(stmt.redirects[1].fd, Some(2));
    assert_eq!(stmt.redirects[1].kind, RedirectKind::DupOutput);
    assert_eq!(redirect_word_target(&stmt.redirects[1]).render(input), "1");
}

#[test]
fn test_parse_redirect_append() {
    let parser = Parser::new("echo hello >> /tmp/out");
    let script = parser.parse().unwrap().file;
    let stmt = &script.body[0];

    assert_eq!(
        expect_simple(stmt).name.render("echo hello >> /tmp/out"),
        "echo"
    );
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::Append);
}

#[test]
fn test_parse_redirect_in() {
    let parser = Parser::new("cat < /tmp/in");
    let script = parser.parse().unwrap().file;
    let stmt = &script.body[0];

    assert_eq!(expect_simple(stmt).name.render("cat < /tmp/in"), "cat");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::Input);
}

#[test]
fn test_parse_redirect_read_write() {
    let input = "exec 8<> /tmp/rw";
    let script = Parser::new(input).parse().unwrap().file;
    let stmt = &script.body[0];
    let cmd = expect_simple(stmt);

    assert_eq!(cmd.name.render(input), "exec");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].fd, Some(8));
    assert_eq!(stmt.redirects[0].kind, RedirectKind::ReadWrite);
    assert_eq!(
        redirect_word_target(&stmt.redirects[0]).render(input),
        "/tmp/rw"
    );
}

#[test]
fn test_parse_named_fd_redirect_read_write() {
    let input = "exec {rw}<> /tmp/rw";
    let script = Parser::new(input).parse().unwrap().file;
    let stmt = &script.body[0];
    let cmd = expect_simple(stmt);

    assert_eq!(cmd.name.render(input), "exec");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].fd_var.as_deref(), Some("rw"));
    assert_eq!(stmt.redirects[0].kind, RedirectKind::ReadWrite);
    assert_eq!(
        redirect_word_target(&stmt.redirects[0]).render(input),
        "/tmp/rw"
    );
}

#[test]
fn test_parse_zsh_here_string_examples_from_upstream() {
    for (input, expected_name) in [
        ("cat <<< \"hello\"\n", "cat"),
        ("grep pattern <<< \"$variable\"\n", "grep"),
        ("cat 0<<< \"data\"\n", "cat"),
    ] {
        let script = Parser::with_dialect(input, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;
        let stmt = &script.body[0];
        let command = expect_simple(stmt);

        assert_eq!(command.name.render(input), expected_name);
        assert_eq!(stmt.redirects.len(), 1);
        assert_eq!(stmt.redirects[0].kind, RedirectKind::HereString);
    }
}

#[test]
fn test_parse_zsh_here_string_pipeline_binds_to_left_command() {
    let input = "command <<< \"input\" | other\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let stmt = &script.body[0];
    let AstCommand::Binary(binary) = &stmt.command else {
        panic!("expected pipeline command");
    };
    assert_eq!(binary.op, BinaryOp::Pipe);

    let left = expect_simple(&binary.left);
    assert_eq!(left.name.render(input), "command");
    assert_eq!(binary.left.redirects.len(), 1);
    assert_eq!(binary.left.redirects[0].kind, RedirectKind::HereString);

    let right = expect_simple(&binary.right);
    assert_eq!(right.name.render(input), "other");
}

#[test]
fn test_parse_process_substitution_argument_with_here_string_inside_outer_process_substitution() {
    let input = "\
readarray -t deps < <(
  grep -Fx \\
    -f <(echo \"${packages[@]}\") \\
    - <<< \"${changed[@]}\"
) || :
";
    let script = Parser::with_dialect(input, ShellDialect::Bash)
        .parse()
        .unwrap()
        .file;

    let stmt = &script.body[0];
    let AstCommand::Binary(binary) = &stmt.command else {
        panic!("expected binary command");
    };
    assert_eq!(binary.op, BinaryOp::Or);

    let readarray = expect_simple(&binary.left);
    assert_eq!(readarray.name.render(input), "readarray");

    let outer_target = binary.left.redirects[0]
        .word_target()
        .expect("expected process substitution redirect target");
    let WordPart::ProcessSubstitution { body, is_input } = &outer_target.parts[0].kind else {
        panic!("expected outer process substitution");
    };
    assert!(*is_input);

    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "grep");
    assert!(
        inner.args.iter().any(|arg| arg
            .parts
            .iter()
            .any(|part| matches!(part.kind, WordPart::ProcessSubstitution { .. }))),
        "expected inner process substitution argument"
    );
    assert_eq!(body[0].redirects.len(), 1);
    assert_eq!(body[0].redirects[0].kind, RedirectKind::HereString);
}

#[test]
fn test_parse_redirect_with_descriptor_on_continuation_line_from_upstream() {
    let input = "echo foo \\\n    2>/dev/null\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let stmt = &script.body[0];
    let command = expect_simple(stmt);

    assert_eq!(command.name.render(input), "echo");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].fd, Some(2));
    assert_eq!(stmt.redirects[0].kind, RedirectKind::Output);
    assert_eq!(
        redirect_word_target(&stmt.redirects[0]).render(input),
        "/dev/null"
    );
}

#[test]
fn test_parse_nested_process_substitutions_inside_while_redirect_in_if_body() {
    let input = "\
if [[ $enabled == true ]]; then
  local -a implicit_tasks
  while IFS='' read -r line; do
    implicit_tasks+=(\"$line\")
  done < <(comm -23 <(printf \"%s\\n\" \"${subproject_tasks[@]}\" | sort) \\
    <(printf \"%s\\n\" \"${root_tasks[@]}\" | sort))
  for task in \"${implicit_tasks[@]}\"; do
    gradle_all_tasks+=(\"$task\")
  done
fi
";
    Parser::with_dialect(input, ShellDialect::Bash)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_multiline_double_bracket_if_with_quoted_command_substitution_and_backgrounded_subshell()
 {
    let input = "\
if [[ $gradle_files_checksum != \"$(cat \"$cache_dir/$cache_name.md5\")\" ||
  ! -f \"$cache_dir/$gradle_files_checksum\" ]]; then
  (__gradle-generate-tasks-cache &> /dev/null &)
fi
";
    Parser::with_dialect(input, ShellDialect::Bash)
        .parse()
        .unwrap();
}

#[test]
fn test_extra_right_paren_after_process_substitution_is_not_swallowed() {
    let input = "echo <(true))\n";
    let error = Parser::with_dialect(input, ShellDialect::Bash)
        .parse()
        .unwrap_err();

    assert!(
        error.to_string().contains("expected command"),
        "unexpected error: {error}"
    );
}

#[test]
fn test_redirect_only_command_parses() {
    let input = ">myfile\n";
    let script = Parser::new(input).parse().unwrap().file;
    let stmt = &script.body[0];
    let command = expect_simple(stmt);

    assert!(command.name.render(input).is_empty());
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::Output);
    assert_eq!(
        redirect_word_target(&stmt.redirects[0]).render(input),
        "myfile"
    );
}

#[test]
fn test_function_conditional_body_absorbs_trailing_redirect() {
    let input = "f() [[ -n x ]] >out\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    assert!(matches!(compound, AstCompoundCommand::Conditional(_)));

    assert_eq!(redirects.len(), 1);
    assert_eq!(redirects[0].kind, RedirectKind::Output);
    assert_eq!(redirect_word_target(&redirects[0]).render(input), "out");
}

#[test]
fn test_prefix_redirect_before_for_loop_is_rejected_in_bash_mode() {
    let input = ">out for item in a b; do echo \"$item\"; done\n";
    let error = Parser::new(input)
        .parse()
        .expect_err("expected parse error");
    assert!(
        error.to_string().contains("expected command"),
        "unexpected error: {error}"
    );
}

#[test]
fn test_prefix_redirect_before_for_loop_is_allowed_in_zsh_mode() {
    let input = ">out for item in a b; do echo \"$item\"; done\n";
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert_eq!(command.targets[0].word.render(input), "item");
    assert_eq!(redirects.len(), 1);
    assert_eq!(redirect_word_target(&redirects[0]).render(input), "out");
}

#[test]
fn test_leaf_spans_track_words_assignments_and_redirects() {
    let script = Parser::new("foo=bar echo hi > out\n").parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.assignments[0].span.start.line, 1);
    assert_eq!(command.assignments[0].span.start.column, 1);
    assert_eq!(command.name.span.start.column, 9);
    assert_eq!(command.args[0].span.start.column, 14);
    assert_eq!(script.body[0].redirects[0].span.start.column, 17);
    assert_eq!(
        redirect_word_target(&script.body[0].redirects[0])
            .span
            .start
            .column,
        19
    );
}

#[test]
fn test_identifier_spans_track_function_loop_assignment_and_fd_var_names() {
    let input = "\
my_fn() { true; }
for item in a; do echo \"$item\"; done
select choice in a; do echo \"$choice\"; done
foo[10]=bar
exec {myfd}>&-
coproc worker { true; }
";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function definition");
    };
    assert_eq!(function.header.entries[0].word.span.slice(input), "my_fn");

    let (compound, _) = expect_compound(&script.body[1]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };
    assert_eq!(command.targets[0].span.slice(input), "item");

    let (compound, _) = expect_compound(&script.body[2]);
    let AstCompoundCommand::Select(command) = compound else {
        panic!("expected select loop");
    };
    assert_eq!(command.variable_span.slice(input), "choice");

    let AstCommand::Simple(command) = &script.body[3].command else {
        panic!("expected assignment-only simple command");
    };
    assert_eq!(command.assignments[0].target.name_span.slice(input), "foo");
    expect_subscript(&command.assignments[0].target, input, "10");

    let _command = expect_simple(&script.body[4]);
    assert_eq!(
        script.body[4].redirects[0]
            .fd_var_span
            .unwrap()
            .slice(input),
        "myfd"
    );

    let (compound, _) = expect_compound(&script.body[5]);
    let AstCompoundCommand::Coproc(command) = compound else {
        panic!("expected coproc command");
    };
    assert_eq!(command.name_span.unwrap().slice(input), "worker");
}
