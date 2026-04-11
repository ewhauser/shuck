use crate::{Checker, Rule, Violation};

pub struct MixedAndOrInCondition;

impl Violation for MixedAndOrInCondition {
    fn rule() -> Rule {
        Rule::MixedAndOrInCondition
    }

    fn message(&self) -> String {
        "mixing `&&` and `||` inside `[[ ... ]]` needs parentheses to stay clear".to_owned()
    }
}

pub fn mixed_and_or_in_condition(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| command.conditional())
        .flat_map(|conditional| conditional.mixed_logical_operator_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MixedAndOrInCondition);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_ungrouped_mixed_logical_operators_in_double_brackets() {
        let source = "\
#!/bin/bash
[[ -n $a && -n $b || -n $c ]]
[[ -n $a || -n $b && -n $c ]]
[[ ( -n $a && -n $b || -n $c ) && -n $d ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MixedAndOrInCondition),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["||", "||", "||"]
        );
    }

    #[test]
    fn ignores_grouped_or_single_operator_logical_conditions() {
        let source = "\
#!/bin/bash
[[ -n $a && ( -n $b || -n $c ) ]]
[[ ( -n $a && -n $b ) || -n $c ]]
[[ -n $a && -n $b && -n $c ]]
[[ -n $a || -n $b || -n $c ]]
[ -n \"$a\" -a -n \"$b\" -o -n \"$c\" ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MixedAndOrInCondition),
        );

        assert!(diagnostics.is_empty());
    }
}
