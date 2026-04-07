use crate::rules::common::expansion::classify_substitution;
use crate::rules::common::query::{
    self, CommandSubstitutionKind, CommandWalkOptions, visit_command_words,
};
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
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let command = visit.command;
            visit_command_words(visit, &mut |word| {
                for substitution in query::iter_word_command_substitutions(word) {
                    if substitution.kind != CommandSubstitutionKind::Command {
                        continue;
                    }

                    let classification = classify_substitution(substitution, source);
                    if classification.stdout_is_discarded() {
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
        let source = "\
opts=$(getopt -o a -- \"$@\" || { usage >&2 && false; })
menu=$(whiptail --menu pick 0 0 0 foo bar 3>&1 1>&2 2>&3)
dialog_out=$(dialog --menu pick 0 0 0 foo bar 3>&1 1>&2 2>&3)
json=$(jq -r . <<< \"$status\" || die >&2)
awk_output=$(awk 'BEGIN { print \"ok\" }' || warn >&2)
choice=$(\"${cmd[@]}\" \"${options[@]}\" 2>&1 >/dev/tty)
out=$(printf quiet >/dev/null; printf loud)
out=$(printf hi >/dev/null 2>&1)
out=$(printf hi 1>/dev/null)
out=$(printf hi > \"$target\")
out=$(printf hi > ${targets[@]})
out=$(printf hi 2>&\"$fd\")
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirectErr),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![8, 9]
        );
    }
}
