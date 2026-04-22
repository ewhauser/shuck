use crate::{
    Checker, CommandSubstitutionKind, Edit, Fix, FixAvailability, Rule, SubstitutionOutputIntent,
    Violation,
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
        Some("remove the stdout redirects from the substitution".to_owned())
    }
}

pub fn subst_with_redirect(checker: &mut Checker) {
    let substitutions = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.substitution_facts()
                .iter()
                .filter(|substitution| substitution.kind() == CommandSubstitutionKind::Command)
                .filter(|substitution| {
                    substitution.stdout_intent() == SubstitutionOutputIntent::Rerouted
                        && !substitution.stdout_redirect_spans().is_empty()
                })
                .cloned()
        })
        .collect::<Vec<_>>();

    let diagnostics = substitutions
        .into_iter()
        .map(|substitution| {
            let edits = substitution
                .stdout_redirect_spans()
                .iter()
                .copied()
                .map(Edit::deletion)
                .collect::<Vec<_>>();
            crate::Diagnostic::new(SubstWithRedirect, substitution.span())
                .with_fix(Fix::unsafe_edits(edits))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\nout=$(printf hi > out.txt)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstWithRedirect));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("remove the stdout redirects from the substitution")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_rerouted_substitutions() {
        let source = "\
#!/bin/sh
out=$(printf loud > out.txt)
err=$(printf hi >&2)
keep=$(printf hi 2>&1)
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
out=$(printf loud )
err=$(printf hi )
keep=$(printf hi 2>&1)
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn keeps_nested_substitution_redirects_scoped_to_their_own_diagnostics() {
        let source = "\
#!/bin/sh
out=$(printf '%s' \"$(printf hi > nested.txt)\" > outer.txt)
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
out=$(printf '%s' \"$(printf hi )\" )
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C057.sh").as_path(),
            &LinterSettings::for_rule(Rule::SubstWithRedirect),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C057_fix_C057.sh", result);
        Ok(())
    }
}
