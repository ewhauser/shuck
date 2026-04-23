use crate::{Checker, Rule, Violation};

use super::tr_common::tr_exact_operand_spans;

pub struct TrUpperRange;

impl Violation for TrUpperRange {
    fn rule() -> Rule {
        Rule::TrUpperRange
    }

    fn message(&self) -> String {
        "use `[:upper:]` instead of `A-Z` in `tr` for locale-aware upper-case matching".to_owned()
    }
}

pub fn tr_upper_range(checker: &mut Checker) {
    let spans = tr_exact_operand_spans(checker, "A-Z");
    checker.report_all_dedup(spans, || TrUpperRange);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_exact_uppercase_ranges_in_tr_operands() {
        let source = "\
#!/bin/sh
tr A-Z xyz < foo
tr abc A-Z < foo
tr A-Z a-z < foo
tr -d A-Z < foo
tr -s 'A-Z' < foo
tr -- \"A-Z\" xyz < foo
tr 0-9 A-Z < foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrUpperRange));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["A-Z", "A-Z", "A-Z", "A-Z", "'A-Z'", "\"A-Z\"", "A-Z"]
        );
    }

    #[test]
    fn reports_uppercase_ranges_inside_command_substitutions() {
        let source = "\
#!/bin/sh
html_comment() {
  printf '%s' \"<!-- $(echo \"$1\" | sed 's/-//g' | tr 'A-Z' 'a-z') -->\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrUpperRange));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'A-Z'"]
        );
    }

    #[test]
    fn ignores_bracketed_and_lookalike_ranges_or_wrapped_tr() {
        let source = "\
#!/bin/sh
tr '[A-Z]' xyz < foo
tr AA-Z xyz < foo
tr A-ZA xyz < foo
tr - xyz < foo
command tr A-Z xyz < foo
builtin tr A-Z xyz < foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrUpperRange));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_shells_outside_the_rule_target_set() {
        let source = "\
#!/bin/mksh
tr A-Z xyz < foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrUpperRange));

        assert!(diagnostics.is_empty());
    }
}
