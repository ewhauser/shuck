use super::*;
#[test]
fn test_heredoc_pipe() {
    let parser = Parser::new("cat <<EOF | sort\nc\na\nb\nEOF\n");
    let script = parser.parse().unwrap().file;
    assert!(
        matches!(&script.body[0].command, AstCommand::Binary(_)),
        "heredoc with pipe should parse as a binary pipe"
    );
}

#[test]
fn test_prefix_heredoc_before_command_in_pipeline_parses() {
    let input = "<<EOF tac | tr '\\n' 'X'\none\ntwo\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let pipeline = expect_binary(&script.body[0]);
    assert_eq!(pipeline.op, BinaryOp::Pipe);
    let command = expect_simple(&pipeline.left);
    assert_eq!(command.name.render(input), "tac");
    assert_eq!(pipeline.left.redirects.len(), 1);
    assert_eq!(pipeline.left.redirects[0].kind, RedirectKind::HereDoc);
}

#[test]
fn test_function_definition_absorbs_trailing_heredoc_redirect() {
    let input = "f() { cat; } <<EOF\nhello\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let function = expect_function(&script.body[0]);
    let (_, redirects) = expect_compound(function.body.as_ref());
    assert!(!function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert_eq!(redirects.len(), 1);
    assert_eq!(redirects[0].kind, RedirectKind::HereDoc);
}

#[test]
fn test_function_body_command_with_heredoc_parses() {
    let input = "f() {\n  read head << EOF\nref: refs/heads/dev/andy\nEOF\n}\nf\n";
    let script = Parser::new(input).parse().unwrap().file;

    assert_eq!(script.body.len(), 2);

    let function = expect_function(&script.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(!function.uses_function_keyword());
    assert!(function.has_name_parens());
    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
}

#[test]
fn test_named_fd_heredoc_redirect_keeps_fd_var_metadata() {
    let input = "exec {docfd}<<EOF\nhello\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let stmt = &script.body[0];
    let command = expect_simple(stmt);
    assert_eq!(command.name.render(input), "exec");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::HereDoc);
    assert_eq!(stmt.redirects[0].fd_var.as_deref(), Some("docfd"));
    assert_eq!(stmt.redirects[0].fd_var_span.unwrap().slice(input), "docfd");
}

#[test]
fn test_named_fd_heredoc_redirect_keeps_fd_var_metadata_across_line_continuation() {
    let input = "exec {docfd}\\\n<<EOF\nhello\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let stmt = &script.body[0];
    let command = expect_simple(stmt);
    assert_eq!(command.name.render(input), "exec");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::HereDoc);
    assert_eq!(stmt.redirects[0].fd_var.as_deref(), Some("docfd"));
    assert_eq!(stmt.redirects[0].fd_var_span.unwrap().slice(input), "docfd");
}

#[test]
fn test_quoted_word_before_line_continuation_heredoc_stays_a_plain_argument() {
    let input = "exec \"{docfd}\"\\\n<<EOF\nhello\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let stmt = &script.body[0];
    let command = expect_simple(stmt);
    assert_eq!(command.name.render(input), "exec");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(input), "{docfd}");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::HereDoc);
    assert_eq!(stmt.redirects[0].fd_var.as_deref(), None);
    assert_eq!(stmt.redirects[0].fd_var_span, None);
}

#[test]
fn test_spaced_word_before_heredoc_stays_a_plain_argument() {
    let input = "echo {docfd} <<EOF\nhello\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let stmt = &script.body[0];
    let command = expect_simple(stmt);
    assert_eq!(command.name.render(input), "echo");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(input), "{docfd}");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::HereDoc);
    assert_eq!(stmt.redirects[0].fd_var.as_deref(), None);
    assert_eq!(stmt.redirects[0].fd_var_span, None);
}

#[test]
fn test_spaced_word_before_output_redirect_stays_a_plain_argument() {
    let input = "echo {docfd} >/tmp/out\n";
    let script = Parser::new(input).parse().unwrap().file;

    let stmt = &script.body[0];
    let command = expect_simple(stmt);
    assert_eq!(command.name.render(input), "echo");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(input), "{docfd}");
    assert_eq!(stmt.redirects.len(), 1);
    assert_eq!(stmt.redirects[0].kind, RedirectKind::Output);
    assert_eq!(stmt.redirects[0].fd_var.as_deref(), None);
    assert_eq!(stmt.redirects[0].fd_var_span, None);
}

