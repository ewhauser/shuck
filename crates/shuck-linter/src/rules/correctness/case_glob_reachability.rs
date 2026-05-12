use shuck_ast::{Pattern, Position, Span};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct CaseGlobReachability;

impl Violation for CaseGlobReachability {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::CaseGlobReachability
    }

    fn message(&self) -> String {
        "this case pattern shadows a later pattern".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the shadowing case pattern".to_owned())
    }
}

pub fn case_glob_reachability(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .case_pattern_shadows()
        .iter()
        .map(|shadow| {
            let span = shadow.shadowing_pattern_span();
            (span, case_pattern_deletion_fix(checker, span, source))
        })
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        let diagnostic = Diagnostic::new(CaseGlobReachability, span);
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

    let shadowing_spans = checker
        .facts()
        .case_pattern_shadows()
        .iter()
        .map(|shadow| shadow.shadowing_pattern_span())
        .collect::<Vec<_>>();

    let all_item_patterns_shadow = item.item().patterns.iter().all(|pattern| {
        shadowing_spans.iter().any(|span| {
            span.start.offset == pattern.span.start.offset
                && span.end.offset == pattern.span.end.offset
        })
    });

    if item.item().patterns.len() == 1 || all_item_patterns_shadow {
        return item_deletion_span(item.item(), source)
            .map(|span| Fix::unsafe_edit(Edit::deletion(span)));
    }

    pattern_alternative_deletion_span(&item.item().patterns, pattern_span, source)
        .map(|span| Fix::unsafe_edit(Edit::deletion(span)))
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
    fn reports_shadowing_patterns_in_same_arm_and_later_arms() {
        let source = "\
#!/bin/sh
case \"$x\" in
  *|foo*) : ;;
  foo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*", "foo*"]
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
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
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
            &LinterSettings::for_rule(Rule::CaseGlobReachability)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_static_quoted_pattern_fragments() {
        let source = "\
#!/bin/sh
case \"$x\" in
  foo\"bar\"*) : ;;
  foobarz) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "foo\"bar\"*");
    }

    #[test]
    fn reports_suffix_globs_shadowing_later_literals() {
        let source = "\
#!/bin/sh
case \"$x\" in
  *foo) : ;;
  barfoo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "*foo");
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
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert!(diagnostics.is_empty());
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
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo*"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_shadowing_case_patterns() {
        let source = "\
#!/bin/sh
case \"$x\" in
  foo) : ;;
  foo) : ;;
esac
case \"$x\" in
  *|foo*) : ;;
  foo) : ;;
esac
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
case \"$x\" in
  foo) : ;;
esac
case \"$x\" in
  foo) : ;;
esac
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_shadowing_case_patterns_unchanged() {
        let source = "#!/bin/sh\ncase \"$x\" in\n  foo) : ;;\n  foo) : ;;\nesac\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C128.sh").as_path(),
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C128_fix_C128.sh", result);
        Ok(())
    }
}
