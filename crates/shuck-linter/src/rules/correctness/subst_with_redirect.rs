use crate::rules::common::expansion::classify_substitution;
use crate::rules::common::query::{self, CommandSubstitutionKind, visit_command_words};
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
    for fact in checker.facts().commands() {
        visit_command_words(fact.visit(), source, &mut |word| {
            for substitution in query::iter_word_command_substitutions(word) {
                if substitution.kind != CommandSubstitutionKind::Command {
                    continue;
                }

                let classification = classify_substitution(substitution, source);
                if classification.stdout_is_rerouted() {
                    spans.push(classification.span);
                }
            }
        });
    }

    for span in spans {
        checker.report(SubstWithRedirect, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_only_rerouted_substitutions() {
        let source = "\
opts=$(getopt -o a -- \"$@\" || { usage >&2 && false; })
menu=$(whiptail --menu pick 0 0 0 foo bar 3>&1 1>&2 2>&3)
dialog_out=$(dialog --menu pick 0 0 0 foo bar 3>&1 1>&2 2>&3)
json=$(jq -r . <<< \"$status\" || die >&2)
awk_output=$(awk 'BEGIN { print \"ok\" }' || warn >&2)
choice=$(\"${cmd[@]}\" \"${options[@]}\" 2>&1 >/dev/tty)
out=$(printf quiet >/dev/null; printf loud > out.txt)
out=$(printf hi > out.txt)
out=$(printf hi >&2)
out=$(printf hi > \"$target\")
out=$(printf hi > ${targets[@]})
out=$(printf hi 2>&\"$fd\")
declare arr[$(printf hi > out.txt)]=1
declare -A map=([$(printf bye > \"$target\")]=1)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstWithRedirect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![8, 9, 10, 11, 13, 14]
        );
    }
}
