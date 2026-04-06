use crate::rules::common::query::{
    self, CommandSubstitutionKind, CommandWalkOptions, visit_command_words,
};
use crate::rules::common::word::{StdoutDisposition, classify_substitution};
use crate::{Checker, Rule, Violation};

pub struct SubstWithRedirect;

impl Violation for SubstWithRedirect {
    fn rule() -> Rule {
        Rule::SubstWithRedirect
    }

    fn message(&self) -> String {
        "command substitution redirects its output away".to_owned()
    }
}

pub fn subst_with_redirect(checker: &mut Checker) {
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
                    if classification.stdout_disposition == StdoutDisposition::RedirectedElsewhere {
                        spans.push(classification.span);
                    }
                }
            });
        },
    );

    for span in spans {
        checker.report(SubstWithRedirect, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_fd_swaps_and_sidecar_stderr_logging() {
        let source = "out=$(whiptail 3>&1 1>&2 2>&3)\nout=$(jq -r . <<< \"$status\" || die >&2)\nout=$(printf hi > out.txt)\nout=$(printf hi >&2)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstWithRedirect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
    }
}
