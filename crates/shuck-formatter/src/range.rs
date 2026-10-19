use std::ops::Range;
use std::path::Path;

use shuck_ast::{Command, CompoundCommand, Span, Stmt, StmtSeq, TextRange, TextSize};
use shuck_indexer::{IndexedHeredoc, RegionIndex};
use shuck_parser::parser::Parser;
use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::{FormattedSource, ResolvedShellFormatOptions, Result, ShellFormatOptions};

/// Result of formatting a range of shell source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedRange {
    /// The source range to replace with [`FormattedRange::formatted`].
    ///
    /// The range covers whole source lines that overlap the smallest complete statement
    /// containing the requested range.
    pub range: TextRange,
    /// The replacement text for [`FormattedRange::range`].
    pub formatted: FormattedSource,
}

/// Formats the smallest statement in a shell source string covering the requested range.
///
/// The edit is taken from formatting the whole file, restricted to the lines of the selected
/// statement (including any heredoc bodies it introduces), so the result matches what full
/// document formatting would produce on those lines. When the file has syntax errors outside
/// the selected statement, the enclosing top-level statements are formatted on their own with
/// the dialect inferred from the whole file instead.
///
/// Returns `Ok(None)` if no complete statement contains the requested range.
///
/// # Arguments
/// - `source`: The full shell script source string.
/// - `range`: The byte range to format.
/// - `path`: Optional file path for dialect and configuration inference.
/// - `options`: Formatting options.
pub fn format_range(
    source: &str,
    range: TextRange,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<Option<FormattedRange>> {
    let resolved = options.resolve_for_format(source, path);
    // Use recovered statements to locate the selection in an incomplete editor buffer.
    let parsed = Parser::with_dialect(source, resolved.dialect()).parse();
    let Some(statement_span) = smallest_statement_span_containing(&parsed.file.body, range) else {
        return Ok(None);
    };
    let regions = RegionIndex::with_source_layout_indexes(source, &parsed.file);
    let heredocs = regions.heredocs();
    let target = statement_lines(source, statement_span, heredocs);

    let top_level = top_level_region(source, &parsed.file.body, target.clone(), heredocs);

    let (region, formatted) = if parsed.is_ok() {
        let formatted = crate::format_file_ast_resolved(source, parsed.file, &resolved)?;
        (0..source.len(), formatted)
    } else {
        // The whole file cannot be formatted, so format the top-level statements that share
        // lines with the selection as a standalone script. Top-level statements format the
        // same way on their own because they sit at indentation level zero.
        let formatted = crate::format_source_resolved(&source[top_level.clone()], &resolved)?;
        (top_level.clone(), formatted)
    };
    let FormattedSource::Formatted(formatted) = formatted else {
        return Ok(Some(FormattedRange {
            range: text_range(target),
            formatted: FormattedSource::Unchanged,
        }));
    };

    // Line hunks are not always independent: the formatter can move text from one line to
    // another, such as a command that followed a heredoc operator. Only accept a partial edit
    // when formatting the edited region still yields the fully formatted region; otherwise
    // widen to the enclosing top-level statements and finally to the whole region.
    let original = &source[region.clone()];
    for lines in [target, top_level] {
        let edit = restrict_to_lines(source, region.clone(), &formatted, lines);
        if partial_edit_is_consistent(source, region.clone(), &edit, &formatted, &resolved)? {
            return Ok(Some(edit));
        }
    }
    Ok(Some(FormattedRange {
        range: text_range(region),
        formatted: if original == formatted {
            FormattedSource::Unchanged
        } else {
            FormattedSource::Formatted(formatted)
        },
    }))
}

/// Returns whether applying `edit` to `region` leaves text that formats to `formatted`.
fn partial_edit_is_consistent(
    source: &str,
    region: Range<usize>,
    edit: &FormattedRange,
    formatted: &str,
    resolved: &ResolvedShellFormatOptions,
) -> Result<bool> {
    let FormattedSource::Formatted(replacement) = &edit.formatted else {
        return Ok(true);
    };
    let edit_start = usize::from(edit.range.start());
    let edit_end = usize::from(edit.range.end());
    let edited = format!(
        "{}{}{}",
        &source[region.start..edit_start],
        replacement,
        &source[edit_end..region.end]
    );
    if edited == formatted {
        return Ok(true);
    }
    Ok(match crate::format_source_resolved(&edited, resolved) {
        Ok(FormattedSource::Formatted(reformatted)) => reformatted == formatted,
        Ok(FormattedSource::Unchanged) | Err(_) => false,
    })
}

/// Returns the line-aligned byte range covering `span` and any heredoc bodies that follow it.
fn statement_lines(source: &str, span: Span, heredocs: &[IndexedHeredoc]) -> Range<usize> {
    let start = line_start(source, span.start.offset());
    let last_byte = span.end.offset().saturating_sub(1).max(span.start.offset());
    let end = line_end(source, last_byte);
    extend_over_heredocs(source, start..end, heredocs)
}

/// Extends a line-aligned range through heredoc bodies introduced on its lines.
fn extend_over_heredocs(
    source: &str,
    mut lines: Range<usize>,
    heredocs: &[IndexedHeredoc],
) -> Range<usize> {
    loop {
        let mut end = lines.end;
        for heredoc in heredocs {
            let body_start = usize::from(heredoc.body_range.start());
            if body_start < lines.start || body_start > end {
                continue;
            }
            let body_end = match heredoc.closing_marker_range {
                Some(marker) => line_end(source, usize::from(marker.end())),
                None => usize::from(heredoc.body_range.end()),
            };
            end = end.max(body_end);
        }
        if end == lines.end {
            return lines;
        }
        lines.end = end;
    }
}

/// Grows `lines` until it covers every top-level statement it overlaps, line-aligned.
fn top_level_region(
    source: &str,
    body: &StmtSeq,
    mut lines: Range<usize>,
    heredocs: &[IndexedHeredoc],
) -> Range<usize> {
    loop {
        let mut grown = lines.clone();
        for stmt in body.as_slice() {
            let stmt_lines = statement_lines(source, stmt.span, heredocs);
            if stmt_lines.start < grown.end && grown.start < stmt_lines.end {
                grown.start = grown.start.min(stmt_lines.start);
                grown.end = grown.end.max(stmt_lines.end);
            }
        }
        if grown == lines {
            return lines;
        }
        lines = grown;
    }
}

fn line_start(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    source.as_bytes()[..offset]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |index| index + 1)
}

