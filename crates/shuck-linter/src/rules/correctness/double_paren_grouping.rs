use crate::{Checker, Rule, Violation};

pub struct DoubleParenGrouping;

impl Violation for DoubleParenGrouping {
    fn rule() -> Rule {
        Rule::DoubleParenGrouping
    }

    fn message(&self) -> String {
        "double parentheses are used to group commands instead of arithmetic".to_owned()
    }
}

pub fn double_paren_grouping(checker: &mut Checker) {
    let spans = checker.facts().double_paren_grouping_spans().to_vec();
    checker.report_all_dedup(spans, || DoubleParenGrouping);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_command_style_double_paren_grouping() {
        let source = "\
#!/bin/sh
((ps aux | grep foo) || kill \"$pid\") 2>/dev/null
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::DoubleParenGrouping));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn ignores_normal_arithmetic_commands() {
        let source = "\
#!/bin/sh
(( i += 1 ))
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::DoubleParenGrouping));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_grouped_bash_arithmetic_expressions() {
        let source = "\
#!/bin/bash
if ((threads>(cpu_height-3)*3 && tty_width>=200)); then :; fi
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::DoubleParenGrouping));

        assert!(diagnostics.is_empty());
    }
}
