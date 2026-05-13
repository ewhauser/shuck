use super::*;

#[test]
fn test_zsh_array_comma_detection_ignores_subscript_ranges_and_glob_qualifiers() {
    let input = r#"arr=(
  $spinner[2,-1]
  $_p9k__display_v[k,k+1]
  ${assoc[$key,$fallback]}
  ${arr[1,${last:-2}]}
  ${(@)${:-{$#parts..1}}/(#m)*/$parent${(pj./.)parts[1,MATCH]}}
  **/*(.om[1,3])
  *.log(#q.om[1,3])
  ${expanded_path}*(N-*,N-/)
  "${dir}"/*(.om[1,3])
  plain(group,with,commas)
  a,b
  ${expanded_path}(N-*,N-/)
)
"#;
    let output = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound array assignment");
    };

    let expected = [
        ("$spinner[2,-1]", false),
        ("$_p9k__display_v[k,k+1]", false),
        ("${assoc[$key,$fallback]}", false),
        ("${arr[1,${last:-2}]}", false),
        (
            "${(@)${:-{$#parts..1}}/(#m)*/$parent${(pj./.)parts[1,MATCH]}}",
            false,
        ),
        ("**/*(.om[1,3])", false),
        ("*.log(#q.om[1,3])", false),
        ("${expanded_path}*(N-*,N-/)", false),
        ("\"${dir}\"/*(.om[1,3])", false),
        ("plain(group,with,commas)", true),
        ("a,b", true),
        ("${expanded_path}(N-*,N-/)", true),
    ];

    assert_eq!(
        array.elements.len(),
        expected.len(),
        "{:#?}",
        array.elements
    );
    for (index, (expected_span, expected_comma)) in expected.into_iter().enumerate() {
        let ArrayElem::Sequential(value) = &array.elements[index] else {
            panic!("expected sequential element at index {index}");
        };
        assert_eq!(value.span.slice(input), expected_span);
        assert_eq!(
            value.has_top_level_unquoted_comma(),
            expected_comma,
            "unexpected comma flag for {}: {:#?}",
            value.span.slice(input),
            value.word
        );
    }
}

