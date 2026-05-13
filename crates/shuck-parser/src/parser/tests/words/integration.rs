use super::*;

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
        assert!(matches!(
            operator.as_ref(),
            ParameterOp::RemoveSuffixLong { .. }
        ));
    }
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
fn test_parse_case_arm_with_fd_clobber_redirect() {
    let source = "case $# in\n  0) shellspec_yield 2>|\"$SHELLSPEC_LEAK_FILE\" ;;\n  *) shellspec_yield \"$@\" 2>|\"$SHELLSPEC_LEAK_FILE\" ;;\nesac\n";
    Parser::new(source).parse().unwrap();
}
