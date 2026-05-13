use super::*;

#[test]
fn test_parse_variable() {
    let parser = Parser::new("echo $HOME");
    let script = parser.parse().unwrap().file;

    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.args.len(), 1);
        assert_eq!(cmd.args[0].parts.len(), 1);
        assert!(matches!(&cmd.args[0].parts[0].kind, WordPart::Variable(v) if v == "HOME"));
    } else {
        panic!("expected simple command");
    }
}

#[test]
fn test_parse_escaped_dollar_expansions_stay_literal_in_script_words() {
    let input = r#"echo \$HOME \${USER}"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 2);

    let expected = [("$HOME", r#"\$HOME"#), ("${USER}", r#"\${USER}"#)];
    for (word, (decoded, syntax)) in command.args.iter().zip(expected) {
        assert_eq!(word.render(input), decoded);
        assert_eq!(word.render_syntax(input), syntax);
        assert!(matches!(
            word.parts.as_slice(),
            [WordPartNode {
                kind: WordPart::Literal(text),
                ..
            }] if text.is_source_backed() && text.as_str(input, word.parts[0].span) == decoded
        ));
    }
}

#[test]
fn test_parse_escaped_braced_parameter_keeps_inner_expansions_live() {
    let input = r#"echo \${x:-$HOME} \${${1}}"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 2);

    let default_template = &command.args[0];
    assert_eq!(default_template.render(input), "${x:-$HOME}");
    assert_eq!(default_template.render_syntax(input), r#"\${x:-$HOME}"#);
    assert!(word_part_tree_contains_variable(
        &default_template.parts,
        "HOME"
    ));

    let indirect_template = &command.args[1];
    assert_eq!(indirect_template.render(input), "${${1}}");
    assert_eq!(indirect_template.render_syntax(input), r#"\${${1}}"#);
    let mut names = Vec::new();
    collect_bourne_parameter_names(&indirect_template.parts, &mut names);
    assert_eq!(names, vec!["1"]);
}

#[test]
fn test_parse_escaped_backslash_then_variable_keeps_variable_live() {
    let input = "echo \\\\$HOME\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];
    assert!(matches!(
        word.parts.as_slice(),
        [
            WordPartNode {
                kind: WordPart::Literal(text),
                ..
            },
            WordPartNode {
                kind: WordPart::Variable(name),
                ..
            }
        ] if text.as_str(input, word.parts[0].span) == "\\"
            && name.as_str() == "HOME"
    ));
}

#[test]
fn test_parse_mixed_quoted_and_cooked_plain_continuation_keeps_variable_live() {
    let input = "echo \"x\"\\\\$HOME\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(matches!(
        word.parts.as_slice(),
        [
            WordPartNode {
                kind: WordPart::DoubleQuoted { parts, .. },
                ..
            },
            WordPartNode {
                kind: WordPart::Literal(text),
                ..
            },
            WordPartNode {
                kind: WordPart::Variable(name),
                ..
            }
        ] if matches!(
            parts.as_slice(),
            [WordPartNode {
                kind: WordPart::Literal(inner),
                ..
            }] if inner.as_str(input, parts[0].span) == "x"
        ) && text.as_str(input, word.parts[1].span) == "\\"
            && name.as_str() == "HOME"
    ));
}

#[test]
fn test_parse_escaped_quote_before_command_substitution_keeps_substitution_live() {
    let input = "echo TERMUX_SUBPKG_INCLUDE=\\\"$(find ${_ADD_PREFIX}lib{,32})\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(
        word.parts
            .iter()
            .any(|part| !matches!(part.kind, WordPart::Literal(_))),
        "parts: {:?}",
        word.parts
    );
    assert_eq!(word.brace_syntax().len(), 0);
}

#[test]
fn test_parse_escaped_command_substitution_stays_literal_in_double_quotes() {
    let input = r#"echo "\$(pwd)""#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.render(input), "$(pwd)");
    assert_eq!(word.render_syntax(input), r#""\$(pwd)""#);
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert!(matches!(
        parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Literal(text),
            ..
        }] if text.is_source_backed() && text.as_str(input, parts[0].span) == "$(pwd)"
    ));
}

#[test]
fn test_parse_escaped_backslash_before_command_substitution_keeps_substitution_live() {
    let input = "echo \"\\\\$(pwd)\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
    assert_eq!(slices, vec![r#"\\"#, "$(pwd)"]);

    let WordPart::Literal(text) = &parts[0].kind else {
        panic!("expected literal backslash prefix");
    };
    assert_eq!(text.as_str(input, parts[0].span), r#"\"#);

    let WordPart::CommandSubstitution { body, .. } = &parts[1].kind else {
        panic!("expected live command substitution");
    };
    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "pwd");
}

#[test]
fn test_parse_escaped_literal_before_command_substitution_keeps_following_substitution_live() {
    let input = "echo $VERSION\\_$(echo hi)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(matches!(
        word.parts.as_slice(),
        [
            WordPartNode {
                kind: WordPart::Variable(name),
                ..
            },
            WordPartNode {
                kind: WordPart::Literal(text),
                ..
            },
            WordPartNode {
                kind: WordPart::CommandSubstitution { body, .. },
                span,
            }
        ] if name.as_str() == "VERSION"
            && text.as_str(input, word.parts[1].span) == "_"
            && span.slice(input) == "$(echo hi)"
            && matches!(&body[0].command, AstCommand::Simple(inner) if inner.name.render(input) == "echo")
    ));
}

#[test]
fn test_parse_escaped_quotes_before_command_substitution_keep_nested_pipeline_live() {
    let input = "#!/bin/sh\necho -n \"\\\"adp_$(echo $var | tr A-Z a-z)\\\": [\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[1];
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    let WordPart::CommandSubstitution { body, .. } = &parts[1].kind else {
        panic!("expected command substitution");
    };

    let AstCommand::Binary(binary) = &body[0].command else {
        panic!("expected piped command");
    };
    let first = expect_simple(&binary.left);
    assert_eq!(first.name.render(input), "echo");
    assert_eq!(first.args[0].render(input), "$var");

    let second = expect_simple(&binary.right);
    assert_eq!(second.name.render(input), "tr");
    assert_eq!(second.args[0].render(input), "A-Z");
    assert_eq!(second.args[1].render(input), "a-z");
}

#[test]
fn test_parse_escaped_quotes_after_default_expansion_keep_command_substitution_live() {
    let input = "label=\",label=\\\"${fallback:=value}$(render value $line)\\\"\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted assignment value");
    };
    let WordPart::CommandSubstitution { body, .. } = &parts[2].kind else {
        panic!("expected command substitution after default expansion");
    };

    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "render");
    assert_eq!(inner.args[1].render(input), "$line");
}

#[test]
fn test_parse_command_substitution_with_piped_tr_after_quoted_variable_keeps_nested_pipeline_live()
{
    let input = "ATLAS_SHARED=$(echo \"$ATLAS_SHARED\"|cut -b 1|tr a-z A-Z)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::CommandSubstitution { body, .. } = &word.parts[0].kind else {
        panic!("expected command substitution");
    };

    let AstCommand::Binary(pipeline) = &body[0].command else {
        panic!("expected piped command");
    };
    let AstCommand::Binary(prefix) = &pipeline.left.command else {
        panic!("expected nested pipeline");
    };
    let first = expect_simple(&prefix.left);
    assert_eq!(first.name.render(input), "echo");
    assert_eq!(first.args[0].render(input), "$ATLAS_SHARED");

    let middle = expect_simple(&prefix.right);
    assert_eq!(middle.name.render(input), "cut");
    assert_eq!(middle.args[0].render(input), "-b");
    assert_eq!(middle.args[1].render(input), "1");

    let tr_command = expect_simple(&pipeline.right);
    assert_eq!(tr_command.name.render(input), "tr");
    assert_eq!(tr_command.args[0].render(input), "a-z");
    assert_eq!(tr_command.args[1].render(input), "A-Z");
}

#[test]
fn test_parse_positional_parameters() {
    let parser = Parser::new("echo $@ $*");
    let script = parser.parse().unwrap().file;

    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.args.len(), 2);
        assert!(matches!(&cmd.args[0].parts[0].kind, WordPart::Variable(v) if v == "@"));
        assert!(matches!(&cmd.args[1].parts[0].kind, WordPart::Variable(v) if v == "*"));
    } else {
        panic!("expected simple command");
    }
}

