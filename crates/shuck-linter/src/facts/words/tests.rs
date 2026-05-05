#[cfg(test)]
mod word_classification_tests {
    use super::*;

    fn parse_commands(source: &str) -> StmtSeq {
        Parser::new(source).parse().unwrap().file.body
    }

    #[test]
    fn detects_alias_positional_parameters_with_runtime_quote_state() {
        assert!(contains_positional_parameter_reference("echo $1"));
        assert!(contains_positional_parameter_reference("echo \"${1}\""));
        assert!(contains_positional_parameter_reference("echo ${#1}"));
        assert!(contains_positional_parameter_reference("echo ${!1}"));
        assert!(contains_positional_parameter_reference(r"echo \$$1"));
        assert!(contains_positional_parameter_reference(r"echo \'$1"));
        assert!(contains_positional_parameter_reference("echo hi# $1"));
        assert!(!contains_positional_parameter_reference(r"echo \$1"));
        assert!(!contains_positional_parameter_reference(r"echo \${1}"));
        assert!(!contains_positional_parameter_reference("echo '$1'"));
        assert!(!contains_positional_parameter_reference("echo hi # $1"));
        assert!(!contains_positional_parameter_reference("echo hi; # $1"));
        assert!(!contains_positional_parameter_reference("echo hi;# $1"));
        assert!(!contains_positional_parameter_reference("echo hi &&# $1"));
        assert!(!contains_positional_parameter_reference("echo $$1"));
    }

    #[test]
    fn classify_word_distinguishes_fixed_literals_and_quoted_expansions() {
        let source = "printf \"literal\" \"prefix$foo\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        let literal = classify_word(&command.args[0], source);
        assert_eq!(literal.quote, WordQuote::FullyQuoted);
        assert_eq!(literal.literalness, WordLiteralness::FixedLiteral);
        assert_eq!(literal.expansion_kind, WordExpansionKind::None);
        assert_eq!(literal.substitution_shape, WordSubstitutionShape::None);

        let expanded = classify_word(&command.args[1], source);
        assert_eq!(expanded.quote, WordQuote::FullyQuoted);
        assert_eq!(expanded.literalness, WordLiteralness::Expanded);
        assert_eq!(expanded.expansion_kind, WordExpansionKind::Scalar);
        assert_eq!(expanded.substitution_shape, WordSubstitutionShape::None);
    }