#[test]
fn test_zsh_for_loop_preserves_paren_glob_qualifier_word_list() {
    let source =
        "for ind_file ( ${^${(von)PUAssocArray}}.ind(DN.) ) {\n  command cat ${ind_file:r}\n}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let (compound, redirects) = expect_compound(&output.file.body[0]);
    let AstCompoundCommand::For(command) = compound else {
        panic!("expected for loop");
    };

    assert!(redirects.is_empty());
    assert_eq!(
        command
            .words
            .as_ref()
            .expect("expected parenthesized word list")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["${^${(von)PUAssocArray}}.ind(DN.)"]
    );
    assert!(matches!(command.syntax, ForSyntax::ParenBrace { .. }));
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_star_dot() {
    let source = "print *(.)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "*(.)");
    assert_eq!(command.args[0].span.slice(source), "*(.)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(.)");
    assert!(matches!(
        qualifiers.fragments.as_slice(),
        [ZshGlobQualifier::Flag { name: '.', span }] if span.slice(source) == "."
    ));
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_star_slash() {
    let source = "print *(/)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "*(/)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(/)");
    assert!(matches!(
        qualifiers.fragments.as_slice(),
        [ZshGlobQualifier::Flag { name: '/', span }] if span.slice(source) == "/"
    ));
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_star_n() {
    let source = "print *(N)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "*(N)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(N)");
    assert!(matches!(
        qualifiers.fragments.as_slice(),
        [ZshGlobQualifier::Flag { name: 'N', span }] if span.slice(source) == "N"
    ));
}

#[test]
fn test_zsh_trailing_glob_qualifier_after_quoted_prefix_stays_single_argument() {
    let source = "print \"$plugin_dir\"/*(:t)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].span.slice(source), "\"$plugin_dir\"/*(:t)");
}

#[test]
fn test_zsh_trailing_glob_qualifier_inside_compound_array_stays_single_element() {
    let source = "plugins=( \"$plugin_dir\"/*(:t) )\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.assignments.len(), 1);
    let assignment = &command.assignments[0];
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(first.span.slice(source), "\"$plugin_dir\"/*(:t)");
}

#[test]
fn test_zsh_quoted_variable_qualifier_inside_compound_array_stays_single_element() {
    let source = "__GREP_ALIAS_CACHES=( \"$__GREP_CACHE_FILE\"(Nm-1) )\nif true; then :; fi\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.assignments.len(), 1);
    let assignment = &command.assignments[0];
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(first.span.slice(source), "\"$__GREP_CACHE_FILE\"(Nm-1)");
    assert!(matches!(
        output.file.body[1].command,
        AstCommand::Compound(_)
    ));
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_recursive_pattern_with_letter_sequence_and_range() {
    let source = "print **/*(.om[1,3])\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "**/*(.om[1,3])");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "**/*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(.om[1,3])");

    let [
        ZshGlobQualifier::Flag {
            name: '.',
            span: dot_span,
        },
        ZshGlobQualifier::LetterSequence {
            text,
            span: letters_span,
        },
        ZshGlobQualifier::NumericArgument {
            span: range_span,
            start,
            end: Some(end),
        },
    ] = qualifiers.fragments.as_slice()
    else {
        panic!("expected dot, letter sequence, and numeric range qualifiers");
    };

    assert_eq!(dot_span.slice(source), ".");
    assert_eq!(letters_span.slice(source), "om");
    assert_eq!(text.slice(source), "om");
    assert_eq!(range_span.slice(source), "[1,3]");
    assert_eq!(start.slice(source), "1");
    assert_eq!(end.slice(source), "3");
}

#[test]
fn test_zsh_trailing_glob_qualifier_parses_prefixed_glob_with_negation() {
    let source = "print foo*(^-)\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "foo*(^-)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "foo*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::Classic);
    assert_eq!(qualifiers.span.slice(source), "(^-)");
    let [
        ZshGlobQualifier::Negation {
            span: negation_span,
        },
        ZshGlobQualifier::Flag {
            name: '-',
            span: flag_span,
        },
    ] = qualifiers.fragments.as_slice()
    else {
        panic!("expected negation and dash flag qualifiers");
    };
    assert_eq!(negation_span.slice(source), "^");
    assert_eq!(flag_span.slice(source), "-");
}

#[test]
fn test_zsh_inline_glob_case_insensitive_control_preserves_segments() {
    let source = "print (#i)*.jpg\n";
    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);

    let [control, segment] = glob.segments.as_slice() else {
        panic!("expected inline control followed by pattern segment");
    };
    let ZshGlobSegment::InlineControl(ZshInlineGlobControl::CaseInsensitive { span }) = control
    else {
        panic!("expected case-insensitive inline control");
    };

    assert_eq!(glob.span.slice(source), "(#i)*.jpg");
    assert_eq!(span.slice(source), "(#i)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*.jpg"
    );
    assert!(glob.qualifiers.is_none());
}

#[test]
fn test_zsh_inline_glob_backreference_control_preserves_segments() {
    let source = "print (#b)(*)\n";
    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);

    let [control, segment] = glob.segments.as_slice() else {
        panic!("expected inline control followed by pattern segment");
    };
    let ZshGlobSegment::InlineControl(ZshInlineGlobControl::Backreferences { span }) = control
    else {
        panic!("expected backreference inline control");
    };

    assert_eq!(glob.span.slice(source), "(#b)(*)");
    assert_eq!(span.slice(source), "(#b)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "(*)"
    );
    assert!(glob.qualifiers.is_none());
}

#[test]
fn test_zsh_extended_glob_inline_backreference_word_requires_option() {
    let source = "print (#b)(*)\n";

    let default_output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let default_command = expect_simple(&default_output.file.body[0]);
    let default_glob = expect_zsh_qualified_glob(&default_command.args[0]);
    let [default_segment] = default_glob.segments.as_slice() else {
        panic!("expected a single source-preserved pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(default_segment).render_syntax(source),
        "(#b)(*)"
    );
    assert!(default_glob.qualifiers.is_none());

    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let command = expect_simple(&output.file.body[0]);
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let [control, segment] = glob.segments.as_slice() else {
        panic!("expected inline control followed by pattern segment");
    };
    assert!(matches!(
        control,
        ZshGlobSegment::InlineControl(ZshInlineGlobControl::Backreferences { span })
            if span.slice(source) == "(#b)"
    ));
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "(*)"
    );
}

#[test]
fn test_zsh_inline_glob_anchor_controls_preserve_segments() {
    let parser = Parser::with_dialect("", ShellDialect::Zsh);

    let (start_len, start) = parser
        .parse_zsh_inline_glob_control("(#s)", Position::new(), 0)
        .expect("expected start-anchor control");
    let (end_len, end) = parser
        .parse_zsh_inline_glob_control("(#e)", Position::new(), 0)
        .expect("expected end-anchor control");

    assert_eq!(start_len, "(#s)".len());
    assert_eq!(end_len, "(#e)".len());
    assert!(matches!(
        start,
        ZshInlineGlobControl::StartAnchor { span } if span.slice("(#s)") == "(#s)"
    ));
    assert!(matches!(
        end,
        ZshInlineGlobControl::EndAnchor { span } if span.slice("(#e)") == "(#e)"
    ));
}

#[test]
fn test_zsh_hash_q_glob_qualifier_parses_terminal_flag_group() {
    let source = "print *.log(#qN)\n";
    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "*.log(#qN)");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*.log"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::HashQ);
    assert_eq!(qualifiers.span.slice(source), "(#qN)");
    assert!(matches!(
        qualifiers.fragments.as_slice(),
        [ZshGlobQualifier::Flag { name: 'N', span }] if span.slice(source) == "N"
    ));
}

