use shuck_ast::Span;

use crate::raw_syntax::{RawShellScanner, matching_raw_command_substitution_close};

#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceView<'source> {
    source: &'source str,
}

impl<'source> SourceView<'source> {
    pub(crate) fn new(source: &'source str) -> Self {
        Self { source }
    }

    pub(crate) fn slice_between(self, start: usize, end: usize) -> Option<&'source str> {
        let lower = start.min(end).min(self.source.len());
        let upper = start.max(end).min(self.source.len());
        self.source.get(lower..upper)
    }

    pub(crate) fn span_slice(self, span: Span) -> Option<&'source str> {
        if span.start.offset >= span.end.offset || span.end.offset > self.source.len() {
            return None;
        }
        Some(span.slice(self.source))
    }

    pub(crate) fn line_indent_before_offset(self, offset: usize) -> Option<&'source str> {
        let offset = offset.min(self.source.len());
        let line_start = self.line_start_before(offset)?;
        let prefix = self.source.get(line_start..offset)?;
        let indent_end = prefix
            .char_indices()
            .find(|(_, ch)| !matches!(ch, ' ' | '\t'))
            .map_or(prefix.len(), |(index, _)| index);
        prefix.get(..indent_end)
    }

    pub(crate) fn line_has_shell_comment_before(self, offset: usize) -> bool {
        let upper = offset.min(self.source.len());
        let Some(line_start) = self.line_start_before(upper) else {
            return false;
        };
        self.shell_comment_start_between(line_start, upper)
            .is_some()
    }

    pub(crate) fn shell_comment_start_between(self, start: usize, end: usize) -> Option<usize> {
        let lower = start.min(end).min(self.source.len());
        let upper = start.max(end).min(self.source.len());
        RawShellScanner::bounded(self.source, upper).find_comment(lower, upper)
    }

    pub(crate) fn branch_keyword_offset(
        self,
        start: usize,
        end: usize,
        keyword: &str,
    ) -> Option<usize> {
        let start = start.min(end).min(self.source.len());
        let end = end.min(self.source.len());
        let mut line_start = start;
        while line_start < end {
            let line_end = self.source[line_start..end]
                .find('\n')
                .map_or(end, |offset| line_start + offset);
            let line = self.source.get(line_start..line_end)?;
            let mut search_start = 0;
            while let Some(relative) = line[search_start..].find(keyword) {
                let keyword_start = search_start + relative;
                let keyword_end = keyword_start + keyword.len();
                if branch_keyword_candidate_matches(line, keyword_start, keyword_end) {
                    return Some(line_start + keyword_start);
                }
                search_start = keyword_end;
            }
            line_start = line_end.saturating_add(1);
        }
        None
    }

    pub(crate) fn last_uncommented_shell_keyword_before(
        self,
        search_end: usize,
        keyword: &str,
    ) -> Option<usize> {
        let mut search_end = search_end.min(self.source.len());
        loop {
            let offset = self.source.get(..search_end)?.rfind(keyword)?;
            let end = offset + keyword.len();
            if shell_keyword_boundaries_match(self.source, offset, end)
                && !self.line_has_shell_comment_before(offset)
            {
                return Some(offset);
            }
            search_end = offset;
        }
    }

    pub(crate) fn last_shell_keyword_start(self, span: Span, keyword: &str) -> Option<usize> {
        let upper = span.end.offset.min(self.source.len());
        let lower = span.start.offset.min(upper);
        self.last_shell_keyword_start_between(lower, upper, keyword)
    }

    pub(crate) fn last_shell_keyword_start_between(
        self,
        lower: usize,
        upper: usize,
        keyword: &str,
    ) -> Option<usize> {
        let upper = upper.min(self.source.len());
        let lower = lower.min(upper);
        let slice = self.source.get(lower..upper)?;
        slice
            .match_indices(keyword)
            .filter_map(|(start, _)| {
                let end = start + keyword.len();
                shell_keyword_boundaries_match(slice, start, end).then_some(lower + start)
            })
            .last()
    }

    pub(crate) fn shell_keyword_at(self, offset: usize, upper: usize, keyword: &str) -> bool {
        let end = offset.saturating_add(keyword.len());
        end <= upper
            && self.source.get(offset..end) == Some(keyword)
            && shell_keyword_boundaries_match(self.source, offset, end)
    }

    pub(crate) fn matching_if_close_start(self, span: Span) -> Option<usize> {
        self.matching_close_keyword_start(span, "fi", |source, offset, upper| {
            shell_control_keyword_at(source, offset, upper, "if").then_some("if".len())
        })
    }

    pub(crate) fn matching_done_close_start(self, span: Span) -> Option<usize> {
        self.matching_close_keyword_start(span, "done", |source, offset, upper| {
            ["for", "select", "while", "until", "foreach", "repeat"]
                .iter()
                .find(|keyword| shell_control_keyword_at(source, offset, upper, keyword))
                .map(|_| {
                    source[offset..]
                        .chars()
                        .take_while(char::is_ascii_alphabetic)
                        .map(char::len_utf8)
                        .sum()
                })
        })
    }

    pub(crate) fn dollar_command_substitution(
        self,
    ) -> Option<DollarCommandSubstitutionSource<'source>> {
        self.source.strip_prefix("$(")?;
        let close_offset = matching_raw_command_substitution_close(self.source, 2)?;
        Some(DollarCommandSubstitutionSource {
            view: self,
            close_offset,
        })
    }

    pub(crate) fn substitution_closes_on_own_line(self) -> bool {
        let Some(close_offset) = self.source.rfind(')') else {
            return false;
        };
        let line_start = self.source[..close_offset]
            .rfind('\n')
            .map_or(0, |newline| newline.saturating_add(1));
        line_start > 0 && self.source[line_start..close_offset].trim().is_empty()
    }

    fn line_start_before(self, offset: usize) -> Option<usize> {
        let prefix = self.source.get(..offset)?;
        Some(
            prefix
                .rfind('\n')
                .map_or(0, |newline| newline.saturating_add(1)),
        )
    }

    fn matching_close_keyword_start(
        self,
        span: Span,
        close_keyword: &str,
        mut open_len_at: impl FnMut(&str, usize, usize) -> Option<usize>,
    ) -> Option<usize> {
        let upper = span.end.offset.min(self.source.len());
        let mut offset = span.start.offset.min(upper);
        let mut depth = 0usize;
        let scanner = RawShellScanner::bounded(self.source, upper);
        while offset < upper {
            let ch = self.source[offset..].chars().next()?;
            if let Some(next) = scanner.skip_quoted_or_comment_at(offset) {
                offset = next;
                continue;
            }

            if let Some(open_len) = open_len_at(self.source, offset, upper) {
                depth = depth.saturating_add(1);
                offset += open_len;
                continue;
            }
            if shell_control_keyword_at(self.source, offset, upper, close_keyword) {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        return Some(offset);
                    }
                }
                offset += close_keyword.len();
                continue;
            }
            offset += ch.len_utf8();
        }
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DollarCommandSubstitutionSource<'source> {
    view: SourceView<'source>,
    close_offset: usize,
}

