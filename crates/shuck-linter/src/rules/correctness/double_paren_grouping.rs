use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct DoubleParenGrouping;

impl Violation for DoubleParenGrouping {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::DoubleParenGrouping
    }

    fn message(&self) -> String {
        "double parentheses are used to group commands instead of arithmetic".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert a space after the first `(`".to_owned())
    }
}

pub fn double_paren_grouping(checker: &mut Checker) {
    checker.report_fact_diagnostics_dedup(|facts, report| {
        for span in facts.double_paren_grouping_spans().iter().copied() {
            report(
                Diagnostic::new(DoubleParenGrouping, span).with_fix(Fix::unsafe_edit(
                    Edit::insertion(span.start.offset + 1, " "),
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
    fn reports_command_style_double_paren_grouping() {
        let source = "\
#!/bin/sh
((ps aux | grep foo) || kill \"$pid\") 2>/dev/null
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::DoubleParenGrouping));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn ignores_normal_arithmetic_commands() {
        let source = "\
#!/bin/sh
(( i += 1 ))
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::DoubleParenGrouping));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_grouped_bash_arithmetic_expressions() {
        let source = "\
#!/bin/bash
if ((threads>(cpu_height-3)*3 && tty_width>=200)); then :; fi
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::DoubleParenGrouping));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_double_paren_grouping() {
        let source = "\
#!/bin/sh
((ps aux | grep foo) || kill \"$pid\") 2>/dev/null
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DoubleParenGrouping),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
( (ps aux | grep foo) || kill \"$pid\") 2>/dev/null
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_normal_arithmetic_commands_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
(( i += 1 ))
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DoubleParenGrouping),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C071.sh").as_path(),
            &LinterSettings::for_rule(Rule::DoubleParenGrouping),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C071_fix_C071.sh", result);
        Ok(())
    }
}