#[test]
fn test_zsh_extended_glob_hash_q_qualifier_requires_option() {
    let source = "print *.log(#qN)\n";

    let default_output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let default_command = expect_simple(&default_output.file.body[0]);
    let default_glob = expect_zsh_qualified_glob(&default_command.args[0]);
    let [default_segment] = default_glob.segments.as_slice() else {
        panic!("expected a single source-preserved pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(default_segment).render_syntax(source),
        "*.log(#qN)"
    );
    assert!(default_glob.qualifiers.is_none());

    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let command = expect_simple(&output.file.body[0]);
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::HashQ);
    assert_eq!(qualifiers.span.slice(source), "(#qN)");
}

#[test]
fn test_zsh_hash_q_glob_qualifier_parses_recursive_pattern_with_letter_sequence_and_range() {
    let source = "print **/*(#q.om[1,3])\n";
    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let qualifiers = expect_zsh_glob_qualifiers(glob);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };

    assert_eq!(glob.span.slice(source), "**/*(#q.om[1,3])");
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "**/*"
    );
    assert_eq!(qualifiers.kind, ZshGlobQualifierKind::HashQ);
    assert_eq!(qualifiers.span.slice(source), "(#q.om[1,3])");

    let [
        ZshGlobQualifier::Flag {
            name: '.',
            span: dot_span,
        },
        ZshGlobQualifier::LetterSequence {
            text,
            span: letters_span,
        },
        ZshGlobQualifier::NumericArgument {
            span: range_span,
            start,
            end: Some(end),
        },
    ] = qualifiers.fragments.as_slice()
    else {
        panic!("expected dot, letter sequence, and numeric range qualifiers");
    };

    assert_eq!(dot_span.slice(source), ".");
    assert_eq!(letters_span.slice(source), "om");
    assert_eq!(text.slice(source), "om");
    assert_eq!(range_span.slice(source), "[1,3]");
    assert_eq!(start.slice(source), "1");
    assert_eq!(end.slice(source), "3");
}

#[test]
fn test_zsh_glob_preserves_unsupported_hash_control_group_as_source_backed_glob() {
    let source = "print *(#a)\n";
    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single source-preserved pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "*(#a)"
    );
}

