use crate::{Checker, CommandSubstitutionKind, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SubstWithRedirectErr;

impl Violation for SubstWithRedirectErr {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::SubstWithRedirectErr
    }

    fn message(&self) -> String {
        "command substitution redirects its output inside the subshell".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the stdout-to-`/dev/null` redirects".to_owned())
    }
}

pub fn subst_with_redirect_err(checker: &mut Checker) {
    let substitutions = checker
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
                .filter(|substitution| !substitution.stdout_redirect_spans().is_empty())
                .filter(|substitution| !substitution.body_is_negated())
                .cloned()
        })
        .collect::<Vec<_>>();

    for substitution in substitutions {
        let diagnostic = crate::Diagnostic::new(SubstWithRedirectErr, substitution.span());
        let edits = substitution
            .stdout_dev_null_redirect_spans()
            .iter()
            .copied()
            .map(Edit::deletion)
            .collect::<Vec<_>>();
        if edits.is_empty() {
            checker.report_diagnostic_dedup(diagnostic);
        } else {
            checker.report_diagnostic_dedup(diagnostic.with_fix(Fix::unsafe_edits(edits)));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn only_reports_substitutions_that_redirect_stdout_away() {
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
out=$(printf hi &>/dev/null)
out=$(printf hi > \"$target\")
out=$(printf hi > ${targets[@]})
out=$(printf hi 2>&\"$fd\")
declare arr[$(printf hi >/dev/null)]=1
declare -A map=([$(printf bye 1>/dev/null)]=1)
if $(command -v python3 &>/dev/null); then
  :
fi
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
            vec![8, 9, 11, 12, 14, 15]
        );
    }

    #[test]
    fn ignores_shellcheck_quiet_output_both_and_negated_substitutions() {
        let source = "\
#!/bin/bash
out=$(printf hi &>/dev/null)
probe=$(! printf hi >/dev/null 2>&1)
if $(command -v python3 &>/dev/null); then
  :
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirectErr),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\nout=$(printf hi >/dev/null 2>&1)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirectErr),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("remove the stdout-to-`/dev/null` redirects")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_discarded_substitutions() {
        let source = "\
#!/bin/sh
out=$(printf hi >/dev/null 2>&1)
other=$(printf hi 1>/dev/null)
keep=$(printf hi > out.txt)
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirectErr),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
out=$(printf hi  2>&1)
other=$(printf hi )
keep=$(printf hi > out.txt)
"
        );
        assert_eq!(result.fixed_diagnostics.len(), 1);
        assert_eq!(
            result.fixed_diagnostics[0].span.slice(&result.fixed_source),
            "$(printf hi > out.txt)"
        );
    }

    #[test]
    fn ignores_indirect_dev_null_redirects_that_shellcheck_keeps_quiet() {
        let source = "\
#!/bin/sh
out=$(printf hi 3>/dev/null 1>&3)
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirectErr),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn keeps_nested_substitution_redirects_scoped_to_their_own_diagnostics() {
        let source = "\
#!/bin/sh
out=$(printf '%s' \"$(printf hi >/dev/null)\" >/dev/null)
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SubstWithRedirectErr),
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
            Path::new("correctness").join("C058.sh").as_path(),
            &LinterSettings::for_rule(Rule::SubstWithRedirectErr),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C058_fix_C058.sh", result);
        Ok(())
    }
}