    #[test]
    fn classify_word_reports_plain_and_mixed_command_substitutions() {
        let source = "printf \"$(date)\" \"prefix$(date)\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        assert_eq!(
            classify_word(&command.args[0], source).substitution_shape,
            WordSubstitutionShape::Plain
        );
        assert_eq!(
            classify_word(&command.args[1], source).substitution_shape,
            WordSubstitutionShape::Mixed
        );
    }

    #[test]
    fn classify_word_treats_escaped_backslash_before_command_substitution_as_mixed() {
        let source = "printf \"\\\\$(printf '%03o' \"$i\")\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        let classification = classify_word(&command.args[0], source);
        assert_eq!(classification.quote, WordQuote::FullyQuoted);
        assert_eq!(classification.literalness, WordLiteralness::Expanded);
        assert_eq!(
            classification.substitution_shape,
            WordSubstitutionShape::Mixed
        );
    }

    #[test]
    fn classify_word_reports_scalar_and_array_expansions() {
        let source = "printf $foo ${arr[@]} ${arr[0]} ${arr[@]:1}\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        assert_eq!(
            classify_word(&command.args[0], source).expansion_kind,
            WordExpansionKind::Scalar
        );
        assert_eq!(
            classify_word(&command.args[1], source).expansion_kind,
            WordExpansionKind::Array
        );
        assert_eq!(
            classify_word(&command.args[2], source).expansion_kind,
            WordExpansionKind::Scalar
        );
        assert_eq!(
            classify_word(&command.args[3], source).expansion_kind,
            WordExpansionKind::Array
        );
    }

    #[test]
    fn plain_parameter_reference_accepts_single_direct_expansions_only() {
        let source = "\
printf '%s\\n' \
$name \"$name\" ${name} \"${name}\" $1 \"$#\" \"$@\" ${*} \
${@:2} ${arr[0]} ${arr[@]} ${!name} ${name:-fallback} \"$@$@\" \"prefix$name\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        let plain = command
            .args
            .iter()
            .skip(1)
            .map(word_is_plain_parameter_reference)
            .collect::<Vec<_>>();

        assert_eq!(
            plain,
            vec![
                true, true, true, true, true, true, true, true, false, false, false, false, false,
                false, false
            ]
        );
    }

    #[test]
    fn classify_test_and_conditional_operands_share_literal_runtime_decisions() {
        let source = "test foo\ntest ~\n[[ \"$re\" ]]\n[[ literal ]]\n[[ ~ ]]\n";
        let commands = parse_commands(source);

        let Command::Simple(simple_test) = &commands[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(
            classify_contextual_operand(
                &simple_test.args[0],
                source,
                ExpansionContext::CommandArgument
            ),
            TestOperandClass::FixedLiteral
        );

        let Command::Simple(runtime_test) = &commands[1].command else {
            panic!("expected simple command");
        };
        assert_eq!(
            classify_contextual_operand(
                &runtime_test.args[0],
                source,
                ExpansionContext::CommandArgument
            ),
            TestOperandClass::RuntimeSensitive
        );

        let Command::Compound(CompoundCommand::Conditional(runtime)) = &commands[2].command else {
            panic!("expected conditional");
        };
        assert_eq!(
            classify_conditional_operand(&runtime.expression, source),
            TestOperandClass::RuntimeSensitive
        );

        let Command::Compound(CompoundCommand::Conditional(literal)) = &commands[3].command else {
            panic!("expected conditional");
        };
        assert_eq!(
            classify_conditional_operand(&literal.expression, source),
            TestOperandClass::FixedLiteral
        );

        let Command::Compound(CompoundCommand::Conditional(runtime)) = &commands[4].command else {
            panic!("expected conditional");
        };
        assert_eq!(
            classify_conditional_operand(&runtime.expression, source),
            TestOperandClass::RuntimeSensitive
        );
    }

    #[test]
    fn contextual_operand_classification_respects_regex_and_case_contexts() {
        let source = "printf ~ *.sh {a,b}\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        assert_eq!(
            classify_contextual_operand(&command.args[0], source, ExpansionContext::RegexOperand),
            TestOperandClass::RuntimeSensitive
        );
        assert_eq!(
            classify_contextual_operand(&command.args[1], source, ExpansionContext::CasePattern),
            TestOperandClass::FixedLiteral
        );
        assert_eq!(
            classify_contextual_operand(&command.args[2], source, ExpansionContext::CasePattern),
            TestOperandClass::FixedLiteral
        );
    }
}

#[cfg(test)]
mod expansion_analysis_tests {
    use shuck_ast::Command;
    use shuck_parser::parser::{Parser, ShellDialect};

    use super::{
        ComparablePathKey, ComparablePathPart, ExpansionAnalysis, ExpansionContext,
        ExpansionValueShape, RedirectDevNullStatus, WordLiteralness, WordQuote,
        analyze_literal_runtime, analyze_redirect_target, analyze_word, comparable_path,
    };
    use crate::{
        FieldSplittingBehavior, GlobDotBehavior, GlobFailureBehavior, PathnameExpansionBehavior,
        PatternOperatorBehavior,
    };

    fn parse_argument_words(source: &str) -> Vec<shuck_ast::Word> {
        let file = Parser::new(source).parse().unwrap().file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        command.args.to_vec()
    }