fn line_end(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    source.as_bytes()[offset..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(source.len(), |index| offset + index + 1)
}

/// A changed run of lines: old line indices replaced by new line indices.
#[derive(Debug, Clone)]
struct Hunk {
    old: Range<usize>,
    new: Range<usize>,
}

/// Converts the formatted `region` into a single edit limited to the `target` lines.
///
/// Changed hunks that do not touch `target` are dropped, so formatting one statement never
/// rewrites unrelated lines.
fn restrict_to_lines(
    source: &str,
    region: Range<usize>,
    formatted: &str,
    target: Range<usize>,
) -> FormattedRange {
    let unchanged = FormattedRange {
        range: text_range(target.clone()),
        formatted: FormattedSource::Unchanged,
    };

    let original = &source[region.clone()];
    let old_lines = original.split_inclusive('\n').collect::<Vec<_>>();
    let new_lines = formatted.split_inclusive('\n').collect::<Vec<_>>();
    let old_offsets = line_offsets(&old_lines);
    let new_offsets = line_offsets(&new_lines);

    let target_start = target.start.saturating_sub(region.start);
    let target_end = target.end.saturating_sub(region.start);
    let first_line = old_offsets.partition_point(|&offset| offset < target_start);
    let end_line = old_offsets[..old_lines.len()].partition_point(|&offset| offset < target_end);
    let target_lines = first_line..end_line.max(first_line);

    let hunks = diff_hunks(&old_lines, &new_lines);
    let mut touched = hunks.iter().filter(|hunk| {
        if hunk.old.is_empty() {
            target_lines.contains(&hunk.old.start) || hunk.old.start == target_lines.end
        } else {
            hunk.old.start < target_lines.end && target_lines.start < hunk.old.end
        }
    });
    let Some(first) = touched.next() else {
        return unchanged;
    };
    let last = touched.next_back().unwrap_or(first);

    let old_range =
        region.start + old_offsets[first.old.start]..region.start + old_offsets[last.old.end];
    let replacement = &formatted[new_offsets[first.new.start]..new_offsets[last.new.end]];
    FormattedRange {
        range: text_range(old_range),
        formatted: FormattedSource::Formatted(replacement.to_owned()),
    }
}

fn diff_hunks(old_lines: &[&str], new_lines: &[&str]) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, old_lines, new_lines) {
        let (old, new) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index..old_index + old_len, new_index..new_index),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index..old_index, new_index..new_index + new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (
                old_index..old_index + old_len,
                new_index..new_index + new_len,
            ),
        };

        // A block whose lines only changed whitespace (typically re-indentation) is split
        // into per-line hunks so selecting one line does not reformat its neighbours.
        let line_for_line = old.len() == new.len()
            && old
                .clone()
                .zip(new.clone())
                .all(|(old, new)| same_non_whitespace(old_lines[old], new_lines[new]));
        if line_for_line && old.len() > 1 {
            hunks.extend(old.zip(new).map(|(old, new)| Hunk {
                old: old..old + 1,
                new: new..new + 1,
            }));
        } else {
            hunks.push(Hunk { old, new });
        }
    }
    hunks
}

