use crate::{Checker, Rule, Violation};

pub struct ArithmeticScoreLine;

impl Violation for ArithmeticScoreLine {
    fn rule() -> Rule {
        Rule::ArithmeticScoreLine
    }

    fn message(&self) -> String {
        "avoid extra parentheses in arithmetic assignments".to_owned()
    }
}

pub fn arithmetic_score_line(checker: &mut Checker) {
    let spans = checker.facts().arithmetic_score_line_spans().to_vec();

    checker.report_all_dedup(spans, || ArithmeticScoreLine);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_redundant_parentheses_in_assignment_arithmetic() {
        let source = "#!/bin/bash\nscore=$(( (1 + 2) ))\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ArithmeticScoreLine));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(1 + 2)");
    }

    #[test]
    fn ignores_arithmetic_expansion_without_wrapping_parentheses() {
        let source = "#!/bin/bash\nscore=$((1 + 2))\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ArithmeticScoreLine));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