    fn analyze_argument_words(source: &str) -> Vec<ExpansionAnalysis> {
        parse_argument_words(source)
            .iter()
            .map(|word| analyze_word(word, source, None))
            .collect()
    }

    fn analyze_argument_words_with_dialect(
        source: &str,
        dialect: ShellDialect,
    ) -> Vec<ExpansionAnalysis> {
        let file = Parser::with_dialect(source, dialect).parse().unwrap().file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        command
            .args
            .iter()
            .map(|word| analyze_word(word, source, None))
            .collect()
    }

    #[test]
    fn analyze_word_tracks_array_values_and_multi_field_expansions_separately() {
        let analyses = analyze_argument_words(
            "printf %s ${arr[@]} \"${arr[*]}\" ${!prefix@} ${!name} ${value@Q}\n",
        );

        assert_eq!(analyses[1].value_shape, ExpansionValueShape::MultiField);
        assert!(analyses[1].array_valued);
        assert!(analyses[1].can_expand_to_multiple_fields);

        assert_eq!(analyses[2].quote, WordQuote::FullyQuoted);
        assert_eq!(analyses[2].value_shape, ExpansionValueShape::Array);
        assert!(analyses[2].array_valued);
        assert!(!analyses[2].can_expand_to_multiple_fields);

        assert_eq!(analyses[3].value_shape, ExpansionValueShape::MultiField);
        assert!(!analyses[3].array_valued);
        assert!(analyses[3].can_expand_to_multiple_fields);

        assert_eq!(analyses[4].value_shape, ExpansionValueShape::Unknown);
        assert_eq!(analyses[4].literalness, WordLiteralness::Expanded);

        assert_eq!(analyses[5].value_shape, ExpansionValueShape::MultiField);
        assert_eq!(analyses[5].literalness, WordLiteralness::Expanded);
        assert!(!analyses[5].array_valued);
    }

    #[test]
    fn analyze_word_marks_prefix_match_at_as_multi_field_even_when_quoted() {
        let analyses = analyze_argument_words("printf %s \"${!prefix@}\" \"${!prefix*}\"\n");

        assert_eq!(analyses[1].value_shape, ExpansionValueShape::MultiField);
        assert!(analyses[1].can_expand_to_multiple_fields);

        assert_eq!(analyses[2].value_shape, ExpansionValueShape::Unknown);
        assert!(!analyses[2].can_expand_to_multiple_fields);
    }

    #[test]
    fn analyze_word_treats_bourne_transformations_as_split_and_glob_hazards() {
        let analyses = analyze_argument_words("printf %s ${name@U}\n");

        assert_eq!(analyses[1].value_shape, ExpansionValueShape::MultiField);
        assert!(analyses[1].hazards.field_splitting);
        assert!(analyses[1].hazards.pathname_matching);
        assert!(analyses[1].can_expand_to_multiple_fields);
    }

    #[test]
    fn analyze_word_distinguishes_typed_zsh_pattern_families() {
        let analyses = analyze_argument_words_with_dialect(
            "print ${(m)foo#${needle}} ${(S)foo/$pattern/$replacement} ${(m)foo:$offset:${length}} ${(m)foo:-$fallback}\n",
            ShellDialect::Zsh,
        );

        assert!(analyses[0].hazards.runtime_pattern);
        assert!(analyses[1].hazards.runtime_pattern);
        assert!(!analyses[2].hazards.runtime_pattern);
        assert!(!analyses[3].hazards.runtime_pattern);
        assert!(
            analyses
                .iter()
                .all(|analysis| analysis.value_shape == ExpansionValueShape::Unknown)
        );
    }

    #[test]
    fn analyze_word_treats_zsh_trailing_glob_qualifiers_as_non_literal_pathname_hazards() {
        let analyses =
            analyze_argument_words_with_dialect("print **/*(.om[1,3])\n", ShellDialect::Zsh);

        assert_eq!(analyses[0].literalness, WordLiteralness::Expanded);
        assert!(!analyses[0].is_fixed_literal());
        assert_eq!(analyses[0].value_shape, ExpansionValueShape::Unknown);
        assert!(analyses[0].hazards.pathname_matching);
        assert!(analyses[0].can_expand_to_multiple_fields);
        assert!(!analyses[0].array_valued);
    }

