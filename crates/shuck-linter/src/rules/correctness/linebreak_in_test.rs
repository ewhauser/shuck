use shuck_ast::Span;

use crate::{Checker, Rule, Violation, static_word_text};

pub struct LinebreakInTest;

impl Violation for LinebreakInTest {
    fn rule() -> Rule {
        Rule::LinebreakInTest
    }

    fn message(&self) -> String {
        "`[` test spans lines without a trailing `\\` before the newline".to_owned()
    }
}

pub fn linebreak_in_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .windows(2)
        .filter_map(|pair| {
            let [current, next] = pair else {
                return None;
            };
            linebreak_in_test_span(current, next, checker.source())
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || LinebreakInTest);
}

fn linebreak_in_test_span(
    current: &crate::CommandFact<'_>,
    next: &crate::CommandFact<'_>,
    source: &str,
) -> Option<Span> {
    if !current.static_utility_name_is("[")
        || !next.static_utility_name_is("]")
        || !next.body_args().is_empty()
    {
        return None;
    }

    let last_arg_is_closing_bracket = current
        .body_args()
        .last()
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("]");
    if last_arg_is_closing_bracket || !current.span().slice(source).ends_with('\n') {
        return None;
    }
    let Some(between) = source.get(current.span().end.offset..next.span().start.offset) else {
        return None;
    };
    if !between.chars().all(|char| matches!(char, ' ' | '\t')) {
        return None;
    }

    let anchor = current
        .body_args()
        .last()
        .map(|word| word.span)
        .or_else(|| current.body_name_word().map(|word| word.span))
        .unwrap_or(current.span());
    span_end_point(anchor)
}

fn span_end_point(span: Span) -> Option<Span> {
    Some(Span::from_positions(span.end, span.end))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_linebreak_between_open_and_close_brackets() {
        let source = "#!/bin/sh\nif [ \"$x\" = y\n]; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 14);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }

    #[test]
    fn ignores_backslash_continued_test_lines() {
        let source = "#!/bin/sh\nif [ \"$x\" = y \\\n]; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_regular_single_line_bracket_tests() {
        let source = "#!/bin/sh\nif [ \"$x\" = y ]; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_other_missing_closing_bracket_shapes() {
        let source = "#!/bin/sh\nif [ \"$x\" = y; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_recovered_command_ordering_regression() {
        let source = concat!(
            "#!/bin/bash\n\n",
            "# Invalid: the quoted home-relative path stays literal in `[ ]`.\n",
            "[ \"$profile\" = \"~/.bashrc\" ]\n\n",
            "# Invalid: either side of the string comparison can carry the quoted `~/...`.\n",
            "[ \"~/.bash_profile\n",
            "[[ \"$profile\" == \"~/.zshrc\" ]]\n\n",
            "# Invalid: single quotes still prevent tilde expansion.\n",
            "[ \"$porfile\" != '~/.config/fish/config.fish' ]\n\n",
            "# Valid: an unquoted tilde expands before the comparison.\n",
            "[ \"$profile\" = ~/.bashr` ]\n\n",
            "# Valid: `~user` is a different lookup and not interchangeable printf '%s\\n' stamp)suffix\n\n",
            "printf '%s\\n' \"$(print`f '%s\\n' 'a b')\"\n",
            "stamp=$(printf '%s\\n' nowith `$HOME`.\n",
            "[ \"$profile\" = \"~user/.bashrc\" ]\n",
        );

        let _ = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));
    }
}
