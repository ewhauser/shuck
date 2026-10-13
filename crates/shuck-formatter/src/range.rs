use std::path::Path;

use shuck_ast::{Command, CompoundCommand, Span, Stmt, StmtSeq, TextRange};

use crate::{FormattedSource, Result, ShellFormatOptions, format_source, parse_and_resolve};

/// Result of formatting a range of shell source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedRange {
    /// The actual source range that was formatted (expanded to the smallest complete statement).
    pub range: TextRange,
    /// The formatted result for the statement.
    pub formatted: FormattedSource,
}

/// Formats the smallest statement in a shell source string covering the requested range.
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
    let parsed = parse_and_resolve(source, path, options)?;
    let Some(statement_span) = smallest_statement_span_containing(&parsed.file.body, range) else {
        return Ok(None);
    };
    let statement_range = statement_span.to_range();
    let statement_source =
        &source[usize::from(statement_range.start())..usize::from(statement_range.end())];
    let formatted = format_source(statement_source, path, options)?;

    Ok(Some(FormattedRange {
        range: statement_range,
        formatted,
    }))
}

fn smallest_statement_span_containing(body: &StmtSeq, range: TextRange) -> Option<Span> {
    let mut best = None;
    collect_statement_span(body, range, &mut best);
    best
}

fn collect_statement_span(body: &StmtSeq, range: TextRange, best: &mut Option<Span>) {
    for stmt in body.as_slice() {
        if !span_contains_text_range(stmt.span, range) {
            continue;
        }
        record_smaller_span(best, stmt.span);
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
    use crate::TextSize;

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
            FormattedSource::Formatted("echo two\n".to_owned())
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
