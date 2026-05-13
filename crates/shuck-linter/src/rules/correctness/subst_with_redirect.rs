use crate::{
    Checker, CommandSubstitutionKind, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation,
};

pub struct SubstWithRedirect;

impl Violation for SubstWithRedirect {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SubstWithRedirect
    }

    fn message(&self) -> String {
        "command substitution redirects its output away".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the stdout redirect from the substitution".to_owned())
    }
}

pub fn subst_with_redirect(checker: &mut Checker) {
    let redirect_spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.substitution_facts()
                .iter()
                .filter(|substitution| substitution.kind() == CommandSubstitutionKind::Command)
                .filter(|substitution| {
                    substitution.stdout_is_discarded() || substitution.stdout_is_rerouted()
                })
                .filter(|substitution| !substitution.body_is_negated())
                .filter_map(|substitution| substitution.stdout_redirect_spans().first().copied())
        })
        .collect::<Vec<_>>();

    for span in redirect_spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(SubstWithRedirect, span)
                .with_fix(Fix::unsafe_edit(Edit::deletion(span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics};

    #[test]
    fn reports_first_redirect_for_rerouted_substitutions() {
        let source = "\
out=$(printf quiet >/dev/null; printf loud > out.txt)
out=$(printf hi > out.txt)
out=$(printf hi >&2)
out=$(printf hi > \"$target\")
out=$(printf hi > ${targets[@]})
out=$(printf hi >a >b)
out=$({ printf hi; } >/dev/tty)
declare arr[$(printf hi > out.txt)]=1
declare -A map=([$(printf bye > \"$target\")]=1)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstWithRedirect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "> out.txt",
                ">&2",
                "> \"$target\"",
                "> ${targets[@]}",
                ">a",
                ">/dev/tty",
                "> out.txt",
                "> \"$target\"",
            ]
        );
    }

    #[test]
    fn ignores_oracle_quiet_redirect_shapes() {
        let source = "\
opts=$(getopt -o a -- \"$@\" || { usage >&2 && false; })
menu=$(whiptail --menu pick 0 0 0 foo bar 3>&1 1>&2 2>&3)
dialog_out=$(dialog --menu pick 0 0 0 foo bar 3>&1 1>&2 2>&3)
json=$(jq -r . <<< \"$status\" || die >&2)
awk_output=$(awk 'BEGIN { print \"ok\" }' || warn >&2)
choice=$(\"${cmd[@]}\" \"${options[@]}\" 2>&1 >/dev/tty)
probe=$(! printf hi >/dev/null 2>&1)
out=$(printf hi &>/dev/null)
out=$(printf hi 2>&\"$fd\")
out=$(printf hi 3>/dev/null 1>&3)
out=$(printf quiet >/dev/null; printf loud)
out=$(printf quiet; printf loud >/dev/null)
out=$({ printf hi >out.txt; })
out=$({ printf hi >&5; })
out=$(printf hi 1>&2)
out=$(printf hi >&\"2\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstWithRedirect));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn keeps_fixture_quiet() -> anyhow::Result<()> {
        let (diagnostics, source) = test_path(
            Path::new("correctness").join("C057.sh").as_path(),
            &LinterSettings::for_rule(Rule::SubstWithRedirect),
        )?;

        assert_diagnostics!("C057_C057.sh", diagnostics, &source);
        Ok(())
    }

    #[test]
    fn reports_direct_redirect_examples() {
        let source = "#!/bin/sh\nout=$(printf hi > out.txt)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstWithRedirect));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "> out.txt");
    }

    #[test]
    fn applies_unsafe_fix_by_deleting_substitution_stdout_redirect() {
        let source = "#!/bin/sh\nout=$(printf hi > out.txt)\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\nout=$(printf hi )\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}