#[test]
fn test_zsh_ksh_glob_word_requires_option() {
    let source = "print @(foo|bar)\n";

    let default_output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let default_command = expect_simple(&default_output.file.body[0]);
    assert_eq!(default_command.args[0].span.slice(source), "@(foo|bar)");
    assert!(!matches!(
        default_command.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let output = parse_zsh_with_options(source, |options| {
        options.ksh_glob = OptionValue::On;
    });
    let command = expect_simple(&output.file.body[0]);
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };
    let [part] = expect_zsh_glob_pattern_segment(segment).parts.as_slice() else {
        panic!("expected a single group part");
    };
    let PatternPart::Group { kind, patterns } = &part.kind else {
        panic!("expected ksh glob group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(source))
            .collect::<Vec<_>>(),
        vec!["foo", "bar"]
    );
}

#[test]
fn test_zsh_extended_glob_tilde_word_requires_option() {
    let source = "print foo~bar\n";

    let default_output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let default_command = expect_simple(&default_output.file.body[0]);
    assert_eq!(default_command.args[0].span.slice(source), "foo~bar");
    assert!(!matches!(
        default_command.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let command = expect_simple(&output.file.body[0]);
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "foo~bar"
    );
}

#[test]
fn test_zsh_extended_glob_hash_repetition_word_requires_option() {
    let source = "print foo##\n";

    let default_output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let default_command = expect_simple(&default_output.file.body[0]);
    assert_eq!(default_command.args[0].span.slice(source), "foo##");
    assert!(!matches!(
        default_command.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let command = expect_simple(&output.file.body[0]);
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "foo##"
    );
}

#[test]
fn test_zsh_sh_glob_disables_bare_optional_suffix_replacement_pattern_word() {
    let source = "print ${(S)value//ohmyzsh(|.git)/X}\n";

    let default_output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let default_command = expect_simple(&default_output.file.body[0]);
    let default_parameter = expect_parameter(&default_command.args[0]);
    let default_operation = match &default_parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation"),
        _ => panic!("expected zsh parameter syntax"),
    };
    let default_pattern = default_operation
        .pattern_word_ast()
        .expect("expected replacement pattern word");
    let default_glob = expect_zsh_qualified_glob(default_pattern);
    let [segment] = default_glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };
    let pattern = expect_zsh_glob_pattern_segment(segment);
    assert_eq!(
        pattern_part_slices(pattern, source),
        vec!["ohmyzsh", "(|.git)"]
    );
    let part = &pattern.parts[1];
    let PatternPart::Group { patterns, .. } = &part.kind else {
        panic!("expected bare zsh group");
    };
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(source))
            .collect::<Vec<_>>(),
        vec!["", ".git"]
    );

    let sh_glob_output = parse_zsh_with_options(source, |options| {
        options.sh_glob = OptionValue::On;
    });
    let sh_glob_command = expect_simple(&sh_glob_output.file.body[0]);
    let sh_glob_parameter = expect_parameter(&sh_glob_command.args[0]);
    let sh_glob_operation = match &sh_glob_parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation"),
        _ => panic!("expected zsh parameter syntax"),
    };
    let sh_glob_pattern = sh_glob_operation
        .pattern_word_ast()
        .expect("expected replacement pattern word");
    assert_eq!(sh_glob_pattern.span.slice(source), "ohmyzsh(|.git)");
    assert!(!matches!(
        sh_glob_pattern.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));
}

#[test]
fn test_zsh_sh_glob_disables_numeric_range_replacement_pattern_word() {
    let source = "print ${(S)value//jobspec:<->/X}\n";

    let default_output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let default_command = expect_simple(&default_output.file.body[0]);
    let default_parameter = expect_parameter(&default_command.args[0]);
    let default_operation = match &default_parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation"),
        _ => panic!("expected zsh parameter syntax"),
    };
    let default_word = default_operation
        .pattern_word_ast()
        .expect("expected replacement pattern word");
    assert!(matches!(
        default_word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let sh_glob_output = parse_zsh_with_options(source, |options| {
        options.sh_glob = OptionValue::On;
    });
    let sh_glob_command = expect_simple(&sh_glob_output.file.body[0]);
    let sh_glob_parameter = expect_parameter(&sh_glob_command.args[0]);
    let sh_glob_operation = match &sh_glob_parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation"),
        _ => panic!("expected zsh parameter syntax"),
    };
    let sh_glob_word = sh_glob_operation
        .pattern_word_ast()
        .expect("expected replacement pattern word");
    assert_eq!(sh_glob_word.span.slice(source), "jobspec:<->");
    assert!(!matches!(
        sh_glob_word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));
}