#[test]
fn test_substring_offset_preserves_postfix_and_nested_length_arithmetic() {
    let input = "echo \"${chars:spin_i++%${#chars}:1}\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let WordPart::DoubleQuoted { parts, .. } = &command.args[0].parts[0].kind else {
        panic!("expected double-quoted word");
    };

    let (_, offset_ast, length_ast) = expect_substring_part(&parts[0].kind);
    let ArithmeticExpr::Binary { left, op, right } = &offset_ast
        .as_ref()
        .expect("expected offset arithmetic AST")
        .kind
    else {
        panic!("expected modulo offset expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Modulo);
    let ArithmeticExpr::Postfix { expr, op } = &left.kind else {
        panic!("expected postfix increment on the left side");
    };
    assert_eq!(*op, ArithmeticPostfixOp::Increment);
    expect_variable(expr, "spin_i");
    expect_shell_word(right, input, "${#chars}");
    expect_number(
        length_ast.as_ref().expect("expected substring length AST"),
        input,
        "1",
    );
}

#[test]
fn test_parameter_forms_preserve_selector_kinds() {
    let input = "echo ${arr[@]} ${arr[*]} ${#arr[@]} ${!arr[*]} ${arr[@]:1:2}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let reference = expect_array_access(&command.args[0]);
    assert_eq!(
        reference.subscript.as_deref().and_then(Subscript::selector),
        Some(SubscriptSelector::At)
    );

    let reference = expect_array_access(&command.args[1]);
    assert_eq!(
        reference.subscript.as_deref().and_then(Subscript::selector),
        Some(SubscriptSelector::Star)
    );

    let reference = expect_array_length_part(&command.args[2].parts[0].kind);
    assert_eq!(
        reference.subscript.as_deref().and_then(Subscript::selector),
        Some(SubscriptSelector::At)
    );

    let reference = expect_array_indices_part(&command.args[3].parts[0].kind);
    assert_eq!(
        reference.subscript.as_deref().and_then(Subscript::selector),
        Some(SubscriptSelector::Star)
    );

    let (reference, _, _) = expect_array_slice_part(&command.args[4].parts[0].kind);
    assert_eq!(
        reference.subscript.as_deref().and_then(Subscript::selector),
        Some(SubscriptSelector::At)
    );
}

#[test]
fn test_braced_special_parameters_parse_as_parameter_accesses() {
    let input = "echo ${#} ${$} ${!}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);

    let hash = expect_array_access(&command.args[0]);
    assert_eq!(hash.name.as_str(), "#");
    assert_eq!(hash.name_span.slice(input), "#");

    let pid = expect_array_access(&command.args[1]);
    assert_eq!(pid.name.as_str(), "$");
    assert_eq!(pid.name_span.slice(input), "$");

    let bang = expect_array_access(&command.args[2]);
    assert_eq!(bang.name.as_str(), "!");
    assert_eq!(bang.name_span.slice(input), "!");
}

#[test]
fn test_indirect_expansions_preserve_reference_structure() {
    let input = "echo ${!tools[$target]} ${!var//$'\\n'/' '}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);

    let (tools, operator, operand, colon_variant) =
        expect_indirect_expansion_part(&command.args[0].parts[0].kind);
    assert_eq!(tools.name.as_str(), "tools");
    assert!(!colon_variant);
    assert!(operator.is_none());
    assert!(operand.is_none());
    let subscript = expect_subscript(tools, input, "$target");
    assert_eq!(subscript.syntax_text(input), "$target");

    let (var, operator, operand, colon_variant) =
        expect_indirect_expansion_part(&command.args[1].parts[0].kind);
    assert_eq!(var.name.as_str(), "var");
    assert!(!colon_variant);
    assert!(operand.is_none());
    match operator {
        Some(ParameterOp::ReplaceAll {
            pattern,
            replacement,
            replacement_word_ast,
        }) => {
            assert_eq!(pattern.render(input), "\n");
            assert_eq!(replacement.slice(input), "' '");
            assert_eq!(replacement_word_ast.render_syntax(input), "' '");
        }
        other => panic!("expected replace-all indirect expansion, got {other:?}"),
    }
}

#[test]
fn test_parse_indirect_special_hash_parameter() {
    let input = "echo ${!#}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indirect {
        reference,
        operator,
        operand,
        ..
    }) = &parameter.syntax
    else {
        panic!("expected indirect special-parameter expansion");
    };

    assert_eq!(reference.name.as_str(), "#");
    assert!(operator.is_none());
    assert!(operand.is_none());
}

#[test]
fn test_parse_special_hash_parameter_prefix_removal() {
    let input = "echo ${##*/}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        reference,
        operator,
        ..
    }) = &parameter.syntax
    else {
        panic!("expected special-parameter operation expansion");
    };

    assert_eq!(reference.name.as_str(), "#");
    match operator.as_ref() {
        ParameterOp::RemovePrefixShort { pattern } => assert_eq!(pattern.render(input), "*/"),
        other => panic!("expected short prefix removal, got {other:?}"),
    }
}

#[test]
fn test_parse_length_of_special_parameters_after_hash_prefix() {
    let input = "echo ${#-} ${#?} ${##}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);

    let first = expect_parameter(&command.args[0]);
    assert!(matches!(
        &first.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { reference })
            if reference.name.as_str() == "-"
    ));

    let second = expect_parameter(&command.args[1]);
    assert!(matches!(
        &second.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { reference })
            if reference.name.as_str() == "?"
    ));

    let third = expect_parameter(&command.args[2]);
    assert!(matches!(
        &third.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { reference })
            if reference.name.as_str() == "#"
    ));
}

#[test]
fn test_parse_special_zero_parameter_prefix_removal_inside_multiline_quote() {
    let input = "\
usage=\"
Example:
  ${0##*/} github_repository
Terraform:
  data \\\"external\\\" \\\"github_repos\\\" {
    program = [\\\"/path/to/${0##*/}\\\", \\\"github_repository\\\"]
  }
usage: ${0##*/} <resource_type>
\"
";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let mut names = Vec::new();
    collect_bourne_parameter_names(&word.parts, &mut names);
    let mut patterns = Vec::new();
    collect_bourne_parameter_trim_patterns(&word.parts, input, &mut patterns);

    assert_eq!(names, vec!["0", "0", "0"]);
    assert_eq!(patterns, vec!["*/", "*/", "*/"]);
}

#[test]
fn test_word_part_spans_track_mixed_expansions() {
    let input = "echo pre${name:-fallback}$(printf hi)$((1+2))post\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let slices = top_level_part_slices(word, input);

    assert_eq!(
        slices,
        vec![
            "pre",
            "${name:-fallback}",
            "$(printf hi)",
            "$((1+2))",
            "post"
        ]
    );
}

#[test]
fn test_word_part_spans_track_quoted_expansions() {
    let input = "echo \"x$HOME$(pwd)y\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(
        top_level_part_slices(word, input),
        vec!["\"x$HOME$(pwd)y\""]
    );
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
    assert_eq!(slices, vec!["x", "$HOME", "$(pwd)", "y"]);
}

#[test]
fn test_mixed_segment_word_preserves_expansion_boundaries() {
    let input = "echo foo\"$bar\"baz\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let slices = top_level_part_slices(word, input);

    assert_eq!(slices, vec!["foo", "\"$bar\"", "baz"]);
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[1].kind else {
        panic!("expected quoted middle segment");
    };
    assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
}

#[test]
fn test_escaped_quote_literal_does_not_truncate_following_variable_name() {
    let input = "echo \\\"$archname\\\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(
        top_level_part_slices(word, input),
        vec!["\\\"", "$archname", "\\\""]
    );
    assert!(word_part_tree_contains_variable(&word.parts, "archname"));
    assert!(!word_part_tree_contains_variable(&word.parts, "archnam"));
}

#[test]
fn test_backtick_command_substitution_inside_double_quotes_preserves_syntax_form() {
    let input = "echo \"pre `printf hi` post\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(is_fully_quoted(word));
    assert_eq!(
        top_level_part_slices(word, input),
        vec!["\"pre `printf hi` post\""]
    );

    let WordPart::DoubleQuoted { parts, dollar } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert!(!dollar);

    let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
    assert_eq!(slices, vec!["pre ", "`printf hi`", " post"]);

    let WordPart::CommandSubstitution {
        body: commands,
        syntax,
    } = &parts[1].kind
    else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::Backtick);

    let inner = expect_simple(&commands[0]);
    assert_eq!(inner.name.render(input), "printf");
    assert_eq!(inner.args[0].render(input), "hi");
}

