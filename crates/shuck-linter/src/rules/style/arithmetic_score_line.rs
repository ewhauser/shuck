use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct ArithmeticScoreLine;

impl Violation for ArithmeticScoreLine {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::ArithmeticScoreLine
    }

    fn message(&self) -> String {
        "avoid extra parentheses in arithmetic assignments".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the extra arithmetic parentheses".to_owned())
    }
}

pub fn arithmetic_score_line(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .arithmetic_score_line_spans()
        .iter()
        .copied()
        .filter_map(|span| arithmetic_score_line_fix(span, source))
        .map(|(span, fix)| Diagnostic::new(ArithmeticScoreLine, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn arithmetic_score_line_fix(span: Span, source: &str) -> Option<(Span, Fix)> {
    let text = span.slice(source);
    let body = text.strip_prefix('(')?.strip_suffix(')')?;
    Some((span, Fix::safe_edit(Edit::replacement(body, span))))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

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

    #[test]
    fn applies_safe_fix_to_redundant_arithmetic_parentheses() {
        let source = "#!/bin/bash\nscore=$(( (1 + 2) ))\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ArithmeticScoreLine),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/bash\nscore=$(( 1 + 2 ))\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}
