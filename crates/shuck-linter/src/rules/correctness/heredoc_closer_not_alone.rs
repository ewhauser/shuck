use shuck_ast::{Position, Redirect, RedirectKind, Span};

use crate::{Checker, Rule, Violation};

pub struct HeredocCloserNotAlone;

impl Violation for HeredocCloserNotAlone {
    fn rule() -> Rule {
        Rule::HeredocCloserNotAlone
    }

    fn message(&self) -> String {
        "this here-document closer must be on its own line".to_owned()
    }
}

pub fn heredoc_closer_not_alone(checker: &mut Checker) {
    let source = checker.source();
    let file_end = checker.ast().span.end.offset;
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirects().iter())
        .filter_map(|redirect| heredoc_closer_not_alone_span(redirect, source, file_end))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || HeredocCloserNotAlone);
}

fn heredoc_closer_not_alone_span(
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

    let mut line_start_offset = heredoc.body.span.start.offset;
    for raw_line in heredoc.body.span.slice(source).split_inclusive('\n') {
        let line_without_newline = raw_line.trim_end_matches('\n').trim_end_matches('\r');
        let (candidate_line, tab_prefix_len) = if heredoc.delimiter.strip_tabs {
            let trimmed = line_without_newline.trim_start_matches('\t');
            (trimmed, line_without_newline.len() - trimmed.len())
        } else {
            (line_without_newline, 0)
        };

        if !candidate_line.ends_with(delimiter) {
            line_start_offset += raw_line.len();
            continue;
        }

        let prefix = &candidate_line[..candidate_line.len() - delimiter.len()];
        if !prefix.chars().any(|ch| !ch.is_whitespace()) {
            line_start_offset += raw_line.len();
            continue;
        }

        let delimiter_start_offset = line_start_offset + tab_prefix_len + prefix.len();
        let delimiter_end_offset = delimiter_start_offset + delimiter.len();
        let start = position_at_offset(source, delimiter_start_offset)?;
        let end = position_at_offset(source, delimiter_end_offset)?;
        return Some(Span::from_positions(start, end));
    }

    None
}

fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }

    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_content_prefixed_terminator_lines() {
        let source = "\
#!/bin/sh
cat <<EOF
x EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.slice(source), "EOF");
    }

    #[test]
    fn reports_content_prefixed_terminators_for_tab_stripped_heredocs() {
        let source = "\
#!/bin/sh
cat <<-EOF
\tx EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.slice(source), "EOF");
    }

    #[test]
    fn ignores_properly_closed_heredocs() {
        let source = "\
#!/bin/sh
cat <<EOF
x
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert!(diagnostics.is_empty());
    }
}
