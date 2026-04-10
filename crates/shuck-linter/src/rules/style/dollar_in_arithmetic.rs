use crate::{Checker, Rule, Violation};

pub struct DollarInArithmetic;

impl Violation for DollarInArithmetic {
    fn rule() -> Rule {
        Rule::DollarInArithmetic
    }

    fn message(&self) -> String {
        "omit the `$` prefix inside arithmetic expansion".to_owned()
    }
}

pub fn dollar_in_arithmetic(checker: &mut Checker) {
    let spans = checker.facts().dollar_in_arithmetic_spans().to_vec();

    checker.report_all_dedup(spans, || DollarInArithmetic);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_dollar_prefixed_arithmetic_variables_in_assignments() {
        let source = "#!/bin/bash\nn=1\nm=$(($n + 1))\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$n");
    }

    #[test]
    fn reports_dollar_prefixed_arithmetic_variables_in_command_arguments() {
        let source = "#!/bin/bash\nn=1\nprintf '%s\\n' \"$(($n + 1))\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$n");
    }

    #[test]
    fn reports_braced_arithmetic_variables_in_command_arguments() {
        let source = "#!/bin/bash\nx=1\nprintf '%s\\n' \"$((${x} + 1))\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "${x}");
    }

    #[test]
    fn reports_dollar_prefixed_variables_in_substring_offset_arithmetic() {
        let source =
            "#!/bin/bash\nrest=abcdef\nlen=2\nprintf '%s\\n' \"${rest:$((${#rest}-$len))}\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$len");
    }

    #[test]
    fn ignores_assignments_without_arithmetic_dollar_variables() {
        let source = "#!/bin/bash\nm=$((1 + 1))\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_positional_parameters_in_arithmetic_expressions() {
        let source = "#!/bin/bash\necho \"$(( $1 / 2 ))\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_array_selector_parameter_accesses_in_arithmetic_expressions() {
        let source = "#!/bin/bash\nver=(1 2)\necho \"$(( ${ver[0]} + 1 ))\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
