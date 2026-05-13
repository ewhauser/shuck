use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct StarGlobRemovalInSh;

impl Violation for StarGlobRemovalInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::StarGlobRemovalInSh
    }

    fn message(&self) -> String {
        "pattern trimming on `$*` or `$@` is not portable in `sh`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("trim a temporary positional-parameter value".to_owned())
    }
}

pub fn star_glob_removal_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    checker.report_fact_diagnostics_dedup(|facts, report| {
        let fix_facts = facts.positional_parameter_trim_fix_facts();
        for fragment in facts.positional_parameter_trim_fragments() {
            let span = fragment.span();
            let diagnostic = Diagnostic::new(StarGlobRemovalInSh, span);
            let diagnostic = match fix_facts.iter().find(|fact| fact.diagnostic_span() == span) {
                Some(fact) => diagnostic.with_fix(Fix::unsafe_edits([
                    Edit::insertion(fact.insertion_offset(), fact.insertion()),
                    Edit::replacement(fact.replacement(), fact.replacement_span()),
                ])),
                None => diagnostic,
            };
            report(diagnostic);
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_positional_parameter_trims_for_star_and_at() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${*%%dBm*}\" \"${*%dBm*}\" \"${*##dBm*}\" \"${*#dBm*}\"
printf '%s\n' \"${@%%dBm*}\" \"${@%dBm*}\" \"${@##dBm*}\" \"${@#dBm*}\"
printf '%s\n' \"${name%%dBm*}\"
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::StarGlobRemovalInSh));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${*%%dBm*}",
                "${*%dBm*}",
                "${*##dBm*}",
                "${*#dBm*}",
                "${@%%dBm*}",
                "${@%dBm*}",
                "${@##dBm*}",
                "${@#dBm*}",
            ]
        );
    }

    #[test]
    fn ignores_star_glob_removal_in_bash() {
        let source = "printf '%s\\n' \"${*%%dBm*}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StarGlobRemovalInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_trim_temporary_positional_parameter_value() {
        let source = "#!/bin/sh\nprintf '%s\\n' \"${*%%dBm*}\"\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StarGlobRemovalInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\n_shuck_positional_params=$*\nprintf '%s\\n' \"${_shuck_positional_params%%dBm*}\"\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
