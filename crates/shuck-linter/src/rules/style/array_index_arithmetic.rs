use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct ArrayIndexArithmetic;

impl Violation for ArrayIndexArithmetic {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::ArrayIndexArithmetic
    }

    fn message(&self) -> String {
        "remove the `$((...))` wrapper from array subscripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the arithmetic expansion wrapper".to_owned())
    }
}

pub fn array_index_arithmetic(checker: &mut Checker) {
    let source = checker.source();
    checker.report_fact_diagnostics_dedup(|facts, report| {
        for span in facts.array_index_arithmetic_spans().iter().copied() {
            if let Some(fix) = array_index_arithmetic_fix(span, source) {
                report(Diagnostic::new(ArrayIndexArithmetic, span).with_fix(fix));
            }
        }
    });
}

fn array_index_arithmetic_fix(span: Span, source: &str) -> Option<Fix> {
    let text = span.slice(source);
    let body = text.strip_prefix("$((")?.strip_suffix("))")?;
    Some(Fix::safe_edit(Edit::replacement(body, span)))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_arithmetic_expansions_inside_assignment_subscripts() {
        let source = "#!/bin/bash\narr[$((1+1))]=x\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayIndexArithmetic),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$((1+1))"]
        );
    }

    #[test]
    fn reports_arithmetic_expansions_inside_declaration_subscripts() {
        let source = "#!/bin/bash\ndeclare arr[$((1+1))]=x\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayIndexArithmetic),
        );

        assert!(!diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_plain_arithmetic_subscripts_without_expansion() {
        let source = "#!/bin/bash\narr[1+1]=x\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayIndexArithmetic),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_safe_fix_to_array_index_arithmetic_wrapper() {
        let source = "#!/bin/bash\narr[$((1+1))]=x\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ArrayIndexArithmetic),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/bash\narr[1+1]=x\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_associative_and_non_lvalue_subscript_contexts() {
        let source = "\
#!/bin/bash
declare -A map
map[$((assoc+1))]=x
map[temp_$((mixed+1))]=y
map=([$((compound+1))]=z)
printf '%s\\n' \"${map[$((read+1))]}\"
[[ -v map[$((check+1))] ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayIndexArithmetic),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
