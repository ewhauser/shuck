use crate::{
    Checker, CommandSubstitutionKind, ExpansionContext, Rule, SubstitutionHostKind, Violation,
    WordFactContext,
};

pub struct EchoedCommandSubstitution;

impl Violation for EchoedCommandSubstitution {
    fn rule() -> Rule {
        Rule::EchoedCommandSubstitution
    }

    fn message(&self) -> String {
        "call the command directly instead of echoing its substitution".to_owned()
    }
}

pub fn echoed_command_substitution(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("echo"))
        .filter_map(|fact| {
            let [word] = fact.body_args() else {
                return None;
            };

            checker
                .facts()
                .word_fact(
                    word.span,
                    WordFactContext::Expansion(ExpansionContext::CommandArgument),
                )
                .filter(|fact| fact.classification().has_plain_command_substitution())
                .filter(|_| {
                    !fact.substitution_facts().iter().any(|substitution| {
                        substitution.kind() == CommandSubstitutionKind::Command
                            && matches!(
                                substitution.host_kind(),
                                SubstitutionHostKind::CommandArgument
                            )
                            && substitution.host_word_span() == word.span
                            && (substitution.is_bash_file_slurp()
                                || substitution.body_has_multiple_statements()
                                || (substitution.uses_backtick_syntax()
                                    && !substitution.unquoted_in_host()))
                    })
                })
                .map(|_| word.span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EchoedCommandSubstitution);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn only_reports_plain_command_substitutions() {
        let source = "echo \"$(date)\"\necho \"date: $(date)\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(diagnostics[0].span.slice(source), "\"$(date)\"");
    }

    #[test]
    fn ignores_echoes_with_extra_arguments() {
        let source = "echo prefix $(date)\necho \"$(date)\" suffix\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_echo_flag_forms_and_bash_file_slurps() {
        let source = "echo -n \"$(date)\"\necho -e \"$(date)\"\necho \"$(< file.txt)\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_single_argument_echoes_inside_binary_command_lists() {
        let source = "value=\"$(true && echo \"$(date)\")\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"$(date)\""]
        );
    }

    #[test]
    fn ignores_quoted_backticks_and_multi_statement_substitutions() {
        let source = r#"SCRIPT=$(echo "`basename "$0"`")
value=$(echo $(printf '%s\n' one; printf '%s\n' two) | tr -d ' ')
"#;
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_echoes_inside_prefixed_nested_command_substitutions() {
        let source = "cp -v $filename $OUT/$(echo $(basename $filename .fuzz))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(basename $filename .fuzz)"]
        );
    }
}