#[test]
fn test_backtick_command_substitution_inside_multiline_double_quotes_preserves_syntax_form() {
    let input = "echo \"\\\n*** ERROR\n`cat lockfile 2>/dev/null`\n\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let WordPart::DoubleQuoted { parts, dollar } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert!(!dollar);

    let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
    assert_eq!(
        slices,
        vec!["\\\n*** ERROR\n", "`cat lockfile 2>/dev/null`", "\n"]
    );

    let WordPart::CommandSubstitution {
        body: commands,
        syntax,
    } = &parts[1].kind
    else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::Backtick);

    let inner = expect_simple(&commands[0]);
    assert_eq!(inner.name.render(input), "cat");
    assert_eq!(inner.args[0].render(input), "lockfile");
    assert_eq!(commands[0].redirects[0].fd, Some(2));
    assert_eq!(
        redirect_word_target(&commands[0].redirects[0]).render(input),
        "/dev/null"
    );
}

#[test]
fn test_dollar_paren_command_substitution_inside_double_quotes_preserves_nested_quoted_argument() {
    let input = "echo \"$(cmd \"$arg\")\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert!(is_fully_quoted(word));
    assert_eq!(
        top_level_part_slices(word, input),
        vec!["\"$(cmd \"$arg\")\""]
    );

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    assert_eq!(
        parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect::<Vec<_>>(),
        vec!["$(cmd \"$arg\")"]
    );

    let WordPart::CommandSubstitution { body, syntax } = &parts[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);

    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "cmd");
    assert_eq!(inner.args[0].render_syntax(input), "\"$arg\"");
}

#[test]
fn test_dollar_paren_command_substitution_inside_double_quotes_with_prefix_keeps_nested_spans_absolute()
 {
    let input = "echo \"pre $(echo hi) post\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    let WordPart::CommandSubstitution { body, syntax } = &parts[1].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);

    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "echo");
    assert_eq!(inner.name.span.slice(input), "echo");
    assert_eq!(inner.args[0].render(input), "hi");
}

#[test]
fn test_dollar_paren_command_substitution_inside_quoted_prefix_with_pipeline_keeps_nested_spans_absolute()
 {
    let input = "echo -n \"\\\"adp_$(echo $var | tr A-Z a-z)\\\": [\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.name.render(input), "echo");
    assert_eq!(command.args[0].render(input), "-n");
    let word = &command.args[1];
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    let WordPart::Literal(text) = &parts[0].kind else {
        panic!("expected literal prefix");
    };
    assert_eq!(text.as_str(input, parts[0].span), "\"adp_");
    let WordPart::CommandSubstitution { body, syntax } = &parts[1].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);

    let AstCommand::Binary(binary) = &body[0].command else {
        panic!("expected pipeline");
    };
    let left = expect_simple(&binary.left);
    assert_eq!(left.name.render(input), "echo");
    assert_eq!(left.args[0].render(input), "$var");

    let right = expect_simple(&binary.right);
    assert_eq!(right.name.render(input), "tr");
    assert_eq!(right.name.span.slice(input), "tr");
    assert_eq!(right.args[0].render(input), "A-Z");
    assert_eq!(right.args[1].render(input), "a-z");
}

#[test]
fn test_dollar_paren_command_substitution_after_multiline_escaped_quote_keeps_nested_spans_absolute()
 {
    let input = "\
echo \"script
  LEFT=\"$left\":\\$base \\
    CHILD=\\$base/$(basename $child) \\
    PATH=$path
    run\" > out
";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let body = first_command_substitution_body(&command.args[0].parts)
        .expect("expected nested command substitution");
    let inner = expect_simple(&body[0]);

    assert_eq!(inner.name.span.slice(input), "basename");
    assert_eq!(inner.args[0].span.slice(input), "$child");
    assert_eq!(inner.args[0].span.start.line, 3);
    assert_eq!(inner.args[0].span.start.column, 29);
}

#[test]
fn test_dollar_paren_command_substitution_inside_double_quotes_handles_nested_arithmetic_with_quoted_right_paren()
 {
    let input = "echo \"$(echo \"$(( $(printf ')') + 1 ))\")\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    let WordPart::CommandSubstitution { body, syntax } = &parts[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);

    let inner = expect_simple(&body[0]);
    assert_eq!(inner.name.render(input), "echo");
    assert_eq!(
        inner.args[0].render_syntax(input),
        "\"$(( $(printf ')') + 1 ))\""
    );
}

#[test]
fn test_process_substitution_like_regex_inside_nested_command_substitution_stays_literal() {
    let input = "value=$(printf '%s\\n' \"<record_id>([^<]*)</record_id>\")\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(value) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::CommandSubstitution { body, .. } = &value.parts[0].kind else {
        panic!("expected command substitution");
    };

    let inner = expect_simple(&body[0]);
    for word in &inner.args {
        assert!(
            !word
                .parts
                .iter()
                .any(|part| matches!(part.kind, WordPart::ProcessSubstitution { .. })),
            "{:#?}",
            word.parts
        );
    }
}

#[test]
fn test_process_substitution_like_regex_inside_nested_pipeline_command_substitution_stays_literal()
{
    let input = "_record_id=$(echo \"$response\" | _egrep_o \"<record_id>([^<]*)</record_id><type>TXT</type><host>$fulldomain</host>\" | _egrep_o \"<record_id>([^<]*)</record_id>\" | sed -r \"s/<record_id>([^<]*)<\\/record_id>/\\1/\" | tail -n 1)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(value) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::CommandSubstitution { body, .. } = &value.parts[0].kind else {
        panic!("expected command substitution");
    };

    fn word_has_process_substitution(word: &Word) -> bool {
        word.parts.iter().any(|part| match &part.kind {
            WordPart::ProcessSubstitution { .. } => true,
            WordPart::DoubleQuoted { parts, .. } => parts
                .iter()
                .any(|part| matches!(part.kind, WordPart::ProcessSubstitution { .. })),
            _ => false,
        })
    }

    fn stmt_has_process_substitution(stmt: &Stmt) -> bool {
        command_has_process_substitution(&stmt.command)
    }

    fn command_has_process_substitution(command: &AstCommand) -> bool {
        match command {
            AstCommand::Simple(command) => command.args.iter().any(word_has_process_substitution),
            AstCommand::Binary(binary)
                if matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll) =>
            {
                stmt_has_process_substitution(&binary.left)
                    || stmt_has_process_substitution(&binary.right)
            }
            _ => false,
        }
    }

    for stmt in body.iter() {
        assert!(!stmt_has_process_substitution(stmt), "{:#?}", stmt);
    }
}

#[test]
fn test_brace_syntax_marks_unquoted_expansion_candidates() {
    let list_input = "{a,b}";
    let list = Parser::parse_word_string(list_input);
    assert_eq!(brace_slices(&list, list_input), vec!["{a,b}"]);
    assert_eq!(
        list.brace_syntax(),
        &[BraceSyntax {
            kind: BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList),
            span: list.span,
            quote_context: BraceQuoteContext::Unquoted,
        }]
    );
    assert!(list.has_active_brace_expansion());

    let sequence_input = "{1..3}";
    let sequence = Parser::parse_word_string(sequence_input);
    assert_eq!(brace_slices(&sequence, sequence_input), vec!["{1..3}"]);
    assert_eq!(
        sequence.brace_syntax()[0].kind,
        BraceSyntaxKind::Expansion(BraceExpansionKind::Sequence)
    );
    assert!(sequence.brace_syntax()[0].expands());
}

#[test]
fn test_brace_syntax_preserves_brace_expansion_suffix_forms() {
    for input in [
        "{a,b}}",
        "{~,~root}/pwd",
        "\"\"{~,~root}/pwd",
        "\\{~,~root}/pwd",
    ] {
        let word = Parser::parse_word_string(input);
        assert_eq!(word.render_syntax(input), input);
        if input == "\\{~,~root}/pwd" {
            assert_eq!(brace_slices(&word, input), Vec::<&str>::new());
        } else {
            let expected = if input == "{a,b}}" {
                "{a,b}"
            } else {
                "{~,~root}"
            };
            assert_eq!(brace_slices(&word, input), vec![expected]);
        }
    }
}

#[test]
fn test_brace_syntax_marks_nested_expansions_separately() {
    let input = "{EGL,GLES{,2,3}}";
    let word = Parser::parse_word_string(input);

    assert_eq!(
        brace_slices(&word, input),
        vec!["{EGL,GLES{,2,3}}", "{,2,3}"]
    );
    assert!(word.brace_syntax().iter().all(|brace| brace.expands()));
}