#[test]
fn test_dynamic_heredoc_delimiter_is_rejected() {
    let parser = Parser::new("cat <<\"$@\"\nbody\n$@\n");
    assert!(
        parser.parse().is_err(),
        "dynamic heredoc delimiter should fail"
    );
}

#[test]
fn test_non_static_heredoc_delimiter_forms_are_rejected() {
    let cases = [
        ("short parameter", "cat <<$bar\n"),
        ("brace parameter", "cat <<${bar}\n"),
        ("command substitution", "cat <<$(bar)\n"),
        ("backquoted command substitution", "cat <<`bar`\n"),
        ("arithmetic expansion", "cat <<$((1 + 2))\n"),
        ("special parameter", "cat <<$-\n"),
        ("quoted parameter expansion", "cat <<\"$bar\"\n"),
    ];

    for (name, input) in cases {
        let error = Parser::new(input).parse().unwrap_err();
        let Error::Parse { message, .. } = error;
        assert_eq!(
            message, "expected static heredoc delimiter",
            "{name} should fail via the static-delimiter check"
        );
    }
}

#[test]
fn test_heredoc_multiple_on_line() {
    let input = "while cat <<E1 && cat <<E2; do cat <<E3; break; done\n1\nE1\n2\nE2\n3\nE3\n";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;
    assert_eq!(script.body.len(), 1);
    let (compound, _) = expect_compound(&script.body[0]);
    if let AstCompoundCommand::While(w) = compound {
        assert!(
            !w.condition.is_empty(),
            "while condition should be non-empty"
        );
        assert!(!w.body.is_empty(), "while body should be non-empty");
    } else {
        panic!("expected While compound command");
    }
}

#[test]
fn test_heredoc_multiple_lines_preserve_while_do_boundary() {
    let input = "while cat <<E1 && cat <<E2\n1\nE1\n2\nE2\ndo\n  cat <<E3\n3\nE3\n  break\ndone\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    assert!(redirects.is_empty());
    let AstCompoundCommand::While(command) = compound else {
        panic!("expected while command");
    };
    assert_eq!(command.condition.len(), 1);
    assert_eq!(command.body.len(), 2);
}

#[test]
fn test_heredoc_target_preserves_body_span() {
    let input = "cat <<'EOF'\nhello $name\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let stmt = &script.body[0];
    let _command = expect_simple(stmt);
    assert_eq!(stmt.redirects.len(), 1);

    let redirect = &stmt.redirects[0];
    let heredoc = redirect_heredoc(redirect);
    assert_eq!(heredoc.body.span.slice(input), "hello $name\n");
    assert!(heredoc_body_is_literal(&heredoc.body));
}

#[test]
fn test_heredoc_delimiter_metadata_tracks_flags_and_spans() {
    let input = "cat <<EOF\nhello\nEOF\ncat <<'EOF'\nhello\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let unquoted_stmt = &script.body[0];
    let _unquoted = expect_simple(unquoted_stmt);
    let unquoted_redirect = &unquoted_stmt.redirects[0];
    let unquoted_heredoc = redirect_heredoc(unquoted_redirect);
    assert_eq!(unquoted_redirect.span.slice(input), "<<EOF");
    assert_eq!(unquoted_heredoc.delimiter.span.slice(input), "EOF");
    assert_eq!(unquoted_heredoc.delimiter.raw.span.slice(input), "EOF");
    assert_eq!(unquoted_heredoc.delimiter.cooked, "EOF");
    assert!(!unquoted_heredoc.delimiter.quoted);
    assert!(unquoted_heredoc.delimiter.expands_body);
    assert!(!unquoted_heredoc.delimiter.strip_tabs);

    let quoted_stmt = &script.body[1];
    let _quoted = expect_simple(quoted_stmt);
    let quoted_redirect = &quoted_stmt.redirects[0];
    let quoted_heredoc = redirect_heredoc(quoted_redirect);
    assert_eq!(quoted_redirect.span.slice(input), "<<'EOF'");
    assert_eq!(quoted_heredoc.delimiter.span.slice(input), "'EOF'");
    assert_eq!(quoted_heredoc.delimiter.raw.span.slice(input), "'EOF'");
    assert_eq!(quoted_heredoc.delimiter.cooked, "EOF");
    assert!(quoted_heredoc.delimiter.quoted);
    assert!(!quoted_heredoc.delimiter.expands_body);
    assert!(!quoted_heredoc.delimiter.strip_tabs);
}

