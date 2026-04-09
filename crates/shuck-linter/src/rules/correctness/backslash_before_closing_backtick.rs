use shuck_ast::Span;

use crate::{Checker, Rule, Violation};

pub struct BackslashBeforeClosingBacktick;

impl Violation for BackslashBeforeClosingBacktick {
    fn rule() -> Rule {
        Rule::BackslashBeforeClosingBacktick
    }

    fn message(&self) -> String {
        "remove the escaped trailing space before closing backtick".to_owned()
    }
}

pub fn backslash_before_closing_backtick(checker: &mut Checker) {
    let spans = checker
        .facts()
        .backtick_fragments()
        .iter()
        .filter_map(|fragment| {
            backslash_before_closing_backtick_span(fragment.span(), checker.source())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BackslashBeforeClosingBacktick);
}

fn backslash_before_closing_backtick_span(span: Span, source: &str) -> Option<Span> {
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
    let end = start.advanced_by("\\");
    Some(Span::from_positions(start, end))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\\"]
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
}