#[test]
fn test_brace_syntax_does_not_merge_dots_across_skipped_expansion_parts() {
    let input = "{1.$x.3}";
    let word = Parser::parse_word_string(input);

    assert_eq!(brace_slices(&word, input), vec!["{1.$x.3}"]);
    assert!(
        word.brace_syntax()
            .iter()
            .all(|brace| brace.treated_literally())
    );
    assert!(!word.has_active_brace_expansion());
}

#[test]
fn test_parse_brace_expansion_with_single_quoted_backslash_member_keeps_following_args() {
    let input = "echo {'a\\',b} next\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 2);
    assert_eq!(command.args[0].span.slice(input), r#"{'a\',b}"#);
    assert_eq!(command.args[1].span.slice(input), "next");
    assert!(command.args[0].has_active_brace_expansion());
}

#[test]
fn test_word_part_spans_track_parenthesized_arithmetic_expansion() {
    let input = "echo $((a <= (1 || 2)))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.parts.len(), 1);
    assert_eq!(
        word.part_span(0).unwrap().slice(input),
        "$((a <= (1 || 2)))"
    );

    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &word.parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "a <= (1 || 2)");
    let expr = expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::LessThanOrEqual);
    expect_variable(left, "a");
    let ArithmeticExpr::Parenthesized { expression } = &right.kind else {
        panic!("expected parenthesized right operand");
    };
    let ArithmeticExpr::Binary {
        left: inner_left,
        op: inner_op,
        right: inner_right,
    } = &expression.kind
    else {
        panic!("expected logical-or inside parentheses");
    };
    assert_eq!(*inner_op, ArithmeticBinaryOp::LogicalOr);
    expect_number(inner_left, input, "1");
    expect_number(inner_right, input, "2");
}

#[test]
fn test_word_part_spans_track_nested_arithmetic_expansion() {
    let input = "echo $(((a) + ((b))))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.parts.len(), 1);
    assert_eq!(word.part_span(0).unwrap().slice(input), "$(((a) + ((b))))");

    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &word.parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "(a) + ((b))");
    let expr = expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    assert!(matches!(left.kind, ArithmeticExpr::Parenthesized { .. }));
    assert!(matches!(right.kind, ArithmeticExpr::Parenthesized { .. }));
}

#[test]
fn test_arithmetic_expansion_inside_double_quotes_preserves_legacy_and_modern_syntax() {
    let input = "echo \"$((1 + 2))\" \"$[3 + 4]\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.args.len(), 2);

    let modern = &command.args[0];
    assert!(is_fully_quoted(modern));
    let WordPart::DoubleQuoted { parts, dollar } = &modern.parts[0].kind else {
        panic!("expected double-quoted modern arithmetic");
    };
    assert!(!dollar);
    assert_eq!(parts[0].span.slice(input), "$((1 + 2))");
    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "1 + 2");
    let expr = expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_number(left, input, "1");
    expect_number(right, input, "2");

    let legacy = &command.args[1];
    assert!(is_fully_quoted(legacy));
    let WordPart::DoubleQuoted { parts, dollar } = &legacy.parts[0].kind else {
        panic!("expected double-quoted legacy arithmetic");
    };
    assert!(!dollar);
    assert_eq!(parts[0].span.slice(input), "$[3 + 4]");
    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::LegacyBracket);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "3 + 4");
    let expr = expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_number(left, input, "3");
    expect_number(right, input, "4");
}

#[test]
fn test_arithmetic_expansion_after_escaped_quote_inside_double_quotes_stays_live() {
    let input = "echo \"prefix \\\" $((++attempt))\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted word");
    };
    let WordPart::ArithmeticExpansion {
        expression,
        expression_ast,
        syntax,
        ..
    } = &parts[1].kind
    else {
        panic!("expected arithmetic expansion after escaped quote");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
    assert!(expression.is_source_backed());
    assert_eq!(expression.slice(input), "++attempt");
    let ArithmeticExpr::Unary { op, expr } = &expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST")
        .kind
    else {
        panic!("expected prefix update expression");
    };
    assert_eq!(*op, ArithmeticUnaryOp::PreIncrement);
    expect_variable(expr, "attempt");
}

#[test]
fn test_arithmetic_expansion_keeps_parameter_in_numeric_literal_shell_word() {
    let input = "echo $((10#$HOUR)) $((0x$byte6 + summand))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let base_literal = &command.args[0];
    let WordPart::ArithmeticExpansion {
        expression_ast,
        syntax,
        ..
    } = &base_literal.parts[0].kind
    else {
        panic!("expected base-literal arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
    let ArithmeticExpr::ShellWord(word) = &expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST")
        .kind
    else {
        panic!("expected shell-word numeric literal");
    };
    assert!(word_part_tree_contains_variable(&word.parts, "HOUR"));

    let mixed = &command.args[1];
    let WordPart::ArithmeticExpansion { expression_ast, .. } = &mixed.parts[0].kind else {
        panic!("expected mixed arithmetic expansion");
    };
    let ArithmeticExpr::Binary { left, op, right } = &expression_ast
        .as_ref()
        .expect("expected typed arithmetic AST")
        .kind
    else {
        panic!("expected arithmetic addition");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    let ArithmeticExpr::ShellWord(word) = &left.kind else {
        panic!("expected shell-word numeric literal on left");
    };
    assert!(word_part_tree_contains_variable(&word.parts, "byte6"));
    expect_variable(right, "summand");
}

#[test]
fn test_word_part_spans_track_unquoted_legacy_arithmetic_expansion() {
    let input = "i=$[ $i - 1 ]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert!(command.args.is_empty());
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::ArithmeticExpansion {
        syntax, expression, ..
    } = &word.parts[0].kind
    else {
        panic!("expected arithmetic expansion");
    };
    assert_eq!(*syntax, ArithmeticExpansionSyntax::LegacyBracket);
    assert!(expression.is_source_backed());
    assert_eq!(word.parts[0].span.slice(input), "$[ $i - 1 ]");
}

#[test]
fn test_parameter_expansion_operand_stays_source_backed() {
    let input = "echo ${var:-$(pwd)}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, _, operand) = expect_parameter_operation_part(&word.parts[0].kind);
    let operand = operand.expect("expected operand");
    assert!(operand.is_source_backed());
    assert_eq!(operand.slice(input), "$(pwd)");

    let parameter = expect_parameter(word);
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        operand_word_ast: Some(operand_word_ast),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected unified bourne operation");
    };
    assert_eq!(operand_word_ast.render(input), "$(pwd)");
    assert_eq!(operand_word_ast.span.slice(input), "$(pwd)");
}

#[test]
fn test_parameter_default_operand_keeps_nested_command_substitution_spans() {
    let input = "NUMJOBS=${NUMJOBS:-\\\" -j $(expr $(nproc) + 1) \\\"}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let parameter = expect_parameter(word);
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        operand_word_ast: Some(operand_word_ast),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected parameter operation");
    };
    let body = operand_word_ast
        .parts
        .iter()
        .find_map(|part| match &part.kind {
            WordPart::CommandSubstitution { body, .. } => Some(body),
            _ => None,
        })
        .expect("expected outer command substitution");
    let expr = expect_simple(&body[0]);
    assert_eq!(expr.name.render(input), "expr");

    let WordPart::CommandSubstitution { .. } = &expr.args[0].parts[0].kind else {
        panic!("expected nested command substitution");
    };
    assert_eq!(expr.args[0].parts[0].span.slice(input), "$(nproc)");
}

#[test]
fn test_parameter_quoted_default_operand_keeps_nested_command_substitution_spans() {
    let input = "NUMJOBS=${NUMJOBS:-\" -j $(expr $(nproc) + 1) \"}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let parameter = expect_parameter(word);
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        operand_word_ast: Some(operand_word_ast),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected parameter operation");
    };
    let WordPart::DoubleQuoted { parts, .. } = &operand_word_ast.parts[0].kind else {
        panic!("expected quoted default operand");
    };
    let body = parts
        .iter()
        .find_map(|part| match &part.kind {
            WordPart::CommandSubstitution { body, .. } => Some(body),
            _ => None,
        })
        .expect("expected outer command substitution");
    let expr = expect_simple(&body[0]);
    assert_eq!(expr.name.render(input), "expr");

    let WordPart::CommandSubstitution { .. } = &expr.args[0].parts[0].kind else {
        panic!("expected nested command substitution");
    };
    assert_eq!(expr.args[0].parts[0].span.slice(input), "$(nproc)");
}