fn same_non_whitespace(left: &str, right: &str) -> bool {
    left.chars()
        .filter(|ch| !ch.is_whitespace())
        .eq(right.chars().filter(|ch| !ch.is_whitespace()))
}

fn line_offsets(lines: &[&str]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(lines.len() + 1);
    let mut offset = 0;
    offsets.push(offset);
    for line in lines {
        offset += line.len();
        offsets.push(offset);
    }
    offsets
}

fn text_range(range: Range<usize>) -> TextRange {
    TextRange::new(text_size(range.start), text_size(range.end))
}

fn text_size(offset: usize) -> TextSize {
    TextSize::new(u32::try_from(offset).unwrap_or(u32::MAX))
}

fn smallest_statement_span_containing(body: &StmtSeq, range: TextRange) -> Option<Span> {
    let mut best = None;
    collect_statement_span(body, range, &mut best);
    best
}

fn collect_statement_span(body: &StmtSeq, range: TextRange, best: &mut Option<Span>) {
    for stmt in body.as_slice() {
        // Selecting a statement's own `;` or `&` still selects the statement.
        let mut extent = stmt.span;
        if let Some(terminator) = stmt.terminator_span
            && terminator.end.offset() > extent.end.offset()
        {
            extent.end = terminator.end;
        }
        if !span_contains_text_range(extent, range) {
            continue;
        }
        record_smaller_span(best, extent);
        collect_nested_statement_spans(stmt, range, best);
    }
}

fn collect_nested_statement_spans(stmt: &Stmt, range: TextRange, best: &mut Option<Span>) {
    match &stmt.command {
        Command::Binary(command) => {
            collect_nested_statement_spans(&command.left, range, best);
            collect_nested_statement_spans(&command.right, range, best);
        }
        Command::Compound(command) => collect_compound_statement_spans(command, range, best),
        Command::Function(command) => collect_nested_statement_spans(&command.body, range, best),
        Command::AnonymousFunction(command) => {
            collect_nested_statement_spans(&command.body, range, best);
        }
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
    }
}

