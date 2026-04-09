use crate::{Checker, Rule, ShellDialect, Violation};

pub struct BasePrefixInArithmetic;

impl Violation for BasePrefixInArithmetic {
    fn rule() -> Rule {
        Rule::BasePrefixInArithmetic
    }

    fn message(&self) -> String {
        "base prefixes like `10#` are not portable in `sh` arithmetic".to_owned()
    }
}

pub fn base_prefix_in_arithmetic(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker.facts().base_prefix_arithmetic_spans().to_vec();
    checker.report_all_dedup(spans, || BasePrefixInArithmetic);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_base_prefixes_in_sh() {
        let source = "\
#!/bin/sh
echo $((10#123))
echo $((10#${foo}))
echo ${foo:10#1:2}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
        );

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["10#123", "10#", "10#1"]
        );
    }

    #[test]
    fn ignores_base_prefixes_in_bash() {
        let source = "\
#!/bin/bash
echo $((10#123))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
        );

        assert!(diagnostics.is_empty());
    }
}
