use std::io::{self, Write};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use annotate_snippets::{AnnotationType, Renderer, Slice, Snippet, SourceAnnotation};
use colored::{ColoredString, Colorize};
use shuck_indexer::LineIndex;

use crate::args::CheckOutputFormatArg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisplayPosition {
    pub(super) line: usize,
    pub(super) column: usize,
}

impl DisplayPosition {
    pub(super) const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisplaySpan {
    pub(super) start: DisplayPosition,
    pub(super) end: DisplayPosition,
}

impl DisplaySpan {
    pub(super) const fn new(start: DisplayPosition, end: DisplayPosition) -> Self {
        Self { start, end }
    }

    pub(super) const fn point(line: usize, column: usize) -> Self {
        let position = DisplayPosition::new(line, column);
        Self::new(position, position)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DisplayedDiagnosticKind {
    ParseError,
    Lint { code: String, severity: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DisplayedDiagnostic {
    pub(super) path: PathBuf,
    pub(super) span: DisplaySpan,
    pub(super) message: String,
    pub(super) kind: DisplayedDiagnosticKind,
    pub(super) source: Option<Arc<str>>,
}

pub(super) fn print_report_to(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
    output_format: CheckOutputFormatArg,
    use_color: bool,
) -> io::Result<()> {
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        match output_format {
            CheckOutputFormatArg::Full => {
                if index > 0 {
                    writeln!(writer)?;
                }

                let rendered = format_full_diagnostic(diagnostic, use_color);
                writer.write_all(rendered.as_bytes())?;
                if !rendered.ends_with('\n') {
                    writeln!(writer)?;
                }
            }
            CheckOutputFormatArg::Concise => {
                writeln!(
                    writer,
                    "{}",
                    format_concise_diagnostic(diagnostic, use_color)
                )?;
            }
        }
    }

    Ok(())
}

fn format_full_diagnostic(diagnostic: &DisplayedDiagnostic, use_color: bool) -> String {
    let Some(source) = diagnostic.source.as_deref() else {
        return format!("{}\n", format_concise_diagnostic(diagnostic, use_color));
    };
    let Some(snippet) = renderable_snippet(diagnostic.span, source) else {
        return format!("{}\n", format_concise_diagnostic(diagnostic, use_color));
    };

    let header = format_full_header(diagnostic, use_color);
    let origin = diagnostic.path.display().to_string();
    let snippet = Snippet {
        title: None,
        footer: vec![],
        slices: vec![Slice {
            source: snippet.source,
            line_start: snippet.line_start,
            origin: Some(origin.as_str()),
            fold: false,
            annotations: vec![SourceAnnotation {
                label: "",
                annotation_type: annotation_type(diagnostic),
                range: (snippet.range.start, snippet.range.end),
            }],
        }],
    };
    let renderer = if use_color {
        Renderer::styled()
    } else {
        Renderer::plain()
    };
    let rendered = renderer.render(snippet).to_string();

    format!("{header}\n{rendered}")
}

fn format_full_header(diagnostic: &DisplayedDiagnostic, use_color: bool) -> String {
    match &diagnostic.kind {
        DisplayedDiagnosticKind::ParseError => format!(
            "{}[{}]: {}",
            paint("error".to_owned(), use_color, |value| value.red().bold()),
            paint("parse-error".to_owned(), use_color, |value| value
                .red()
                .bold()),
            diagnostic.message
        ),
        DisplayedDiagnosticKind::Lint { code, severity } => format!(
            "{}[{}]: {}",
            format_severity(severity, use_color),
            paint(code.clone(), use_color, |value| value.cyan().bold()),
            diagnostic.message
        ),
    }
}

fn format_concise_diagnostic(diagnostic: &DisplayedDiagnostic, use_color: bool) -> String {
    let path = paint(diagnostic.path.display().to_string(), use_color, |value| {
        value.bold()
    });
    let line = paint(diagnostic.span.start.line.to_string(), use_color, |value| {
        value.cyan()
    });
    let column = paint(
        diagnostic.span.start.column.to_string(),
        use_color,
        |value| value.cyan(),
    );

    match &diagnostic.kind {
        DisplayedDiagnosticKind::ParseError => {
            let label = paint("parse error".to_owned(), use_color, |value| {
                value.red().bold()
            });
            format!("{path}:{line}:{column}: {label} {}", diagnostic.message)
        }
        DisplayedDiagnosticKind::Lint { code, severity } => {
            let severity = format_severity(severity, use_color);
            let code = paint(code.clone(), use_color, |value| value.cyan().bold());
            format!(
                "{path}:{line}:{column}: {severity}[{code}] {}",
                diagnostic.message
            )
        }
    }
}

fn format_severity(severity: &str, use_color: bool) -> String {
    paint(severity.to_owned(), use_color, |value| match severity {
        "error" => value.red().bold(),
        "warning" => value.yellow().bold(),
        "info" | "hint" => value.blue().bold(),
        _ => value.bold(),
    })
}

fn paint(
    value: String,
    use_color: bool,
    style: impl FnOnce(ColoredString) -> ColoredString,
) -> String {
    if use_color {
        style(value.normal()).to_string()
    } else {
        value
    }
}

fn annotation_type(diagnostic: &DisplayedDiagnostic) -> AnnotationType {
    match &diagnostic.kind {
        DisplayedDiagnosticKind::ParseError => AnnotationType::Error,
        DisplayedDiagnosticKind::Lint { .. } => AnnotationType::Error,
    }
}

struct RenderableSnippet<'a> {
    source: &'a str,
    line_start: usize,
    range: Range<usize>,
}

fn renderable_snippet(span: DisplaySpan, source: &str) -> Option<RenderableSnippet<'_>> {
    let line_index = LineIndex::new(source);
    let start = position_offset(span.start, &line_index, source)?;
    let end = position_offset(span.end, &line_index, source)?;
    let line_start = span.start.line;
    let snippet_start = usize::from(line_index.line_start(line_start)?);
    let snippet_end = snippet_end_offset(span.end.line.max(span.start.line), &line_index, source)?;
    let absolute_range = highlighted_range(start..end.max(start), span.start, &line_index, source);

    Some(RenderableSnippet {
        source: &source[snippet_start..snippet_end],
        line_start,
        range: (absolute_range.start - snippet_start)..(absolute_range.end - snippet_start),
    })
}

fn highlighted_range(
    range: Range<usize>,
    position: DisplayPosition,
    line_index: &LineIndex,
    source: &str,
) -> Range<usize> {
    if range.start != range.end {
        return range;
    }

    let line_start = usize::from(line_index.line_start(position.line).unwrap_or_default());
    let line_end = usize::from(
        line_index
            .line_range(position.line, source)
            .map(|range| range.end())
            .unwrap_or_default(),
    );

    if range.start < line_end {
        let next = source[range.start..]
            .chars()
            .next()
            .map(|ch| range.start + ch.len_utf8())
            .unwrap_or(range.start);
        range.start..next
    } else if range.start > line_start {
        let previous = source[..range.start]
            .chars()
            .next_back()
            .map(|ch| range.start - ch.len_utf8())
            .unwrap_or(range.start);
        previous..range.start
    } else {
        range
    }
}

fn position_offset(
    position: DisplayPosition,
    line_index: &LineIndex,
    source: &str,
) -> Option<usize> {
    let line_start = usize::from(line_index.line_start(position.line)?);
    let line_range = line_index.line_range(position.line, source)?;
    let line_end = usize::from(line_range.end());
    let requested = line_start.saturating_add(position.column.saturating_sub(1));
    Some(requested.min(line_end))
}

fn snippet_end_offset(line: usize, line_index: &LineIndex, source: &str) -> Option<usize> {
    Some(usize::from(line_index.line_range(line, source)?.end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint_diagnostic(
        path: &str,
        span: DisplaySpan,
        message: &str,
        severity: &str,
        code: &str,
        source: &str,
    ) -> DisplayedDiagnostic {
        DisplayedDiagnostic {
            path: PathBuf::from(path),
            span,
            message: message.to_owned(),
            kind: DisplayedDiagnosticKind::Lint {
                code: code.to_owned(),
                severity: severity.to_owned(),
            },
            source: Some(Arc::<str>::from(source)),
        }
    }

    fn parse_diagnostic(
        path: &str,
        line: usize,
        column: usize,
        message: &str,
        source: &str,
    ) -> DisplayedDiagnostic {
        DisplayedDiagnostic {
            path: PathBuf::from(path),
            span: DisplaySpan::point(line, column),
            message: message.to_owned(),
            kind: DisplayedDiagnosticKind::ParseError,
            source: Some(Arc::<str>::from(source)),
        }
    }

    fn render_full(diagnostic: &DisplayedDiagnostic) -> String {
        format_full_diagnostic(diagnostic, false)
    }

    #[test]
    fn renders_single_line_lint_snippet() {
        let diagnostic = lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(1, 6), DisplayPosition::new(1, 10)),
            "legacy backticks",
            "warning",
            "S005",
            "echo `pwd`\n",
        );

        insta::assert_snapshot!(render_full(&diagnostic), @r"
warning[S005]: legacy backticks
 --> script.sh:1:6
  |
1 | echo `pwd`
  |      ^^^^
  |
");
    }

    #[test]
    fn renders_multi_line_lint_snippet() {
        let diagnostic = lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(2, 4), DisplayPosition::new(3, 9)),
            "quoted regular expression literal",
            "error",
            "C010",
            "if true; then\n  [[ $foo =~ \"bar\"\n    && $bar ]]\nfi\n",
        );

        insta::assert_snapshot!(render_full(&diagnostic), @r#"
error[C010]: quoted regular expression literal
 --> script.sh:2:4
  |
2 |     [[ $foo =~ "bar"
  |  ____^
3 | |     && $bar ]]
  | |________^
  |
"#);
    }

    #[test]
    fn renders_parse_error_snippet() {
        let diagnostic = parse_diagnostic(
            "broken.sh",
            2,
            1,
            "unterminated construct",
            "#!/bin/bash\nif true\n",
        );

        insta::assert_snapshot!(render_full(&diagnostic), @r"
error[parse-error]: unterminated construct
 --> broken.sh:2:1
  |
2 | if true
  | ^
  |
");
    }

    #[test]
    fn keeps_tabs_and_unicode_aligned() {
        let diagnostic = lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(2, 8), DisplayPosition::new(2, 12)),
            "legacy backticks",
            "warning",
            "S005",
            "printf '🔉'\n\tfoo=`pwd`\n",
        );

        insta::assert_snapshot!(render_full(&diagnostic), @r"
warning[S005]: legacy backticks
 --> script.sh:2:8
  |
2 | 	foo=`pwd`
  |       ^^^
  |
");
    }

    #[test]
    fn renders_concise_output_exactly() {
        let diagnostic = lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(3, 14), DisplayPosition::new(3, 18)),
            "example message",
            "warning",
            "C014",
            "echo ok\n",
        );

        assert_eq!(
            format_concise_diagnostic(&diagnostic, false),
            "script.sh:3:14: warning[C014] example message"
        );
    }
}
