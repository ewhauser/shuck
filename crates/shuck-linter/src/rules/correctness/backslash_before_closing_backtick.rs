use shuck_ast::Span;

use crate::{Checker, FixAvailability, Rule, Violation};

pub struct BackslashBeforeClosingBacktick;

impl Violation for BackslashBeforeClosingBacktick {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::BackslashBeforeClosingBacktick
    }

    fn message(&self) -> String {
        "remove the escaped trailing space before closing backtick".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the backslash before the closing backtick".to_owned())
    }
}

pub fn backslash_before_closing_backtick(checker: &mut Checker) {
    let spans = checker
        .facts()
        .backtick_fragments()
        .iter()
        .filter_map(|fragment| {
            backslash_before_closing_backtick_spans(fragment.span(), checker.source())
        })
        .collect::<Vec<_>>();

    for (report_span, fix_span) in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(BackslashBeforeClosingBacktick, report_span)
                .with_fix(crate::Fix::unsafe_edit(crate::Edit::deletion(fix_span))),
        );
    }
}

fn backslash_before_closing_backtick_spans(span: Span, source: &str) -> Option<(Span, Span)> {
    let text = span.slice(source);
    if !text.starts_with('`') || !text.ends_with('`') || text.len() < 3 {
        return None;
    }

    let inner = &text[1..text.len() - 1];
    let trailing_spaces = inner.bytes().rev().take_while(|byte| *byte == b' ').count();
    if trailing_spaces == 0 || trailing_spaces >= inner.len() {
        return None;
    }

    let backslash_index = inner.len() - trailing_spaces - 1;
    if inner.as_bytes()[backslash_index] != b'\\' {
        return None;
    }

    let prefix = &text[..1 + backslash_index];
    let start = span.start.advanced_by(prefix);
    let report_span = Span::from_positions(start, start);
    let fix_span = Span::from_positions(start, start.advanced_by("\\"));
    Some((report_span, fix_span))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::test::{test_path_with_fix, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};
    use std::path::Path;

    #[test]
    fn reports_backslash_space_before_closing_backtick() {
        let source = "\
#!/bin/bash
# shellcheck disable=2006
ARCH=`uname -a | cut -f12 -d\\ `
echo \"$ARCH\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BackslashBeforeClosingBacktick),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(3, 29, 3, 29)]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.span.slice(source).is_empty())
        );
    }

    #[test]
    fn ignores_backticks_without_trailing_escaped_space() {
        let source = "\
#!/bin/bash
# shellcheck disable=2006
ARCH=`uname -a | cut -f12 -d ','`
echo \"$ARCH\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BackslashBeforeClosingBacktick),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_backslash_before_closing_backtick() {
        let source = "\
#!/bin/bash
# shellcheck disable=2006
ARCH=`uname -a | cut -f12 -d\\ `
echo \"$ARCH\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BackslashBeforeClosingBacktick),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
# shellcheck disable=2006
ARCH=`uname -a | cut -f12 -d `
echo \"$ARCH\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_similar_backticks_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
# shellcheck disable=2006
ARCH=`uname -a | cut -f12 -d ','`
echo \"$ARCH\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BackslashBeforeClosingBacktick),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C069.sh").as_path(),
            &LinterSettings::for_rule(Rule::BackslashBeforeClosingBacktick),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C069_fix_C069.sh", result);
        Ok(())
    }
}