impl<'source> DollarCommandSubstitutionSource<'source> {
    pub(crate) fn body(self) -> Option<&'source str> {
        self.view.source.get(2..self.close_offset)
    }

    pub(crate) fn closed_slice(self) -> Option<&'source str> {
        self.view.source.get(..self.close_offset.saturating_add(1))
    }

    pub(crate) fn closes_on_own_line(self) -> bool {
        self.view.substitution_closes_on_own_line()
    }
}

pub(crate) fn dollar_command_substitution_body(raw: &str) -> Option<&str> {
    SourceView::new(raw).dollar_command_substitution()?.body()
}

pub(crate) fn dollar_command_substitution_slice(raw: &str) -> Option<&str> {
    SourceView::new(raw)
        .dollar_command_substitution()?
        .closed_slice()
}

pub(crate) fn command_substitution_source_starts_with_body_line(raw: &str) -> bool {
    raw.starts_with(['\n', '\r'])
        || raw
            .strip_prefix("$(")
            .is_some_and(|after_open| after_open.starts_with(['\n', '\r']))
}

pub(crate) fn command_substitution_source_closes_on_own_line(raw: &str) -> bool {
    SourceView::new(raw)
        .dollar_command_substitution()
        .is_some_and(DollarCommandSubstitutionSource::closes_on_own_line)
}