#[test]
fn test_zsh_sh_glob_still_allows_ksh_groups_when_enabled() {
    let source = "print @(foo|bar)\n";
    let output = parse_zsh_with_options(source, |options| {
        options.sh_glob = OptionValue::On;
        options.ksh_glob = OptionValue::On;
    });
    let command = expect_simple(&output.file.body[0]);
    let glob = expect_zsh_qualified_glob(&command.args[0]);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "@(foo|bar)"
    );
}

#[test]
fn test_non_zsh_dialects_do_not_special_case_trailing_glob_qualifiers() {
    for syntax in [
        "*(.)",
        "*(/)",
        "*(N)",
        "**/*(.om[1,3])",
        "foo*(^-)",
        "(#i)*.jpg",
        "(#b)(*)",
        "*.log(#qN)",
        "**/*(#q.om[1,3])",
    ] {
        let source = format!("print {syntax}\n");

        for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
            let output = Parser::with_dialect(&source, dialect).parse().unwrap();
            let AstCommand::Simple(command) = &output.file.body[0].command else {
                panic!("expected simple command");
            };

            assert_eq!(
                command.args[0].span.slice(&source),
                syntax,
                "expected non-zsh dialect {dialect:?} to preserve {syntax:?} as a plain word",
            );
            assert!(
                !matches!(
                    command.args[0].parts.as_slice(),
                    [WordPartNode {
                        kind: WordPart::ZshQualifiedGlob(_),
                        ..
                    }]
                ),
                "unexpected zsh-qualified glob node for {syntax:?} in {dialect:?}",
            );
        }
    }
}

