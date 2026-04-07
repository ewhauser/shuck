use crate::rules::common::{
    command,
    query::{self, CommandWalkOptions},
    span,
    word::classify_word,
};
use crate::{Checker, Rule, Violation};
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
    let source = checker.source();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let normalized = command::normalize_command(command, source);
            if !normalized.effective_name_is("echo") {
                return;
            }

            let [word] = normalized.body_args() else {
                return;
            };

            if classify_word(word, source).has_plain_command_substitution() {
                for span in span::command_substitution_part_spans(word) {
                    checker.report_dedup(EchoedCommandSubstitution, span);
                }
            }
        },
    );
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
