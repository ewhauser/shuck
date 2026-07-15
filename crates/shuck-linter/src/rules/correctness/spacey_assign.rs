use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct SpaceyAssign;

impl Violation for SpaceyAssign {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SpaceyAssign
    }

    fn message(&self) -> String {
        "assignment spacing makes this run as a command".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove spaces around `=`".to_owned())
    }
}

pub fn spacey_assign(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    checker.report_fact_diagnostics_dedup(|facts, report| {
        for fact in facts.command_facts().spacey_assignment_facts() {
            report(
                Diagnostic::new(SpaceyAssign, fact.diagnostic_span()).with_fix(Fix::unsafe_edit(
                    Edit::replacement(fact.replacement(), fact.diagnostic_span()),
                )),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_variable_like_commands_followed_by_equals_words() {
        let source = "\
#!/bin/sh
name = demo
empty =
joined =demo
read = value
test = foo
time = 1
FOO=1 name = value
time timed = value
time FOO=1 timed = value
f() { inside = \"$value\"; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceyAssign));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "name = demo",
                "empty =",
                "joined =demo",
                "read = value",
                "test = foo",
                "time = 1",
                "name = value",
                "timed = value",
                "timed = value",
                "inside = \"$value\""
            ]
        );
    }

    #[test]
    fn ignores_assignments_declarations_comparisons_and_quoted_equals() {
        let source = "\
#!/bin/sh
name=value
name= value
export name = demo
name == demo
name \"=\" demo
name \\= demo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceyAssign));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn skips_zsh() {
        let source = "\
#!/bin/zsh
name = demo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceyAssign));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_join_assignment_words() {
        let source = "#!/bin/sh\nname = demo\nempty =\njoined =demo\ninside = \"$value\"\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SpaceyAssign),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nname=demo\nempty=\njoined=demo\ninside=\"$value\"\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_spacey_assignments_unchanged_when_fixing() {
        let source = "#!/bin/sh\nname=value\nname= value\nname == demo\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SpaceyAssign),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C053.sh").as_path(),
            &LinterSettings::for_rule(Rule::SpaceyAssign),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C053_fix_C053.sh", result);
        Ok(())
    }
}