#[test]
fn test_parameter_default_operand_does_not_absorb_later_double_quoted_expansion() {
    let input = "echo \"${home:-\"${default}\"}'${foo}'\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let [
        WordPartNode {
            kind: WordPart::DoubleQuoted { parts, .. },
            ..
        },
    ] = word.parts.as_slice()
    else {
        panic!("expected one double-quoted argument");
    };

    let [first, second, third, fourth] = parts.as_slice() else {
        panic!("expected parameter, quote, parameter, quote parts: {parts:#?}");
    };

    let WordPart::Parameter(parameter) = &first.kind else {
        panic!("expected leading parameter expansion, got {:?}", first.kind);
    };
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        operand: Some(operand),
        operand_word_ast: Some(operand_word_ast),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected parameter operation with parsed operand");
    };
    assert_eq!(operand.slice(input), "\"${default}\"");
    assert_eq!(operand_word_ast.render(input), "${default}");
    let [
        WordPartNode {
            kind:
                WordPart::DoubleQuoted {
                    parts: operand_parts,
                    ..
                },
            ..
        },
    ] = operand_word_ast.parts.as_slice()
    else {
        panic!("expected quoted operand word");
    };
    let [
        WordPartNode {
            kind: WordPart::Parameter(parameter),
            ..
        },
    ] = operand_parts.as_slice()
    else {
        panic!("expected nested parameter expansion inside operand");
    };
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) =
        &parameter.syntax
    else {
        panic!("expected operand to preserve the nested default expansion");
    };
    assert_eq!(reference.name.as_str(), "default");

    assert_eq!(second.span.slice(input), "'");
    let WordPart::Parameter(parameter) = &third.kind else {
        panic!("expected later parameter expansion, got {:?}", third.kind);
    };
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) =
        &parameter.syntax
    else {
        panic!("expected simple access expansion");
    };
    assert_eq!(reference.name.as_str(), "foo");
    assert_eq!(fourth.span.slice(input), "'");
}

#[test]
fn test_literal_brace_before_quoted_nested_default_does_not_absorb_later_expansion() {
    let input = "echo \"${outer:-{{\"${inner}}\"}}${after}\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let [
        WordPartNode {
            kind: WordPart::DoubleQuoted { parts, .. },
            ..
        },
    ] = word.parts.as_slice()
    else {
        panic!("expected one double-quoted argument");
    };

    let [first, second] = parts.as_slice() else {
        panic!("expected outer and later parameter expansions: {parts:#?}");
    };

    let WordPart::Parameter(parameter) = &first.kind else {
        panic!("expected leading parameter expansion, got {:?}", first.kind);
    };
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
        operand: Some(operand),
        operand_word_ast: Some(operand_word_ast),
        ..
    }) = &parameter.syntax
    else {
        panic!("expected outer parameter operation with parsed operand");
    };
    assert_eq!(operand.slice(input), "{{\"${inner}}\"}");

    let [literal_prefix, quoted_suffix, literal_suffix] = operand_word_ast.parts.as_slice() else {
        panic!("expected literal, quoted, literal operand structure: {operand_word_ast:#?}");
    };
    assert_eq!(literal_prefix.span.slice(input), "{{");
    let WordPart::DoubleQuoted {
        parts: quoted_parts,
        ..
    } = &quoted_suffix.kind
    else {
        panic!(
            "expected quoted middle segment in outer operand, got {:?}",
            quoted_suffix.kind
        );
    };
    let [nested, quoted_literal_suffix] = quoted_parts.as_slice() else {
        panic!("expected nested parameter and literal brace in quoted suffix");
    };
    let WordPart::Parameter(parameter) = &nested.kind else {
        panic!("expected nested parameter expansion, got {:?}", nested.kind);
    };
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) =
        &parameter.syntax
    else {
        panic!("expected inner access expansion");
    };
    assert_eq!(reference.name.as_str(), "inner");
    assert_eq!(quoted_literal_suffix.span.slice(input), "}");
    assert_eq!(literal_suffix.span.slice(input), "}");

    let WordPart::Parameter(parameter) = &second.kind else {
        panic!("expected later parameter expansion, got {:?}", second.kind);
    };
    let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) =
        &parameter.syntax
    else {
        panic!("expected later access expansion");
    };
    assert_eq!(reference.name.as_str(), "after");
}

#[test]
fn test_case_modification_operands_consume_nested_parameter_expansions() {
    let input = "echo ${name^^${pat}} ${arr[1],,${pat}}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let (_, upper_operator, upper_operand) =
        expect_parameter_operation_part(&command.args[0].parts[0].kind);
    assert!(matches!(upper_operator, ParameterOp::UpperAll));
    assert_eq!(
        upper_operand
            .expect("expected upper case-modification operand")
            .slice(input),
        "${pat}"
    );

    let (lower_reference, lower_operator, lower_operand) =
        expect_parameter_operation_part(&command.args[1].parts[0].kind);
    expect_subscript(lower_reference, input, "1");
    assert!(matches!(lower_operator, ParameterOp::LowerAll));
    assert_eq!(
        lower_operand
            .expect("expected lower case-modification operand")
            .slice(input),
        "${pat}"
    );
}

#[test]
fn test_parameter_expansion_trim_operand_accepts_literal_left_brace_after_multiline_quote() {
    let input = "dns_servercow_info='ServerCow.de\nSite: ServerCow.de\n'\n\nf(){\n  if true; then\n    txtvalue_old=${response#*{\\\"name\\\":\\\"\"$_sub_domain\"\\\",\\\"ttl\\\":20,\\\"type\\\":\\\"TXT\\\",\\\"content\\\":\\\"}\n  fi\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[1].command else {
        panic!("expected function definition");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let (if_compound, redirects) = expect_compound(&body[0]);
    let AstCompoundCommand::If(if_command) = if_compound else {
        panic!("expected if command");
    };
    assert!(redirects.is_empty());
    let command = expect_simple(&if_command.then_branch[0]);
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::RemovePrefixShort { pattern } = operator else {
        panic!("expected short-prefix trim operator");
    };
    assert!(pattern.render(input).contains("$_sub_domain"));
    assert!(pattern.parts.iter().any(|part| {
        matches!(
            &part.kind,
            PatternPart::Word(word)
                if word_part_tree_contains_variable(&word.parts, "_sub_domain")
        )
    }));
}

#[test]
fn test_parameter_expansion_trim_operand_accepts_balanced_literal_braces() {
    let input = "echo ${var#foo{bar}}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::RemovePrefixShort { pattern } = operator else {
        panic!("expected short-prefix trim operator");
    };
    assert_eq!(pattern.render(input), "foo{bar}");
}

#[test]
fn test_parameter_expansion_trim_operand_tracks_nested_parameter_expansions() {
    let input = "echo ${var#${prefix:-fallback}}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::RemovePrefixShort { pattern } = operator else {
        panic!("expected short-prefix trim operator");
    };
    assert_eq!(pattern.render(input), "${prefix:-fallback}");
    assert!(matches!(
        &pattern.parts[..],
        [PatternPartNode {
            kind: PatternPart::Word(word),
            ..
        }] if matches!(
            &word.parts[..],
            [WordPartNode {
                kind: WordPart::Parameter(_) | WordPart::ParameterExpansion { .. },
                ..
            }]
        )
    ));
}

#[test]
fn test_parameter_replacement_pattern_stays_source_backed() {
    let input = "echo ${var/foo/bar}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceFirst {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-first operator");
    };

    assert_eq!(pattern.render(input), "foo");
    assert_eq!(pattern.parts.len(), 1);
    assert!(matches!(
        &pattern.parts[0].kind,
        PatternPart::Literal(text) if text.is_source_backed()
    ));
    assert!(replacement.is_source_backed());
    assert_eq!(replacement.slice(input), "bar");
    assert_eq!(replacement_word_ast.render(input), "bar");
    assert_eq!(replacement_word_ast.span.slice(input), "bar");
}

#[test]
fn test_parameter_trim_pattern_preserves_quoted_fragments_around_expansions() {
    let input = "echo ${var#\"pre\"$suffix'-'*}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::RemovePrefixShort { pattern } = operator else {
        panic!("expected short-prefix trim operator");
    };

    assert!(matches!(
        &pattern.parts[..],
        [
            PatternPartNode {
                kind: PatternPart::Word(first),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(second),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(third),
                ..
            },
            PatternPartNode {
                kind: PatternPart::AnyString,
                ..
            }
        ] if first.is_fully_quoted()
            && matches!(
                &second.parts[..],
                [WordPartNode {
                    kind: WordPart::Variable(name),
                    ..
                }] if name.as_str() == "suffix"
            )
            && third.is_fully_quoted()
    ));
}

