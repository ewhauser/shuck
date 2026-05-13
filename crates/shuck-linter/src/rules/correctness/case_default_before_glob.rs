use shuck_ast::{Pattern, Span};

use super::case_item_delete::case_item_deletion_span;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct CaseDefaultBeforeGlob;

impl Violation for CaseDefaultBeforeGlob {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::CaseDefaultBeforeGlob
    }

    fn message(&self) -> String {
        "this case pattern is unreachable because an earlier pattern already matches it".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the unreachable case pattern".to_owned())
    }
}

pub fn case_default_before_glob(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .case_pattern_shadows()
        .iter()
        .map(|shadow| {
            let span = shadow.shadowed_pattern_span();
            (span, case_pattern_deletion_fix(checker, span, source))
        })
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        let diagnostic = Diagnostic::new(CaseDefaultBeforeGlob, span);
        if let Some(fix) = fix {
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
) -> Option<Fix> {
    let item = checker.facts().case_items().iter().find(|item| {
        item.item().patterns.iter().any(|pattern| {
            pattern.span.start.offset == pattern_span.start.offset
                && pattern.span.end.offset == pattern_span.end.offset
        })
    })?;

    let shadowed_spans = checker
        .facts()
        .case_pattern_shadows()
        .iter()
        .map(|shadow| shadow.shadowed_pattern_span())
        .collect::<Vec<_>>();

    let all_item_patterns_shadowed = item.item().patterns.iter().all(|pattern| {
        shadowed_spans.iter().any(|span| {
            span.start.offset == pattern.span.start.offset
                && span.end.offset == pattern.span.end.offset
        })
    });

    if item.item().patterns.len() == 1 || all_item_patterns_shadowed {
        return case_item_deletion_span(item.item(), source)
            .map(|span| Fix::unsafe_edit(Edit::deletion(span)));
    }

    pattern_alternative_deletion_span(&item.item().patterns, pattern_span, source)
        .map(|span| Fix::unsafe_edit(Edit::deletion(span)))
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
    fn reports_shadowed_patterns_in_same_arm_and_later_arms() {
        let source = "\
#!/bin/sh
case \"$x\" in
  *|foo*) : ;;
  foo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo*", "foo"]
        );
    }

    #[test]
    fn ignores_unanalyzed_patterns_and_continue_matching_arms() {
        let source = "\
#!/bin/bash
shopt -s extglob
case \"$x\" in
  [ab]*) : ;;
  afoo) : ;;
esac
case \"$x\" in
  @(foo|bar)*) : ;;
  fooz) : ;;
esac
case \"$x\" in
  foo) : ;;&
  foo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_extended_glob_patterns_when_reachability_is_uncertain() {
        let source = "\
#!/usr/bin/env zsh
setopt extended_glob
case \"$x\" in
  foo^bar) : ;;
  fooz) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_literals_shadowed_by_suffix_globs() {
        let source = "\
#!/bin/sh
case \"$x\" in
  *foo) : ;;
  barfoo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "barfoo");
    }

    #[test]
    fn ignores_escaped_wildcards_that_are_meant_to_be_literal() {
        let source = "\
#!/bin/sh
case \"$x\" in
  \\?) : ;;
  :) : ;;
  *) : ;;
esac
case \"$x\" in
  lts/\\*) : ;;
  lts/*) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn only_reports_the_first_later_pattern_for_each_shadowing_glob() {
        let source = "\
#!/bin/sh
case \"$x\" in
  foo*) : ;;
  foobar) : ;;
  foobaz) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foobar"]
        );
    }

    #[test]
    fn treats_fallthrough_arms_as_shadowing_sources_until_matching_resumes() {
        let source = "\
#!/bin/bash
case \"$x\" in
  foo*) : ;&
  bar) : ;;
  foobar) : ;;
esac
case \"$x\" in
  foo*) : ;&
  bar) : ;;&
  foobar) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foobar"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_shadowed_case_patterns() {
        let source = "\
#!/bin/sh
case \"$x\" in
  *|foo*) : ;;
  foo) : ;;
esac
case \"$x\" in
  default|foo*) : ;;
  foo) : ;;
esac
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
case \"$x\" in
  *) : ;;
esac
case \"$x\" in
  default|foo*) : ;;
esac
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn unsafe_fix_for_inline_shadowed_case_arm_stops_before_esac() {
        let source = "#!/bin/sh\ncase \"$x\" in *) : ;; foo) : ;; esac\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\ncase \"$x\" in *) : ;; esac\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_shadowed_case_patterns_unchanged() {
        let source = "#!/bin/sh\ncase \"$x\" in\n  foo*) : ;;\n  foo) : ;;\nesac\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C129.sh").as_path(),
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C129_fix_C129.sh", result);
        Ok(())
    }
}
