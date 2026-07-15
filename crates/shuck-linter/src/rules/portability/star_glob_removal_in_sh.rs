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
        for fragment in facts.words().positional_parameter_trim_fragments() {
            let diagnostic = Diagnostic::new(StarGlobRemovalInSh, fragment.span());
            let diagnostic = match fragment.fix() {
                Some(fix) => {
                    let edits =
                        std::iter::once(Edit::insertion(fix.insertion_offset(), fix.insertion()))
                            .chain(fix.replacements().iter().map(|replacement| {
                                Edit::replacement(replacement.replacement(), replacement.span())
                            }));
                    diagnostic.with_fix(Fix::unsafe_edits(edits))
                }
                None => diagnostic,
            };
            report(diagnostic);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

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

    #[test]
    fn applies_one_unsafe_fix_for_multiple_trims_in_one_command() {
        let source = "#!/bin/sh\nprintf '%s\\n' \"${*%%a*}\" \"${*%%b*}\"\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StarGlobRemovalInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\n_shuck_positional_params=$*\nprintf '%s\\n' \"${_shuck_positional_params%%a*}\" \"${_shuck_positional_params%%b*}\"\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_mixed_star_and_at_trims_unfixed() {
        let source = "#!/bin/sh\nprintf '%s\\n' \"${*%%a*}\" \"${@%%b*}\"\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::StarGlobRemovalInSh));

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
    }

    #[test]
    fn leaves_control_operator_continuations_unfixed() {
        let source = "#!/bin/sh\nprintf '%s\\n' ok |\\\n  printf '%s\\n' \"${*%%dBm*}\"\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StarGlobRemovalInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("portability").join("X081.sh").as_path(),
            &LinterSettings::for_rule(Rule::StarGlobRemovalInSh),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("X081_fix_X081.sh", result);
        Ok(())
    }
}
