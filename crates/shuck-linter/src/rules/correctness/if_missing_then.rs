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
    // Parse diagnostics synthesize this rule before the normal lint walk runs.
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

    #[test]
    fn reports_missing_then_from_parse_diagnostics() {
        let source = "\
#!/bin/sh
if [ \"$x\" ]
  echo ok
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfMissingThen));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code(), "C064");
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }
}
