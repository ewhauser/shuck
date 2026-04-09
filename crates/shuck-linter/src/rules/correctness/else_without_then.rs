use crate::{Checker, Rule, Violation};

pub struct ElseWithoutThen;

impl Violation for ElseWithoutThen {
    fn rule() -> Rule {
        Rule::ElseWithoutThen
    }

    fn message(&self) -> String {
        "else clause appears before a `then` keyword".to_owned()
    }
}

pub fn else_without_then(_checker: &mut Checker) {
    // The parser rejects malformed `if ... else` blocks without `then` before lints run.
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_well_formed_if_else_blocks() {
        let source = "\
#!/bin/sh
if [ \"$x\" ]; then
  echo a
else
  echo b
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ElseWithoutThen));

        assert!(diagnostics.is_empty());
    }
}
