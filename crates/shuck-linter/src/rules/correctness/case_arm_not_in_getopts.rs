use shuck_ast::{Pattern, Position, Span};

use crate::facts::CaseItemFact;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct CaseArmNotInGetopts {
    option: Option<char>,
}

impl Violation for CaseArmNotInGetopts {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

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

    fn fix_title(&self) -> Option<String> {
        Some("delete the undeclared case pattern".to_owned())
    }
}

pub fn case_arm_not_in_getopts(checker: &mut Checker) {
    let source = checker.source();
    let unexpected = checker
        .facts()
        .getopts_cases()
        .iter()
        .flat_map(|fact| {
            fact.unexpected_case_labels()
                .iter()
                .map(|label| (label.span(), Some(label.label()), false))
                .chain(
                    fact.invalid_case_pattern_spans()
                        .iter()
                        .copied()
                        .map(|span| (span, None, true)),
                )
        })
        .collect::<Vec<_>>();

    for (span, option, delete_item) in unexpected {
        let diagnostic = Diagnostic::new(CaseArmNotInGetopts { option }, span);
        if let Some(fix) = case_pattern_deletion_fix(checker, span, source, delete_item) {
            checker.report_diagnostic_dedup(diagnostic.with_fix(fix));
        } else {
            checker.report_diagnostic_dedup(diagnostic);
        }
    }
}

fn case_pattern_deletion_fix(
    checker: &Checker<'_>,
    pattern_span: Span,
    source: &str,
    delete_item: bool,
) -> Option<Fix> {
    let item = checker.facts().case_items().iter().find(|item| {
        item.item().patterns.iter().any(|pattern| {
            pattern.span.start.offset == pattern_span.start.offset
                && pattern.span.end.offset == pattern_span.end.offset
        })
    })?;

    if delete_item || item.item().patterns.len() == 1 || all_item_patterns_reported(checker, item) {
        return item_deletion_span(item.item(), source)
            .map(|span| Fix::unsafe_edit(Edit::deletion(span)));
    }

    pattern_alternative_deletion_span(&item.item().patterns, pattern_span, source)
        .map(|span| Fix::unsafe_edit(Edit::deletion(span)))
}

fn all_item_patterns_reported(checker: &Checker<'_>, item: &CaseItemFact<'_>) -> bool {
    let reported_spans = checker
        .facts()
        .getopts_cases()
        .iter()
        .flat_map(|fact| {
            fact.unexpected_case_labels()
                .iter()
                .map(|label| label.span())
                .chain(fact.invalid_case_pattern_spans().iter().copied())
        })
        .collect::<Vec<_>>();

    item.item().patterns.iter().all(|pattern| {
        reported_spans.iter().any(|span| {
            span.start.offset == pattern.span.start.offset
                && span.end.offset == pattern.span.end.offset
        })
    })
}

fn item_deletion_span(item: &shuck_ast::CaseItem, source: &str) -> Option<Span> {
    let first = item.patterns.first()?.span;
    let end = item.terminator_span.unwrap_or(item.body.span).end;
    let mut start_offset = first.start.offset;
    while start_offset > 0 {
        let previous = source.as_bytes()[start_offset - 1];
        if previous == b'\n' {
            break;
        }
        if !previous.is_ascii_whitespace() {
            break;
        }
        start_offset -= 1;
    }
    let mut end_offset = end.offset;
    while end_offset < source.len() {
        let byte = source.as_bytes()[end_offset];
        end_offset += 1;
        if byte == b'\n' {
            break;
        }
    }

    Some(Span::from_positions(
        Position {
            offset: start_offset,
            line: first.start.line,
            column: first
                .start
                .column
                .saturating_sub(first.start.offset.saturating_sub(start_offset)),
        },
        end.advanced_by(&source[end.offset..end_offset]),
    ))
}

fn pattern_alternative_deletion_span(
    patterns: &[Pattern],
    pattern_span: Span,
    source: &str,
) -> Option<Span> {
    let index = patterns.iter().position(|pattern| {
        pattern.span.start.offset == pattern_span.start.offset
            && pattern.span.end.offset == pattern_span.end.offset
    })?;

    if index + 1 < patterns.len() {
        let next_start = patterns[index + 1].span.start.offset;
        let pipe = source[pattern_span.end.offset..next_start].find('|')?;
        let end = pattern_span
            .end
            .advanced_by(&source[pattern_span.end.offset..pattern_span.end.offset + pipe + 1]);
        return Some(Span::from_positions(pattern_span.start, end));
    }

    if index > 0 {
        let previous_end = patterns[index - 1].span.end.offset;
        let pipe = source[previous_end..pattern_span.start.offset].rfind('|')?;
        let start = patterns[index - 1]
            .span
            .end
            .advanced_by(&source[previous_end..previous_end + pipe]);
        return Some(Span::from_positions(start, pattern_span.end));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn reports_pipeline_rhs_handlers() {
        let source = "\
while getopts 'a' opt; do
  printf '%s\\n' \"$opt\" | case \"$opt\" in
    b) : ;;
  esac
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::CaseArmNotInGetopts));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "b");
    }

    #[test]
    fn applies_unsafe_fix_to_undeclared_getopts_patterns() {
        let source = "\
while getopts 'a' opt; do
  case \"$opt\" in
    a|b) : ;;
    c) : ;;
    de) : ;;
  esac
done
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CaseArmNotInGetopts),
            Applicability::Unsafe,
        );

        assert_eq!(
            result.fixed_source,
            "\
while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
  esac
done
"
        );
        assert_eq!(result.fixes_applied, 3);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_getopts_patterns_unchanged() {
        let source = "\
while getopts 'a' opt; do
  case \"$opt\" in
    b) : ;;
  esac
done
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CaseArmNotInGetopts),
            Applicability::Safe,
        );

        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixes_applied, 0);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C135.sh").as_path(),
            &LinterSettings::for_rule(Rule::CaseArmNotInGetopts),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C135_fix_C135.sh", result);
        Ok(())
    }
}
