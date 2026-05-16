#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]
//! Shell formatting entrypoints built on top of `shuck-parser` and `shuck-format`.
//!
//! Most callers will use [`format_source`] for source text or [`format_file_ast`] when they
//! already have a parsed shell AST.
#![recursion_limit = "256"]

//! Shell script formatter with configurable style options.

#[allow(missing_docs)]
mod ast_format;
#[allow(missing_docs)]
mod command;
#[allow(missing_docs)]
mod comments;
#[allow(missing_docs)]
mod context;
#[allow(missing_docs)]
mod facts;
#[allow(missing_docs)]
mod generated;
#[allow(missing_docs)]
mod options;
#[allow(missing_docs)]
mod prelude;
#[allow(missing_docs)]
mod redirect;
#[allow(missing_docs)]
mod script;
#[allow(missing_docs)]
mod shared_traits;
#[allow(missing_docs)]
mod simplify;
#[allow(missing_docs)]
mod streaming;
#[allow(missing_docs)]
mod word;

use std::path::Path;

use shuck_ast::File;
use shuck_format::{FormatResult, LineEnding};
use shuck_parser::{Error as ParseError, parser::Parser};

#[cfg(feature = "benchmarking")]
use crate::facts::FormatterFacts;

/// Formatter option types exposed by the shell formatter.
pub use crate::options::{ResolvedShellFormatOptions, ShellDialect, ShellFormatOptions};
/// Indentation styles supported by the underlying pretty-printer.
pub use shuck_format::IndentStyle;

/// Formatter specialized for shell formatting contexts.
pub type ShellFormatter<'source, 'buf> =
    shuck_format::Formatter<context::ShellFormatContext<'source>>;

pub(crate) trait FormatNodeRule<N> {
    fn fmt(&self, node: &N, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()>;
}

/// Result of formatting shell source.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormattedSource {
    Unchanged,
    Formatted(String),
}

#[allow(missing_docs)]
impl FormattedSource {
    #[must_use]
    pub fn is_changed(&self) -> bool {
        matches!(self, Self::Formatted(_))
    }
}

