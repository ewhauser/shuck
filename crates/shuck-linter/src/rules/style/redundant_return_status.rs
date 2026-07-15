use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct RedundantReturnStatus;

impl Violation for RedundantReturnStatus {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::RedundantReturnStatus
    }

    fn message(&self) -> String {
        "function already propagates the last command status".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the redundant `return $?`".to_owned())
    }
}

pub fn redundant_return_status(checker: &mut Checker) {
    checker.report_fact_diagnostics_dedup(|facts, report| {
        for span in facts
            .command_facts()
            .redundant_return_status_spans()
            .iter()
            .copied()
        {
            report(
                Diagnostic::new(RedundantReturnStatus, span)
                    .with_fix(Fix::safe_edit(Edit::deletion(span))),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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
        assert_eq!(diagnostics[0].span.slice(source), "  return $?\n");
    }

    #[test]
    fn applies_safe_fix_to_delete_redundant_return_status() {
        let source = "#!/bin/sh\nf() {\n  echo hello >/dev/null\n  return $?\n}\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nf() {\n  echo hello >/dev/null\n}\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_deletes_statement_terminators() {
        let source = "#!/bin/sh\nf(){ false; return $?; }\ng(){ false; return $? ; }\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nf(){ false;  }\ng(){ false;  }\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
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

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S038.sh").as_path(),
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("S038_fix_S038.sh", result);
        Ok(())
    }
}