#[test]
fn test_parameter_replacement_pattern_preserves_mixed_quote_fragments() {
    let input = "echo ${var//\"pre\"$suffix'-'/x}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceAll {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-all operator");
    };

    assert_eq!(
        pattern_part_slices(pattern, input),
        vec!["\"pre\"", "$suffix", "'-'"]
    );
    assert_eq!(replacement.slice(input), "x");
    assert_eq!(replacement_word_ast.render(input), "x");
    assert!(matches!(
        &pattern.parts[..],
        [
            PatternPartNode {
                kind: PatternPart::Word(first),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(second),
                ..
            },
            PatternPartNode {
                kind: PatternPart::Word(third),
                ..
            }
        ] if first.is_fully_quoted()
            && matches!(
                &second.parts[..],
                [WordPartNode {
                    kind: WordPart::Variable(name),
                    ..
                }] if name.as_str() == "suffix"
            )
            && third.is_fully_quoted()
    ));
}

#[test]
fn test_parameter_replacement_pattern_cooks_escaped_slash() {
    let input = r#"echo ${var/foo\/bar/baz}"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceFirst {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-first operator");
    };

    assert_eq!(pattern.render(input), "foo/bar");
    assert_eq!(pattern.parts.len(), 1);
    assert!(matches!(
        &pattern.parts[0].kind,
        PatternPart::Literal(text) if !text.is_source_backed() && text == "foo/bar"
    ));
    assert!(replacement.is_source_backed());
    assert_eq!(replacement_word_ast.render(input), "baz");
    assert_eq!(replacement.slice(input), "baz");
}

#[test]
fn test_parameter_replacement_pattern_keeps_escaped_dollar_literal() {
    let input = r#"echo "${d//\$ORIGIN/$origin}""#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted argument");
    };
    let (_, operator, _) = expect_parameter_operation_part(&parts[0].kind);
    let ParameterOp::ReplaceAll { pattern, .. } = operator else {
        panic!("expected replace-all operator");
    };

    assert_eq!(pattern.render(input), "$ORIGIN");
    assert!(matches!(
        &pattern.parts[..],
        [PatternPartNode {
            kind: PatternPart::Literal(text),
            ..
        }] if text == "$ORIGIN"
    ));
}

#[test]
fn test_parameter_replacement_word_keeps_escaped_single_quotes_literal() {
    let input = r#"echo ${dest_dir//\'/\'\\\'\'}"#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceAll {
        replacement: _,
        replacement_word_ast,
        ..
    } = operator
    else {
        panic!("expected replace-all operator");
    };

    assert!(!replacement_word_ast.parts.iter().any(|part| {
        matches!(part.kind, WordPart::SingleQuoted { .. })
            && part.span.slice(input).ends_with("\\'")
    }));
}

#[test]
fn test_parameter_replacement_spans_cover_complex_pattern_and_replacement_bodies() {
    let input = "\
echo ${dest_dir//\\'/\\'\\\\\\'\\'} ${TERMUX_PKG_VERSION_EDITED//${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}/${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}} ${GITHUB_GRAPHQL_QUERIES[$BATCH * $BATCH_SIZE]//\\\\/} ${run_depends/${i}/${dep}}\n\
";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(
        command.args[0]
            .parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect::<Vec<_>>(),
        vec![r#"${dest_dir//\'/\'\\\'\'}"#]
    );

    let spans = command
        .args
        .iter()
        .map(|word| word.parts[0].span.slice(input))
        .collect::<Vec<_>>();
    assert_eq!(
        spans,
        vec![
            "${dest_dir//\\'/\\'\\\\\\'\\'}",
            "${TERMUX_PKG_VERSION_EDITED//${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}/${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}}",
            "${GITHUB_GRAPHQL_QUERIES[$BATCH * $BATCH_SIZE]//\\\\/}",
            "${run_depends/${i}/${dep}}",
        ]
    );

    let (_, operator, _) = expect_parameter_operation_part(&command.args[1].parts[0].kind);
    let ParameterOp::ReplaceAll {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-all operator");
    };
    assert_eq!(
        pattern.render(input),
        "${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}"
    );
    assert_eq!(
        replacement.slice(input),
        "${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}"
    );
    assert_eq!(
        replacement_word_ast.render(input),
        "${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}"
    );

    let (_, operator, _) = expect_parameter_operation_part(&command.args[3].parts[0].kind);
    let ParameterOp::ReplaceFirst {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-first operator");
    };
    assert_eq!(pattern.render(input), "${i}");
    assert_eq!(replacement.slice(input), "${dep}");
    assert_eq!(replacement_word_ast.render(input), "${dep}");
}

#[test]
fn test_read_replacement_pattern_stops_before_unescaped_delimiter() {
    let input = "crypt=${crypt//\\\\/\\\\\\\\}\n";
    let parser = Parser::new(input);
    let offset = input.find("//").expect("expected replacement operator") + 2;
    let mut chars = input[offset..].chars().peekable();
    let mut cursor = Position::new().advanced_by(&input[..offset]);

    let pattern = parser.read_replacement_pattern(&mut chars, &mut cursor, true);
    assert_eq!(pattern.slice(input), "\\\\");
    assert_eq!(cursor.offset, offset + 2);
}

#[test]
fn test_decode_cooked_word_keeps_variable_after_literal_backslash() {
    let cooked = r#"\$HOME"#;
    let span = Span::from_positions(Position::new(), Position::new().advanced_by(cooked));
    let word = Parser::new("").decode_word_text(cooked, span, span.start, false);

    assert_eq!(word.parts.len(), 2);
    let WordPart::Literal(text) = &word.parts[0].kind else {
        panic!("expected literal backslash prefix");
    };
    assert_eq!(text.as_str("", word.parts[0].span), "\\");
    assert!(matches!(
        &word.parts[1].kind,
        WordPart::Variable(name) if name.as_str() == "HOME"
    ));
}

#[test]
fn test_parse_arithmetic_command_with_command_substitution() {
    let input = "(($(date -u) > DATE))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(command.expr_span.unwrap().slice(input), "$(date -u) > DATE");
    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::GreaterThan);
    expect_shell_word(left, input, "$(date -u)");
    expect_variable(right, "DATE");
}

#[test]
fn test_parse_arithmetic_command_accepts_command_substitutions_and_quoted_words() {
    let input = "(( \"$(date -u)\" + '3' ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    let ArithmeticExpr::ShellWord(left_word) = &left.kind else {
        panic!("expected quoted shell word on left");
    };
    assert_eq!(left_word.span.slice(input), "\"$(date -u)\"");
    let ArithmeticExpr::ShellWord(right_word) = &right.kind else {
        panic!("expected quoted shell word on right");
    };
    assert_eq!(right_word.span.slice(input), "'3'");
}

#[test]
fn test_parse_parameter_expansion_operands_allow_quoted_and_escaped_right_brace() {
    let input = r###"echo "${var#\}}"
echo "${var#'}'}"
echo "${var#"}"}"
echo "${var-\}}"
echo "${var-'}'}"
echo "${var-"}"}"
"###;

    let script = Parser::new(input).parse().unwrap().file;
    assert_eq!(script.body.len(), 6);
}

#[test]
fn test_parse_parameter_slices_preserve_shell_style_offsets() {
    let input = "echo \"${arg:$index:1}\" \"${@:1:$package_type_nargs}\" \"${@:$(( $package_type_nargs + 1 ))}\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let WordPart::DoubleQuoted {
        parts: first_parts, ..
    } = &command.args[0].parts[0].kind
    else {
        panic!("expected first double-quoted word");
    };
    let (_, first_offset_ast, first_length_ast) = expect_substring_part(&first_parts[0].kind);
    let ArithmeticExpr::ShellWord(first_offset_word) =
        &first_offset_ast.as_ref().expect("expected offset AST").kind
    else {
        panic!("expected shell-word offset");
    };
    assert_eq!(first_offset_word.span.slice(input), "$index");
    assert_eq!(
        first_length_ast
            .as_ref()
            .expect("expected first length AST")
            .span
            .slice(input),
        "1"
    );

    let WordPart::DoubleQuoted {
        parts: second_parts,
        ..
    } = &command.args[1].parts[0].kind
    else {
        panic!("expected second double-quoted word");
    };
    let (_, second_offset_ast, second_length_ast) = expect_substring_part(&second_parts[0].kind);
    assert_eq!(
        second_offset_ast
            .as_ref()
            .expect("expected second offset AST")
            .span
            .slice(input),
        "1"
    );
    let ArithmeticExpr::ShellWord(second_length_word) = &second_length_ast
        .as_ref()
        .expect("expected second length AST")
        .kind
    else {
        panic!("expected shell-word length");
    };
    assert_eq!(second_length_word.span.slice(input), "$package_type_nargs");

    let WordPart::DoubleQuoted {
        parts: third_parts, ..
    } = &command.args[2].parts[0].kind
    else {
        panic!("expected third double-quoted word");
    };
    let (_, third_offset_ast, third_length_ast) = expect_substring_part(&third_parts[0].kind);
    assert!(third_length_ast.is_none());
    let ArithmeticExpr::ShellWord(third_offset_word) = &third_offset_ast
        .as_ref()
        .expect("expected third offset AST")
        .kind
    else {
        panic!("expected shell-word arithmetic expansion");
    };
    assert_eq!(
        third_offset_word.span.slice(input),
        "$(( $package_type_nargs + 1 ))"
    );
}