#[test]
fn test_heredoc_delimiter_preserves_mixed_quoted_raw_and_cooked_value() {
    let input = "cat <<'EOF'\"2\"\nbody\nEOF2\n";
    let script = Parser::new(input).parse().unwrap().file;

    let stmt = &script.body[0];
    let _command = expect_simple(stmt);
    let redirect = &stmt.redirects[0];
    let heredoc = redirect_heredoc(redirect);

    assert_eq!(redirect.span.slice(input), "<<'EOF'\"2\"");
    assert_eq!(heredoc.delimiter.raw.span.slice(input), "'EOF'\"2\"");
    assert_eq!(heredoc.delimiter.cooked, "EOF2");
    assert!(heredoc.delimiter.quoted);
    assert!(!heredoc.delimiter.expands_body);
}

#[test]
fn test_backslash_escaped_heredoc_delimiter_is_treated_as_quoted_static_text() {
    let input = "cat <<\\EOF\nhello $name\nEOF\n";
    for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
        let script = Parser::with_dialect(input, dialect).parse().unwrap().file;

        let stmt = &script.body[0];
        let _command = expect_simple(stmt);
        let redirect = &stmt.redirects[0];
        let heredoc = redirect_heredoc(redirect);

        assert_eq!(redirect.span.slice(input), "<<\\EOF");
        assert_eq!(heredoc.delimiter.span.slice(input), "\\EOF");
        assert_eq!(heredoc.delimiter.raw.span.slice(input), "\\EOF");
        assert_eq!(heredoc.delimiter.cooked, "EOF", "dialect: {dialect:?}");
        assert!(heredoc.delimiter.quoted, "dialect: {dialect:?}");
        assert!(!heredoc.delimiter.expands_body, "dialect: {dialect:?}");
        assert!(!heredoc.delimiter.strip_tabs, "dialect: {dialect:?}");
        assert!(
            heredoc_body_is_literal(&heredoc.body),
            "dialect: {dialect:?}"
        );
        assert_eq!(heredoc.body.render(input), "hello $name\n");
    }
}

#[test]
fn test_backslash_escaped_heredoc_inside_command_substitution_stays_quoted_in_posix() {
    let input = "\
build=\"$(command cat <<\\EOF
hello $name
EOF
)\"
";
    let script = Parser::with_dialect(input, ShellDialect::Posix)
        .parse()
        .unwrap()
        .file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted assignment value");
    };
    let WordPart::CommandSubstitution { body, .. } = &parts[0].kind else {
        panic!("expected command substitution");
    };
    let command = expect_simple(&body[0]);
    assert_eq!(
        command.args.len(),
        1,
        "args: {:?}",
        command
            .args
            .iter()
            .map(|word| word.render(input))
            .collect::<Vec<_>>()
    );
    assert_eq!(command.args[0].render(input), "cat");
    assert_eq!(body[0].redirects.len(), 1);
    let heredoc = redirect_heredoc(&body[0].redirects[0]);

    assert!(heredoc.delimiter.quoted);
    assert!(!heredoc.delimiter.expands_body);
    assert!(heredoc_body_is_literal(&heredoc.body));
}

#[test]
fn test_posix_quoted_heredoc_in_command_substitution_does_not_leak_body_statements() {
    let input = "\
build=\"$(command cat <<\\END
outdir=\"$(command pwd)\"

if command -v mktemp >/dev/null 2>&1; then
  workdir=\"$(command mktemp -d \"${TMPDIR:-/tmp}\"/gitstatus-build.XXXXXXXXXX)\"
else
  workdir=\"${TMPDIR:-/tmp}/gitstatus-build.tmp.$$\" 
  command mkdir -- \"$workdir\"
fi

if [ -n \"$gitstatus_install_tools\" ]; then
  case \"$gitstatus_kernel\" in
    darwin)
      if command -v port >/dev/null 2>&1; then
        sudo port -N install libiconv cmake wget
      elif command -v brew >/dev/null 2>&1; then
        for formula in libiconv cmake git wget; do
          if command brew ls --version \"$formula\" &>/dev/null; then
            command brew upgrade \"$formula\"
          else
            command brew install \"$formula\"
          fi
        done
      fi
    ;;
  esac
fi

case \"$gitstatus_cpu\" in
  powerpc64|powerpc64le)
    archflag=\"-mcpu\"
  ;;
  *)
    archflag=\"-march\"
  ;;
esac

case \"$gitstatus_arch\" in
  e2k)
    nopltflag=\"\"
  ;;
  *)
    nopltflag=\"-fno-plt\"
  ;;
esac

