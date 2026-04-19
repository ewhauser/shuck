use crate::{Checker, Rule, Violation};

pub struct GetoptsInvalidFlagHandler;

impl Violation for GetoptsInvalidFlagHandler {
    fn rule() -> Rule {
        Rule::GetoptsInvalidFlagHandler
    }

    fn message(&self) -> String {
        "this getopts handler should include a catch-all or literal \\? arm for invalid flags"
            .to_owned()
    }
}

pub fn getopts_invalid_flag_handler(checker: &mut Checker) {
    let spans = checker
        .facts()
        .getopts_cases()
        .iter()
        .filter(|fact| fact.missing_invalid_flag_handler())
        .map(|fact| fact.case_span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GetoptsInvalidFlagHandler);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_cases_without_invalid_flag_handlers() {
        let source = "\
while getopts 'ab' opt; do
  case \"$opt\" in
    a) : ;;
    b) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsInvalidFlagHandler),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "case \"$opt\" in\n    a) : ;;\n    b) : ;;\n  esac"
        );
    }

    #[test]
    fn reports_silent_mode_handlers_that_only_check_missing_arguments() {
        let source = "\
while getopts ':a' opt; do
  case \"$opt\" in
    a) : ;;
    :) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsInvalidFlagHandler),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn ignores_explicit_question_handlers_wildcards_and_unknown_coverage() {
        let source = "\
while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
    \\?) : ;;
  esac
done
while getopts 'ab' opt; do
  case \"$opt\" in
    a) : ;;
    *) : ;;
  esac
done
while getopts 'ab' opt; do
  case \"$opt\" in
    [ab]) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsInvalidFlagHandler),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn still_reports_when_other_case_arms_are_also_wrong() {
        let source = "\
while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
    b) : ;;
  esac
done
while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
    ab) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsInvalidFlagHandler),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 8]
        );
    }

    #[test]
    fn ignores_function_local_cases_before_the_real_getopts_handler() {
        let source = "\
while getopts 'ab' opt; do
  helper() {
    case \"$opt\" in
      a) : ;;
    esac
  }

  case \"$opt\" in
    a) : ;;
    \\?) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsInvalidFlagHandler),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_branch_local_cases_before_the_real_getopts_handler() {
        let source = "\
while getopts 'ab' opt; do
  if true; then
    case \"$opt\" in
      a) : ;;
    esac
  fi

  case \"$opt\" in
    a) : ;;
    \\?) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsInvalidFlagHandler),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_brace_group_wrapped_handlers() {
        let source = "\
while getopts 'a' opt; do
  {
    case \"$opt\" in
      a) : ;;
    esac
  }
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsInvalidFlagHandler),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn reports_pipeline_rhs_handlers() {
        let source = "\
while getopts 'a' opt; do
  printf '%s\\n' \"$opt\" | case \"$opt\" in
    a) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsInvalidFlagHandler),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }
}
