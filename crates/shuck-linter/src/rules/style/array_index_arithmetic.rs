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
    checker.report_fact_slice_dedup(
        |facts| facts.array_index_arithmetic_spans(),
        || ArrayIndexArithmetic,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
