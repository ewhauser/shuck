use crate::rules::common::expansion::ExpansionContext;
use crate::{Checker, Rule, Violation, WordFactContext};
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
        })
        .flat_map(|fact| fact.command_substitution_spans().iter().copied())
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
        assert_eq!(diagnostics[0].span.slice(source), "$(date)");
    }
}
