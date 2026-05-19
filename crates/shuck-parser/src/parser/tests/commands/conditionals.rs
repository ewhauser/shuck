use super::*;

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
fn test_posix_function_parses_negated_bracket_if_body() {
    let input = "\
recent_file(){
    if ! [ -f \"$file\" ]; then
        return 1
    elif find \"$file\" -mtime -\"$days\" -print | grep -q .; then
        return 0
    else
        local days_ago_in_seconds
        days_ago_in_seconds=\"$(date -d \"$days days ago\" '+%s')\"
        if is_mac; then
            if [ \"$(stat -f '%m' \"$file\")\" -ge \"$days_ago_in_seconds\" ]; then
                return 0
            else
                return 1
            fi
        elif [ \"$(stat -c '%Y' \"$file\")\" -ge \"$days_ago_in_seconds\" ]; then
            return 0
        else
            return 1
        fi
    fi
}
";
    let parsed = Parser::new(input).parse().unwrap();

    assert!(matches!(
        expect_function(&parsed.file.body[0]).body.command,
        AstCommand::Compound(..)
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
fn test_parse_conditional_accepts_adjacent_group_closes_after_rhs_words() {
    let input = "[[ -n $brew_prefix && (($brew_prefix != \"/usr\" && $brew_prefix != \"/usr/local\") || (is_mac && osx_using_default_compiler && $CFLAGS =~ (^|\\ )-isysroot\\ )) ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    assert_eq!(binary.op, ConditionalBinaryOp::And);
    assert!(matches!(
        binary.right.as_ref(),
        ConditionalExpr::Parenthesized(_)
    ));
}

#[test]
fn test_parse_conditional_stops_quoted_rhs_before_adjacent_group_closes() {
    for input in ["[[ (( x == \"foo\" )) ]]\n", "[[ (( x =~ \"foo\" )) ]]\n"] {
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command for {input}");
        };

        assert!(matches!(
            command.expression,
            ConditionalExpr::Parenthesized(_)
        ));
    }
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
    let script = parse_zsh_with_options(input, |options| {
        options.extended_glob = OptionValue::On;
    })
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
    let script = parse_zsh_with_options(input, |options| {
        options.extended_glob = OptionValue::On;
    })
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
fn test_zsh_conditional_ksh_glob_requires_option() {
    let input = "[[ $mode == @(disable|enable) ]]\n";

    let default_script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (default_compound, _) = expect_compound(&default_script.body[0]);
    let AstCompoundCommand::Conditional(default_command) = default_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(default_binary) = &default_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(default_pattern) = default_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(default_pattern.render_syntax(input), "@(disable|enable)");
    let [prefix, group] = default_pattern.parts.as_slice() else {
        panic!("expected literal prefix followed by bare zsh group");
    };
    assert!(matches!(&prefix.kind, PatternPart::Literal(_)));
    let PatternPart::Group { kind, patterns } = &group.kind else {
        panic!("expected bare zsh group after literal prefix");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["disable", "enable"]
    );

    let script = parse_zsh_with_options(input, |options| {
        options.ksh_glob = OptionValue::On;
    })
    .file;
    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(pattern.render_syntax(input), "@(disable|enable)");
    let glob = expect_pattern_zsh_qualified_glob(pattern);
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
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["disable", "enable"]
    );
}

#[test]
fn test_zsh_conditional_prefixed_bare_group_reparse_handles_single_glob_word() {
    let input = "[[ $mode == *@(disable|enable) ]]\n";

    let default_script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (default_compound, _) = expect_compound(&default_script.body[0]);
    let AstCompoundCommand::Conditional(default_command) = default_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(default_binary) = &default_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(default_pattern) = default_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(default_pattern.render_syntax(input), "*@(disable|enable)");
    let [wildcard, prefix, group] = default_pattern.parts.as_slice() else {
        panic!("expected wildcard, literal prefix, and bare zsh group");
    };
    assert!(matches!(&wildcard.kind, PatternPart::AnyString));
    assert!(matches!(&prefix.kind, PatternPart::Literal(_)));
    let PatternPart::Group { kind, patterns } = &group.kind else {
        panic!("expected bare zsh group after literal prefix");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["disable", "enable"]
    );

    let script = parse_zsh_with_options(input, |options| {
        options.ksh_glob = OptionValue::On;
    })
    .file;
    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(pattern.render_syntax(input), "*@(disable|enable)");
    let glob = expect_pattern_zsh_qualified_glob(pattern);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };
    assert!(matches!(
        expect_zsh_glob_pattern_segment(segment).parts.as_slice(),
        [
            PatternPartNode {
                kind: PatternPart::AnyString,
                ..
            },
            PatternPartNode {
                kind: PatternPart::Group {
                    kind: PatternGroupKind::ExactlyOne,
                    ..
                },
                ..
            }
        ]
    ));
}

#[test]
fn test_zsh_conditional_extended_glob_backreference_requires_option() {
    let input = "[[ $buf == (#b)(*) ]]\n";

    let default_script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (default_compound, _) = expect_compound(&default_script.body[0]);
    let AstCompoundCommand::Conditional(default_command) = default_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(default_binary) = &default_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(default_pattern) = default_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    let default_glob = expect_pattern_zsh_qualified_glob(default_pattern);
    let [default_segment] = default_glob.segments.as_slice() else {
        panic!("expected a single source-preserved pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(default_segment).render_syntax(input),
        "(#b)(*)"
    );

    let script = parse_zsh_with_options(input, |options| {
        options.extended_glob = OptionValue::On;
    })
    .file;
    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(binary) = &command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    let glob = expect_pattern_zsh_qualified_glob(pattern);
    let [control, segment] = glob.segments.as_slice() else {
        panic!("expected inline control followed by pattern segment");
    };
    assert!(matches!(
        control,
        ZshGlobSegment::InlineControl(ZshInlineGlobControl::Backreferences { span })
            if span.slice(input) == "(#b)"
    ));
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(input),
        "(*)"
    );
}

#[test]
fn test_zsh_conditional_bare_group_requires_sh_glob_off() {
    let input = "[[ $mode == (foo|bar) ]]\n";

    let default_script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (default_compound, _) = expect_compound(&default_script.body[0]);
    let AstCompoundCommand::Conditional(default_command) = default_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(default_binary) = &default_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(default_pattern) = default_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert!(matches!(
        default_pattern.parts.as_slice(),
        [PatternPartNode {
            kind: PatternPart::Word(word),
            ..
        }] if matches!(
            word.parts.as_slice(),
            [WordPartNode {
                kind: WordPart::ZshQualifiedGlob(_),
                ..
            }]
        )
    ));

    let sh_glob_script = parse_zsh_with_options(input, |options| {
        options.sh_glob = OptionValue::On;
    })
    .file;
    let (sh_glob_compound, _) = expect_compound(&sh_glob_script.body[0]);
    let AstCompoundCommand::Conditional(sh_glob_command) = sh_glob_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(sh_glob_binary) = &sh_glob_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(sh_glob_pattern) = sh_glob_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(sh_glob_pattern.render(input), "(foo|bar)");
    assert!(!matches!(
        sh_glob_pattern.parts.as_slice(),
        [PatternPartNode {
            kind: PatternPart::Word(word),
            ..
        }] if matches!(
            word.parts.as_slice(),
            [WordPartNode {
                kind: WordPart::ZshQualifiedGlob(_),
                ..
            }]
        )
    ));
}

#[test]
fn test_parse_zsh_conditional_pattern_with_hash_repetition_after_char_class() {
    let input = "[[ $_p9k__ret == (#b)Python\\ ([[:digit:].]##)* ]]\n";
    parse_zsh_with_options(input, |options| {
        options.extended_glob = OptionValue::On;
    });
}

#[test]
fn test_parse_zsh_conditional_pattern_with_numeric_range_prefix_and_and_rhs() {
    let input = "[[ $load == <->(|.<->) && $load != $_p9k__load_value ]]\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_zsh_conditional_numeric_range_requires_hyphen() {
    let literal_input = "[[ $jobspec == jobspec:<123> ]]\n";
    let literal_script = Parser::with_dialect(literal_input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (literal_compound, _) = expect_compound(&literal_script.body[0]);
    let AstCompoundCommand::Conditional(literal_command) = literal_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(literal_binary) = &literal_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(literal_pattern) = literal_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    assert_eq!(literal_pattern.render(literal_input), "jobspec:<123>");
    assert!(!pattern_has_zsh_qualified_glob(literal_pattern));

    let range_input = "[[ $jobspec == jobspec:<1-9> ]]\n";
    let range_script = Parser::with_dialect(range_input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (range_compound, _) = expect_compound(&range_script.body[0]);
    let AstCompoundCommand::Conditional(range_command) = range_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(range_binary) = &range_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(range_pattern) = range_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    let glob = expect_pattern_zsh_qualified_glob(range_pattern);
    let [segment] = glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };
    assert_eq!(
        expect_zsh_glob_pattern_segment(segment).render_syntax(range_input),
        "jobspec:<1-9>"
    );
}

#[test]
fn test_zsh_setopt_ksh_glob_changes_following_conditional_pattern_parse() {
    let input = concat!(
        "[[ $mode == @(disable|enable) ]]\n",
        "setopt ksh_glob\n",
        "[[ $mode == @(disable|enable) ]]\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (first_compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(first_command) = first_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(first_binary) = &first_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(first_pattern) = first_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    let [first_prefix, first_group] = first_pattern.parts.as_slice() else {
        panic!("expected literal prefix followed by bare zsh group");
    };
    assert!(matches!(&first_prefix.kind, PatternPart::Literal(_)));
    assert!(matches!(
        &first_group.kind,
        PatternPart::Group {
            kind: PatternGroupKind::ExactlyOne,
            ..
        }
    ));

    let (second_compound, _) = expect_compound(&script.body[2]);
    let AstCompoundCommand::Conditional(second_command) = second_compound else {
        panic!("expected conditional compound command");
    };
    let ConditionalExpr::Binary(second_binary) = &second_command.expression else {
        panic!("expected binary conditional");
    };
    let ConditionalExpr::Pattern(second_pattern) = second_binary.right.as_ref() else {
        panic!("expected pattern rhs");
    };
    let second_glob = expect_pattern_zsh_qualified_glob(second_pattern);
    let [second_segment] = second_glob.segments.as_slice() else {
        panic!("expected a single pattern segment");
    };
    assert!(matches!(
        expect_zsh_glob_pattern_segment(second_segment)
            .parts
            .as_slice(),
        [PatternPartNode {
            kind: PatternPart::Group {
                kind: PatternGroupKind::ExactlyOne,
                ..
            },
            ..
        }]
    ));
}

#[test]
fn test_parse_zsh_conditional_pattern_rhs_accepts_hash_q_glob_qualifier() {
    let input = "[[ foo.txt == *.txt(#qN) ]]\n";
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
    assert_eq!(pattern.render(input), "*.txt(#qN)");
}

#[test]
fn test_parse_zsh_conditional_pattern_rhs_accepts_negated_hash_q_glob_qualifier() {
    let input = "[[ $file != *.tmp(#qN) ]]\n";
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
    assert_eq!(pattern.render(input), "*.tmp(#qN)");
}

#[test]
fn test_parse_zsh_remaining_upstream_conditionals_and_control_flow_examples() {
    for input in [
        "[[ ( a == b ) || ( c == d ) ]]\n",
        "[[ ! -f file ]]\n",
        "[[ -f file1 && ( -r file1 || -w file1 ) ]]\n",
        "case $var in\n    *.txt) echo \"text file\" ;;\n    *.sh) echo \"shell script\" ;;\n    *) echo \"other\" ;;\nesac\n",
        "if [[ $a -eq $b ]]; then\n    echo \"equal\"\nfi\n",
        "if [[ $a -gt 5 ]]; then\n    echo \"greater\"\nelif [[ $a -lt 0 ]]; then\n    echo \"less\"\nelse\n    echo \"between\"\nfi\n",
        "until [[ $count -ge 10 ]]; do\n    echo $count\n    ((count++))\ndone\n",
        "while true; do\n    if [[ $done ]]; then\n        break\n    fi\n    echo \"working\"\ndone\n",
    ] {
        Parser::with_dialect(input, ShellDialect::Zsh)
            .parse()
            .unwrap();
    }
}

#[test]
fn test_parse_zsh_newer_than_conditional_from_upstream() {
    let input = "[[ file1 -nt file2 ]]\n";
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
    assert_eq!(binary.op, ConditionalBinaryOp::NewerThan);
}

#[test]
fn test_parse_zsh_remaining_upstream_conditional_glob_examples() {
    for input in [
        "[[ $file == *.txt ]]\n",
        "[[ $name != (#i)*.TMP ]]\n",
        "[[ -f *.log(.) ]]\n",
    ] {
        parse_zsh_with_options(input, |options| {
            options.extended_glob = OptionValue::On;
        });
    }
}

#[test]
fn test_parse_zsh_conditional_group_with_arithmetic_subexpression() {
    let input = "until [[ $i -gt 99 || ( $i -ge $((length - ellen)) || $dir == $part ) && ( (( ${#expn} == 1 )) || $dir = $expn ) ]]; do\n  :\ndone\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
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
fn test_parse_conditional_regex_rhs_keeps_parameter_pattern_with_literal_paren() {
    let input = "[[ \"${2}\" =~ ^${__theme%% (*} ]]\n";
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
    assert_eq!(word.render(input), "^${__theme%% (*}");
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
fn test_parse_if_condition_accepts_chained_double_brackets_with_regex_brace_literal() {
    let input = "if [[ $MOTD ]] && ! [[ $MOTD =~ ^{ ]]; then\n  :\nfi\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::If(command) = compound else {
        panic!("expected if command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.condition.len(), 1);
    assert_eq!(expect_binary(&command.condition[0]).op, BinaryOp::And);
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
