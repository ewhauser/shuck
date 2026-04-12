use crate::{Checker, Rule, Violation};

pub struct GetoptsOptionNotInCase {
    option: char,
}

impl Violation for GetoptsOptionNotInCase {
    fn rule() -> Rule {
        Rule::GetoptsOptionNotInCase
    }

    fn message(&self) -> String {
        format!(
            "getopts option -{} is not handled by this case statement",
            self.option
        )
    }
}

pub fn getopts_option_not_in_case(checker: &mut Checker) {
    let missing = checker
        .facts()
        .getopts_cases()
        .iter()
        .flat_map(|fact| {
            fact.missing_options()
                .iter()
                .map(|option| (fact.case_span(), option.option()))
        })
        .collect::<Vec<_>>();

    for (span, option) in missing {
        checker.report(GetoptsOptionNotInCase { option }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_each_missing_option_on_the_matching_case_statement() {
        let source = "\
while getopts ':ab:c' opt; do
  case \"$opt\" in
    a) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.span.start.line == 2)
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>(),
            vec![
                "getopts option -b is not handled by this case statement",
                "getopts option -c is not handled by this case statement",
            ]
        );
    }

    #[test]
    fn ignores_case_arms_that_cover_all_options() {
        let source = "\
while getopts ':ab' opt; do
  case \"$opt\" in
    a|b) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn only_considers_the_first_matching_case_on_the_getopts_variable() {
        let source = "\
while getopts ':ab' opt; do
  case \"$opt\" in
    a) : ;;
  esac
  case \"$opt\" in
    b) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "getopts option -b is not handled by this case statement"
        );
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn ignores_case_statements_for_other_variables() {
        let source = "\
while getopts ':ab' opt; do
  case \"$other\" in
    a|b) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_missing_explicit_labels_when_a_fallback_arm_exists() {
        let source = "\
while getopts 'qw:c:h' opt; do
  case \"$opt\" in
    w) : ;;
    c) : ;;
    h) : ;;
    *) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_missing_explicit_labels_when_fallback_returns_early() {
        let source = "\
f() {
  while getopts 'd:c:t:p' opt \"$@\"; do
    case \"$opt\" in
      d) : ;;
      t) : ;;
      p) : ;;
      *) return 1 ;;
    esac
  done
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_function_local_cases_before_the_real_getopts_handler() {
        let source = "\
while getopts ':ab' opt; do
  helper() {
    case \"$opt\" in
      a) : ;;
    esac
  }

  case \"$opt\" in
    a|b) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_nonliteral_patterns_when_coverage_is_unknown() {
        let source = "\
while getopts 'ab' opt; do
  case \"$opt\" in
    [ab]) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn static_multi_character_patterns_do_not_hide_missing_options() {
        let source = "\
while getopts 'ac' opt; do
  case \"$opt\" in
    a) : ;;
    ab) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "getopts option -c is not handled by this case statement"
        );
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
    a|b) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_short_circuit_case_branches_before_the_real_getopts_handler() {
        let source = "\
while getopts 'ab' opt; do
  true && {
    case \"$opt\" in
      a) : ;;
    esac
  }

  case \"$opt\" in
    a|b) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_subshell_cases_before_the_real_getopts_handler() {
        let source = "\
while getopts 'ab' opt; do
  (
    case \"$opt\" in
      a) : ;;
    esac
  )

  case \"$opt\" in
    a|b) : ;;
  esac
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GetoptsOptionNotInCase),
        );

        assert!(diagnostics.is_empty());
    }
}