#[test]
fn test_command_substitution_spans_are_absolute() {
    let script = Parser::new("out=$(\n  printf '%s\\n' $x\n)\n")
        .parse()
        .unwrap()
        .file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::CommandSubstitution {
        body: commands,
        syntax,
    } = &word.parts[0].kind
    else {
        panic!("expected command substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);
    let inner = expect_simple(&commands[0]);

    assert_eq!(inner.name.span.start.line, 2);
    assert_eq!(inner.name.span.start.column, 3);
    assert_eq!(inner.args[0].span.start.line, 2);
    assert_eq!(inner.args[1].span.start.column, 17);
}

#[test]
fn test_prefixed_nested_command_substitution_keeps_command_names() {
    let input = "cp -v $filename $OUT/$(echo $(basename $filename .fuzz))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let echo_body = command.args[2]
        .parts
        .iter()
        .find_map(|part| match &part.kind {
            WordPart::CommandSubstitution { body, .. } => Some(body),
            _ => None,
        })
        .expect("expected outer command substitution");
    let echo = expect_simple(&echo_body[0]);
    assert_eq!(echo.name.render(input), "echo");

    let basename_body = echo.args[0]
        .parts
        .iter()
        .find_map(|part| match &part.kind {
            WordPart::CommandSubstitution { body, .. } => Some(body),
            _ => None,
        })
        .expect("expected nested basename command substitution");
    let basename = expect_simple(&basename_body[0]);
    assert_eq!(basename.name.render(input), "basename");
}

#[test]
fn test_prefixed_nested_quoted_command_substitution_keeps_command_names() {
    let input = "\
value=\"$(\n\
       [[ \"$config_file\" == *\"$theme.cfg\" ]] && echo \"$(basename \"$config_file\")\"\n\
    )\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected quoted assignment value");
    };
    let WordPart::CommandSubstitution { body, .. } = &parts[0].kind else {
        panic!("expected outer command substitution");
    };
    let AstCommand::Binary(binary) = &body[0].command else {
        panic!("expected binary command");
    };

    let echo = expect_simple(&binary.right);
    assert_eq!(echo.name.render(input), "echo");

    let WordPart::DoubleQuoted {
        parts: echo_parts, ..
    } = &echo.args[0].parts[0].kind
    else {
        panic!("expected quoted echo argument");
    };
    let WordPart::CommandSubstitution {
        body: basename_body,
        ..
    } = &echo_parts[0].kind
    else {
        panic!("expected nested basename substitution");
    };
    let basename = expect_simple(&basename_body[0]);
    assert_eq!(basename.name.render(input), "basename");
}

#[test]
fn test_prefixed_nested_escaped_quoted_command_substitution_keeps_command_names() {
    let input = "#!/bin/sh\necho -n \"\\\"adp_$(echo $var | tr A-Z a-z)\\\": [\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let WordPart::DoubleQuoted { parts, .. } = &command.args[1].parts[0].kind else {
        panic!("expected quoted echo argument");
    };
    let body = parts
        .iter()
        .find_map(|part| match &part.kind {
            WordPart::CommandSubstitution { body, .. } => Some(body),
            _ => None,
        })
        .expect("expected nested command substitution");

    let AstCommand::Binary(pipeline) = &body[0].command else {
        panic!("expected piped command");
    };
    let echo = expect_simple(&pipeline.left);
    assert_eq!(echo.name.render(input), "echo");

    let tr_command = expect_simple(&pipeline.right);
    assert_eq!(tr_command.name.render(input), "tr");
    assert_eq!(tr_command.args[0].render(input), "A-Z");
    assert_eq!(tr_command.args[1].render(input), "a-z");
}

#[test]
fn test_prefixed_nested_escaped_quoted_command_substitution_keeps_command_names_after_apostrophe() {
    let input = "#!/bin/sh\necho -n \"it's \\\"adp_$(echo $var | tr A-Z a-z)\\\": [\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let WordPart::DoubleQuoted { parts, .. } = &command.args[1].parts[0].kind else {
        panic!("expected quoted echo argument");
    };
    let body = parts
        .iter()
        .find_map(|part| match &part.kind {
            WordPart::CommandSubstitution { body, .. } => Some(body),
            _ => None,
        })
        .expect("expected nested command substitution");

    let AstCommand::Binary(pipeline) = &body[0].command else {
        panic!("expected piped command");
    };
    let echo = expect_simple(&pipeline.left);
    assert_eq!(echo.name.render(input), "echo");

    let tr_command = expect_simple(&pipeline.right);
    assert_eq!(tr_command.name.render(input), "tr");
    assert_eq!(tr_command.args[0].render(input), "A-Z");
    assert_eq!(tr_command.args[1].render(input), "a-z");
}

#[test]
fn test_parse_command_substitution_with_open_paren_inside_double_quotes() {
    Parser::new("x=$(echo \"(\")\n").parse().unwrap();
}

#[test]
fn test_parse_command_substitution_with_case_pattern_right_paren() {
    let input = "echo $(foo=a; case $foo in [0-9]) echo number;; [a-z]) echo letter ;; esac)\n";
    Parser::new(input).parse().unwrap();
}

#[test]
fn test_alias_expansion_can_form_a_for_loop_header() {
    let input = "\
shopt -s expand_aliases
alias FOR1='for '
alias FOR2='FOR1 '
alias eye1='i '
alias eye2='eye1 '
alias IN='in '
alias onetwo='1 2 '
FOR2 eye2 IN onetwo 3; do echo $i; done
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(command) = script.body.last() else {
        panic!("expected final command to be a for loop");
    };
    let (compound, _) = expect_compound(command);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected final command to be a for loop");
    };
    assert_eq!(command.targets[0].name.as_deref(), Some("i"));
    assert_eq!(command.words.as_ref().map(|words| words.len()), Some(3));
}

#[test]
fn test_alias_expansion_can_open_a_brace_group() {
    let input = "\
shopt -s expand_aliases
alias LEFT='{'
LEFT echo one; echo two; }
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(command) = script.body.last() else {
        panic!("expected final command to be a brace group");
    };
    let (compound, _) = expect_compound(command);
    let AstCompoundCommand::BraceGroup(commands) = compound else {
        panic!("expected final command to be a brace group");
    };
    assert_eq!(commands.len(), 2);
    assert!(matches!(commands[0].command, AstCommand::Simple(_)));
    assert!(matches!(commands[1].command, AstCommand::Simple(_)));
}

#[test]
fn test_alias_expansion_can_open_a_subshell() {
    let input = "\
shopt -s expand_aliases
alias LEFT='('
LEFT echo one; echo two )
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(command) = script.body.last() else {
        panic!("expected final command to be a subshell");
    };
    let (compound, _) = expect_compound(command);
    let AstCompoundCommand::Subshell(commands) = compound else {
        panic!("expected final command to be a subshell");
    };
    assert_eq!(commands.len(), 2);
    assert!(matches!(commands[0].command, AstCommand::Simple(_)));
    assert!(matches!(commands[1].command, AstCommand::Simple(_)));
}

#[test]
fn test_alias_expansion_with_trailing_space_expands_next_word() {
    let input = "\
shopt -s expand_aliases
alias greet='echo '
alias subject='hello'
greet subject
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(stmt) = script.body.last() else {
        panic!("expected final command to be a simple command");
    };
    let command = expect_simple(stmt);

    assert_eq!(command.name.render(input), "echo");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(input), "hello");

    let WordPart::Literal(name_text) = &command.name.parts[0].kind else {
        panic!("expected alias-expanded command name to stay literal");
    };
    let WordPart::Literal(arg_text) = &command.args[0].parts[0].kind else {
        panic!("expected alias-expanded arg to stay literal");
    };

    assert!(!name_text.is_source_backed());
    assert!(!arg_text.is_source_backed());
}

