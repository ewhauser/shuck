use crate::{Checker, Rule, Violation};

pub struct ExprSubstrInTest;

impl Violation for ExprSubstrInTest {
    fn rule() -> Rule {
        Rule::ExprSubstrInTest
    }

    fn message(&self) -> String {
        "this uses an `expr` string helper instead of shell string operations".to_owned()
    }
}

pub fn expr_substr_in_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            fact.options()
                .expr()
                .and_then(|expr| expr.string_helper_span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ExprSubstrInTest);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_expr_string_helpers_in_multiple_contexts() {
        let source = "\
#!/bin/sh
x=$(expr length \"$mode\")
if ! expr index \"$mode\" 'w' >/dev/null; then echo r; fi
if test \"`expr substr $(uname -s) 1 5`\" = \"Linux\"; then echo linux; fi
expr match \"$mode\" 'w'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExprSubstrInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["length", "index", "substr", "match"]
        );
    }

    #[test]
    fn ignores_expr_arithmetic_and_non_helper_string_forms() {
        let source = "\
#!/bin/sh
x=$(expr 1 + 2)
expr \"$a\" = \"$b\"
expr \"$a\" : '.*'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExprSubstrInTest));

        assert!(diagnostics.is_empty());
    }
}
