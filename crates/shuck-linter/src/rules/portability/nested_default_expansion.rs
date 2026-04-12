use crate::{Checker, Rule, Violation};

pub struct NestedDefaultExpansion;

impl Violation for NestedDefaultExpansion {
    fn rule() -> Rule {
        Rule::NestedDefaultExpansion
    }

    fn message(&self) -> String {
        "nested default-value expansions are not portable in `sh`".to_owned()
    }
}

pub fn nested_default_expansion(_: &mut Checker) {}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn ignores_nested_default_expansion_operands_in_sh() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${outer:-${inner:-fallback}}\" \"${outer:-$inner}\" \"${outer:-fallback}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NestedDefaultExpansion),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_nested_default_expansion_in_bash() {
        let source = "printf '%s\n' \"${outer:-${inner:-fallback}}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NestedDefaultExpansion).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