    #[test]
    fn analyze_word_treats_zsh_inline_glob_controls_as_non_literal_pathname_hazards() {
        let analyses = analyze_argument_words_with_dialect("print (#i)*.jpg\n", ShellDialect::Zsh);

        assert_eq!(analyses[0].literalness, WordLiteralness::Expanded);
        assert!(!analyses[0].is_fixed_literal());
        assert_eq!(analyses[0].value_shape, ExpansionValueShape::Unknown);
        assert!(analyses[0].hazards.pathname_matching);
        assert!(analyses[0].can_expand_to_multiple_fields);
        assert!(!analyses[0].array_valued);
    }

    #[test]
    fn analyze_word_suppresses_zsh_glob_fanout_when_glob_is_disabled() {
        let source = "print *.jpg\n";
        let file = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        let options = shuck_semantic::ZshOptionState {
            glob: shuck_semantic::OptionValue::Off,
            ..shuck_semantic::ZshOptionState::zsh_default()
        };
        let analysis = analyze_word(&command.args[0], source, Some(&options));

        assert!(!analysis.hazards.pathname_matching);
        assert!(!analysis.can_expand_to_multiple_fields);
    }

    #[test]
    fn analyze_word_records_option_sensitive_expansion_behaviors() {
        let source = "print $name\n";
        let file = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        let native = analyze_word(
            &command.args[0],
            source,
            Some(&shuck_semantic::ZshOptionState::zsh_default()),
        );
        let split_options = shuck_semantic::ZshOptionState {
            sh_word_split: shuck_semantic::OptionValue::On,
            ..shuck_semantic::ZshOptionState::zsh_default()
        };
        let split = analyze_word(&command.args[0], source, Some(&split_options));
        let no_glob_options = shuck_semantic::ZshOptionState {
            glob: shuck_semantic::OptionValue::Off,
            ..shuck_semantic::ZshOptionState::zsh_default()
        };
        let no_glob = analyze_word(&command.args[0], source, Some(&no_glob_options));

        assert_eq!(
            native.field_splitting_behavior,
            FieldSplittingBehavior::Never
        );
        assert_eq!(
            native.pathname_expansion_behavior,
            PathnameExpansionBehavior::LiteralGlobsOnly
        );
        assert_eq!(
            split.field_splitting_behavior,
            FieldSplittingBehavior::UnquotedOnly
        );
        assert!(split.hazards.field_splitting);
        assert_eq!(
            no_glob.pathname_expansion_behavior,
            PathnameExpansionBehavior::Disabled
        );
        assert!(!no_glob.hazards.pathname_matching);
    }

    #[test]
    fn analyze_word_treats_unknown_option_state_as_ambiguous_behavior() {
        let source = "print $name\n";
        let file = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        let analysis = analyze_word(
            &command.args[0],
            source,
            Some(&shuck_semantic::ZshOptionState {
                sh_word_split: shuck_semantic::OptionValue::Unknown,
                glob: shuck_semantic::OptionValue::Unknown,
                glob_subst: shuck_semantic::OptionValue::Unknown,
                ..shuck_semantic::ZshOptionState::zsh_default()
            }),
        );

        assert_eq!(
            analysis.field_splitting_behavior,
            FieldSplittingBehavior::Ambiguous
        );
        assert_eq!(
            analysis.pathname_expansion_behavior,
            PathnameExpansionBehavior::Ambiguous
        );
        assert!(analysis.hazards.field_splitting);
        assert!(analysis.hazards.pathname_matching);
        assert!(analysis.can_expand_to_multiple_fields);
    }

