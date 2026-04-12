use crate::{Checker, Rule, Violation};

pub struct RedundantReturnStatus;

impl Violation for RedundantReturnStatus {
    fn rule() -> Rule {
        Rule::RedundantReturnStatus
    }

    fn message(&self) -> String {
        "function already propagates the last command status".to_owned()
    }
}

pub fn redundant_return_status(checker: &mut Checker) {
    checker.report_all_dedup(
        checker.facts().redundant_return_status_spans().to_vec(),
        || RedundantReturnStatus,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_returning_the_previous_status_inside_functions() {
        let source = "\
#!/bin/sh
f() {
  false
  return $?
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::RedundantReturnStatus);
    }

    #[test]
    fn ignores_returns_outside_functions_and_with_explicit_statuses() {
        let source = "\
#!/bin/sh
return $?
f() {
  return 1
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_non_terminal_returns_inside_function_branches() {
        let source = "\
#!/bin/sh
f() {
  if cond; then
    false
    return $?
  fi
  echo done
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_terminal_returns_inside_final_if_branches() {
        let source = "\
#!/bin/sh
f() {
  if cond; then
    false
    return $?
  fi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$?");
    }
}
