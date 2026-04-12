use crate::{Checker, Rule, Violation};

pub struct CaseArmNotInGetopts {
    option: Option<char>,
}

impl Violation for CaseArmNotInGetopts {
    fn rule() -> Rule {
        Rule::CaseArmNotInGetopts
    }

    fn message(&self) -> String {
        match self.option {
            Some(option) => format!(
                "this case arm handles -{}, but getopts does not declare it",
                option
            ),
            None => "this case arm does not match any option declared by getopts".to_owned(),
        }
    }
}

pub fn case_arm_not_in_getopts(checker: &mut Checker) {
    let unexpected = checker
        .facts()
        .getopts_cases()
        .iter()
        .flat_map(|fact| {
            fact.unexpected_case_labels()
                .iter()
                .map(|label| (label.span(), Some(label.label())))
                .chain(
                    fact.invalid_case_pattern_spans()
                        .iter()
                        .copied()
                        .map(|span| (span, None)),
                )
        })
        .collect::<Vec<_>>();

    for (span, option) in unexpected {
        checker.report(CaseArmNotInGetopts { option }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_case_labels_that_are_not_declared_by_getopts() {
        let source = "\
while getopts ':a:d:h' OPT; do
  case \"$OPT\" in
    a) : ;;
    d) : ;;
    k) : ;;
    h) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "k");
        assert_eq!(
            diagnostics[0].message,
            "this case arm handles -k, but getopts does not declare it"
        );
    }

    #[test]
    fn reports_only_the_undeclared_alternatives_in_a_multi_pattern_arm() {
        let source = "\
while getopts 'a' opt; do
  case \"$opt\" in
    a|b) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "b");
    }

    #[test]
    fn ignores_special_getopts_error_handlers_and_fallbacks() {
        let source = "\
while getopts ':a' opt; do
  case \"$opt\" in
    a) : ;;
    \\?) : ;;
    :) : ;;
    *) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn only_considers_the_first_matching_case_on_the_getopts_variable() {
        let source = "\
while getopts 'a' opt; do
  case \"$opt\" in
    b) : ;;
  esac
  case \"$opt\" in
    a) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "b");
    }

    #[test]
    fn ignores_function_local_cases_before_the_real_getopts_handler() {
        let source = "\
while getopts 'a' opt; do
  helper() {
    case \"$opt\" in
      b) : ;;
    esac
  }

  case \"$opt\" in
    a) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_static_multi_character_patterns_as_invalid_getopts_arms() {
        let source = "\
while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
    ab) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "ab");
        assert_eq!(
            diagnostics[0].message,
            "this case arm does not match any option declared by getopts"
        );
    }

    #[test]
    fn ignores_branch_local_cases_before_the_real_getopts_handler() {
        let source = "\
while getopts 'a' opt; do
  if true; then
    case \"$opt\" in
      b) : ;;
    esac
  fi

  case \"$opt\" in
    a) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_short_circuit_case_branches_before_the_real_getopts_handler() {
        let source = "\
while getopts 'a' opt; do
  true && {
    case \"$opt\" in
      b) : ;;
    esac
  }

  case \"$opt\" in
    a) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_undeclared_case_arms_when_getopts_declares_no_options() {
        let source = "\
while getopts ':' opt; do
  case \"$opt\" in
    a) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "a");
    }

    #[test]
    fn ignores_subshell_cases_before_the_real_getopts_handler() {
        let source = "\
while getopts 'a' opt; do
  (
    case \"$opt\" in
      b) : ;;
    esac
  )

  case \"$opt\" in
    a) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_brace_group_wrapped_handlers() {
        let source = "\
while getopts 'a' opt; do
  {
    case \"$opt\" in
      b) : ;;
    esac
  }
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "b");
    }
}
