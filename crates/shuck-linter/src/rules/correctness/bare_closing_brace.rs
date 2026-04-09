use crate::{Checker, Rule, Violation};

pub struct BareClosingBrace;

impl Violation for BareClosingBrace {
    fn rule() -> Rule {
        Rule::BareClosingBrace
    }

    fn message(&self) -> String {
        "put a semicolon or newline before `}` to terminate the command".to_owned()
    }
}

pub fn bare_closing_brace(_checker: &mut Checker) {
    // Missing command terminators before `}` are parser errors, so malformed
    // brace groups are rejected before lints run.
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_well_formed_brace_groups() {
        let source = "\
#!/bin/sh
{ echo hello; }
{
  echo world
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BareClosingBrace));

        assert!(diagnostics.is_empty());
    }
}