#[test]
fn test_zsh_additional_upstream_glob_examples_parse_as_single_arguments() {
    for (command_name, syntax) in [
        ("print", "*(@)"),
        ("print", "*(=)"),
        ("print", "*(/^F)"),
        ("print", "*(Lk+10)"),
        ("print", "*(.Lm+5)"),
        ("print", "*(:h)"),
        ("print", "*(:e)"),
        ("print", "*(:u)"),
        ("print", "*(:l)"),
        ("print", "*(.)(:t)"),
        ("ls", "glob.tmp/**(.)"),
        ("echo", "file(#q.)"),
        ("ls", "*.c~*foo*"),
        ("ls", "^*.txt"),
        ("echo", "(#a2)pattern"),
        ("echo", "(#iq)*.txt"),
        ("echo", "**/*(.N)"),
        ("echo", "**/*(#qN.)~*test*"),
    ] {
        let source = format!("{command_name} {syntax}\n");
        let output = Parser::with_dialect(&source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let command = expect_simple(&output.file.body[0]);

        assert_eq!(command.args.len(), 1, "expected one arg for {syntax:?}");
        assert_eq!(command.args[0].span.slice(&source), syntax);
    }
}

#[test]
fn test_zsh_remaining_upstream_glob_qualifier_examples_parse_as_single_arguments() {
    for (command_name, syntax) in [
        ("print", "*(/)"),
        ("print", "*(*)"),
        ("print", "*(%)"),
        ("print", "*(:t)"),
        ("print", "*(:r)"),
        ("echo", "*(Lk+100)"),
        ("echo", "*(Lm-1)"),
        ("echo", "*(Lg2)"),
        ("echo", "*(mh-1)"),
        ("echo", "*(mm+30)"),
        ("echo", "*(ms-3600)"),
        ("echo", "*(cD-1)"),
        ("echo", "*(om)"),
        ("echo", "*(On)"),
        ("echo", "*(oL)"),
        ("echo", "*(oc)"),
        ("echo", "*.txt(:r)"),
        ("echo", "*.tar.gz(:r:r)"),
        ("echo", "/path/file(:h)"),
        ("echo", "/path/file(:t)"),
        ("ls", "**/*.py(mh-24.L+1k)"),
    ] {
        let source = format!("{command_name} {syntax}\n");
        let output = Parser::with_dialect(&source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let command = expect_simple(&output.file.body[0]);

        assert_eq!(command.args.len(), 1, "expected one arg for {syntax:?}");
        assert_eq!(command.args[0].span.slice(&source), syntax);
    }
}

#[test]
fn test_zsh_remaining_upstream_extended_glob_examples_parse_as_single_arguments() {
    for syntax in [
        "(#a1)approx*",
        "(#a3)vague*",
        "(#i)*.TXT",
        "(#b)(*).backup~$match[1]",
        "*.txt(#qN.)",
        "(#i)*.txt(.)",
        "(#q)test*(N)",
        "(#l)FILE*(.om)",
    ] {
        let source = format!("echo {syntax}\n");
        let output = parse_zsh_with_options(&source, |options| {
            options.extended_glob = OptionValue::On;
        });
        let command = expect_simple(&output.file.body[0]);

        assert_eq!(command.args.len(), 1, "expected one arg for {syntax:?}");
        assert_eq!(command.args[0].span.slice(&source), syntax);
    }
}

#[test]
fn test_zsh_remaining_upstream_complex_glob_examples_parse() {
    for source in [
        "print glob.tmp/**/*~*/dir3(/*|(#e))(/)\n",
        "find . -name \"*.c\" -o -name \"*.h\" | grep -v test\n",
        "echo ${files[@]:#*.tmp}\n",
    ] {
        parse_zsh_with_options(source, |options| {
            options.extended_glob = OptionValue::On;
        });
    }
}

#[test]
fn test_zsh_upstream_alternative_glob_examples_parse() {
    for source in ["echo file.(txt|doc|pdf)\n", "echo file.{txt,doc,pdf}\n"] {
        Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
    }
}

#[test]
fn test_zsh_upstream_alternative_glob_examples_preserve_full_argument_spans() {
    for syntax in ["file.(txt|doc|pdf)", "file.{txt,doc,pdf}"] {
        let source = format!("echo {syntax}\n");
        let output = Parser::with_dialect(&source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let command = expect_simple(&output.file.body[0]);

        assert_eq!(command.args.len(), 1, "expected one arg for {syntax:?}");
        assert_eq!(command.args[0].span.slice(&source), syntax);
    }
}

#[test]
fn test_zsh_numeric_glob_range_example_parses() {
    Parser::with_dialect("ls <1-100>.txt\n", ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_zsh_glob_qualifier_in_command_substitution_preserves_inner_argument() {
    let source = "files=$(echo *(.))\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let body =
        first_command_substitution_body(&word.parts).expect("expected command substitution body");
    let inner = expect_simple(&body[0]);

    assert_eq!(inner.name.render(source), "echo");
    assert_eq!(inner.args.len(), 1);
    assert_eq!(inner.args[0].span.slice(source), "*(.)");
}

#[test]
fn test_zsh_additional_upstream_pattern_removal_examples_parse() {
    for source in [
        "echo \"${arr[@]:#pattern}\"\n",
        "echo \"${arr[@]:%pattern}\"\n",
        "filtered=(\"${(@)ingest:#--ingest=*}\")\n",
        "basenames=(\"${(@)paths:##*/}\")\n",
        "echo \"${arr[@]:# }\"\n",
    ] {
        Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
    }
}

#[test]
fn test_parse_zsh_array_assignment_with_word_target_and_glob_qualifier() {
    let source = "dirs=( /proc/${^$(pidof zsh):#$$}/cwd(N:A) )\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(
        array.span.slice(source),
        "( /proc/${^$(pidof zsh):#$$}/cwd(N:A) )"
    );
}

#[test]
fn test_parse_zsh_assignment_with_nested_subscript_pattern_range() {
    let source = "in_alias=($in_alias[$in_alias[(i)<1->],-1])\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.assignments.len(), 1);
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(
        array.span.slice(source),
        "($in_alias[$in_alias[(i)<1->],-1])"
    );
}

#[test]
fn test_zsh_replacement_pattern_word_requires_extended_glob() {
    let source = "print ${(S)value//(#m)o/X}\n";

    let default_output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let default_command = expect_simple(&default_output.file.body[0]);
    let default_parameter = expect_parameter(&default_command.args[0]);
    let default_operation = match &default_parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation"),
        _ => panic!("expected zsh parameter syntax"),
    };
    let default_pattern = default_operation
        .pattern_word_ast()
        .expect("expected replacement pattern word");
    assert!(!matches!(
        default_pattern.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let output = parse_zsh_with_options(source, |options| {
        options.extended_glob = OptionValue::On;
    });
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);
    let operation = match &parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation"),
        _ => panic!("expected zsh parameter syntax"),
    };
    let pattern = operation
        .pattern_word_ast()
        .expect("expected replacement pattern word");
    let glob = expect_zsh_qualified_glob(pattern);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single source-preserved pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(source),
        "(#m)o"
    );
}

#[test]
fn test_zsh_setopt_and_unsetopt_sh_glob_change_following_replacement_pattern_parse() {
    let source = concat!(
        "print ${(S)value//jobspec:<->/X}\n",
        "setopt sh_glob\n",
        "print ${(S)value//jobspec:<->/X}\n",
        "unsetopt sh_glob\n",
        "print ${(S)value//jobspec:<->/X}\n",
    );
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let first = expect_simple(&output.file.body[0]);
    let first_parameter = expect_parameter(&first.args[0]);
    let first_pattern = match &first_parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation")
            .pattern_word_ast()
            .expect("expected replacement pattern word"),
        _ => panic!("expected zsh parameter syntax"),
    };
    assert!(matches!(
        first_pattern.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let second = expect_simple(&output.file.body[2]);
    let second_parameter = expect_parameter(&second.args[0]);
    let second_pattern = match &second_parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation")
            .pattern_word_ast()
            .expect("expected replacement pattern word"),
        _ => panic!("expected zsh parameter syntax"),
    };
    assert!(!matches!(
        second_pattern.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let third = expect_simple(&output.file.body[4]);
    let third_parameter = expect_parameter(&third.args[0]);
    let third_pattern = match &third_parameter.syntax {
        ParameterExpansionSyntax::Zsh(parameter) => parameter
            .operation
            .as_ref()
            .expect("expected replacement operation")
            .pattern_word_ast()
            .expect("expected replacement pattern word"),
        _ => panic!("expected zsh parameter syntax"),
    };
    assert!(matches!(
        third_pattern.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));
}

#[test]
fn test_zsh_setopt_ksh_glob_changes_following_word_parse() {
    let source = concat!(
        "print @(foo|bar)\n",
        "setopt ksh_glob\n",
        "print @(foo|bar)\n",
    );
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let first = expect_simple(&output.file.body[0]);
    assert!(!matches!(
        first.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let second = expect_simple(&output.file.body[2]);
    assert!(matches!(
        second.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));
}

#[test]
fn test_zsh_emulate_extended_glob_changes_following_word_parse() {
    let source = concat!(
        "print foo##\n",
        "emulate -L zsh -o extended_glob\n",
        "print foo##\n",
    );
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let first = expect_simple(&output.file.body[0]);
    assert!(!matches!(
        first.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));

    let second = expect_simple(&output.file.body[2]);
    assert!(matches!(
        second.args[0].parts.as_slice(),
        [WordPartNode {
            kind: WordPart::ZshQualifiedGlob(_),
            ..
        }]
    ));
}

#[test]
fn test_parse_zsh_compound_array_with_nested_groups_and_qualifiers() {
    let source = "local -a bats=( /sys/class/power_supply/(CMB*|BAT*|*battery)/(FN) )\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_arithmetic_shell_word_with_short_length_subscript_pattern() {
    let source = "if (( ! $#functions[(i)n(odenv|vm)] )); then\n  return 1\nfi\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}
