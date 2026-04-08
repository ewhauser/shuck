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
