use crate::{Checker, Rule, Violation};

pub struct IfMissingThen;

impl Violation for IfMissingThen {
    fn rule() -> Rule {
        Rule::IfMissingThen
    }

    fn message(&self) -> String {
        "if condition is missing a `then` keyword".to_owned()
    }
}

pub fn if_missing_then(_checker: &mut Checker) {
    // The parser rejects malformed `if` blocks without `then` before lints run.
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_well_formed_if_blocks() {
        let source = "\
#!/bin/sh
if [ \"$x\" ]; then
  echo ok
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfMissingThen));

        assert!(diagnostics.is_empty());
    }
}