/// Errors that can occur while parsing or formatting shell source.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    Parse {
        message: String,
        line: usize,
        column: usize,
    },
    Internal(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse {
                message,
                line,
                column,
            } => {
                if *line > 0 {
                    write!(f, "parse error at line {line}, column {column}: {message}")
                } else {
                    write!(f, "parse error: {message}")
                }
            }
            Self::Internal(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<shuck_format::FormatError> for FormatError {
    fn from(error: shuck_format::FormatError) -> Self {
        Self::Internal(error.to_string())
    }
}

/// Convenient result alias for shell formatting operations.
pub type Result<T> = std::result::Result<T, FormatError>;

/// Formats a shell source string using the provided options.
pub fn format_source(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    let resolved = options.resolve(source, path);
    let dialect = resolved.dialect();
    let parsed = Parser::with_dialect(source, dialect).parse();
    if parsed.is_err() {
        return Err(map_parse_error(parsed.strict_error()));
    }

    format_file_ast(source, parsed.file, path, options)
}

#[doc(hidden)]
pub fn source_is_formatted(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<bool> {
    let resolved = options.resolve(source, path);
    let dialect = resolved.dialect();
    let parsed = Parser::with_dialect(source, dialect).parse();
    if parsed.is_err() {
        return Err(map_parse_error(parsed.strict_error()));
    }

    check_file(source, parsed.file, resolved)
}

/// Formats a parsed shell file using the provided options.
pub fn format_file_ast(
    source: &str,
    file: File,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    let resolved = options.resolve(source, path);
    format_file(source, file, resolved)
}

fn format_file(
    source: &str,
    file: File,
    resolved: ResolvedShellFormatOptions,
) -> Result<FormattedSource> {
    let output = format_output(source, file, &resolved)?;

    Ok(formatted_source_from_output(source, output))
}

fn check_file(source: &str, mut file: File, resolved: ResolvedShellFormatOptions) -> Result<bool> {
    if resolved.simplify() {
        simplify::simplify_file(&mut file, source);
    }

    if !resolved.minify()
        && !resolved.simplify()
        && !source_may_need_case_comment_alignment(source)
        && !source_may_need_branch_comment_relocation(source)
    {
        return streaming::format_file_streaming_matches_source(source, &file, &resolved);
    }

    let output = format_output(source, file, &resolved)?;
    Ok(output == source)
}

fn format_output(
    source: &str,
    mut file: File,
    resolved: &ResolvedShellFormatOptions,
) -> Result<String> {
    if resolved.simplify() || resolved.minify() {
        simplify::simplify_file(&mut file, source);
    }

    let mut output = streaming::format_file_streaming(source, &file, resolved)?;
    if resolved.minify() {
        preserve_initial_shebang(source, &mut output, resolved.line_ending());
    } else {
        align_adjacent_case_terminator_comments(&mut output, resolved.line_ending());
        relocate_branch_leading_comments(&mut output, resolved.line_ending());
    }
    ensure_single_trailing_newline(&mut output, resolved.line_ending());

    Ok(output)
}

fn align_adjacent_case_terminator_comments(output: &mut String, line_ending: LineEnding) {
    let line_ending = line_ending_str(line_ending);
    let had_trailing_line_ending = trailing_line_ending_start(output).is_some();
    let body = output.trim_end_matches(['\r', '\n']);
    if body.is_empty() {
        return;
    }

    let mut lines = body
        .split(line_ending)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let Some(_) = case_alignment_inline_comment_code_width(&lines[index]) else {
            index += 1;
            continue;
        };

        let start = index;
        let mut has_case_terminator =
            case_terminator_inline_comment_code_width(&lines[index]).is_some();
        index += 1;
        while index < lines.len()
            && case_alignment_inline_comment_code_width(&lines[index]).is_some()
        {
            has_case_terminator |=
                case_terminator_inline_comment_code_width(&lines[index]).is_some();
            index += 1;
        }

        if index - start < 2 || !has_case_terminator {
            continue;
        }

        let target = lines[start..index]
            .iter()
            .filter_map(|line| case_alignment_inline_comment_code_width(line))
            .max()
            .unwrap_or(0)
            + 1;
        for line in &mut lines[start..index] {
            align_inline_comment_to_column(line, target);
        }
    }

    let mut aligned = lines.join(line_ending);
    if had_trailing_line_ending {
        aligned.push_str(line_ending);
    }
    *output = aligned;
}

fn case_terminator_inline_comment_code_width(line: &str) -> Option<usize> {
    let comment_start = line.find('#')?;
    let code = line[..comment_start].trim_end();
    if code.trim().is_empty() || !(code.contains(";;") || code.contains(";&")) {
        return None;
    }
    Some(code.chars().count())
}

fn case_alignment_inline_comment_code_width(line: &str) -> Option<usize> {
    let comment_start = line.find('#')?;
    let code = line[..comment_start].trim_end();
    if code.trim().is_empty() {
        return None;
    }
    (code.contains(";;") || code.contains(";&") || code.trim_end().ends_with(')'))
        .then_some(code.chars().count())
}

fn align_inline_comment_to_column(line: &mut String, target: usize) {
    let Some(comment_start) = line.find('#') else {
        return;
    };
    let code = line[..comment_start].trim_end();
    let comment = &line[comment_start..];
    let padding = target.saturating_sub(code.chars().count()).max(1);
    let mut aligned = String::with_capacity(code.len() + padding + comment.len());
    aligned.push_str(code);
    aligned.push_str(&" ".repeat(padding));
    aligned.push_str(comment);
    *line = aligned;
}

fn source_may_need_case_comment_alignment(source: &str) -> bool {
    source
        .lines()
        .any(|line| case_terminator_inline_comment_code_width(line).is_some())
}

fn source_may_need_branch_comment_relocation(source: &str) -> bool {
    let lines = source.lines().collect::<Vec<_>>();
    lines.windows(2).any(|window| {
        window[0].trim_start().starts_with('#')
            && (window[1].trim_start() == "else" || window[1].trim_start().starts_with("elif "))
    })
}

fn relocate_branch_leading_comments(output: &mut String, line_ending: LineEnding) {
    let line_ending = line_ending_str(line_ending);
    let had_trailing_line_ending = trailing_line_ending_start(output).is_some();
    let body = output.trim_end_matches(['\r', '\n']);
    if body.is_empty() {
        return;
    }

    let mut lines = body
        .split(line_ending)
        .map(str::to_string)
        .collect::<Vec<_>>();
    for index in 1..lines.len().saturating_sub(2) {
        if !lines[index].trim_start().starts_with('#') || !lines[index + 1].is_empty() {
            continue;
        }
        let next_trimmed = lines[index + 2].trim_start();
        if !(next_trimmed == "else" || next_trimmed.starts_with("elif ")) {
            continue;
        }

        let comment_indent = leading_whitespace_len(&lines[index]);
        let keyword_indent = leading_whitespace_len(&lines[index + 2]);
        if comment_indent <= keyword_indent {
            continue;
        }

        let keyword_prefix = lines[index + 2][..keyword_indent].to_string();
        let comment = lines[index].trim_start().to_string();
        lines[index].clear();
        lines[index + 1] = format!("{keyword_prefix}{comment}");
    }

    let mut relocated = lines.join(line_ending);
    if had_trailing_line_ending {
        relocated.push_str(line_ending);
    }
    *output = relocated;
}

fn leading_whitespace_len(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}

fn formatted_source_from_output(source: &str, output: String) -> FormattedSource {
    if output == source {
        FormattedSource::Unchanged
    } else {
        FormattedSource::Formatted(output)
    }
}

#[cfg(feature = "benchmarking")]
#[doc(hidden)]
#[must_use]
pub fn build_formatter_facts(source: &str, file: &File) -> usize {
    let resolved = ShellFormatOptions::default().resolve(source, None);
    FormatterFacts::build(source, file, &resolved).len()
}

fn ensure_single_trailing_newline(output: &mut String, line_ending: LineEnding) {
    while has_multiple_trailing_line_endings(output) {
        truncate_trailing_line_ending(output);
    }
    if trailing_line_ending_start(output).is_none() {
        if trailing_backslash_count(output) % 2 == 1 {
            output.push('\\');
        }
        output.push_str(line_ending_str(line_ending));
    }
}

fn has_multiple_trailing_line_endings(text: &str) -> bool {
    trailing_line_ending_start(text)
        .and_then(|start| trailing_line_ending_start(&text[..start]))
        .is_some()
}

fn truncate_trailing_line_ending(output: &mut String) {
    if let Some(start) = trailing_line_ending_start(output) {
        output.truncate(start);
    }
}

fn trailing_line_ending_start(text: &str) -> Option<usize> {
    if text.ends_with("\r\n") {
        Some(text.len() - 2)
    } else if text.ends_with('\n') {
        Some(text.len() - 1)
    } else {
        None
    }
}

fn line_ending_str(line_ending: LineEnding) -> &'static str {
    match line_ending {
        LineEnding::Lf => "\n",
        LineEnding::CrLf => "\r\n",
    }
}

fn preserve_initial_shebang(source: &str, output: &mut String, line_ending: LineEnding) {
    if !source.starts_with("#!") || output.starts_with("#!") {
        return;
    }

    let shebang_end = source.find(['\r', '\n']).unwrap_or(source.len());
    let shebang = &source[..shebang_end];
    let line_ending = line_ending_str(line_ending);
    let body = output.trim_start_matches(['\r', '\n']);

    let mut prefixed = String::with_capacity(shebang.len() + line_ending.len() + body.len());
    prefixed.push_str(shebang);
    prefixed.push_str(line_ending);
    prefixed.push_str(body);
    *output = prefixed;
}

fn trailing_backslash_count(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
}

fn map_parse_error(error: ParseError) -> FormatError {
    match error {
        ParseError::Parse {
            message,
            line,
            column,
        } => FormatError::Parse {
            message,
            line,
            column,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use shuck_ast::{AssignmentValue, Command};
    use shuck_linter::{AnalysisRequest, Diagnostic, LinterSettings};
    use shuck_parser::ShellDialect as ParseShellDialect;

    use super::*;

    fn parse_for_ast_format(
        source: &str,
        path: Option<&Path>,
        options: &ShellFormatOptions,
    ) -> shuck_parser::parser::ParseResult {
        let dialect = options.resolve(source, path).dialect();
        Parser::with_dialect(source, dialect).parse().unwrap()
    }

    fn assert_source_and_ast_paths_match(
        source: &str,
        path: Option<&Path>,
        options: &ShellFormatOptions,
    ) {
        let parsed = parse_for_ast_format(source, path, options);
        let from_source = format_source(source, path, options).unwrap();
        let from_ast = format_file_ast(source, parsed.file, path, options).unwrap();
        assert_eq!(from_source, from_ast);
        assert_eq!(
            source_is_formatted(source, path, options).unwrap(),
            matches!(from_source, FormattedSource::Unchanged)
        );
    }

    fn format_to_string(source: &str, path: Option<&Path>, options: &ShellFormatOptions) -> String {
        match format_source(source, path, options).unwrap() {
            FormattedSource::Unchanged => source.to_string(),
            FormattedSource::Formatted(formatted) => formatted,
        }
    }

    fn assert_idempotent(source: &str, path: Option<&Path>, options: &ShellFormatOptions) {
        let once = format_to_string(source, path, options);
        let twice = format_to_string(&once, path, options);
        assert_eq!(once, twice);
    }

    fn lint_source_posix_strict(source: &str, path: &Path) -> Vec<Diagnostic> {
        let parse_result = Parser::with_dialect(source, ParseShellDialect::Posix).parse();
        assert!(
            !parse_result.is_err(),
            "strict parse failed for {}: {}",
            path.display(),
            parse_result.strict_error()
        );
        let settings = LinterSettings::default().with_analyzed_paths([path.to_path_buf()]);
        AnalysisRequest::from_parse_result(&parse_result, source, &settings)
            .with_source_path(path)
            .lint()
    }

    fn diagnostic_count(diagnostics: &[Diagnostic], code: &str) -> usize {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code() == code)
            .count()
    }

    #[test]
    fn format_file_ast_requires_explicit_clone_for_ast_reuse() {
        let source = "echo $(( $a + ${b} ))\n";
        let path = Some(Path::new("reuse.bash"));
        let options = ShellFormatOptions::default().with_simplify(true);
        let parsed = parse_for_ast_format(source, path, &options);

        let first = format_file_ast(source, parsed.file.clone(), path, &options).unwrap();
        let second = format_file_ast(source, parsed.file, path, &options).unwrap();

        assert_eq!(first, second);
        assert_eq!(first, format_source(source, path, &options).unwrap());
    }

    #[test]
    fn formats_simple_command_with_tabs_by_default() {
        let formatted = format_source(
            "#!/bin/bash\n echo   hi\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("#!/bin/bash\necho hi\n".to_string())
        );
        assert!(
            !source_is_formatted(
                "#!/bin/bash\n echo   hi\n",
                None,
                &ShellFormatOptions::default()
            )
            .unwrap()
        );
    }

    #[test]
    fn preserves_inline_comments() {
        let formatted =
            format_source("echo hi    # note\n", None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo hi # note\n".to_string())
        );
    }

    #[test]
    fn preserves_source_backed_double_quoted_backslashes_like_shfmt() {
        let source = "fgc=\"\\e[38;2;${red};${green};${blue}m\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn formats_process_substitution_bodies_like_shfmt() {
        let source = "read -r misc_var < <(${sensor_comm} measure_temp 2>/dev/null ||true)\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "read -r misc_var < <(${sensor_comm} measure_temp 2>/dev/null || true)\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn aligns_adjacent_trailing_comments_like_shfmt() {
        let source = "printf -v backspace \"\\u7F\" #? Backspace set to DELETE\nprintf -v backspace_real \"\\u08\" #? Real backspace\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "printf -v backspace \"\\u7F\"      #? Backspace set to DELETE\nprintf -v backspace_real \"\\u08\" #? Real backspace\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn check_path_reports_already_formatted_sources() {
        assert!(
            source_is_formatted(
                "echo hi\n",
                Some(Path::new("script.sh")),
                &ShellFormatOptions::default()
            )
            .unwrap()
        );
    }

    #[test]
    fn formats_heredoc_command_heads_structurally() {
        let formatted = format_source(
            "cat<<EOF\nhello\nEOF\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("cat <<EOF\nhello\nEOF\n".to_string())
        );
    }

    #[test]
    fn formats_nested_heredoc_commands_without_indenting_body() {
        let formatted = format_source(
            "if true; then\ncat<<EOF\nhello\nEOF\nfi\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("if true; then\n\tcat <<EOF\nhello\nEOF\nfi\n".to_string())
        );
    }

    #[test]
    fn preserves_multiline_if_body_comments() {
        let formatted = format_source(
            "if true; then\n# note\necho hi\nfi\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("if true; then\n\t# note\n\techo hi\nfi\n".to_string())
        );
    }

    #[test]
    fn preserves_comments_inside_elif_bodies() {
        let source = "foo() {\nif a; then\none\nelif b; then\n# note\n two\nfi\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "foo() {\n\tif a; then\n\t\tone\n\telif b; then\n\t\t# note\n\t\ttwo\n\tfi\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_comments_after_if_blocks() {
        let formatted = format_source(
            "if true; then\necho hi\nfi\n# after\necho bye\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if true; then\n\techo hi\nfi\n# after\necho bye\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_comments_after_function_blocks() {
        let formatted = format_source(
            "foo() {\necho hi\n}\n# after\nbar\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("foo() {\n\techo hi\n}\n# after\nbar\n".to_string())
        );
    }

    #[test]
    fn preserves_heredoc_trailing_comments_without_duplication() {
        let formatted = format_source(
            "cat <<EOF # note\nhi\nEOF\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_quoted_heredoc_delimiters_idempotently() {
        assert_idempotent(
            "cat <<'EOF_264'\ndelta\nEOF_264\n",
            Some(Path::new("quoted_heredoc.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn formats_decl_heredoc_heads_structurally() {
        let formatted = format_source(
            "declare -x foo=1<<EOF\nhi\nEOF\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("declare -x foo=1 <<EOF\nhi\nEOF\n".to_string())
        );
    }

    #[test]
    fn standalone_assignments_do_not_gain_trailing_spaces() {
        let formatted = format_source("x=1\n", None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_blank_lines_between_commands() {
        let formatted =
            format_source("set -u\n\nfoo\n", None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_blank_lines_after_functions() {
        let source = "foo() {\n  echo hi\n}\n\nbar\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);
        let formatted =
            format_source(source, Some(Path::new("function_gap.bash")), &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("foo() {\n\techo hi\n}\n\nbar\n".to_string())
        );
    }

    #[test]
    fn preserves_blank_lines_after_leading_comments() {
        let formatted = format_source(
            "#!/usr/bin/env bash\n\nset -u\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn parsed_assignment_value_render_syntax_is_trimmed() {
        let parsed = parse_for_ast_format("x=1\n", None, &ShellFormatOptions::default());
        let Command::Simple(command) = &parsed.file.body[0].command else {
            panic!("expected a simple command");
        };

        let AssignmentValue::Scalar(value) = &command.assignments[0].value else {
            panic!("expected a scalar assignment");
        };

        assert_eq!(value.render_syntax("x=1\n"), "1");
    }

    #[test]
    fn preserves_escaped_quotes_in_double_quoted_assignments() {
        let source = "fzf_completion=\"source \\\"$fzf_base/shell/completion.${shell}\\\"\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_escaped_quotes_around_parameter_expansion_in_double_quotes() {
        let source = "nvm_echo \"Running node LTS \\\"${NVM_LTS-}\\\" -> $(nvm_version \"${VERSION}\")$(nvm use --silent \"${VERSION}\" && nvm_print_npm_version)\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_nested_parameter_expansions_inside_quoted_strings() {
        let source = "nvm_err \"N/A: version \\\"${PREFIXED_VERSION:-$PROVIDED_VERSION}\\\" is not yet installed.\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_default_redirect_spacing_without_space_redirects() {
        let source = "archi=$(uname -smo 2> /dev/null || uname -sm)\n";
        let options = ShellFormatOptions::default();
        let formatted = format_source(source, None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "archi=$(uname -smo 2>/dev/null || uname -sm)\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_explicit_default_redirect_fds() {
        let source = "cmd 1>/dev/null\ncmd 0<input\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_line_continuation_before_trailing_redirects() {
        let source = "nvm_echo_with_colors nvm_err_with_colors \\\n  >/dev/null 2>&1\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "nvm_echo_with_colors nvm_err_with_colors \\\n\t>/dev/null 2>&1\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_line_continuation_before_later_arguments_like_shfmt() {
        let source = "notify-send \"title\" \"long body\" \\\n  -i face-glasses -t 10000\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "notify-send \"title\" \"long body\" \\\n\t-i face-glasses -t 10000\n".to_string()
            )
        );
    }

    #[test]
    fn collapses_no_space_continuation_before_quoted_argument_like_shfmt() {
        let source =
            "notify-send -u normal\\\n  \"title\" \"long body\"\\\n  -i face-glasses -t 10000\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "notify-send -u normal \"title\" \"long body\" \\\n\t-i face-glasses -t 10000\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn collapses_no_space_continuation_before_variable_argument_like_shfmt() {
        let source = "cmd -rs\\\n  ${reverse_string}\\\n  -fg x\\\n  -fg y\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("cmd -rs ${reverse_string} \\\n\t-fg x -fg y\n".to_string())
        );
    }

    #[test]
    fn preserves_empty_command_substitutions() {
        let source = "result=$()\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn formats_multiline_command_substitutions() {
        let source = "result=$(\necho foo\necho bar\n)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("result=$(\n\techo foo\n\techo bar\n)\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn formats_command_substitutions_with_rendered_line_breaks_as_multiline() {
        let source = "result=$(echo foo\necho bar)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("result=$(\n\techo foo\n\techo bar\n)\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn spaces_command_substitution_before_inline_subshell() {
        let source = "result=$( (foo) || bar)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn formats_multiline_command_substitutions_with_compound_commands() {
        let source = "result=$(\nif foo; then\necho hi\nelse\necho bye\nfi\n)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "result=$(\n\tif foo; then\n\t\techo hi\n\telse\n\t\techo bye\n\tfi\n)\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn command_substitutions_with_comments_fall_back_to_raw_source() {
        let source = "result=$(echo foo # keep comment\necho bar)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn command_substitutions_with_heredocs_format_shell_lines_but_not_bodies() {
        let source = "result=$(cat <<EOF\nhello\nEOF\n)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("result=$(\n\tcat <<EOF\nhello\nEOF\n)\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_post_heredoc_quote_line_like_shfmt() {
        let source = "f() {\n  echo \"$(\n    cat <<EOS\nbody\nEOS\n  )\n\" | tr -d \"\\\\\"\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "f() {\n\techo \"$(\n\t\tcat <<EOS\nbody\nEOS\n\t)\n\" | tr -d \"\\\\\"\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn formats_tab_stripped_heredoc_indentation_like_shfmt() {
        let source = "f() {\n  cat <<-EOF\n\thi\n\tEOF\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("f() {\n\tcat <<-EOF\n\t\thi\n\tEOF\n}\n".to_string())
        );
    }

    #[test]
    fn command_substitution_bounds_do_not_capture_following_comments() {
        let source = "value=$(pwd)\n# after\nnext\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn dirname_command_substitution_bounds_do_not_capture_following_comments() {
        let source = "cd \"$(dirname \"${BASH_SOURCE[0]}\")\"\n# after\nnext\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn nested_command_substitution_bounds_do_not_capture_following_comments() {
        let source = "INFO=\"$(which \"${COMMAND}\") ($(type \"${COMMAND}\" | command awk '{print $4}'))\"\n# after\nnext\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_conditional_command_substitutions_with_nested_quoted_arguments() {
        let source = "[[ \"$(get_permission \"$1\")\" != \"$(id -u)\" ]]\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_file_not_grpowned_command_substitution_shape() {
        let source = "[[ \" $(id -G \"${USER}\") \" != *\" $(get_group \"$1\") \"* ]]\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_long_suffix_trim_operators_in_words() {
        let source = "package_url=\"${package_url%%#*}\"\necho \"${1%%.*}\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_parameter_replacements_and_slice_offsets() {
        let source = "if [ \"$package_url\" != \"${package_url/\\#}\" ]; then\n  echo \"${arg:$index:1}\"\n  local fetch_args=(\"$package_name\" \"${@:1:$package_type_nargs}\")\n  for arg in \"${@:$(( $package_type_nargs + 1 ))}\"; do\n    echo \"$arg\"\n  done\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if [ \"$package_url\" != \"${package_url/\\#/}\" ]; then\n\techo \"${arg:$index:1}\"\n\tlocal fetch_args=(\"$package_name\" \"${@:1:$package_type_nargs}\")\n\tfor arg in \"${@:$(($package_type_nargs + 1))}\"; do\n\t\techo \"$arg\"\n\tdone\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn formats_major_minor_multiline_command_substitution_like_shfmt() {
        let source = "major_minor() {\n  echo \"${1%%.*}.$(\n    x=\"${1#*.}\"\n    echo \"${x%%.*}\"\n  )\"\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "major_minor() {\n\techo \"${1%%.*}.$(\n\t\tx=\"${1#*.}\"\n\t\techo \"${x%%.*}\"\n\t)\"\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn compacts_inline_brace_group_command_substitution_like_shfmt() {
        let source = "num=\"$({ getconf _NPROCESSORS_ONLN ||\n             grep -c ^processor /proc/cpuinfo; } 2>/dev/null)\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "num=\"$({ getconf _NPROCESSORS_ONLN ||\n\tgrep -c ^processor /proc/cpuinfo; } 2>/dev/null)\"\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn formats_nvm_args_command_substitution_without_losing_sed_body_layout() {
        let source = "ARGS=$(\n  nvm_echo \"$@\" | command sed \"\n    s/--progress-bar /--progress=bar /\n    s/-s /-q /\n  \"\n)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "ARGS=$(\n\tnvm_echo \"$@\" | command sed \"\n    s/--progress-bar /--progress=bar /\n    s/-s /-q /\n  \"\n)\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn formats_inline_command_substitution_with_continuations_as_multiline() {
        let source = "VERSIONS=\"$(command find foo \\\n  | command sed -e \"\n    s#x#y#;\n  \" \\\n    -e z)\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "VERSIONS=\"$(\n\tcommand find foo |\n\t\tcommand sed -e \"\n    s#x#y#;\n  \" \\\n\t\t\t-e z\n)\"\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_break_before_leading_pipeline_operator_like_shfmt() {
        let source =
            "VERSIONS=\"$(command find foo \\\n  | command sed q \\\n  | command sort \\\n)\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "VERSIONS=\"$(\n\tcommand find foo |\n\t\tcommand sed q |\n\t\tcommand sort\n)\"\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn keeps_inline_brace_group_command_substitution_with_pipeline_like_shfmt() {
        let source = "nvm_err \"awk: $(nvm_command_info awk), $({ command awk --version 2>/dev/null || command awk -W version; } \\\n  | command head -n 1)\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "nvm_err \"awk: $(nvm_command_info awk), $({ command awk --version 2>/dev/null || command awk -W version; } |\n\tcommand head -n 1)\"\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn keeps_inline_command_substitution_for_single_command_with_multiline_string() {
        let source = "ARGS=$(nvm_echo \"$@\" | command sed \"\n    s/--progress-bar /--progress=bar /\n    s/-s /-q /\n  \")\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_assignment_string_continuation_indentation() {
        let source = "f() {\n  label=\"one |\n                      two |\n                      three\"\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "f() {\n\tlabel=\"one |\n                      two |\n                      three\"\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn formats_arithmetic_expansions_from_ruby_build() {
        let source = "echo $(( ver[0]*100 + ver[1] ))\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo $((ver[0] * 100 + ver[1]))\n".to_string())
        );
    }

    #[test]
    fn formats_arithmetic_commands_like_shfmt() {
        let source = "if ((system_version < lower_bound || system_version >= upper_bound)); then\n  return 1\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if ((system_version < lower_bound || system_version >= upper_bound)); then\n\treturn 1\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn formats_shell_style_variables_inside_arithmetic_commands_like_shfmt() {
        let source = "if (($tty_width<80 | $tty_height<24)); then\n  return 1\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if (($tty_width < 80 | $tty_height < 24)); then\n\treturn 1\nfi\n".to_string()
            )
        );
    }

    #[test]
    fn formats_arithmetic_for_headers_like_shfmt() {
        let source = "for ((i=0;i<${#items[@]};i++)); do\n  echo \"$i\"\ndone\nfor ((;;)); do\n  break\ndone\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "for ((i = 0; i < ${#items[@]}; i++)); do\n\techo \"$i\"\ndone\nfor (( ; ; )); do\n\tbreak\ndone\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn formats_arithmetic_for_comma_headers_like_shfmt() {
        let source = "for ((i=0;i<=100;i++,y=0)); do\n  :\ndone\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "for ((i = 0; i <= 100; i++, y = 0)); do\n\t:\ndone\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_multiline_compound_assignment_in_declare() {
        let source = "declare -a items=(\"one\" \"two\"\n  \"three\" \"four\")\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "declare -a items=(\"one\" \"two\"\n\t\"three\" \"four\")\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_multiline_conditional_layout_like_shfmt() {
        let source = "if [[ a == b &&\n      c == d ]]; then\n  echo ok\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [[ a == b &&\n\tc == d ]]; then\n\techo ok\nfi\n".to_string()
            )
        );
    }

    #[test]
    fn formats_multiline_conditional_groups_like_shfmt() {
        let source = "if [[ -n $brew_prefix && ( $prefix == foo/* || \\\n       $prefix == bar/* ) ]]; then\n  return 1\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [[ -n $brew_prefix && ($prefix == foo/* ||\n\t$prefix == bar/*) ]]; then\n\treturn 1\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_then_suffix_comments_and_indents_body() {
        let source = "if foo; then # note\n  bar\nelif baz; then # alt\n  qux\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if foo; then # note\n\tbar\nelif baz; then # alt\n\tqux\nfi\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_multiline_elif_continuation_headers_like_shfmt() {
        let source =
            "if true; then\n  :\nelif \\\n  { foo; } \\\n  || { bar; } \\\n; then\n  :\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if true; then\n\t:\nelif\n\t{ foo; } ||\n\t\t{ bar; } \\\n\t\t;\nthen\n\t:\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn keeps_pre_elif_comments_with_the_elif_branch() {
        let source =
            "if foo; then\n  bar\n# first\n# second\nelif baz &&\n  qux; then\n  zap\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if foo; then\n\tbar\n# first\n# second\nelif baz &&\n\tqux; then\n\tzap\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn keeps_pre_else_comments_with_the_else_branch() {
        let source = "if foo; then\n  fn() {\n    bar\n  }\n\n# else branch\nelse\n  baz\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if foo; then\n\tfn() {\n\t\tbar\n\t}\n\n# else branch\nelse\n\tbaz\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn keeps_body_indented_comments_before_elif_at_body_depth_like_shfmt() {
        let source = "if [[ $letter =~ [a-z] ]]; then\n  string_out=x\n  #if [[ $font ]]; then string_out=y; fi\nelif [[ $letter =~ [A-Z] ]]; then\n  string_out=z\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [[ $letter =~ [a-z] ]]; then\n\tstring_out=x\n\t#if [[ $font ]]; then string_out=y; fi\nelif [[ $letter =~ [A-Z] ]]; then\n\tstring_out=z\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn keeps_simple_then_else_on_one_line() {
        let source = "if foo; then a=1; else b=2; fi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn keeps_short_then_body_inline_before_multiline_else_like_shfmt() {
        let source = "if ((items > height)); then pages=$((items / height + 1)); else height=$items; unset pages; fi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if ((items > height)); then pages=$((items / height + 1)); else\n\theight=$items\n\tunset pages\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_inline_else_body_when_then_branch_is_multiline() {
        let source = "if [[ $letter == \"█\" ]]; then\n  b_color=x\nelse b_color=y; fi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [[ $letter == \"█\" ]]; then\n\tb_color=x\nelse b_color=y; fi\n".to_string()
            )
        );
    }

    #[test]
    fn keeps_multiline_compound_assignment_on_else_line_like_shfmt() {
        let source = "if foo; then\n  bar\nelse desc+=(\"one\"\n  \"two\"); fi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if foo; then\n\tbar\nelse desc+=(\"one\"\n\t\"two\"); fi\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_inline_elif_body_when_then_branch_is_multiline() {
        let source = "if ((per_second == 1 & unit_mult == 1)); then\n  per_second=\"/s\"\nelif ((per_second == 1)); then per_second=\"ps\"; fi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if ((per_second == 1 & unit_mult == 1)); then\n\tper_second=\"/s\"\nelif ((per_second == 1)); then per_second=\"ps\"; fi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn expands_final_elif_body_before_multiline_else_like_shfmt() {
        let source = "if [[ $pos = cpu ]]; then\n  percent=32\nelif [[ $pos = mem ]]; then percent=40\nelse percent=28; fi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [[ $pos = cpu ]]; then\n\tpercent=32\nelif [[ $pos = mem ]]; then\n\tpercent=40\nelse percent=28; fi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn keeps_short_then_body_inline_before_inline_elif_like_shfmt() {
        let source = "if ((acolor>100)); then acolor=100; elif ((acolor<0)); then acolor=0; fi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if ((acolor > 100)); then acolor=100; elif ((acolor < 0)); then acolor=0; fi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn does_not_inline_final_elif_body_when_fi_is_on_next_line_like_shfmt() {
        let source = "if [[ -z $no_guide ]]; then\n  ((height--))\nelif [[ -n $invert ]]; then ((line--))\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [[ -z $no_guide ]]; then\n\t((height--))\nelif [[ -n $invert ]]; then\n\t((line--))\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_shell_style_variables_inside_arithmetic_expansions() {
        let source = "index=$(($index + 1))\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn formats_shell_style_variables_inside_arithmetic_expansions_like_shfmt() {
        let source = "index=$(($index+1))\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("index=$(($index + 1))\n".to_string())
        );
    }

    #[test]
    fn formats_arithmetic_array_subscripts_like_shfmt() {
        let source = "found=\"${line_array[$((line_pos+match_key))]}\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "found=\"${line_array[$((line_pos + match_key))]}\"\n".to_string()
            )
        );
    }

    #[test]
    fn formats_arithmetic_expansions_inside_mixed_array_subscripts_like_shfmt() {
        let source = "print -v graph_array[y] -t \"${graph_symbol[${invert:+-}$(( (input_array[x]*virt_height/100)-next_value ))]}\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "print -v graph_array[y] -t \"${graph_symbol[${invert:+-}$(((input_array[x] * virt_height / 100) - next_value))]}\"\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_plain_array_subscript_arithmetic_like_shfmt() {
        let source = "org_value=${input_array[offset+x]}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn formats_braced_shell_style_variables_inside_arithmetic_expansions_like_shfmt() {
        let source = "echo $(( ${ver[0]}*100 + ${ver[1]} ))\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo $((${ver[0]} * 100 + ${ver[1]}))\n".to_string())
        );
    }

    #[test]
    fn formats_arithmetic_expansions_from_pyenv_python_build() {
        let source =
            "for arg in \"${@:$(( $package_type_nargs + 1 ))}\"; do\n  echo \"$arg\"\ndone\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "for arg in \"${@:$(($package_type_nargs + 1))}\"; do\n\techo \"$arg\"\ndone\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn formats_arithmetic_expansions_inside_parameter_slices_like_shfmt() {
        let source =
            "graph_array[i]=\"${graph_array[i]::$search}${graph_array[i]:$((search+1))}\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "graph_array[i]=\"${graph_array[i]::$search}${graph_array[i]:$((search + 1))}\"\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn formats_parenthesized_negative_parameter_offsets_like_shfmt() {
        let source = "filter_string=\"${filter: (-$((width-35-reverse_pos)))}\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "filter_string=\"${filter:(-$((width - 35 - reverse_pos)))}\"\n".to_string()
            )
        );
    }

    #[test]
    fn formats_arithmetic_expansions_inside_parameter_defaults_like_shfmt() {
        let source = "create_box -h ${desc_height:-$((${#selected_desc[@]}+2))}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "create_box -h ${desc_height:-$((${#selected_desc[@]} + 2))}\n".to_string()
            )
        );
    }

    #[test]
    fn formats_arithmetic_subscripts_inside_parameter_operations_like_shfmt() {
        let source = "tree_compare1=\"${proc_array[$((count+1))]%'|'*}\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "tree_compare1=\"${proc_array[$((count + 1))]%'|'*}\"\n".to_string()
            )
        );
    }

    #[test]
    fn formats_arithmetic_expansions_inside_assignment_subscripts_like_shfmt() {
        let source = "cpu[temp_$((threads/2+i))]=\"${core_value}\"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "cpu[temp_$((threads / 2 + i))]=\"${core_value}\"\n".to_string()
            )
        );
    }

    #[test]
    fn normalizes_backtick_command_substitutions_to_dollar_paren() {
        let source = "local computed_checksum=`echo \"$($checksum_command < \"$filename\")\" | tr [A-Z] [a-z]`\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "local computed_checksum=$(echo \"$($checksum_command <\"$filename\")\" | tr [A-Z] [a-z])\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn backtick_in_redirect_is_idempotent() {
        let source = "declare -x foo=1<\\`OF\nhi\nEF\n";
        assert_idempotent(source, None, &ShellFormatOptions::default());
    }

    #[test]
    fn preserves_backslash_continued_simple_commands_from_fzf_install() {
        let source = "create_file \"$bind_file\" \\\n  'function fish_user_key_bindings' \\\n  '  fzf --fish | source' \\\n  'end'\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "create_file \"$bind_file\" \\\n\t'function fish_user_key_bindings' \\\n\t'  fzf --fish | source' \\\n\t'end'\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_backslash_continued_simple_commands_from_homebrew_install() {
        let source = "\"$1\" --enable-frozen-string-literal --disable=gems,did_you_mean,rubyopt -rrubygems -e \\\n    \"abort if Gem::Version.new(RUBY_VERSION) < \\\n              Gem::Version.new('${REQUIRED_RUBY_VERSION}')\" 2>/dev/null\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "\"$1\" --enable-frozen-string-literal --disable=gems,did_you_mean,rubyopt -rrubygems -e \\\n\t\"abort if Gem::Version.new(RUBY_VERSION) < \\\n              Gem::Version.new('${REQUIRED_RUBY_VERSION}')\" 2>/dev/null\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn formats_pipeline_inside_broken_list_like_shfmt() {
        let source = "foo &&\n  a |\n    b |\n    c\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("foo &&\n\ta |\n\tb |\n\t\tc\n".to_string())
        );
    }

    #[test]
    fn only_breaks_pipeline_operators_that_break_in_source() {
        let source = "sort_versions() {\n  sed 's/x/y/' |\n    LC_ALL=C sort -t. -k 1,1 | awk '{print $2}'\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "sort_versions() {\n\tsed 's/x/y/' |\n\t\tLC_ALL=C sort -t. -k 1,1 | awk '{print $2}'\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_leading_redirect_placement_in_nvm_err_helpers() {
        let source = "nvm_err() {\n  >&2 nvm_echo \"$@\"\n}\n\nnvm_err_with_colors() {\n  >&2 nvm_echo_with_colors \"$@\"\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "nvm_err() {\n\t>&2 nvm_echo \"$@\"\n}\n\nnvm_err_with_colors() {\n\t>&2 nvm_echo_with_colors \"$@\"\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_inline_negated_subshell_conditions() {
        let source = "if ! (try_curl \"$url\" || try_wget \"$url\"); then\n\treturn 1\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn negated_subshell_conditions_do_not_capture_later_file_comments() {
        let source = "download() {\n  local url\n  url=https://github.com/junegunn/fzf/releases/download/v$version/${1}\n  set -o pipefail\n  if ! (try_curl $url || try_wget $url); then\n    set +o pipefail\n    binary_error=\"Failed to download with curl and wget\"\n    return\n  fi\n  set +o pipefail\n}\n\n# Try to download binary executable\narchi=$(uname -smo 2> /dev/null || uname -sm)\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "download() {\n\tlocal url\n\turl=https://github.com/junegunn/fzf/releases/download/v$version/${1}\n\tset -o pipefail\n\tif ! (try_curl $url || try_wget $url); then\n\t\tset +o pipefail\n\t\tbinary_error=\"Failed to download with curl and wget\"\n\t\treturn\n\tfi\n\tset +o pipefail\n}\n\n# Try to download binary executable\narchi=$(uname -smo 2>/dev/null || uname -sm)\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_else_branch_comments_inside_the_branch() {
        let source = "if foo; then\n  bar\nelse\n  # branch comment\n  baz\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if foo; then\n\tbar\nelse\n\t# branch comment\n\tbaz\nfi\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_blank_line_after_else_like_shfmt() {
        let source = "if foo; then\n  bar\nelse\n\n  if baz; then\n    qux\n  fi\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if foo; then\n\tbar\nelse\n\n\tif baz; then\n\t\tqux\n\tfi\nfi\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_blank_line_before_else_like_shfmt() {
        let source = "if foo; then\n  bar\n\nelse\n  baz\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("if foo; then\n\tbar\n\nelse\n\tbaz\nfi\n".to_string())
        );
    }

    #[test]
    fn preserves_blank_line_before_fi_like_shfmt() {
        let source = "if foo; then\n  bar\n\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("if foo; then\n\tbar\n\nfi\n".to_string())
        );
    }

    #[test]
    fn formats_multiline_compound_assignments_structurally() {
        let source = "directories=(\n  bin\n  etc\n  Frameworks\n)\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "directories=(\n\tbin\n\tetc\n\tFrameworks\n)\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_case_pattern_escapes() {
        let source = "case \"$archi\" in\nDarwin\\ arm64*) download foo ;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_brace_group_wrapper_comments() {
        let source = "# c\n{ # note\na=1\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("# c\n{ # note\n\ta=1\n}\n".to_string())
        );
    }

    #[test]
    fn preserves_group_body_comments_after_open_suffix() {
        let source = "{ # open\n\n# inner\nx=1\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("{ # open\n\n\t# inner\n\tx=1\n}\n".to_string())
        );
    }

    #[test]
    fn standalone_brace_groups_do_not_consume_later_file_comments() {
        let source = "[ -n \"$x\" ] && {\nset -x\n}\n# later\nnext\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "[ -n \"$x\" ] && {\n\tset -x\n}\n# later\nnext\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_single_line_function_bodies() {
        let source = "tty_escape() { printf \"\\\\033[%sm\" \"$1\"; }\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_single_line_subshells() {
        let source = "(cd \"$fzf_base\"/bin && rm -f fzf && ln -sf \"$which_fzf\" fzf)\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_single_line_subshells_inside_case_bodies() {
        let source = "build_package_pyston() {\n  build_package_copy\n  mkdir -p \"${PREFIX_PATH}/bin\" \"${PREFIX_PATH}/lib\"\n  local bin\n  shopt -s nullglob\n  for bin in \"bin/\"*; do\n    if [ -f \"${bin}\" ] && [ -x \"${bin}\" ] && [ ! -L \"${bin}\" ]; then\n      case \"${bin##*/}\" in\n      \"pyston\"* )\n        ( cd \"${PREFIX_PATH}/bin\" && ln -fs \"${bin##*/}\" \"python\" )\n        ;;\n      esac\n    fi\n  done\n  shopt -u nullglob\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "build_package_pyston() {\n\tbuild_package_copy\n\tmkdir -p \"${PREFIX_PATH}/bin\" \"${PREFIX_PATH}/lib\"\n\tlocal bin\n\tshopt -s nullglob\n\tfor bin in \"bin/\"*; do\n\t\tif [ -f \"${bin}\" ] && [ -x \"${bin}\" ] && [ ! -L \"${bin}\" ]; then\n\t\t\tcase \"${bin##*/}\" in\n\t\t\t\"pyston\"*)\n\t\t\t\t(cd \"${PREFIX_PATH}/bin\" && ln -fs \"${bin##*/}\" \"python\")\n\t\t\t\t;;\n\t\t\tesac\n\t\tfi\n\tdone\n\tshopt -u nullglob\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_inline_case_commands_like_shfmt() {
        let source = "if case \"$line\" in *'='*) true ;; *) false ;; esac; then\nx=1\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if case \"$line\" in *'='*) true ;; *) false ;; esac then\n\tx=1\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_standalone_inline_case_commands_like_shfmt() {
        let source = "f() {\n  case \"${1-}\" in iojs-*) return 0 ;; esac\n  return 1\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "f() {\n\tcase \"${1-}\" in iojs-*) return 0 ;; esac\n\treturn 1\n}\n".to_string()
            )
        );
    }

    #[test]
    fn case_arm_comments_do_not_attach_to_previous_arms() {
        let source = "case \"$option\" in\n\"h\" | \"help\")\nusage\n;;\n\"g\" | \"debug\")\nDEBUG=true\n# Disable optimization\nPYTHON_CFLAGS=-O0\n;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case \"$option\" in\n\"h\" | \"help\")\n\tusage\n\t;;\n\"g\" | \"debug\")\n\tDEBUG=true\n\t# Disable optimization\n\tPYTHON_CFLAGS=-O0\n\t;;\nesac\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_case_pattern_suffix_comments_like_shfmt() {
        let source = "case \"$keypress\" in\nleft) #* Move left\nprocess_left\n;;\nright) #* Move right\nprocess_right\n;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case \"$keypress\" in\nleft) #* Move left\n\tprocess_left\n\t;;\nright) #* Move right\n\tprocess_right\n\t;;\nesac\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_case_pattern_suffix_comment_before_empty_body_terminator_like_shfmt() {
        let source = "case \"$keypress\" in\nleft) #* Move left\n;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case \"$keypress\" in\nleft) #* Move left\n\t;;\nesac\n".to_string()
            )
        );
    }

    #[test]
    fn keeps_commented_case_arm_before_next_pattern_like_shfmt() {
        let source = "case \"$font\" in\n\"sans-serif italic\")\nlower=1\n;;\n#\"sans-serif bold italic\") lower=2;;\n\"script\")\nlower=3\n;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case \"$font\" in\n\"sans-serif italic\")\n\tlower=1\n\t;;\n#\"sans-serif bold italic\") lower=2;;\n\"script\")\n\tlower=3\n\t;;\nesac\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_multiline_case_pattern_lists_like_shfmt() {
        let source = "case \"${DEFINITION_PATH##*/}\" in\n\"2.\"* | \\\n  \"3.0\" | \"3.1\" | \\\n  \"3.2\"*)\npackage_option python configure --enable-unicode=ucs4\n;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case \"${DEFINITION_PATH##*/}\" in\n\"2.\"* | \\\n\t\"3.0\" | \"3.1\" | \\\n\t\"3.2\"*)\n\tpackage_option python configure --enable-unicode=ucs4\n\t;;\nesac\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn normalizes_case_terminator_comment_padding_like_shfmt() {
        let source = "case $flag in\n-a)\nall=1\n;;                 # all\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case $flag in\n-a)\n\tall=1\n\t;; # all\nesac\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_blank_line_before_case_terminator_like_shfmt() {
        let source = "case $flag in\n-a)\nfoo\n\n;;\n-b)\nbar\n;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case $flag in\n-a)\n\tfoo\n\n\t;;\n-b)\n\tbar\n\t;;\nesac\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_blank_line_before_esac_like_shfmt() {
        let source = "case $flag in\n-a)\nfoo\n;;\n\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("case $flag in\n-a)\n\tfoo\n\t;;\n\nesac\n".to_string())
        );
    }

    #[test]
    fn preserves_inline_group_redirect_suffixes() {
        let source = "build_package_activepython() {\n  local package_name=\"$1\"\n  { bash \"install.sh\" --install-dir \"${PREFIX_PATH}\"\n  } >&4 2>&1\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "build_package_activepython() {\n\tlocal package_name=\"$1\"\n\t{\n\t\tbash \"install.sh\" --install-dir \"${PREFIX_PATH}\"\n\t} >&4 2>&1\n}\n".to_string()
            )
        );
    }

    #[test]
    fn list_ending_in_multiline_group_does_not_add_blank_before_next_statement() {
        let source = "foo && {\n  echo\n}\nbar\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("foo && {\n\techo\n}\nbar\n".to_string())
        );
    }

    #[test]
    fn then_suffix_comments_do_not_flatten_nested_body_indentation() {
        let source = "f() {\n  if [ -f x ]; then # pypy 2.x\n    if [ -z y ]; then\n      local X=1\n    fi\n  fi\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "f() {\n\tif [ -f x ]; then # pypy 2.x\n\t\tif [ -z y ]; then\n\t\t\tlocal X=1\n\t\tfi\n\tfi\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_multiline_conditions_after_if_keyword_like_shfmt() {
        let source = "if [ -n \"$brew_prefix\" ] &&\n\n  [ -n \"$CFLAGS\" ]; then # comment\nexport CPPFLAGS=1\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [ -n \"$brew_prefix\" ] &&\n\n\t[ -n \"$CFLAGS\" ]; then # comment\n\texport CPPFLAGS=1\nfi\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_multiline_elif_conditions_after_keyword_like_shfmt() {
        let source = "if foo; then\nbar\nelif ! nvm_echo \"$1\" | nvm_grep -q \"$2\" &&\n  ! nvm_echo \"$1\" | nvm_grep -q \"$3\"; then\nbaz\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if foo; then\n\tbar\nelif ! nvm_echo \"$1\" | nvm_grep -q \"$2\" &&\n\t! nvm_echo \"$1\" | nvm_grep -q \"$3\"; then\n\tbaz\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn folds_multiline_condition_semicolon_line_like_shfmt() {
        let source = "if [ \"$(uname)\" = \"Linux\" ] \\\n  && [ \"${NVM_ARCH}\" = arm64 ]\\\n; then\nNVM_ARCH=armv7l\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [ \"$(uname)\" = \"Linux\" ] &&\n\t[ \"${NVM_ARCH}\" = arm64 ]; then\n\tNVM_ARCH=armv7l\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn moves_multiline_condition_comment_to_then_suffix_like_shfmt() {
        let source = "if [[ -n $brew_prefix && ( ( $brew_prefix != \"/usr\" && $brew_prefix != \"/usr/local\" ) \n  #when -isysroot is passed\n  || ( is_mac && osx_using_default_compiler && $CFLAGS =~ (^|\\ )-isysroot\\  ) ) ]]; then\necho ok\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "if [[ -n $brew_prefix && (($brew_prefix != \"/usr\" && $brew_prefix != \"/usr/local\") ||\n\n\t(is_mac && osx_using_default_compiler && $CFLAGS =~ (^|\\ )-isysroot\\ )) ]]; then #when -isysroot is passed\n\techo ok\nfi\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_full_line_comments_inside_continued_command_lists() {
        let source = "f() {\n  command -v brew && \\\n    # first\n    # second\n    brew_prefix=x && \\\n    [[ -n \"$brew_prefix\" ]]\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "f() {\n\tcommand -v brew &&\n\t\t# first\n\t\t# second\n\t\tbrew_prefix=x &&\n\t\t[[ -n \"$brew_prefix\" ]]\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_break_before_leading_list_operator_like_shfmt() {
        let source = "f() {\n  nvm_version_greater_than_or_equal_to \"${NODE_VERSION}\" v0.8.6 \\\n  && ! nvm_version_greater_than_or_equal_to \"${NODE_VERSION}\" v1.0.0\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "f() {\n\tnvm_version_greater_than_or_equal_to \"${NODE_VERSION}\" v0.8.6 &&\n\t\t! nvm_version_greater_than_or_equal_to \"${NODE_VERSION}\" v1.0.0\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn keeps_same_line_list_operator_after_multiline_group_like_shfmt() {
        let source = "(\n  echo one\n) || return $?\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("(\n\techo one\n) || return $?\n".to_string())
        );
    }

    #[test]
    fn does_not_insert_blank_before_following_subshell_like_shfmt() {
        let source = "ohai \"Downloading and installing Homebrew...\"\n(\n  cd x >/dev/null || return\n) || exit 1\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "ohai \"Downloading and installing Homebrew...\"\n(\n\tcd x >/dev/null || return\n) || exit 1\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn trailing_comments_on_function_closing_braces_do_not_poison_following_layout() {
        let source = "foo() {\necho hi\n} # trailing\nbar\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("foo() {\n\techo hi\n} # trailing\nbar\n".to_string())
        );
    }

    #[test]
    fn preserves_escaped_command_names() {
        let source = "\\grep -q foo file\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_ansi_c_quoted_assignment_values() {
        let source = "x=$'\\n'\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_ansi_c_quoted_condition_patterns() {
        let source = "[[ \"$c\" == $'\\r' || \"$c\" == $'\\n' ]]\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_function_bodies() {
        let source = "version_gt() {\n\t[[ \"${1%.*}\" -gt \"${2%.*}\" ]]\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_process_substitution_redirect_spacing() {
        let source = "cat < <(which -a foo)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_numbered_output_redirects_tight_like_shfmt() {
        let source = "exec 19>\"${config_dir}/tracing.log\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_fd_duplication_redirect_targets() {
        let source = "cmd 2>&$fd\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_regex_operands_in_conditionals() {
        let source = "[[ \"$x\" =~ \"git version \"([^ ]*).* ]]\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_explicit_line_break_after_list_operator() {
        let source = "command -v curl >/dev/null &&\n  if [[ $1 =~ tar.gz$ ]]; then\n    curl -fL $1 | tar $tar_opts\n  else\n    echo nope\n  fi\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);
        let formatted =
            format_source(source, Some(Path::new("list_operator_if.bash")), &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "command -v curl >/dev/null &&\n\tif [[ $1 =~ tar.gz$ ]]; then\n\t\tcurl -fL $1 | tar $tar_opts\n\telse\n\t\techo nope\n\tfi\n".to_string()
            )
        );
    }

    #[test]
    fn binary_next_line_pipeline_keeps_heredoc_body_unindented() {
        let options = ShellFormatOptions::default().with_binary_next_line(true);
        let formatted =
            format_source("cat foo | \\\ncat<<EOF\nhello\nEOF\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("cat foo \\\n\t| cat <<EOF\nhello\nEOF\n".to_string())
        );
    }

    #[test]
    fn binary_next_line_does_not_force_single_line_pipelines_to_wrap() {
        let options = ShellFormatOptions::default().with_binary_next_line(true);
        let formatted = format_source("cat foo | cat bar\n", None, &options).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn honors_function_next_line_option() {
        let source = "foo(){\necho hi\n}\n";
        let options = ShellFormatOptions::default().with_function_next_line(true);
        let formatted = format_source(source, None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("foo()\n{\n\techo hi\n}\n".to_string())
        );
    }

    #[test]
    fn minify_drops_comments_but_preserves_shebang() {
        let options = ShellFormatOptions::default().with_minify(true);
        let formatted = format_source("#!/bin/bash\necho hi # note\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("#!/bin/bash\necho hi\n".to_string())
        );
    }

    #[test]
    fn formats_case_items_with_expected_indentation() {
        let source = "case $x in\na) echo a;;\nb) echo b;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case $x in\na) echo a ;;\nb) echo b ;;\nesac\n".to_string()
            )
        );
    }

    #[test]
    fn switch_case_indent_indents_patterns_and_bodies() {
        let source = "case $x in\na) echo a;;\nesac\n";
        let options = ShellFormatOptions::default().with_switch_case_indent(true);
        let formatted = format_source(source, None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("case $x in\n\ta)\n\t\techo a\n\t\t;;\nesac\n".to_string())
        );
    }

    #[test]
    fn space_redirects_insert_spaces_between_operator_and_target() {
        let options = ShellFormatOptions::default().with_space_redirects(true);
        let formatted = format_source("echo hi>out\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo hi > out\n".to_string())
        );
    }

    #[test]
    fn keep_padding_preserves_safe_verbatim_regions() {
        let options = ShellFormatOptions::default().with_keep_padding(true);
        let formatted = format_source("a=1  b=2\n", None, &options).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn keep_padding_still_formats_unpadded_syntax() {
        let options = ShellFormatOptions::default().with_keep_padding(true);
        let formatted = format_source("#!/bin/bash\n echo hi\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("#!/bin/bash\necho hi\n".to_string())
        );
        assert!(!source_is_formatted("#!/bin/bash\n echo hi\n", None, &options).unwrap());
    }

    #[test]
    fn normalizes_extra_crlf_trailing_newlines() {
        let source = "echo hi\r\n\r\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, Some(Path::new("test.sh")), &options).unwrap(),
            FormattedSource::Formatted("echo hi\r\n".to_string())
        );
        assert!(!source_is_formatted(source, Some(Path::new("test.sh")), &options).unwrap());
    }

    #[test]
    fn never_split_prefers_compact_layouts() {
        let options = ShellFormatOptions::default().with_never_split(true);
        let formatted = format_source("if true; then\necho hi\nfi\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("if true; then echo hi; fi\n".to_string())
        );
    }

    #[test]
    fn auto_dialect_honors_env_shebang() {
        let error = format_source(
            "#!/usr/bin/env sh\n[[ foo == bar ]]\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            FormatError::Parse { message, .. } if message.contains("[[ ]] conditionals")
        ));

        let split_error = format_source(
            "#!/usr/bin/env -S sh -e\n[[ foo == bar ]]\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(
            split_error,
            FormatError::Parse { message, .. } if message.contains("[[ ]] conditionals")
        ));
    }

    #[test]
    fn auto_dialect_honors_zsh_paths_and_shebangs() {
        let path_formatted = format_source(
            "print ${(m)foo}\n",
            Some(Path::new("script.zsh")),
            &ShellFormatOptions::default(),
        )
        .unwrap();
        assert_eq!(path_formatted, FormattedSource::Unchanged);

        let shebang_formatted = format_source(
            "#!/usr/bin/env zsh\nprint ${(m)foo}\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();
        assert_eq!(shebang_formatted, FormattedSource::Unchanged);

        let split_shebang_formatted = format_source(
            "#!/usr/bin/env -S zsh -f\nprint ${(m)foo}\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();
        assert_eq!(split_shebang_formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn zsh_only_forms_round_trip_without_corruption() {
        let source = "\
print ${(M)${(k)parameters[@]}:#__gitcomp_builtin_*}
print ${(m)foo#${needle}} ${(S)foo/$pattern/$replacement} ${(m)foo:$offset:${length}} ${(m)foo:^other}
print (#i)*.jpg (#b)(*) *.log(#qN) **/*(#q.om[1,3])
repeat 3 print hi
for version ($versions); do print $version; done
for key value in a b c d; { print -r -- \"$key:$value\"; }
for 1 2 3; do print -r -- \"$1|$2|$3\"; done
if [[ -n $foo ]] { print foo; } else { print bar; }
{ print body; } always { print cleanup; }
print quiet &|
print hidden &!
";
        let options = ShellFormatOptions::default()
            .with_dialect(ShellDialect::Zsh)
            .with_simplify(true);

        assert_eq!(
            format_source(source, Some(Path::new("script.zsh")), &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, Some(Path::new("script.zsh")), &options);
    }

    #[test]
    fn mksh_dialect_formats_select_commands() {
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Mksh);
        let formatted = format_source(
            "select name in foo; do echo \"$name\"; done\n",
            None,
            &options,
        )
        .unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn posix_dialect_propagates_parse_errors() {
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Posix);
        let error =
            format_source("[[ foo == bar ]]\n", Some(Path::new("test.sh")), &options).unwrap_err();

        assert!(matches!(
            error,
            FormatError::Parse { message, .. } if message.contains("[[ ]] conditionals")
        ));
    }

    #[test]
    fn preserves_quoted_associative_subscript_syntax_when_formatting() {
        let options = ShellFormatOptions::default();

        for source in [
            "printf '%s\\n' ${assoc[\"key\"]}\n",
            "printf '%s\\n' ${assoc['k']}\n",
            "[[ -v assoc[\"k\"] ]]\n",
            "declare -A assoc=([\"key\"]=v ['alt']+=w)\n",
        ] {
            assert_eq!(
                format_source(source, None, &options).unwrap(),
                FormattedSource::Unchanged
            );
            assert_source_and_ast_paths_match(source, None, &options);
        }
    }

    #[test]
    fn preserves_prefix_match_selector_kind_when_formatting() {
        let source = "printf '%s\\n' \"${!prefix@}\" \"${!prefix*}\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn format_file_ast_matches_format_source_for_formatter_fixtures() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle-fixtures");
        let cases = vec![
            (
                "function_next_line.sh",
                "function_next_line.sh",
                ShellFormatOptions::default().with_function_next_line(true),
            ),
            (
                "case_default.sh",
                "case_default.sh",
                ShellFormatOptions::default(),
            ),
            (
                "space_redirects.sh",
                "space_redirects.sh",
                ShellFormatOptions::default().with_space_redirects(true),
            ),
            (
                "keep_padding.sh",
                "keep_padding.sh",
                ShellFormatOptions::default().with_keep_padding(true),
            ),
            (
                "never_split.sh",
                "never_split.sh",
                ShellFormatOptions::default().with_never_split(true),
            ),
            (
                "nested_heredoc.sh",
                "nested_heredoc.sh",
                ShellFormatOptions::default(),
            ),
            (
                "binary_next_line.sh",
                "binary_next_line.sh",
                ShellFormatOptions::default().with_binary_next_line(true),
            ),
            (
                "simplify.sh",
                "simplify.bash",
                ShellFormatOptions::default().with_simplify(true),
            ),
            (
                "minify.sh",
                "minify.sh",
                ShellFormatOptions::default().with_minify(true),
            ),
            (
                "mksh_select.sh",
                "script.mksh",
                ShellFormatOptions::default().with_dialect(ShellDialect::Mksh),
            ),
        ];

        for (fixture, filename, options) in cases {
            let source = fs::read_to_string(fixture_root.join(fixture)).unwrap();
            assert_source_and_ast_paths_match(&source, Some(Path::new(filename)), &options);
        }

        assert_source_and_ast_paths_match(
            "if true; then\n# note\necho hi\nfi\n",
            Some(Path::new("if_body_comment.sh")),
            &ShellFormatOptions::default(),
        );
        assert_source_and_ast_paths_match(
            "cat <<EOF # note\nhi\nEOF\n",
            Some(Path::new("heredoc_trailing_comment.sh")),
            &ShellFormatOptions::default(),
        );
        assert_source_and_ast_paths_match(
            "declare -x foo=1<<EOF\nhi\nEOF\n",
            Some(Path::new("decl_heredoc.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn stable_formatter_fixtures_are_idempotent() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle-fixtures");
        let cases = vec![
            (
                "function_next_line.sh",
                "function_next_line.sh",
                ShellFormatOptions::default().with_function_next_line(true),
            ),
            (
                "case_default.sh",
                "case_default.sh",
                ShellFormatOptions::default(),
            ),
            (
                "space_redirects.sh",
                "space_redirects.sh",
                ShellFormatOptions::default().with_space_redirects(true),
            ),
            (
                "keep_padding.sh",
                "keep_padding.sh",
                ShellFormatOptions::default().with_keep_padding(true),
            ),
            (
                "never_split.sh",
                "never_split.sh",
                ShellFormatOptions::default().with_never_split(true),
            ),
            (
                "nested_heredoc.sh",
                "nested_heredoc.sh",
                ShellFormatOptions::default(),
            ),
            (
                "simplify.sh",
                "simplify.bash",
                ShellFormatOptions::default().with_simplify(true),
            ),
            (
                "minify.sh",
                "minify.sh",
                ShellFormatOptions::default().with_minify(true),
            ),
            (
                "mksh_select.sh",
                "script.mksh",
                ShellFormatOptions::default().with_dialect(ShellDialect::Mksh),
            ),
        ];

        for (fixture, filename, options) in cases {
            let source = fs::read_to_string(fixture_root.join(fixture)).unwrap();
            assert_idempotent(&source, Some(Path::new(filename)), &options);
        }

        for (source, filename, options) in [
            (
                "if true; then\n\techo hi\nfi\n",
                "if_body.sh",
                ShellFormatOptions::default(),
            ),
            (
                "foo() {\n\techo hi\n}\n",
                "func.sh",
                ShellFormatOptions::default(),
            ),
            (
                "echo hi > out\n",
                "redirect.sh",
                ShellFormatOptions::default().with_space_redirects(true),
            ),
        ] {
            assert_idempotent(source, Some(Path::new(filename)), &options);
        }
    }

    #[test]
    fn preserves_group_spacing_idempotently_for_nested_subshells() {
        assert_idempotent(
            "foo(foo()) \n",
            Some(Path::new("nested_subshell_spacing.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_blank_lines_after_multiline_subshells_idempotently() {
        assert_idempotent(
            "(\n\techo hi\n)\n\nfoo() {\n\techo bye\n}\n",
            Some(Path::new("multiline_subshell_gap.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_blank_lines_after_multiline_brace_groups_idempotently() {
        assert_idempotent(
            "{\n\techo hi\n}\n\nfoo() {\n\techo bye\n}\n",
            Some(Path::new("multiline_brace_gap.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_blank_line_before_group_close_like_shfmt() {
        let source = "prefer_openssl11() {\n  PYTHON_BUILD_MACPORTS_OPENSSL_FORMULA=x\n  export PYTHON_BUILD_MACPORTS_OPENSSL_FORMULA\n\n}\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "prefer_openssl11() {\n\tPYTHON_BUILD_MACPORTS_OPENSSL_FORMULA=x\n\texport PYTHON_BUILD_MACPORTS_OPENSSL_FORMULA\n\n}\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_blank_line_after_for_do_like_shfmt() {
        let source = "for ((i = 0; i <= 100; i++)); do\n\n  echo \"$i\"\ndone\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "for ((i = 0; i <= 100; i++)); do\n\n\techo \"$i\"\ndone\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_blank_line_before_for_done_like_shfmt() {
        let source = "for ((i = 0; i <= 100; i++)); do\n  echo \"$i\"\n\ndone\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "for ((i = 0; i <= 100; i++)); do\n\techo \"$i\"\n\ndone\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_blank_line_after_until_do_like_shfmt() {
        let source = "until ((y == done_val)); do\n\n  echo \"$y\"\ndone\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "until ((y == done_val)); do\n\n\techo \"$y\"\ndone\n".to_string()
            )
        );
    }

    #[test]
    fn preserves_blank_line_after_do_before_leading_comment_like_shfmt() {
        let source = "until foo; do\n\n  # comment\n  bar\ndone\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("until foo; do\n\n\t# comment\n\tbar\ndone\n".to_string())
        );
    }

    #[test]
    fn preserves_while_do_suffix_comments_like_shfmt() {
        let source = "while IFS= read -r -u ${pycoproc[0]} -t 1 output; do #2>/dev/null\n  echo \"$output\"\ndone\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "while IFS= read -r -u ${pycoproc[0]} -t 1 output; do #2>/dev/null\n\techo \"$output\"\ndone\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn preserves_spacing_after_nested_multiline_subshells_before_simple_commands() {
        assert_idempotent(
            "(\n\t(\n\t\tfalse\n\t)\n)\ngrep \"name delta\"\n",
            Some(Path::new("nested_multiline_subshell_then_stmt.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_spacing_after_nested_multiline_subshells_before_other_groups() {
        assert_idempotent(
            "(\n\t(\n\t\tfalse\n\t)\n)\n(\n\twhile true; do\n\t\t:\n\tdone\n)\n",
            Some(Path::new("nested_multiline_subshell_then_group.bash")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_spacing_after_subshells_that_end_with_function_definitions() {
        assert_idempotent(
            "foo() {\n\t(\n\t\tbar() {\n\t\t\techo hi\n\t\t}\n\t)\n\n\tprintf x\n}\n",
            Some(Path::new("subshell_function_tail_gap.bash")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_shebang_spacing_before_nested_multiline_groups_idempotently() {
        assert_idempotent(
            "#!/usr/bin/env bash\n\n(\n\t(\n\t\techo hi\n\t)\n)\n",
            Some(Path::new("shebang_nested_groups.bash")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_legacy_bracket_arithmetic_idempotently() {
        assert_idempotent(
            "#!/bin/sh\n\ni=$[$i+1]\n",
            Some(Path::new("legacy_bracket_arithmetic.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn invalid_shebang_like_line_does_not_move_off_the_first_line() {
        let source = "#!/bin/bash<echo hi # note\n";
        let formatted = match format_source(
            source,
            Some(Path::new("fuzz.sh")),
            &ShellFormatOptions::default(),
        )
        .unwrap()
        {
            FormattedSource::Unchanged => source.to_string(),
            FormattedSource::Formatted(formatted) => formatted,
        };

        assert_eq!(formatted, source);
    }

    #[test]
    fn preserves_conditional_words_that_look_like_unary_operators() {
        assert_idempotent(
            "[[ -n]]\n",
            Some(Path::new("fuzz.bash")),
            &ShellFormatOptions::default(),
        );
        assert_idempotent(
            "[[ ! -n]]\n",
            Some(Path::new("fuzz.bash")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn keeps_unterminated_heredoc_closers_on_their_own_lines() {
        let source = "x foo=1<<EOF=1<<EOF\nhi";
        let formatted = format_to_string(
            source,
            Some(Path::new("fuzz.sh")),
            &ShellFormatOptions::default(),
        );

        assert_eq!(formatted, "x foo=1 <<EOF=1 <<EOF\nhi\nEOF=1\nEOF\n");
        assert_idempotent(
            source,
            Some(Path::new("fuzz.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_body_lines_when_synthesizing_missing_heredoc_closers() {
        let source = "d8<<EGF\nhi\nEOF";
        let formatted = format_to_string(
            source,
            Some(Path::new("fuzz.sh")),
            &ShellFormatOptions::default(),
        );

        assert_eq!(formatted, "d8 <<EGF\nhi\nEOF\nEGF\n");
        assert_idempotent(
            source,
            Some(Path::new("fuzz.sh")),
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn keeps_trailing_backslash_pipeline_reproducer_parseable() {
        let source = "cat foo | \\\\t foo | \\";
        let path = Some(Path::new("fuzz.sh"));
        let options = ShellFormatOptions::default();

        let once = format_to_string(source, path, &options);
        let twice = format_source(&once, path, &options).unwrap_or_else(|err| {
            panic!("second format pass failed: {err}\nformatted source:\n{once:?}")
        });
        let twice = match twice {
            FormattedSource::Unchanged => once.clone(),
            FormattedSource::Formatted(formatted) => formatted,
        };

        assert_eq!(once, twice);
    }

    #[test]
    fn does_not_introduce_unused_assignment_for_backslash_heredoc_reproducer() {
        let source = "cat foo E\r\nnnnnnnnnn1jn \\\ncat=,EOlo\nEOF\n";
        let path = Path::new("fuzz.sh");
        let formatted = format_to_string(source, Some(path), &ShellFormatOptions::default());

        let original_c001 = diagnostic_count(&lint_source_posix_strict(source, path), "C001");
        let formatted_c001 = diagnostic_count(&lint_source_posix_strict(&formatted, path), "C001");

        assert!(
            formatted_c001 <= original_c001,
            "formatter introduced extra C001 diagnostics: original={original_c001}, formatted={formatted_c001}\nformatted source:\n{formatted:?}"
        );
    }
}
