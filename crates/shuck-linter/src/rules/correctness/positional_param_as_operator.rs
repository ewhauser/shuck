use crate::{Checker, Rule, Violation};

pub struct PositionalParamAsOperator;

impl Violation for PositionalParamAsOperator {
    fn rule() -> Rule {
        Rule::PositionalParamAsOperator
    }

    fn message(&self) -> String {
        "positional parameter is used where an arithmetic operator is expected".to_owned()
    }
}

pub fn positional_param_as_operator(checker: &mut Checker) {
    checker.report_fact_slice_dedup(
        |facts| facts.positional_parameter_operator_spans(),
        || PositionalParamAsOperator,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_positional_parameters_in_operator_slots() {
        let source = "\
#!/bin/sh
echo \"$(( x $1 y ))\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalParamAsOperator),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 7);
        assert_eq!(diagnostics[0].span.end, diagnostics[0].span.start);
    }

    #[test]
    fn ignores_positional_parameters_used_as_operands() {
        let source = "\
#!/bin/sh
echo \"$(( $1 + y ))\"
echo \"$(( x + $1 ))\"
echo \"$(( x ? $1 : y ))\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalParamAsOperator),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_identifier_prefixed_positional_parameters_in_arithmetic_words() {
        let source = "\
#!/bin/sh
echo \"$(( value + prefix$1 ))\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalParamAsOperator),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 7);
        assert_eq!(diagnostics[0].span.end, diagnostics[0].span.start);
    }

    #[test]
    fn ignores_suffix_only_and_expansion_led_arithmetic_words() {
        let source = "\
#!/bin/sh
base=8#
echo \"$(( $1suffix ))\"
echo \"$(( ${1}suffix ))\"
echo \"$(( ${base}$1 ))\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalParamAsOperator),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_base_prefixed_literals_that_use_positional_parameters() {
        let source = "\
#!/bin/sh
echo \"$(( 0x$1 ))\"
echo \"$(( 0x${1}${2} ^ 0x200 ))\"
echo \"$(( 16#$1 ))\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalParamAsOperator),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_other_non_positional_parameter_expansions_in_arithmetic() {
        let source = "\
#!/bin/sh
echo \"$(( $_value ))\"
echo \"$(( ${name} ))\"
echo \"$(( ${#1} ))\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalParamAsOperator),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_dollar_tokens_inside_single_quoted_command_substitutions() {
        let source = "\
#!/bin/sh
echo \"$(( $(awk 'END {printf $5}') / 1024 ))\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalParamAsOperator),
        );

        assert!(diagnostics.is_empty());
    }
}
