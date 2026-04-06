use crate::rules::common::query::{
    self, CommandSubstitutionKind, CommandWalkOptions, visit_command_words,
};
use crate::rules::common::word::{StdoutDisposition, classify_substitution};
use crate::{Checker, Rule, Violation};

pub struct SubstWithRedirectErr;

impl Violation for SubstWithRedirectErr {
    fn rule() -> Rule {
        Rule::SubstWithRedirectErr
    }

    fn message(&self) -> String {
        "command substitution redirects its output inside the subshell".to_owned()
    }
}

pub fn subst_with_redirect_err(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            visit_command_words(command, &mut |word| {
                for substitution in query::iter_word_command_substitutions(word) {
                    if substitution.kind != CommandSubstitutionKind::Command {
                        continue;
                    }

                    let classification = classify_substitution(substitution, source);
                    if classification.stdout_disposition == StdoutDisposition::RedirectedToDevNull {
                        spans.push(classification.span);
                    }
                }
            });
        },
    );

    for span in spans {
        checker.report(SubstWithRedirectErr, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn only_reports_substitutions_that_drop_output_to_dev_null() {
        let source = "out=$(whiptail 3>&1 1>&2 2>&3)\nout=$(printf hi >/dev/null 2>&1)\nout=$(printf hi 1>/dev/null)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirectErr),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }
}