    #[test]
    fn analyze_word_keeps_glob_subst_off_when_glob_state_is_unknown() {
        let source = "print $name\n";
        let file = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        let analysis = analyze_word(
            &command.args[0],
            source,
            Some(&shuck_semantic::ZshOptionState {
                glob: shuck_semantic::OptionValue::Unknown,
                glob_subst: shuck_semantic::OptionValue::Off,
                ..shuck_semantic::ZshOptionState::zsh_default()
            }),
        );

        assert_eq!(
            analysis.pathname_expansion_behavior,
            PathnameExpansionBehavior::LiteralGlobsOnlyOrDisabled
        );
        assert!(!analysis.hazards.pathname_matching);
        assert!(!analysis.can_expand_to_multiple_fields);
    }

    #[test]
    fn analyze_word_applies_zsh_modifier_overrides_to_behaviors() {
        let source = "print ${=name} ${~name} ${~~name}\n";
        let file = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        let options = shuck_semantic::ZshOptionState::zsh_default();
        let split = analyze_word(&command.args[0], source, Some(&options));
        let glob = analyze_word(&command.args[1], source, Some(&options));
        let no_glob_subst = analyze_word(&command.args[2], source, Some(&options));

        assert_eq!(
            split.field_splitting_behavior,
            FieldSplittingBehavior::UnquotedOnly
        );
        assert!(split.hazards.field_splitting);
        assert_eq!(
            glob.pathname_expansion_behavior,
            PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted
        );
        assert!(glob.hazards.pathname_matching);
        assert_eq!(
            no_glob_subst.pathname_expansion_behavior,
            PathnameExpansionBehavior::LiteralGlobsOnly
        );
        assert!(!no_glob_subst.hazards.pathname_matching);
    }

    #[test]
    fn analyze_literal_runtime_tracks_globs_in_mixed_words() {
        let source =
            "printf '%s\\n' \"$basedir/\"* \"$(dirname \"$0\")\"/../docs/usage/distrobox*\n";
        let words = parse_argument_words(source);
        let first = analyze_literal_runtime(&words[1], source, ExpansionContext::ForList, None);
        let second = analyze_literal_runtime(&words[2], source, ExpansionContext::ForList, None);

        assert!(first.hazards.pathname_matching);
        assert!(first.is_runtime_sensitive());
        assert!(second.hazards.pathname_matching);
        assert!(second.is_runtime_sensitive());
    }

