use crate::{Checker, Rule, Violation};

pub struct EmptyFunctionBody;

impl Violation for EmptyFunctionBody {
    fn rule() -> Rule {
        Rule::EmptyFunctionBody
    }

    fn message(&self) -> String {
        "function body is empty; add `:` or `true` as a placeholder command".to_owned()
    }
}

pub fn empty_function_body(_checker: &mut Checker) {
    // Empty brace-group function bodies are rejected by the parser before lints run.
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_well_formed_function_bodies() {
        let source = "\
#!/bin/sh
f() { :; }
g() { true; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EmptyFunctionBody));

        assert!(diagnostics.is_empty());
    }
}
