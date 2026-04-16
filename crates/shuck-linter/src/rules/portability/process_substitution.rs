use crate::{Checker, CommandSubstitutionKind, Rule, ShellDialect, Violation};

pub struct ProcessSubstitution;

impl Violation for ProcessSubstitution {
    fn rule() -> Rule {
        Rule::ProcessSubstitution
    }

    fn message(&self) -> String {
        "process substitution is not portable in `sh`".to_owned()
    }
}

pub fn process_substitution(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.substitution_facts()
                .iter()
                .filter(|substitution| {
                    matches!(
                        substitution.kind(),
                        CommandSubstitutionKind::ProcessInput
                            | CommandSubstitutionKind::ProcessOutput
                    )
                })
                .map(|substitution| substitution.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ProcessSubstitution);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_process_substitution_spans() {
        let source = "\
#!/bin/sh
cat <(printf '%s\n' hi) > >(wc -c)
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ProcessSubstitution));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["<(printf '%s\n' hi)", ">(wc -c)"]
        );
    }

    #[test]
    fn ignores_process_substitution_in_bash() {
        let source = "cat <(printf hi)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ProcessSubstitution).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_regex_text_that_only_looks_like_process_substitution() {
        let source = "\
#!/bin/sh
value=$(printf '%s\n' \"<record_id>([^<]*)</record_id>\")
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ProcessSubstitution));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_regex_text_in_nested_pipeline_command_substitution() {
        let source = "\
#!/bin/sh
_record_id=$(echo \"$response\" | _egrep_o \"<record_id>([^<]*)</record_id><type>TXT</type><host>$fulldomain</host>\" | _egrep_o \"<record_id>([^<]*)</record_id>\" | sed -r \"s/<record_id>([^<]*)<\\/record_id>/\\1/\" | tail -n 1)
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ProcessSubstitution));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
