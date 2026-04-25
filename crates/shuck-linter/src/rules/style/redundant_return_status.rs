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
    checker.report_fact_slice_dedup(
        |facts| facts.redundant_return_status_spans(),
        || RedundantReturnStatus,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_returning_the_previous_status_inside_functions() {
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

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
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
    fn ignores_terminal_returns_inside_final_if_branches() {
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

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_returns_after_terminal_compound_commands() {
        let source = "\
#!/bin/sh
f() {
  if cond; then
    false
  fi
  return $?
}
g() {
  : | false
  return $?
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_control_flow_predecessors_and_backgrounded_returns() {
        let source = "\
#!/bin/sh
f() {
  return 1
  return $?
}
g() {
  false
  return $? &
}
h() {
  false
  x=1 return $?
}
i() {
  false
  return $? >out
}
j() {
  ! {
    false
    return $?
  }
}
k() {
  {
    false
    return $?
  } &
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