cflags=\"$archflag=$gitstatus_cpu $nopltflag -D_FORTIFY_SOURCE=2 -Wformat -Werror=format-security -fpie\"
END
)\"
";
    let script = Parser::with_dialect(input, ShellDialect::Posix)
        .parse()
        .unwrap()
        .file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted assignment value");
    };
    assert_eq!(parts.len(), 1, "parts: {parts:#?}");
    let WordPart::CommandSubstitution { body, .. } = &parts[0].kind else {
        panic!("expected command substitution");
    };

    assert_eq!(body.len(), 1, "body: {body:#?}");
    let _command = expect_simple(&body[0]);
    let heredoc = redirect_heredoc(&body[0].redirects[0]);
    assert!(heredoc.delimiter.quoted);
    assert!(!heredoc.delimiter.expands_body);
    assert!(heredoc_body_is_literal(&heredoc.body));
}

#[test]
fn test_bash_quoted_heredoc_in_command_substitution_does_not_leak_body_statements() {
    let input = "\
build=\"$(command cat <<\\END
outdir=\"$(command pwd)\"

if command -v mktemp >/dev/null 2>&1; then
  workdir=\"$(command mktemp -d \"${TMPDIR:-/tmp}\"/gitstatus-build.XXXXXXXXXX)\"
else
  workdir=\"${TMPDIR:-/tmp}/gitstatus-build.tmp.$$\" 
  command mkdir -- \"$workdir\"
fi

if [ -n \"$gitstatus_install_tools\" ]; then
  case \"$gitstatus_kernel\" in
    darwin)
      if command -v port >/dev/null 2>&1; then
        sudo port -N install libiconv cmake wget
      elif command -v brew >/dev/null 2>&1; then
        for formula in libiconv cmake git wget; do
          if command brew ls --version \"$formula\" &>/dev/null; then
            command brew upgrade \"$formula\"
          else
            command brew install \"$formula\"
          fi
        done
      fi
    ;;
  esac
fi

case \"$gitstatus_cpu\" in
  powerpc64|powerpc64le)
    archflag=\"-mcpu\"
  ;;
  *)
    archflag=\"-march\"
  ;;
esac

case \"$gitstatus_arch\" in
  e2k)
    nopltflag=\"\"
  ;;
  *)
    nopltflag=\"-fno-plt\"
  ;;
esac

cflags=\"$archflag=$gitstatus_cpu $nopltflag -D_FORTIFY_SOURCE=2 -Wformat -Werror=format-security -fpie\"
command cat >&2 <<-END
\tSUCCESS
\tEND
END
)\"
";
    let script = Parser::with_dialect(input, ShellDialect::Bash)
        .parse()
        .unwrap()
        .file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted assignment value");
    };
    let WordPart::CommandSubstitution { body, .. } = &parts[0].kind else {
        panic!("expected command substitution");
    };

    assert_eq!(body.len(), 1, "body: {body:#?}");
    let _command = expect_simple(&body[0]);
    let heredoc = redirect_heredoc(&body[0].redirects[0]);
    assert!(heredoc.delimiter.quoted);
    assert!(!heredoc.delimiter.expands_body);
    assert!(heredoc_body_is_literal(&heredoc.body));
}

#[test]
fn test_heredoc_strip_tabs_sets_delimiter_metadata() {
    let input = "cat <<-EOF\n\t$NAME\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let stmt = &script.body[0];
    let _command = expect_simple(stmt);
    let redirect = &stmt.redirects[0];
    let heredoc = redirect_heredoc(redirect);

    assert_eq!(redirect.span.slice(input), "<<-EOF");
    assert!(heredoc.delimiter.strip_tabs);
    assert!(heredoc.delimiter.expands_body);
    assert_eq!(heredoc.delimiter.cooked, "EOF");
}

#[test]
fn test_heredoc_targets_preserve_quoted_and_unquoted_decode_behavior() {
    let input = "cat <<EOF\nhello $name\nEOF\ncat <<'EOF'\nhello $name\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let unquoted_target = &redirect_heredoc(&script.body[0].redirects[0]).body;
    assert!(!heredoc_body_is_literal(unquoted_target));
    assert_eq!(unquoted_target.render(input), "hello $name\n");
    let unquoted_slices = heredoc_top_level_part_slices(unquoted_target, input);
    assert_eq!(unquoted_slices, vec!["hello ", "$name", "\n"]);
    assert!(matches!(
        unquoted_target.parts[1].kind,
        shuck_ast::HeredocBodyPart::Variable(_)
    ));

    let quoted_target = &redirect_heredoc(&script.body[1].redirects[0]).body;
    assert!(heredoc_body_is_literal(quoted_target));
    assert_eq!(quoted_target.render(input), "hello $name\n");
    assert!(matches!(
        quoted_target.parts.as_slice(),
        [part] if matches!(&part.kind, shuck_ast::HeredocBodyPart::Literal(_))
    ));
}

