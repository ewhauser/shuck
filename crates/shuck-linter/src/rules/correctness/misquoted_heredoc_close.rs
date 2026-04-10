use shuck_ast::{Redirect, RedirectKind, Span};

use crate::{Checker, Rule, Violation};

pub struct MisquotedHeredocClose;

impl Violation for MisquotedHeredocClose {
    fn rule() -> Rule {
        Rule::MisquotedHeredocClose
    }

    fn message(&self) -> String {
        "this here-document closing marker is only a near match".to_owned()
    }
}

pub fn misquoted_heredoc_close(checker: &mut Checker) {
    let source = checker.source();
    let file_end = checker.ast().span.end.offset;
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirects().iter())
        .filter_map(|redirect| misquoted_heredoc_close_span(redirect, source, file_end))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MisquotedHeredocClose);
}

fn misquoted_heredoc_close_span(
    redirect: &Redirect,
    source: &str,
    file_end: usize,
) -> Option<Span> {
    if !matches!(
        redirect.kind,
        RedirectKind::HereDoc | RedirectKind::HereDocStrip
    ) {
        return None;
    }

    let heredoc = redirect.heredoc()?;
    if heredoc.body.span.end.offset != file_end {
        return None;
    }

    let delimiter = heredoc.delimiter.cooked.as_str();
    if delimiter.is_empty() {
        return None;
    }

    for raw_line in heredoc.body.span.slice(source).split_inclusive('\n') {
        let line_without_newline = raw_line.trim_end_matches('\n').trim_end_matches('\r');
        let candidate_line = if heredoc.delimiter.strip_tabs {
            line_without_newline.trim_start_matches('\t')
        } else {
            line_without_newline
        };
        if candidate_line == delimiter {
            continue;
        }

        if is_quoted_delimiter_variant(candidate_line, delimiter) {
            return Some(redirect.span);
        }
    }

    None
}

fn is_quoted_delimiter_variant(candidate_line: &str, delimiter: &str) -> bool {
    if candidate_line == delimiter {
        return false;
    }

    trim_quote_like_wrappers(candidate_line) == delimiter
}

fn trim_quote_like_wrappers(text: &str) -> &str {
    text.trim_matches(|ch| matches!(ch, '\'' | '"' | '\\'))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_close_match_for_quoted_delimiter() {
        let source = "\
#!/bin/bash
cat <<'BLOCK'
x
'BLOCK'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn ignores_content_plus_delimiter_lines_to_avoid_c144_overlap() {
        let source = "\
#!/bin/sh
cat <<EOF
x EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_properly_closed_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
x
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert!(diagnostics.is_empty());
    }
}
