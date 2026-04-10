use shuck_ast::{Position, Redirect, RedirectKind, Span};

use crate::{Checker, Rule, Violation};

pub struct HeredocEndSpace;

impl Violation for HeredocEndSpace {
    fn rule() -> Rule {
        Rule::HeredocEndSpace
    }

    fn message(&self) -> String {
        "remove trailing whitespace after this here-document terminator".to_owned()
    }
}

pub fn heredoc_end_space(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirects().iter())
        .filter_map(|redirect| heredoc_end_space_span(redirect, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || HeredocEndSpace);
}

fn heredoc_end_space_span(redirect: &Redirect, source: &str) -> Option<Span> {
    if !matches!(
        redirect.kind,
        RedirectKind::HereDoc | RedirectKind::HereDocStrip
    ) {
        return None;
    }

    let heredoc = redirect.heredoc()?;
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

        let Some(trailing) = candidate_line.strip_prefix(delimiter) else {
            line_start_offset += raw_line.len();
            continue;
        };
        if trailing.is_empty() || !trailing.chars().all(|ch| matches!(ch, ' ' | '\t')) {
            line_start_offset += raw_line.len();
            continue;
        }

        let trailing_start_offset = line_start_offset + tab_prefix_len + delimiter.len();
        let trailing_end_offset = trailing_start_offset + trailing.len();
        let start = position_at_offset(source, trailing_start_offset)?;
        let end = position_at_offset(source, trailing_end_offset)?;
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
    fn reports_trailing_space_after_heredoc_terminator() {
        let source = "\
#!/bin/sh
cat <<EOF
ok
EOF 
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), " ");
    }

    #[test]
    fn reports_trailing_tab_after_heredoc_terminator() {
        let source = "#!/bin/sh\ncat <<EOF\nok\nEOF\t\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), "\t");
    }

    #[test]
    fn reports_trailing_space_for_tab_stripped_heredoc_terminator() {
        let source = "\
#!/bin/sh
cat <<-EOF
\tvalue
\tEOF 
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), " ");
    }

    #[test]
    fn ignores_properly_terminated_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
ok
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_only_the_first_bad_terminator_per_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
value
EOF 
EOF\t
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), " ");
    }
}