#[test]
fn test_unquoted_heredoc_body_preserves_multiple_quoted_fragments() {
    let input = "cat <<EOF\nbefore '$HOME' and \"$USER\"\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let body = &redirect_heredoc(&script.body[0].redirects[0]).body;

    assert!(!heredoc_body_is_literal(body));
    assert_eq!(
        heredoc_top_level_part_slices(body, input),
        vec!["before '", "$HOME", "' and \"", "$USER", "\"\n"]
    );
    assert!(matches!(
        body.parts[1].kind,
        shuck_ast::HeredocBodyPart::Variable(_)
    ));
    assert!(matches!(
        body.parts[3].kind,
        shuck_ast::HeredocBodyPart::Variable(_)
    ));
}

#[test]
fn test_unquoted_heredoc_body_keeps_dollar_quoted_forms_literal() {
    let input = "cat <<EOF\n$'line\\n' $\"hello\" ${name}\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let body = &redirect_heredoc(&script.body[0].redirects[0]).body;

    assert_eq!(
        heredoc_top_level_part_slices(body, input),
        vec!["$'line\\n' ", "$\"hello\" ", "${name}", "\n"]
    );
    assert!(matches!(
        body.parts[0].kind,
        shuck_ast::HeredocBodyPart::Literal(_)
    ));
    assert!(matches!(
        body.parts[1].kind,
        shuck_ast::HeredocBodyPart::Literal(_)
    ));
    assert!(matches!(
        body.parts[2].kind,
        shuck_ast::HeredocBodyPart::Parameter(_)
    ));
}

#[test]
fn test_unquoted_heredoc_body_keeps_later_expansions_live_after_quoted_lines() {
    let input = "\
cat <<EOF > \"$archname\"
#!/bin/sh
ORIG_UMASK=`umask`
if test \"$KEEP_UMASK\" = n; then
    umask 077
fi

CRCsum=\"$CRCsum\"
archdirname=\"$archdirname\"
EOF
";
    let script = Parser::new(input).parse().unwrap().file;

    let body = &redirect_heredoc(&script.body[0].redirects[0]).body;
    let slices = heredoc_top_level_part_slices(body, input);

    assert!(
        body.parts.iter().any(|part| {
            matches!(
                &part.kind,
                shuck_ast::HeredocBodyPart::Variable(name) if name.as_str() == "CRCsum"
            )
        }),
        "expected heredoc body to keep $CRCsum live: {slices:?}"
    );
    assert!(
        body.parts.iter().any(|part| {
            matches!(
                &part.kind,
                shuck_ast::HeredocBodyPart::Variable(name) if name.as_str() == "archdirname"
            )
        }),
        "expected heredoc body to keep $archdirname live: {slices:?}"
    );
}

#[test]
fn test_unquoted_heredoc_body_leaves_unmatched_single_quote_literal() {
    let input = "cat <<EOF\n'$HOME\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let body = &redirect_heredoc(&script.body[0].redirects[0]).body;

    assert!(
        !body
            .parts
            .iter()
            .any(|part| matches!(part.kind, shuck_ast::HeredocBodyPart::Parameter(_)))
    );
    assert_eq!(body.render_syntax(input), "'$HOME\n");
}

#[test]
fn test_strip_tabs_heredoc_body_preserves_single_quoted_fragments() {
    let input = "cat <<-EOF\n\t'$HOME'\nEOF\n";
    let script = Parser::new(input).parse().unwrap().file;

    let heredoc = redirect_heredoc(&script.body[0].redirects[0]);

    assert!(heredoc.delimiter.strip_tabs);
    assert_eq!(heredoc.body.parts.len(), 3);
    assert!(matches!(
        heredoc.body.parts[0].kind,
        shuck_ast::HeredocBodyPart::Literal(_)
    ));
    assert!(matches!(
        heredoc.body.parts[1].kind,
        shuck_ast::HeredocBodyPart::Variable(_)
    ));
    assert!(matches!(
        heredoc.body.parts[2].kind,
        shuck_ast::HeredocBodyPart::Literal(_)
    ));
    assert_eq!(heredoc.body.render_syntax(input), "'$HOME'\n");
}

