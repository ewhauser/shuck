use crate::{Checker, Rule, Violation};

pub struct ExprArithmetic;

impl Violation for ExprArithmetic {
    fn rule() -> Rule {
        Rule::ExprArithmetic
    }

    fn message(&self) -> String {
        "use shell arithmetic instead of `expr` for numeric operations".to_owned()
    }
}

pub fn expr_arithmetic(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("expr"))
        .filter(|fact| {
            fact.options()
                .expr()
                .is_some_and(|expr| expr.uses_arithmetic_operator())
        })
        .filter_map(|fact| fact.arena_body_name_word(source).map(|word| word.span()))
        .collect::<Vec<_>>();

    checker.report_all(spans, || ExprArithmetic);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_expr_command_name() {
        let source = "#!/bin/sh\nx=$(expr 1 + 2)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExprArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "expr");
    }

    #[test]
    fn ignores_expr_string_forms() {
        let source = "#!/bin/sh\nx=$(expr substr foo 1 2)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExprArithmetic));

        assert!(diagnostics.is_empty());
    }
}
