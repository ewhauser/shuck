use crate::{Checker, Rule, Violation};

use super::loop_control_outside_loop::loop_control_violations;

pub struct ContinueOutsideLoopInFunction;

impl Violation for ContinueOutsideLoopInFunction {
    fn rule() -> Rule {
        Rule::ContinueOutsideLoopInFunction
    }

    fn message(&self) -> String {
        "`continue` inside a function must be inside a loop".into()
    }
}

pub fn continue_outside_loop_in_function(checker: &mut Checker) {
    for (_, span, _) in loop_control_violations(checker, true, true) {
        checker.report(ContinueOutsideLoopInFunction, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