fn collect_compound_statement_spans(
    command: &CompoundCommand,
    range: TextRange,
    best: &mut Option<Span>,
) {
    match command {
        CompoundCommand::If(command) => {
            collect_statement_span(&command.condition, range, best);
            collect_statement_span(&command.then_branch, range, best);
            for (condition, body) in &command.elif_branches {
                collect_statement_span(condition, range, best);
                collect_statement_span(body, range, best);
            }
            if let Some(body) = &command.else_branch {
                collect_statement_span(body, range, best);
            }
        }
        CompoundCommand::For(command) => collect_statement_span(&command.body, range, best),
        CompoundCommand::Repeat(command) => collect_statement_span(&command.body, range, best),
        CompoundCommand::Foreach(command) => collect_statement_span(&command.body, range, best),
        CompoundCommand::ArithmeticFor(command) => {
            collect_statement_span(&command.body, range, best)
        }
        CompoundCommand::While(command) => {
            collect_statement_span(&command.condition, range, best);
            collect_statement_span(&command.body, range, best);
        }
        CompoundCommand::Until(command) => {
            collect_statement_span(&command.condition, range, best);
            collect_statement_span(&command.body, range, best);
        }
        CompoundCommand::Case(command) => {
            for item in &command.cases {
                collect_statement_span(&item.body, range, best);
            }
        }
        CompoundCommand::Select(command) => collect_statement_span(&command.body, range, best),
        CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
            collect_statement_span(body, range, best);
        }
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_nested_statement_spans(command, range, best);
            }
        }
        CompoundCommand::Coproc(command) => {
            collect_nested_statement_spans(&command.body, range, best);
        }
        CompoundCommand::Always(command) => {
            collect_statement_span(&command.body, range, best);
            collect_statement_span(&command.always_body, range, best);
        }
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
    }
}

fn record_smaller_span(best: &mut Option<Span>, candidate: Span) {
    if best.is_none_or(|current| span_width(candidate) < span_width(current)) {
        *best = Some(candidate);
    }
}

fn span_width(span: Span) -> usize {
    span.end.offset().saturating_sub(span.start.offset())
}

