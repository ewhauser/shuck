use shuck_ast::{Position, Redirect, RedirectKind, Span};

use crate::{Checker, Rule, Violation};

pub struct SpacedTabstripClose;

impl Violation for SpacedTabstripClose {
    fn rule() -> Rule {
        Rule::SpacedTabstripClose
    }

    fn message(&self) -> String {
        "this `<<-` closer must be indented with tabs only".to_owned()
    }
}

pub fn spaced_tabstrip_close(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirects().iter())
        .flat_map(|redirect| spaced_tabstrip_close_spans(redirect, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SpacedTabstripClose);
}

fn spaced_tabstrip_close_spans(redirect: &Redirect, source: &str) -> Vec<Span> {
    if redirect.kind != RedirectKind::HereDocStrip {
        return Vec::new();
    }

    let Some(heredoc) = redirect.heredoc() else {
        return Vec::new();
    };
    let delimiter = heredoc.delimiter.cooked.as_str();
    if delimiter.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut line_start_offset = heredoc.body.span.start.offset;
    for raw_line in heredoc.body.span.slice(source).split_inclusive('\n') {
        let line_without_newline = raw_line.trim_end_matches('\n').trim_end_matches('\r');
        if is_spaced_tabstrip_close_line(line_without_newline, delimiter)
            && let Some(position) = position_at_offset(source, line_start_offset)
        {
            spans.push(Span::from_positions(position, position));
        }
        line_start_offset += raw_line.len();
    }

    spans
}

fn is_spaced_tabstrip_close_line(line: &str, delimiter: &str) -> bool {
    if line.trim_start_matches('\t') == delimiter {
        return false;
    }

    let line_without_trailing_ws = line.trim_end_matches([' ', '\t']);
    let leading_len = line_without_trailing_ws.len()
        - line_without_trailing_ws
            .trim_start_matches([' ', '\t'])
            .len();
    if leading_len == 0 {
        return false;
    }

    let leading = &line_without_trailing_ws[..leading_len];
    let rest = &line_without_trailing_ws[leading_len..];
    leading.contains(' ') && rest == delimiter
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
    fn reports_space_indented_tabstrip_close_candidate() {
        let source = "\
#!/bin/sh
cat <<-END
x
  END
END
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn reports_mixed_tab_and_space_before_candidate_close() {
        let source = "\
#!/bin/sh
cat <<-END
x
\t END
END
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn ignores_tab_indented_close_candidates_without_spaces() {
        let source = "\
#!/bin/sh
cat <<-END
x
\tEND
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_plain_heredoc_close_candidates() {
        let source = "\
#!/bin/sh
cat <<END
x
  END
END
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_each_spaced_candidate_line() {
        let source = "\
#!/bin/sh
cat <<-END
x
  END
 \tEND
END
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[1].span.start.line, 5);
    }
}