#[test]
fn test_alias_expansion_with_trailing_space_waits_until_replay_finishes() {
    let input = "\
shopt -s expand_aliases
alias e_='for i in 1 2 3; do echo '
e_ $i; done
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(stmt) = script.body.last() else {
        panic!("expected final command to be a for loop");
    };
    let (compound, _) = expect_compound(stmt);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected final command to be a for loop");
    };

    assert_eq!(command.targets[0].name.as_deref(), Some("i"));
    assert_eq!(command.words.as_ref().map(|words| words.len()), Some(3));

    let Some(body_stmt) = command.body.first() else {
        panic!("expected loop body command");
    };
    let body_command = expect_simple(body_stmt);
    assert_eq!(body_command.name.render(input), "echo");
    assert_eq!(body_command.args.len(), 1);
    assert_eq!(body_command.args[0].render(input), "$i");
}

#[test]
fn test_alias_expansion_recursive_guard_stops_self_reference() {
    let input = "\
shopt -s expand_aliases
alias loop='loop echo'
loop
";
    let script = Parser::new(input).parse().unwrap().file;

    let Some(stmt) = script.body.last() else {
        panic!("expected final command to be a simple command");
    };
    let command = expect_simple(stmt);

    assert_eq!(command.name.render(input), "loop");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(input), "echo");
}

#[test]
fn test_parse_malformed_parameter_replacement_fuzz_regression_does_not_panic() {
    const INPUT: &[u8] = &[
        0x23, 0x21, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x62, 0x61, 0x73, 0x68, 0x0a, 0x0a, 0x23, 0x20,
        0x49, 0x6e, 0x76, 0x64, 0x66, 0x22, 0x20, 0x22, 0x64, 0x6f, 0x63, 0x22, 0x29, 0x0a, 0x65,
        0x78, 0x74, 0x73, 0x3d, 0x22, 0x24, 0x7b, 0x65, 0x78, 0x74, 0x73, 0x5b, 0x2a, 0x5d, 0x7d,
        0x22, 0x0a, 0x65, 0x78, 0x74, 0x73, 0x2b, 0x3d, 0x22, 0x20, 0x24, 0x7b, 0x65, 0x78, 0x74,
        0x73, 0x5e, 0x5e, 0x7d, 0x22, 0x0a, 0x65, 0x63, 0xdd, 0x8d, 0x65, 0x75, 0x20, 0x65, 0x61,
        0x73, 0x64, 0x20, 0x73, 0x73, 0x63, 0x61, 0x6c, 0x61, 0x72, 0x20, 0x74, 0x65, 0x78, 0x74,
        0x40, 0x2e, 0x0a, 0x65, 0x78, 0x74, 0x73, 0x3d, 0x28, 0x5d, 0x74, 0x78, 0x74, 0x22, 0x20,
        0x22, 0x70, 0x64, 0x66, 0x22, 0x20, 0x22, 0x64, 0x6f, 0x63, 0x22, 0x29, 0x0a, 0x65, 0x78,
        0x74, 0x73, 0x3d, 0x22, 0x24, 0x7b, 0x65, 0x78, 0x74, 0x69, 0x6e, 0x2f, 0x62, 0x61, 0x73,
        0x68, 0x0a, 0x0a, 0x23, 0x20, 0x49, 0x6e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x76, 0x64, 0x66, 0x22, 0x20,
        0x22, 0x64, 0x6f, 0x63, 0x22, 0x29, 0x0a, 0x65, 0x78, 0x74, 0x73, 0x3d, 0x22, 0x24, 0x7b,
        0x65, 0x78, 0x74, 0x73, 0x5b, 0x2a, 0x5d, 0x7d, 0x22, 0x0a, 0x65, 0x78, 0x74, 0x73, 0x2b,
        0x3d, 0x22, 0x20, 0x24, 0x7b, 0x65, 0x78, 0x74, 0x73, 0x5e, 0x2f, 0x00, 0x00, 0x00, 0x65,
        0x63, 0xdd, 0x8d, 0x65, 0x75, 0x20, 0x65, 0x61, 0x73, 0x64, 0x20, 0x73, 0x73, 0x63, 0x61,
        0x6c, 0x61, 0x72, 0x20, 0x74, 0x65, 0x78, 0x74, 0x2e, 0x0a, 0x65, 0x78, 0x74, 0x73, 0x3d,
        0x28, 0x5d, 0x74, 0x78, 0x74, 0x22, 0x20, 0x22, 0x70, 0x64, 0x66, 0x22, 0x20, 0x22, 0x64,
        0x6f, 0x63, 0x22, 0x29, 0x0a, 0x65, 0x78, 0x74, 0x73, 0x3d, 0x22, 0x24, 0x7b, 0x65, 0x78,
        0x74, 0x2f, 0x5b, 0x2a, 0x5d, 0x7d, 0x22, 0x0a, 0x65, 0x78, 0x74, 0x73, 0x2b, 0x3d, 0x31,
        0x20, 0x24, 0x7b, 0x65, 0x78, 0x74, 0x73, 0x5e, 0x5e, 0x7d, 0x22, 0x0a, 0x65, 0x63, 0x68,
        0x6f, 0x20, 0x22, 0x24, 0x65, 0x78, 0x74, 0x73, 0x22, 0x2a, 0x0a, 0x23, 0x20, 0x56, 0x61,
        0x6c, 0x2b, 0x2b, 0x3a, 0x20, 0x66, 0x6c, 0x61, 0x74, 0x74, 0x65, 0x6e, 0x69, 0x6e, 0x5f,
        0x00, 0x69, 0x6e, 0x74, 0x6f, 0x2f, 0x5b, 0x2a, 0x5d, 0x7d, 0x22, 0x0a, 0x65, 0x78, 0x74,
        0x73, 0x2b, 0x3d, 0x32, 0x20, 0x24, 0x7b, 0x65, 0x78, 0x74, 0x73, 0x5e, 0x5e, 0x7d, 0x22,
        0x0a, 0x65, 0x63, 0x68, 0x6f, 0x20, 0x22, 0x24, 0x65, 0x78, 0x74, 0x73, 0x22, 0x2a, 0x0a,
        0x23, 0x20, 0x56, 0x61, 0x6c, 0x2b, 0x2b, 0x3a, 0x20, 0x66, 0x6c, 0x61, 0x74, 0x74, 0x65,
        0x6e, 0x69, 0x6e, 0x5f, 0x00, 0x69, 0x6e, 0x74, 0x6f, 0x20, 0x61, 0x20, 0x64, 0x4e, 0x66,
        0x66, 0x65, 0x72, 0x65, 0x6e, 0x74, 0x20, 0x73,
    ];

    let input = std::str::from_utf8(INPUT).unwrap();

    let case_script = format!(
        "case \"test.txt\" in {}) echo match;; *) echo no;; esac\n",
        input
    );
    let _ = Parser::new(&case_script).parse();

    let conditional_script = format!("if [[ \"hello.world\" == {} ]]; then echo y; fi\n", input);
    let _ = Parser::new(&conditional_script).parse();
}

#[test]
fn test_parse_arithmetic_utf8_parameter_replacement_fuzz_regression_does_not_panic() {
    const INPUT: &str = "#!/ di[@]}\")\")#!/bin/$x)\narr=($(]x ib) echo#b;;\n#!/bi/nbash\n\narr=${in/bas\nunc(#!${in/sa\nar\0\0\0)\0\0<\0<\0=\u{2018}woin/sa\u{2019}es\0 \"two worac\ntr";

    let script = format!("echo $(({}))\n", INPUT);
    let _ = Parser::new(&script).parse();
}

#[test]
fn test_parse_glob_utf8_parameter_replacement_fuzz_regression_does_not_panic() {
    const INPUT: &str = "'a\0b'\nar=(\n\n{)${!f/e¢}\")\ni";

    let case_script = format!(
        "case \"test.txt\" in {}) echo match;; *) echo no;; esac\n",
        INPUT
    );
    let _ = Parser::new(&case_script).parse();

    let conditional_script = format!("if [[ \"hello.world\" == {} ]]; then echo y; fi\n", INPUT);
    let _ = Parser::new(&conditional_script).parse();
}
