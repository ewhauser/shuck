use crate::{Checker, Rule, Violation};

pub struct LegacyArithmeticExpansion;

impl Violation for LegacyArithmeticExpansion {
    fn rule() -> Rule {
        Rule::LegacyArithmeticExpansion
    }

    fn message(&self) -> String {
        "prefer `$((...))` over legacy `$[...]` arithmetic expansion".to_owned()
    }
}

pub fn legacy_arithmetic_expansion(checker: &mut Checker) {
    let spans = checker
        .facts()
        .legacy_arithmetic_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || LegacyArithmeticExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_each_legacy_arithmetic_fragment() {
        let source = "echo \"$[1 + 2]\" '$[ignored]' \"$[3 + 4]\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$[1 + 2]", "$[3 + 4]"]
        );
    }

    #[test]
    fn reports_nested_legacy_arithmetic_fragments() {
        let source = "#!/bin/bash\necho $[$[1 + 2] + 3]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$[$[1 + 2] + 3]", "$[1 + 2]"]
        );
    }
}
