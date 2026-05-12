use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

use super::loop_control_outside_loop::loop_control_violations;

pub struct ContinueOutsideLoopInFunction;

impl Violation for ContinueOutsideLoopInFunction {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::ContinueOutsideLoopInFunction
    }

    fn message(&self) -> String {
        "`continue` inside a function must be inside a loop".into()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace `continue` with `return`".to_owned())
    }
}

pub fn continue_outside_loop_in_function(checker: &mut Checker) {
    for (_, span, _) in loop_control_violations(checker, true, true) {
        checker.report_diagnostic_dedup(
            Diagnostic::new(ContinueOutsideLoopInFunction, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement("return", span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_continue_inside_a_function_outside_a_loop() {
        let source = "\
#!/bin/sh
termux_step_make() {
\tcontinue 2
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ContinueOutsideLoopInFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "continue");
    }

    #[test]
    fn ignores_continue_inside_a_loop() {
        let source = "\
#!/bin/sh
f() {
\twhile true; do
\t\tcontinue
\tdone
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ContinueOutsideLoopInFunction),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_continue_inside_function_loop_brace_group() {
        let source = "\
#!/bin/bash
f() {
  for gpu in \"${gpus[@]}\"; do
    [[ \"$gpu\" == Intel ]] && { unset -v gpu; continue; }
  done
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ContinueOutsideLoopInFunction),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_continue_inside_function_subshells() {
        let source = "\
#!/bin/sh
f() {
\t(
\t\tcontinue
\t)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ContinueOutsideLoopInFunction),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_top_level_continue() {
        let source = "\
#!/bin/sh
continue
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ContinueOutsideLoopInFunction),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn only_reports_the_function_specific_rule_when_both_are_enabled() {
        let source = "\
#!/bin/sh
f() {
\tcontinue
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([
                Rule::LoopControlOutsideLoop,
                Rule::ContinueOutsideLoopInFunction,
            ]),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ContinueOutsideLoopInFunction);
        assert_eq!(diagnostics[0].span.slice(source), "continue");
    }

    #[test]
    fn applies_unsafe_fix_to_continue_inside_function_outside_loop() {
        let source = "\
#!/bin/sh
f() {
\tcontinue 2
}
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ContinueOutsideLoopInFunction),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
f() {
\treturn 2
}
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_continue_unchanged() {
        let source = "#!/bin/sh\nf() {\n\tcontinue\n}\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ContinueOutsideLoopInFunction),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C126.sh").as_path(),
            &LinterSettings::for_rule(Rule::ContinueOutsideLoopInFunction),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C126_fix_C126.sh", result);
        Ok(())
    }
}
