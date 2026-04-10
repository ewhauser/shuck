use crate::facts::WordFactHostKind;
use crate::{Checker, Rule, Violation};

pub struct ArrayIndexArithmetic;

impl Violation for ArrayIndexArithmetic {
    fn rule() -> Rule {
        Rule::ArrayIndexArithmetic
    }

    fn message(&self) -> String {
        "remove the `$((...))` wrapper from array subscripts".to_owned()
    }
}

pub fn array_index_arithmetic(checker: &mut Checker) {
    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| {
            matches!(
                fact.host_kind(),
                WordFactHostKind::AssignmentTargetSubscript
                    | WordFactHostKind::DeclarationNameSubscript
                    | WordFactHostKind::ArrayKeySubscript
                    | WordFactHostKind::ConditionalVarRefSubscript
            )
        })
        .flat_map(|fact| fact.arithmetic_expansion_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArrayIndexArithmetic);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_arithmetic_expansions_inside_assignment_subscripts() {
        let source = "#!/bin/bash\narr[$((1+1))]=x\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ArrayIndexArithmetic));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$((1+1))"]
        );
    }

    #[test]
    fn reports_arithmetic_expansions_inside_array_keys() {
        let source = "#!/bin/bash\ndeclare -A map=([foo[$((1+1))]]=x)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ArrayIndexArithmetic));

        assert!(!diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_plain_arithmetic_subscripts_without_expansion() {
        let source = "#!/bin/bash\narr[1+1]=x\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ArrayIndexArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