    #[test]
    fn analyze_literal_runtime_respects_sh_file_expansion_tilde_order() {
        let source = "print ~$USER ~root/$USER ~/\"$USER\" ~$USER/x\n";
        let words = parse_argument_words(source);
        let native_options = shuck_semantic::ZshOptionState::zsh_default();
        let sh_file_options = shuck_semantic::ZshOptionState {
            sh_file_expansion: shuck_semantic::OptionValue::On,
            ..shuck_semantic::ZshOptionState::zsh_default()
        };
        let unknown_options = shuck_semantic::ZshOptionState {
            sh_file_expansion: shuck_semantic::OptionValue::Unknown,
            ..shuck_semantic::ZshOptionState::zsh_default()
        };

        let native_dynamic_user = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&native_options),
        );
        let sh_file_dynamic_user = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&sh_file_options),
        );
        let sh_file_literal_user = analyze_literal_runtime(
            &words[1],
            source,
            ExpansionContext::CommandArgument,
            Some(&sh_file_options),
        );
        let sh_file_home = analyze_literal_runtime(
            &words[2],
            source,
            ExpansionContext::CommandArgument,
            Some(&sh_file_options),
        );
        let unknown_dynamic_user = analyze_literal_runtime(
            &words[3],
            source,
            ExpansionContext::CommandArgument,
            Some(&unknown_options),
        );
        let default_dynamic_user =
            analyze_literal_runtime(&words[0], source, ExpansionContext::CommandArgument, None);

        assert!(!default_dynamic_user.hazards.tilde_expansion);
        assert!(!default_dynamic_user.is_runtime_sensitive());
        assert!(native_dynamic_user.hazards.tilde_expansion);
        assert!(!sh_file_dynamic_user.is_runtime_sensitive());
        assert!(!sh_file_dynamic_user.hazards.tilde_expansion);
        assert!(sh_file_literal_user.hazards.tilde_expansion);
        assert!(sh_file_home.hazards.tilde_expansion);
        assert!(unknown_dynamic_user.hazards.tilde_expansion);
    }

    #[test]
    fn analyze_literal_runtime_records_glob_failure_behavior() {
        let source = "print *.jpg\n";
        let words = parse_argument_words(source);
        let native = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&shuck_semantic::ZshOptionState::zsh_default()),
        );
        let null_glob = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&shuck_semantic::ZshOptionState {
                null_glob: shuck_semantic::OptionValue::On,
                ..shuck_semantic::ZshOptionState::zsh_default()
            }),
        );
        let no_glob = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&shuck_semantic::ZshOptionState {
                glob: shuck_semantic::OptionValue::Off,
                ..shuck_semantic::ZshOptionState::zsh_default()
            }),
        );

        assert_eq!(
            native.glob_failure_behavior,
            GlobFailureBehavior::ErrorOnNoMatch
        );
        assert_eq!(
            null_glob.glob_failure_behavior,
            GlobFailureBehavior::DropUnmatchedPattern
        );
        assert_eq!(
            no_glob.pathname_expansion_behavior,
            PathnameExpansionBehavior::Disabled
        );
        assert_eq!(
            no_glob.glob_failure_behavior,
            GlobFailureBehavior::KeepLiteralOnNoMatch
        );
    }

    #[test]
    fn analyze_literal_runtime_respects_no_glob_precedence_over_failure_modes() {
        let source = "print *.jpg\n";
        let words = parse_argument_words(source);
        let analysis = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&shuck_semantic::ZshOptionState {
                glob: shuck_semantic::OptionValue::Off,
                null_glob: shuck_semantic::OptionValue::On,
                csh_null_glob: shuck_semantic::OptionValue::On,
                nomatch: shuck_semantic::OptionValue::On,
                ..shuck_semantic::ZshOptionState::zsh_default()
            }),
        );

        assert_eq!(
            analysis.pathname_expansion_behavior,
            PathnameExpansionBehavior::Disabled
        );
        assert_eq!(
            analysis.glob_failure_behavior,
            GlobFailureBehavior::KeepLiteralOnNoMatch
        );
        assert!(!analysis.hazards.pathname_matching);
        assert!(!analysis.is_runtime_sensitive());
    }

    #[test]
    fn analyze_literal_runtime_distinguishes_nomatch_and_csh_null_glob() {
        let source = "print *.jpg\n";
        let words = parse_argument_words(source);
        let keep_literal = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&shuck_semantic::ZshOptionState {
                nomatch: shuck_semantic::OptionValue::Off,
                ..shuck_semantic::ZshOptionState::zsh_default()
            }),
        );
        let csh_null_glob = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&shuck_semantic::ZshOptionState {
                csh_null_glob: shuck_semantic::OptionValue::On,
                ..shuck_semantic::ZshOptionState::zsh_default()
            }),
        );

        assert_eq!(
            keep_literal.glob_failure_behavior,
            GlobFailureBehavior::KeepLiteralOnNoMatch
        );
        assert_eq!(
            csh_null_glob.glob_failure_behavior,
            GlobFailureBehavior::CshNullGlob
        );
        assert!(csh_null_glob.hazards.pathname_matching);
    }

    #[test]
    fn analyze_literal_runtime_records_glob_pattern_behaviors() {
        let source = "print *.jpg\n";
        let words = parse_argument_words(source);
        let analysis = analyze_literal_runtime(
            &words[0],
            source,
            ExpansionContext::CommandArgument,
            Some(&shuck_semantic::ZshOptionState {
                glob_dots: shuck_semantic::OptionValue::On,
                extended_glob: shuck_semantic::OptionValue::On,
                ksh_glob: shuck_semantic::OptionValue::Unknown,
                sh_glob: shuck_semantic::OptionValue::On,
                ..shuck_semantic::ZshOptionState::zsh_default()
            }),
        );

        assert_eq!(
            analysis.glob_dot_behavior,
            GlobDotBehavior::DotfilesIncluded
        );
        assert_eq!(
            analysis.glob_pattern_behavior.extended_glob(),
            PatternOperatorBehavior::Enabled
        );
        assert_eq!(
            analysis.glob_pattern_behavior.ksh_glob(),
            PatternOperatorBehavior::Ambiguous
        );
        assert_eq!(
            analysis.glob_pattern_behavior.sh_glob(),
            PatternOperatorBehavior::Enabled
        );
    }

    #[test]
    fn analyze_redirect_target_distinguishes_descriptor_dups_and_dev_null() {
        let static_dup_source = "echo hi 2>&3\n";
        let static_dup_file = Parser::new(static_dup_source).parse().unwrap().file;
        let Command::Simple(_) = &static_dup_file.body[0].command else {
            panic!("expected simple command");
        };
        let static_dup = analyze_redirect_target(
            &static_dup_file.body[0].redirects[0],
            static_dup_source,
            None,
        )
        .expect("expected redirect analysis");
        assert!(static_dup.is_descriptor_dup());
        assert_eq!(static_dup.numeric_descriptor_target, Some(3));
        assert!(!static_dup.is_runtime_sensitive());

        let file_source = "echo hi > /dev/null\n";
        let file_commands = Parser::new(file_source).parse().unwrap().file;
        let Command::Simple(_) = &file_commands.body[0].command else {
            panic!("expected simple command");
        };
        let file = analyze_redirect_target(&file_commands.body[0].redirects[0], file_source, None)
            .expect("expected redirect analysis");
        assert!(file.is_file_target());
        assert!(file.is_definitely_dev_null());
        assert!(!file.is_runtime_sensitive());

        let maybe_source = "echo hi > \"$target\"\n";
        let maybe_commands = Parser::new(maybe_source).parse().unwrap().file;
        let Command::Simple(_) = &maybe_commands.body[0].command else {
            panic!("expected simple command");
        };
        let maybe =
            analyze_redirect_target(&maybe_commands.body[0].redirects[0], maybe_source, None)
                .expect("expected redirect analysis");
        assert!(maybe.is_file_target());
        assert_eq!(
            maybe.dev_null_status,
            Some(RedirectDevNullStatus::MaybeDevNull)
        );
        assert!(maybe.is_runtime_sensitive());

        let fanout_source = "echo hi > ${targets[@]}\n";
        let fanout_commands = Parser::new(fanout_source).parse().unwrap().file;
        let Command::Simple(_) = &fanout_commands.body[0].command else {
            panic!("expected simple command");
        };
        let fanout =
            analyze_redirect_target(&fanout_commands.body[0].redirects[0], fanout_source, None)
                .expect("expected redirect analysis");
        assert!(fanout.can_expand_to_multiple_fields());
        assert!(fanout.is_runtime_sensitive());

        let tilde_source = "echo hi > ~/*.log\n";
        let tilde_commands = Parser::new(tilde_source).parse().unwrap().file;
        let Command::Simple(_) = &tilde_commands.body[0].command else {
            panic!("expected simple command");
        };
        let tilde =
            analyze_redirect_target(&tilde_commands.body[0].redirects[0], tilde_source, None)
                .expect("expected redirect analysis");
        assert!(tilde.is_file_target());
        assert_eq!(
            tilde.dev_null_status,
            Some(RedirectDevNullStatus::MaybeDevNull)
        );
        assert!(tilde.runtime_literal.is_runtime_sensitive());
        assert!(tilde.is_runtime_sensitive());
    }

    #[test]
    fn comparable_path_accepts_simple_literals_and_single_parameter_expansions() {
        let source = "cmd foo \"$src\" \"${dst}\" ~/.zshrc \"$dir/Cargo.toml\" $tmpf \"$@\" \"$(printf hi)\" <(cat) *.log /dev/null /dev/tty /dev/stdin /dev/fd/0 /proc/self/fd/1\n";
        let words = parse_argument_words(source);

        assert_eq!(
            comparable_path(&words[0], source, ExpansionContext::CommandArgument, None)
                .expect("expected literal path")
                .key(),
            &ComparablePathKey::Literal("foo".into())
        );
        assert_eq!(
            comparable_path(&words[1], source, ExpansionContext::CommandArgument, None)
                .expect("expected parameter path")
                .key(),
            &ComparablePathKey::Parameter("src".into())
        );
        assert_eq!(
            comparable_path(&words[2], source, ExpansionContext::CommandArgument, None)
                .expect("expected parameter path")
                .key(),
            &ComparablePathKey::Parameter("dst".into())
        );
        assert_eq!(
            comparable_path(&words[3], source, ExpansionContext::CommandArgument, None)
                .expect("expected tilde literal")
                .key(),
            &ComparablePathKey::Literal("~/.zshrc".into())
        );
        assert_eq!(
            comparable_path(&words[4], source, ExpansionContext::CommandArgument, None)
                .expect("expected path template")
                .key(),
            &ComparablePathKey::Template(
                [
                    ComparablePathPart::Parameter("dir".into()),
                    ComparablePathPart::Literal("/Cargo.toml".into()),
                ]
                .into()
            )
        );
        assert_eq!(
            comparable_path(&words[5], source, ExpansionContext::CommandArgument, None)
                .expect("expected bare parameter path")
                .key(),
            &ComparablePathKey::Parameter("tmpf".into())
        );
        assert!(
            comparable_path(&words[6], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[7], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[8], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[9], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[10], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[11], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[12], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[13], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[14], source, ExpansionContext::CommandArgument, None).is_none()
        );
    }

    #[test]
    fn analyze_literal_runtime_tracks_context_sensitive_literals() {
        let source = "printf ~ ~user x=~ *.sh {a,b} \"~\" '*.sh' \"{a,b}\"\n";
        let words = parse_argument_words(source);

        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::CommandArgument, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[1], source, ExpansionContext::CommandArgument, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[2], source, ExpansionContext::CommandArgument, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[3], source, ExpansionContext::CommandArgument, None)
                .hazards
                .pathname_matching
        );
        assert!(
            analyze_literal_runtime(&words[4], source, ExpansionContext::CommandArgument, None)
                .hazards
                .brace_fanout
        );

        assert!(
            !analyze_literal_runtime(&words[5], source, ExpansionContext::CommandArgument, None)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[6], source, ExpansionContext::CommandArgument, None)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[7], source, ExpansionContext::CommandArgument, None)
                .is_runtime_sensitive()
        );

        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::StringTestOperand, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::RegexOperand, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            !analyze_literal_runtime(&words[3], source, ExpansionContext::StringTestOperand, None)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[4], source, ExpansionContext::CasePattern, None)
                .is_runtime_sensitive()
        );
    }

    #[test]
    fn analyze_literal_runtime_treats_loop_lists_like_argument_lists() {
        let source = "printf ~ *.sh {a,b}\n";
        let words = parse_argument_words(source);

        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::ForList, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[1], source, ExpansionContext::ForList, None)
                .hazards
                .pathname_matching
        );
        assert!(
            analyze_literal_runtime(&words[2], source, ExpansionContext::ForList, None)
                .hazards
                .brace_fanout
        );
    }
}