fn span_contains_text_range(span: Span, range: TextRange) -> bool {
    span.start.offset() <= usize::from(range.start())
        && usize::from(range.end()) <= span.end.offset()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_source;

    fn range(start: u32, end: u32) -> TextRange {
        TextRange::new(TextSize::new(start), TextSize::new(end))
    }

    fn range_of(source: &str, needle: &str) -> TextRange {
        let start = source.find(needle).expect("needle should be in source");
        text_range(start..start + needle.len())
    }

    /// Formats `requested` and applies the resulting edit to `source`.
    fn apply_range(source: &str, requested: TextRange, path: Option<&Path>) -> String {
        let result = format_range(source, requested, path, &ShellFormatOptions::default())
            .expect("range formatting should succeed")
            .expect("a statement should contain the range");
        match result.formatted {
            FormattedSource::Unchanged => source.to_owned(),
            FormattedSource::Formatted(text) => {
                let start = usize::from(result.range.start());
                let end = usize::from(result.range.end());
                format!("{}{}{}", &source[..start], text, &source[end..])
            }
        }
    }

    fn format_whole(source: &str, path: Option<&Path>) -> String {
        match format_source(source, path, &ShellFormatOptions::default())
            .expect("whole-file formatting should succeed")
        {
            FormattedSource::Unchanged => source.to_owned(),
            FormattedSource::Formatted(text) => text,
        }
    }

    #[test]
    fn format_range_keeps_heredoc_body_with_its_command() {
        let source = "cat   <<EOF\nbody\nEOF\necho   x\n";
        assert_eq!(
            apply_range(source, range(0, 3), None),
            "cat <<EOF\nbody\nEOF\necho   x\n"
        );
    }

    #[test]
    fn format_range_keeps_heredoc_body_when_file_has_syntax_errors() {
        let source = "cat   <<EOF\nbody\nEOF\necho   x\nif true; then\n";
        assert_eq!(
            apply_range(source, range(0, 3), None),
            "cat <<EOF\nbody\nEOF\necho   x\nif true; then\n"
        );
    }

    #[test]
    fn format_range_keeps_heredoc_body_after_later_command_on_line() {
        let source = "cat <<EOF;   echo   y\nbody\nEOF\n";
        let requested = range_of(source, "echo");
        assert_eq!(
            apply_range(source, requested, None),
            format_whole(source, None)
        );
    }

    #[test]
    fn format_range_uses_dialect_from_whole_file_shebang() {
        let source = "#!/bin/bash\n((x=5, y=x*2)); echo $y\n";
        let path = Path::new("script.sh");
        let formatted = apply_range(source, range(25, 27), Some(path));
        assert_eq!(formatted, format_whole(source, Some(path)));
        assert!(formatted.contains("\n((x"), "{formatted:?}");
    }

    #[test]
    fn format_range_uses_zsh_dialect_from_shebang() {
        let source = "#!/bin/zsh\nfor f (a b) print   $f\n";
        let path = Path::new("script.sh");
        let requested = range_of(source, "print");
        assert_eq!(
            apply_range(source, requested, Some(path)),
            format_whole(source, Some(path))
        );
    }

    #[test]
    fn format_range_uses_whole_file_dialect_when_file_has_syntax_errors() {
        let source = "#!/bin/zsh\nfor f (a b) print   $f\nif true; then\n";
        let path = Path::new("script.sh");
        let requested = range_of(source, "print");
        let formatted = apply_range(source, requested, Some(path));
        let expected_line = format_whole("#!/bin/zsh\nfor f (a b) print   $f\n", Some(path));
        assert_eq!(formatted, format!("{expected_line}if true; then\n"));
    }

    #[test]
    fn format_range_preserves_separator_inside_one_line_compound() {
        let source = "if true; then echo   two; fi\n";
        let requested = range_of(source, "echo");
        assert_eq!(
            apply_range(source, requested, None),
            format_whole(source, None)
        );
    }

    #[test]
    fn format_range_formats_case_arm_statement() {
        let source = "case x in\na) echo   x;;\nesac\n";
        let requested = range_of(source, "echo");
        assert_eq!(
            apply_range(source, requested, None),
            format_whole(source, None)
        );
    }

    #[test]
    fn format_range_indents_nested_statement_at_its_depth() {
        let source = "if   true; then\nif   true; then\necho hi\nfi\nfi\n";
        let requested = range(16, 31);
        assert_eq!(
            apply_range(source, requested, None),
            "if   true; then\n\tif true; then\n\t\techo hi\n\tfi\nfi\n"
        );
    }

    #[test]
    fn format_range_indents_nested_statement_when_file_has_syntax_errors() {
        let source = "if true; then\nif   true; then\necho hi\nfi\nfi\nif\n";
        let requested = range(14, 29);
        assert_eq!(
            apply_range(source, requested, None),
            "if true; then\n\tif true; then\n\t\techo hi\n\tfi\nfi\nif\n"
        );
    }

    #[test]
    fn format_range_leaves_other_lines_alone() {
        let source = "echo   a\necho   b\necho   c\n";
        let requested = range_of(source, "echo   b");
        assert_eq!(
            apply_range(source, requested, None),
            "echo   a\necho b\necho   c\n"
        );
    }

    #[test]
    fn format_range_reindents_only_selected_line_of_block() {
        let source = "if true; then\n  echo a\n  echo b\nfi\n";
        let requested = range_of(source, "echo b");
        assert_eq!(
            apply_range(source, requested, None),
            "if true; then\n  echo a\n\techo b\nfi\n"
        );
    }

    #[test]
    fn format_range_accepts_range_inside_a_token() {
        let source = "echo   hi\n";
        assert_eq!(apply_range(source, range(1, 2), None), "echo hi\n");
    }

    #[test]
    fn format_range_accepts_range_inside_a_multibyte_char() {
        let source = "echo   h\u{e9}llo\n";
        let inside =
            u32::try_from(source.find('\u{e9}').expect("accented char") + 1).expect("offset fits");
        assert!(!source.is_char_boundary(inside as usize));
        assert_eq!(
            apply_range(source, range(inside, inside), None),
            "echo h\u{e9}llo\n"
        );
        assert_eq!(
            apply_range(source, range(2, inside), None),
            "echo h\u{e9}llo\n"
        );
    }

    #[test]
    fn format_range_edits_reformat_to_whole_file_output_at_every_offset() {
        let sources = [
            "cat   <<EOF;   echo   y\nbody\nEOF\necho   z\n",
            "if true; then echo   two; fi\nfor x in a b; do\n  echo   $x &\ndone\n",
            "case x in\na) echo   x;;\n  b)   echo y ;;\nesac\n",
            "f() {\n  cat <<-EOF |   grep h\u{e9}\n\tbody\n\tEOF\n  echo   \u{e9}t\u{e9}\n}\n",
            "#!/bin/bash\n((x=5, y=x*2)); echo $y\n[[ -n $y ]]   &&   echo   ok\n",
        ];
        for source in sources {
            let expected = format_whole(source, None);
            for offset in 0..=source.len() {
                let offset = u32::try_from(offset).expect("offset fits");
                let Some(result) = format_range(
                    source,
                    range(offset, offset),
                    None,
                    &ShellFormatOptions::default(),
                )
                .expect("range formatting should succeed") else {
                    continue;
                };
                let edited = match result.formatted {
                    FormattedSource::Unchanged => source.to_owned(),
                    FormattedSource::Formatted(text) => {
                        let start = usize::from(result.range.start());
                        let end = usize::from(result.range.end());
                        format!("{}{}{}", &source[..start], text, &source[end..])
                    }
                };
                assert_eq!(
                    format_whole(&edited, None),
                    expected,
                    "source {source:?} offset {offset} produced {edited:?}"
                );
            }
        }
    }

    #[test]
    fn format_range_returns_none_for_range_past_end_of_file() {
        let source = "echo hi\n";
        let result = format_range(source, range(20, 30), None, &ShellFormatOptions::default())
            .expect("range formatting should succeed");
        assert!(result.is_none());
    }

    #[test]
    fn format_range_formats_statement_before_unrelated_syntax_error() {
        let source = "echo   hi\nif true; then\n";
        let result = format_range(
            source,
            TextRange::new(TextSize::new(0), TextSize::new(9)),
            None,
            &ShellFormatOptions::default(),
        )
        .expect("an unrelated syntax error should not prevent formatting")
        .expect("the valid statement should be selected");

        assert_eq!(
            result.range,
            TextRange::new(TextSize::new(0), TextSize::new(10))
        );
        assert_eq!(
            result.formatted,
            FormattedSource::Formatted("echo hi\n".to_owned())
        );
    }

    #[test]
    fn format_range_does_not_format_incomplete_selected_statement() {
        let source = "if true; then\necho   hi\n";
        let result = format_range(
            source,
            TextRange::new(TextSize::new(0), TextSize::new(source.len() as u32)),
            None,
            &ShellFormatOptions::default(),
        );

        assert!(matches!(result, Err(_) | Ok(None)));
    }

    #[test]
    fn format_range_formats_targeted_statement() {
        let source = "echo one\nif true; then\necho   two\nfi\n";
        // Target "echo   two" which starts at offset 23 and ends at 34
        let range = TextRange::new(TextSize::new(23), TextSize::new(33));
        let options = ShellFormatOptions::default();

        let result = format_range(source, range, None, &options)
            .expect("format_range should succeed")
            .expect("should find statement containing range");

        assert_eq!(
            result.range,
            TextRange::new(TextSize::new(23), TextSize::new(34))
        );
        assert_eq!(
            result.formatted,
            FormattedSource::Formatted("\techo two\n".to_owned())
        );
    }

    #[test]
    fn format_range_returns_unchanged_for_already_formatted_statement() {
        let source = "echo one\necho two\n";
        let range = TextRange::new(TextSize::new(9), TextSize::new(17));
        let options = ShellFormatOptions::default();

        let result = format_range(source, range, None, &options)
            .expect("format_range should succeed")
            .expect("should find statement containing range");

        assert_eq!(
            result.range,
            TextRange::new(TextSize::new(9), TextSize::new(18))
        );
        assert_eq!(result.formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn format_range_returns_none_for_range_spanning_partial_statements() {
        let source = "echo one\necho two\n";
        // Range from middle of first statement to middle of second
        let range = TextRange::new(TextSize::new(2), TextSize::new(12));
        let options = ShellFormatOptions::default();

        let result =
            format_range(source, range, None, &options).expect("format_range should succeed");

        assert!(result.is_none());
    }
}
