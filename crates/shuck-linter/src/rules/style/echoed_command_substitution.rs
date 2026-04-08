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
        .flat_map(|fact| {
            fact.substitution_facts()
                .iter()
                .filter(|substitution| {
                    substitution.kind() == CommandSubstitutionKind::Command
                        && matches!(
                            substitution.host_kind(),
                            SubstitutionHostKind::CommandArgument
                        )
                })
                .filter_map(|substitution| {
                    checker
                        .facts()
                        .word_fact(
                            substitution.host_word_span(),
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        )
                        .filter(|fact| fact.classification().has_plain_command_substitution())
                        .map(|_| substitution.host_word_span())
                })
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
    fn reports_plain_substitutions_in_any_echo_argument() {
        let source = "echo prefix $(date)\necho \"$(date)\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(date)", "\"$(date)\""]
        );
    }
}
