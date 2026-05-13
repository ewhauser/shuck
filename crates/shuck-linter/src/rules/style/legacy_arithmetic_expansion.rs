use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct LegacyArithmeticExpansion;

impl Violation for LegacyArithmeticExpansion {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LegacyArithmeticExpansion
    }

    fn message(&self) -> String {
        "prefer `$((...))` over legacy `$[...]` arithmetic expansion".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite as `$((...))`".to_owned())
    }
}

pub fn legacy_arithmetic_expansion(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .legacy_arithmetic_fragments()
        .iter()
        .filter_map(|fragment| legacy_arithmetic_fix(fragment.span(), source))
        .map(|(span, fix)| Diagnostic::new(LegacyArithmeticExpansion, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn legacy_arithmetic_fix(span: Span, source: &str) -> Option<(Span, Fix)> {
    let text = span.slice(source);
    let body = text.strip_prefix("$[")?.strip_suffix(']')?;
    Some((
        span,
        Fix::safe_edit(Edit::replacement(format!("$(({body}))"), span)),
    ))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn anchors_on_each_legacy_arithmetic_fragment() {
        let source = "echo \"$[1 + 2]\" '$[ignored]' \"$[3 + 4]\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$[1 + 2]", "$[3 + 4]"]
        );
    }

    #[test]
    fn reports_nested_legacy_arithmetic_fragments() {
        let source = "#!/bin/bash\necho $[$[1 + 2] + 3]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$[$[1 + 2] + 3]", "$[1 + 2]"]
        );
    }

    #[test]
    fn applies_safe_fix_to_legacy_arithmetic_fragments() {
        let source = "#!/bin/bash\necho $[1 + 2]\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticExpansion),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/bash\necho $((1 + 2))\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}