pub(crate) fn command_substitution_source_prefers_continued_inline_body(raw: &str) -> bool {
    let Some(after_open) = raw.strip_prefix("$(") else {
        return false;
    };
    if after_open.starts_with(['\n', '\r']) {
        return false;
    }

    raw.lines()
        .any(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with('\\'))
}

pub(crate) fn substitution_source_closes_on_own_line(raw: &str) -> bool {
    SourceView::new(raw).substitution_closes_on_own_line()
}

fn branch_keyword_candidate_matches(line: &str, start: usize, end: usize) -> bool {
    if !shell_keyword_boundaries_match(line, start, end) {
        return false;
    }

    let prefix = &line[..start];
    let trimmed = prefix.trim_start_matches([' ', '\t']);
    if trimmed.starts_with('#') {
        return false;
    }

    let before = prefix.trim_end_matches([' ', '\t']);
    before.is_empty() || before.ends_with(';') || before.ends_with('&')
}

fn shell_control_keyword_at(source: &str, offset: usize, upper: usize, keyword: &str) -> bool {
    SourceView::new(source).shell_keyword_at(offset, upper, keyword)
        && shell_keyword_has_command_prefix(source, offset)
}

fn shell_keyword_has_command_prefix(source: &str, offset: usize) -> bool {
    let prefix = &source[..offset];
    let Some((previous_offset, previous)) = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t' | '\r'))
    else {
        return true;
    };

    if previous == '\n' || matches!(previous, ';' | '&' | '|' | '(' | ')' | '{' | '!') {
        return true;
    }

    let word_end = previous_offset + previous.len_utf8();
    let word_start = prefix[..word_end]
        .char_indices()
        .rev()
        .find(|(_, ch)| !is_shell_keyword_char(*ch))
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    matches!(
        &prefix[word_start..word_end],
        "do" | "then" | "else" | "elif" | "time" | "coproc"
    )
}

fn shell_keyword_boundaries_match(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    before.is_none_or(|ch| !is_shell_keyword_char(ch))
        && after.is_none_or(|ch| !is_shell_keyword_char(ch))
}

fn is_shell_keyword_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_substitution_view_slices_body_with_nested_close() {
        let raw = "$(echo $(date); echo done) trailing";
        let view = SourceView::new(raw).dollar_command_substitution().unwrap();

        assert_eq!(view.body(), Some("echo $(date); echo done"));
        assert_eq!(view.closed_slice(), Some("$(echo $(date); echo done)"));
    }

    #[test]
    fn command_substitution_view_reports_source_shape() {
        assert!(command_substitution_source_starts_with_body_line(
            "$(\necho ok\n)"
        ));
        assert!(command_substitution_source_closes_on_own_line(
            "$(echo ok\n)"
        ));
        assert!(!command_substitution_source_prefers_continued_inline_body(
            "$(\necho ok \\"
        ));
        assert!(command_substitution_source_prefers_continued_inline_body(
            "$(echo ok \\\n  && echo again)"
        ));
    }

    #[test]
    fn source_view_skips_comments_when_matching_close_keywords() {
        let source = "if true; then\n  echo fi # fi\nfi\n";
        let span = Span {
            start: shuck_ast::Position {
                line: 1,
                column: 1,
                offset: 0,
            },
            end: shuck_ast::Position {
                line: 3,
                column: 3,
                offset: source.len(),
            },
        };

        assert_eq!(
            SourceView::new(source).matching_if_close_start(span),
            source.rfind("fi")
        );
    }
}
