use crate::{Checker, Rule, Violation};

pub struct SingleLetterCaseLabel;

impl Violation for SingleLetterCaseLabel {
    fn rule() -> Rule {
        Rule::SingleLetterCaseLabel
    }

    fn message(&self) -> String {
        "double-check bare single-letter case labels in this getopts handler".to_owned()
    }
}

pub fn single_letter_case_label(checker: &mut Checker) {
    let spans = checker
        .facts()
        .getopts_cases()
        .iter()
        .filter(|fact| !fact.has_fallback_pattern())
        .filter(|fact| !fact.missing_options().is_empty())
        .filter(|fact| fact.unexpected_case_labels().is_empty())
        .flat_map(|fact| fact.handled_case_labels().iter().copied())
        .filter(|label| label.is_bare_single_letter())
        .map(|label| label.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SingleLetterCaseLabel);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_bare_single_letter_labels_in_incomplete_getopts_cases() {
        let source = "\
while getopts 'hb:c:' opt; do
  case \"$opt\" in
    h) : ;;
    b) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SingleLetterCaseLabel));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["h", "b"]
        );
    }

    #[test]
    fn ignores_quoted_labels_complete_handlers_and_fallback_cases() {
        let source = "\
while getopts 'hb:c:' opt; do
  case \"$opt\" in
    \"h\") : ;;
    \"b\") : ;;
  esac
done
while getopts 'hb:c:' opt; do
  case \"$opt\" in
    h) : ;;
    b) : ;;
    c) : ;;
  esac
done
while getopts 'hb:c:' opt; do
  case \"$opt\" in
    h) : ;;
    b) : ;;
    *) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SingleLetterCaseLabel));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_cases_with_unexpected_or_special_labels() {
        let source = "\
while getopts ':a' opt; do
  case \"$opt\" in
    a) : ;;
    \\?) : ;;
    :) : ;;
  esac
done
while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
    b) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SingleLetterCaseLabel));

        assert!(diagnostics.is_empty());
    }
}
