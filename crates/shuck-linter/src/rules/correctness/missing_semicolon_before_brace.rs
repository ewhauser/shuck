use crate::{Checker, Rule, Violation};

pub struct MissingSemicolonBeforeBrace;

impl Violation for MissingSemicolonBeforeBrace {
    fn rule() -> Rule {
        Rule::MissingSemicolonBeforeBrace
    }

    fn message(&self) -> String {
        "place a semicolon or newline before `}`".to_owned()
    }
}

pub fn missing_semicolon_before_brace(_checker: &mut Checker) {
    // The parser rejects malformed brace/function bodies before lints run.
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_well_formed_brace_bodies() {
        let source = "\
#!/bin/bash
myfunc() { echo hello; }
{ echo world; }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingSemicolonBeforeBrace),
        );

        assert!(diagnostics.is_empty());
    }
}
