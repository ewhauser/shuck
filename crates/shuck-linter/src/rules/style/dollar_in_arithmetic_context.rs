use crate::{Checker, Rule, Violation};

pub struct DollarInArithmeticContext;

impl Violation for DollarInArithmeticContext {
    fn rule() -> Rule {
        Rule::DollarInArithmeticContext
    }

    fn message(&self) -> String {
        "omit the `$` prefix inside `((...))` arithmetic".to_owned()
    }
}

pub fn dollar_in_arithmetic_context(checker: &mut Checker) {
    let spans = checker
        .facts()
        .dollar_in_arithmetic_context_spans()
        .to_vec();

    checker.report_all_dedup(spans, || DollarInArithmeticContext);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_dollar_prefixed_arithmetic_variables_in_command_context() {
        let source = "#!/bin/bash\nx=1\n(( $x + 1 ))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarInArithmeticContext),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$x");
    }

    #[test]
    fn reports_braced_arithmetic_variables_in_command_context() {
        let source = "#!/bin/bash\nx=1\n(( ${x} + 1 ))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarInArithmeticContext),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "${x}");
    }

    #[test]
    fn reports_dollar_prefixed_variables_in_arithmetic_for_clauses() {
        let source = "#!/bin/bash\nlimit=3\nfor (( i=$limit; i > 0; i-- )); do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarInArithmeticContext),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$limit");
    }

    #[test]
    fn ignores_arithmetic_expansion_forms() {
        let source = "#!/bin/bash\nx=1\nm=$(( $x + 1 ))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarInArithmeticContext),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_positional_parameters_length_and_array_accesses() {
        let source =
            "#!/bin/bash\nx=1\n(( $1 + 1 ))\n(( ${#x} + 1 ))\nver=(1 2)\n(( ${ver[0]} + 1 ))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarInArithmeticContext),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