#[test]
fn test_strip_tabs_heredoc_command_substitution_keeps_nested_command_spans() {
    let input = "\
case \"${tag_type}\" in
\t*)
\t\ttermux_error_exit <<-EndOfError
\t\t\tERROR: Invalid TERMUX_PKG_UPDATE_TAG_TYPE: '${tag_type}'.
\t\t\tAllowed values: 'newest-tag', 'latest-release-tag', 'latest-regex'.
\t\tEndOfError
\t;;
esac

case \"${http_code}\" in
\t404)
\t\ttermux_error_exit <<-EndOfError
\t\t\tNo '${tag_type}' found. (${api_url})
\t\t\tHTTP code: ${http_code}
\t\t\tTry using '$(
\t\t\t\tif [[ \"${tag_type}\" == \"newest-tag\" ]]; then
\t\t\t\t\techo \"latest-release-tag\"
\t\t\t\telse
\t\t\t\t\techo \"newest-tag\"
\t\t\t\tfi
\t\t\t)'.
\t\tEndOfError
\t;;
esac
";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[1]);
    let AstCompoundCommand::Case(case) = compound else {
        panic!("expected case command");
    };
    let command = expect_simple(&case.cases[0].body[0]);
    let heredoc = redirect_heredoc(&case.cases[0].body[0].redirects[0]);

    assert_eq!(command.name.render(input), "termux_error_exit");

    let command_substitution = heredoc
        .body
        .parts
        .iter()
        .find_map(|part| match &part.kind {
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, .. } => Some(body),
            _ => None,
        })
        .expect("expected command substitution inside heredoc");

    let (if_compound, _) = expect_compound(&command_substitution[0]);
    let AstCompoundCommand::If(if_command) = if_compound else {
        panic!("expected if command inside command substitution");
    };

    let AstCommand::Compound(AstCompoundCommand::Conditional(conditional)) =
        &if_command.condition[0].command
    else {
        panic!("expected conditional command in if header");
    };

    assert_eq!(
        conditional.span.slice(input),
        "[[ \"${tag_type}\" == \"newest-tag\" ]]"
    );
    assert_eq!(
        expect_simple(&if_command.then_branch[0]).span.slice(input),
        "echo \"latest-release-tag\"\n"
    );
    assert_eq!(
        expect_simple(&if_command.else_branch.as_ref().unwrap()[0])
            .span
            .slice(input),
        "echo \"newest-tag\"\n"
    );
}

#[test]
fn test_comment_ranges_heredoc_no_false_comments() {
    // Lines with # inside a heredoc must NOT produce Comment entries
    let source = "cat <<EOF\n# not a comment\nline two\nEOF\n# real\n";
    let output = Parser::new(source).parse().unwrap();
    assert_comment_ranges_valid(source, &output);
    // Only the real comment after EOF should be collected
    let texts: Vec<&str> = collect_file_comments(&output.file)
        .iter()
        .map(|c| c.range.slice(source))
        .collect();
    assert!(
        !texts.iter().any(|t| t.contains("not a comment")),
        "heredoc body produced a false comment: {texts:?}"
    );
}

#[test]
fn test_comment_ranges_heredoc_with_unicode() {
    let source = "cat <<EOF\n# 你好\ncafé\nEOF\n# end\n";
    let output = Parser::new(source).parse().unwrap();
    assert_comment_ranges_valid(source, &output);
}

#[test]
fn test_comment_ranges_heredoc_desktop_entry() {
    // Reproduces the pattern from the distrobox corpus file:
    // a heredoc containing lines with ${var} expansions and no actual comments
    let source = r#"cat << EOF > "${HOME}/test.desktop"
[Desktop Entry]
Name=${entry_name}
GenericName=Terminal entering ${entry_name}
Comment=Terminal entering ${entry_name}
Categories=Distrobox;System;Utility
Exec=${distrobox_path}/distrobox enter ${extra_flags} ${container_name}
Icon=${icon}
Terminal=true
Type=Application
EOF
# done
"#;
    let output = Parser::new(source).parse().unwrap();
    assert_comment_ranges_valid(source, &output);
    let texts: Vec<&str> = collect_file_comments(&output.file)
        .iter()
        .map(|c| c.range.slice(source))
        .collect();
    // None of the heredoc lines should appear as comments
    for text in &texts {
        assert!(
            !text.contains("Desktop") && !text.contains("entry_name"),
            "heredoc body leaked as comment: {text:?}"
        );
    }
}
