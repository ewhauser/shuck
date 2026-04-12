use crate::{Checker, Rule, Violation};

use super::tr_common::tr_exact_operand_spans;

pub struct TrLowerRange;

impl Violation for TrLowerRange {
    fn rule() -> Rule {
        Rule::TrLowerRange
    }

    fn message(&self) -> String {
        "use `[:lower:]` instead of `a-z` in `tr` for locale-aware lower-case matching".to_owned()
    }
}

pub fn tr_lower_range(checker: &mut Checker) {
    let spans = tr_exact_operand_spans(checker, "a-z");
    checker.report_all_dedup(spans, || TrLowerRange);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_exact_lowercase_ranges_in_tr_operands() {
        let source = "\
#!/bin/sh
tr a-z xyz < foo
tr abc a-z < foo
tr a-z A-Z < foo
tr -d a-z < foo
tr -s 'a-z' < foo
tr -- \"a-z\" xyz < foo
tr 0-9 a-z < foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrLowerRange));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["a-z", "a-z", "a-z", "a-z", "'a-z'", "\"a-z\"", "a-z"]
        );
    }

    #[test]
    fn ignores_bracketed_and_lookalike_ranges_or_wrapped_tr() {
        let source = "\
#!/bin/sh
tr '[a-z]' xyz < foo
tr aa-z xyz < foo
tr a-zA xyz < foo
tr - xyz < foo
command tr a-z xyz < foo
builtin tr a-z xyz < foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrLowerRange));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_shells_outside_the_rule_target_set() {
        let source = "\
#!/bin/mksh
tr a-z xyz < foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrLowerRange));

        assert!(diagnostics.is_empty());
    }
}
