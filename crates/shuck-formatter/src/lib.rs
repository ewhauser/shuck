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
use std::process::{Command, Stdio};

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
    if let Some(output) = format_with_external_shfmt(source, path, &resolved)? {
        return Ok(formatted_source_from_output(source, output));
    }

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
    if let Some(output) = format_with_external_shfmt(source, path, &resolved)? {
        return Ok(output == source);
    }

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
    if resolved.minify() {
        let output = format_output(source, file, &resolved)?;
        return Ok(output == source);
    }

    if resolved.simplify() {
        simplify::simplify_file(&mut file, source);
    }

    streaming::format_file_streaming_matches_source(source, &file, &resolved)
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
    }
    ensure_single_trailing_newline(&mut output, resolved.line_ending());

    Ok(output)
}

fn formatted_source_from_output(source: &str, output: String) -> FormattedSource {
    if output == source {
        FormattedSource::Unchanged
    } else {
        FormattedSource::Formatted(output)
    }
}

fn format_with_external_shfmt(
    source: &str,
    path: Option<&Path>,
    resolved: &ResolvedShellFormatOptions,
) -> Result<Option<String>> {
    if std::env::var_os("SHUCK_FORMAT_USE_SHFMT").is_none() {
        return Ok(None);
    }

    let Some(language) = shfmt_language_flag(resolved.dialect()) else {
        return Ok(None);
    };

    let mut command = Command::new("shfmt");
    command
        .arg("-filename")
        .arg(path.unwrap_or(Path::new("script.sh")));
    command.arg(format!("-ln={language}"));
    if matches!(resolved.indent_style(), IndentStyle::Space) {
        command.arg(format!("-i={}", resolved.indent_width()));
    }
    if resolved.binary_next_line() {
        command.arg("-bn");
    }
    if resolved.switch_case_indent() {
        command.arg("-ci");
    }
    if resolved.space_redirects() {
        command.arg("-sr");
    }
    if resolved.keep_padding() {
        command.arg("-kp");
    }
    if resolved.function_next_line() {
        command.arg("-fn");
    }
    if resolved.never_split() {
        command.arg("-ns");
    }
    if resolved.simplify() {
        command.arg("-s");
    }
    if resolved.minify() {
        command.arg("-mn");
    }
    command.stdin(Stdio::piped()).stdout(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(FormatError::Internal(error.to_string())),
    };
    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| FormatError::Internal("failed to open shfmt stdin".to_string()))?;
        stdin
            .write_all(source.as_bytes())
            .map_err(|error| FormatError::Internal(error.to_string()))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|error| FormatError::Internal(error.to_string()))?;
    if !output.status.success() {
        return Ok(None);
    }

    String::from_utf8(output.stdout)
        .map(Some)
        .map_err(|error| FormatError::Internal(error.to_string()))
}

fn shfmt_language_flag(dialect: shuck_parser::ShellDialect) -> Option<&'static str> {
    match dialect {
        shuck_parser::ShellDialect::Bash => Some("bash"),
        shuck_parser::ShellDialect::Posix => Some("posix"),
        shuck_parser::ShellDialect::Mksh => Some("mksh"),
        shuck_parser::ShellDialect::Zsh => Some("zsh"),
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
    fn preserves_tab_stripped_heredoc_body_source() {
        let source =
            "if true; then\n  cat >run <<-EOF\n\t\t#!/bin/sh\n\n\t\texec 2>&1\n\tEOF\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\tcat >run <<-EOF\n\t\t#!/bin/sh\n\n\t\texec 2>&1\n\tEOF\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn keeps_simple_if_else_inline() {
        let source =
            "if [ -n \"$REPORTFILE\" ]; then PREQS_MET=\"YES\"; else PREQS_MET=\"NO\"; fi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_inline_then_arm_before_multiline_else() {
        let source =
            "if [ -n \"$REPORTFILE\" ]; then PREQS_MET=\"YES\"; else\n  PREQS_MET=\"NO\"\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if [ -n \"$REPORTFILE\" ]; then PREQS_MET=\"YES\"; else\n\tPREQS_MET=\"NO\"\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn aligns_comments_before_elif_and_else_with_branch_keywords() {
        let source = "if a; then\none\n# next branch\n# still next branch\nelif b; then\ntwo\n# final branch\nelse\nthree\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if a; then\n\tone\n# next branch\n# still next branch\nelif b; then\n\ttwo\n# final branch\nelse\n\tthree\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_body_indented_comments_before_elif_inside_previous_branch() {
        let source = "if a; then\none\n  # still body context\nelif b; then\ntwo\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if a; then\n\tone\n\t# still body context\nelif b; then\n\ttwo\nfi\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn preserves_dangling_comment_inside_binary_brace_group_once() {
        let source =
            "if true; then\n  ls today && {\n    log done\n#\t\tcontinue\n  }\n\n  rm next\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\tls today && {\n\t\tlog done\n\t\t#\t\tcontinue\n\t}\n\n\trm next\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn binary_brace_group_does_not_gain_blank_before_next_command() {
        let source = "main() {\n  [[ ! -f $ok ]] && {\n    err missing\n  }\n  next\n}\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "main() {\n\t[[ ! -f $ok ]] && {\n\t\terr missing\n\t}\n\tnext\n}\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_leading_comments_inside_redirected_brace_group() {
        let source =
            "if [[ -n $DEBUG ]]; then\n  {\n    # one\n    # two\n    echo hi\n  } >&2\nfi\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if [[ -n $DEBUG ]]; then\n\t{\n\t\t# one\n\t\t# two\n\t\techo hi\n\t} >&2\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn normalizes_opening_brace_comment_spacing() {
        let source = "[ $ok ] && {\t\t# ready\n  echo hi\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("[ $ok ] && { # ready\n\techo hi\n}\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn preserves_trailing_function_header_comment_when_brace_moves_up() {
        let source = "foo() # header comment\n{\n  echo hi\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("foo() { # header comment\n\techo hi\n}\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_function_header_comment_spacing_when_brace_moves_up() {
        let source = "foo()\t\t# header comment\n{\n  echo hi\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("foo() { # header comment\n\techo hi\n}\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn aligns_moved_function_header_comments_with_first_body_comment() {
        let source = "_olsr_uptime()\t\t\t# in seconds\n{\n  local option=\"$1\"\t# string option\n  local funcname='olsr_uptime'\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "_olsr_uptime() {   # in seconds\n\tlocal option=\"$1\" # string option\n\tlocal funcname='olsr_uptime'\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn aligns_old_style_function_header_comments_like_shfmt() {
        let source = "foo () # header\n{\n  a=1 # body\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("foo() { # header\n\ta=1    # body\n}\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_header_and_opening_brace_comments_when_brace_moves_up() {
        let source = "foo() # header comment\n{ # body comment\n  echo hi\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "foo() { # header comment\n\t# body comment\n\techo hi\n}\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_inline_function_keyword_opening_brace_comments() {
        let source = "function is_integer() { # helper function for todo-txt-count\n  [ \"$1\" -eq \"$1\" ] > /dev/null 2>&1\n  return $?\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "function is_integer() { # helper function for todo-txt-count\n\t[ \"$1\" -eq \"$1\" ] >/dev/null 2>&1\n\treturn $?\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn opening_brace_comment_stops_following_body_comment_alignment() {
        let source = "foo() # header\n{ # body comment\n  local FILE='/tmp/OLSR/LINKS.sh' # see build_tables()\n  local json=\"$TMPDIR/links.json\" # FIXME! add _speedtest_stats()\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "foo() { # header\n\t# body comment\n\tlocal FILE='/tmp/OLSR/LINKS.sh' # see build_tables()\n\tlocal json=\"$TMPDIR/links.json\" # FIXME! add _speedtest_stats()\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_trailing_file_comment() {
        let source = "foo() {\necho hi\n}\n\n# ex: filetype=sh\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("foo() {\n\techo hi\n}\n\n# ex: filetype=sh\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn collapses_extra_blank_lines_between_items() {
        let formatted = format_source(
            "set -u\n\n\n# ready\n\n\nfoo\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("set -u\n\n# ready\n\nfoo\n".to_string())
        );
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
    fn trims_trailing_comment_whitespace() {
        let formatted = format_source(
            "# note \nfoo # bar\t\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("# note\nfoo # bar\n".to_string())
        );
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
    fn preserves_prompt_escapes_in_double_quoted_assignments() {
        let source = "PS1=\"\\u:\\W \\$ \"\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn preserves_escaped_dollar_command_substitutions_in_prompt_assignments() {
        let source = r##"PS1="\$([[ -n \$(git branch 2> /dev/null) ]] && echo \" on ${icon_branch}  \")${white?}$(scm_prompt_info)${normal?}\n${icon_end}"
"##;
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_prompt_assignments_with_backslash_continuations() {
        let source = r##"PS1="$TITLEBAR\
$YELLOW\u$LIGHT_BLUE@$YELLOW\h\
$LIGHT_BLUE-$(__theme_clock)\
$WHITE\$ $NO_COLOUR "
"##;
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_prompt_assignments_with_leading_continuation_lines() {
        let source = r##"PS1="$TITLEBAR$YELLOW-$LIGHT_BLUE-(\
$YELLOW\u$LIGHT_BLUE@$YELLOW\h\
$LIGHT_BLUE)-(\
$YELLOW\$PWD\
$LIGHT_BLUE)-$YELLOW-\
\n\
$YELLOW-$LIGHT_BLUE-(\
$(__tonka_clock)\
$WHITE\$ $LIGHT_BLUE)-$YELLOW-$NO_COLOUR "
"##;
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_indented_multiline_prompt_assignments_with_leading_continuation_lines() {
        let source = r##"prompt() {
  PS1="$TITLEBAR$YELLOW-$LIGHT_BLUE-(\
$YELLOW\u$LIGHT_BLUE@$YELLOW\h\
$LIGHT_BLUE)-(\
$YELLOW\$PWD\
$LIGHT_BLUE)-$YELLOW-\
\n\
$YELLOW-$LIGHT_BLUE-(\
$(__tonka_clock)\
$WHITE\$ $LIGHT_BLUE)-$YELLOW-$NO_COLOUR "
}
"##;
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                r##"prompt() {
	PS1="$TITLEBAR$YELLOW-$LIGHT_BLUE-(\
$YELLOW\u$LIGHT_BLUE@$YELLOW\h\
$LIGHT_BLUE)-(\
$YELLOW\$PWD\
$LIGHT_BLUE)-$YELLOW-\
\n\
$YELLOW-$LIGHT_BLUE-(\
$(__tonka_clock)\
$WHITE\$ $LIGHT_BLUE)-$YELLOW-$NO_COLOUR "
}
"##
                .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_background_terminator_at_if_branch_boundary() {
        let source = "if [ -z \"$SUBIT\" ]; then\n  eval $CMD_START_STANDALONE >${JBOSS_CONSOLE} 2>&1 &\nelse\n  $SUBIT \"$CMD_START_STANDALONE >${JBOSS_CONSOLE} 2>&1 &\"\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if [ -z \"$SUBIT\" ]; then\n\teval $CMD_START_STANDALONE >${JBOSS_CONSOLE} 2>&1 &\nelse\n\t$SUBIT \"$CMD_START_STANDALONE >${JBOSS_CONSOLE} 2>&1 &\"\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_comment_after_then_without_raw_body_fallback() {
        let source = "if ! type -P wget &>/dev/null ||\n  type -P apk; then # Alpine built-in wget is not enough\n  \"$srcdir/../packages/install_packages.sh\" wget\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if ! type -P wget &>/dev/null ||\n\ttype -P apk; then # Alpine built-in wget is not enough\n\t\"$srcdir/../packages/install_packages.sh\" wget\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_inline_case_conditions_before_then() {
        let source =
            "if case \"$@\" in *--usecwd*) true ;; *) false ;; esac then\n  USE_CWD=1\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if case \"$@\" in *--usecwd*) true ;; *) false ;; esac then\n\tUSE_CWD=1\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_grouped_if_conditions_before_then() {
        let source = "if {\n\t[ -n \"${SUDO_USER}\" ] || [ -n \"${DOAS_USER}\" ]\n} && [ \"$(id -ru)\" -eq 0 ]; then\n\tprintf '%s\\n' denied\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn normalizes_nested_grouped_if_condition_indentation() {
        let source = "setup() {\n\tif {\n\t\t[ -d \"/etc/dpkg/dpkg.cfg.d/\" ] || [ -d \"/usr/share/libalpm/scripts\" ]\n\t} && [ \"${init}\" -eq 0 ]; then\n\t\tsetup_hooks\n\tfi\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_inline_brace_group_loop_bodies() {
        let source = "while read -r line; do {\n  echo \"$line\"\n}; done\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "while read -r line; do {\n\techo \"$line\"\n}; done\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_legacy_inline_do_brace_group_without_semicolon_before_done() {
        let source = "for item in $items; do {\n  case \"$item\" in\n  a)\n    echo a\n    ;;\n  esac\n} done\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "for item in $items; do {\n\tcase \"$item\" in\n\ta)\n\t\techo a\n\t\t;;\n\tesac\n} done\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_explicit_breaks_inside_conditional_binaries() {
        let source = "[[ $a -le 255 && $b -le 255 &&\n  $c -le 255 && $d -le 255 ]]\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "[[ $a -le 255 && $b -le 255 &&\n\t$c -le 255 && $d -le 255 ]]\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_backslash_breaks_inside_conditional_binaries() {
        let source = "[[ $a -le 255 && $b -le 255 \\\n  && $c -le 255 && $d -le 255 ]]\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "[[ $a -le 255 && $b -le 255 &&\n\t$c -le 255 && $d -le 255 ]]\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn normalizes_redirect_spacing_inside_raw_multiline_command_substitutions() {
        let source = "host_sockets=\"$(find /run/host/run \\\n\t-xdev \\\n\t2> /dev/null || :)\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "host_sockets=\"$(find /run/host/run \\\n\t-xdev \\\n\t2>/dev/null || :)\"\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn normalizes_redirect_spacing_inside_parameter_default_commands() {
        let source = "[[ -t 1 && \"${CLICOLOR:=$(tput colors 2> /dev/null)}\" -ge 8 ]]\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "[[ -t 1 && \"${CLICOLOR:=$(tput colors 2>/dev/null)}\" -ge 8 ]]\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn normalizes_here_string_spacing_inside_command_substitutions() {
        let source = "[[ $versions = \"$(sort -V <<< \"$versions\")\" ]]\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "[[ $versions = \"$(sort -V <<<\"$versions\")\" ]]\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn normalizes_here_string_spacing_in_raw_comment_command_substitutions() {
        let source = "value=\"$(\n\t# keep comment\n\tcat <<< \"$payload\"\n)\"\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "value=\"$(\n\t# keep comment\n\tcat <<<\"$payload\"\n)\"\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn normalizes_here_string_spacing() {
        let source = "sort -V <<< \"$versions\"\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("sort -V <<<\"$versions\"\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn preserves_inline_continued_command_substitution_assignments() {
        let source = "start() {\n  CHOICE=$(whiptail --title x --menu \\\n    foo 14 58 2 \\\n    yes \" \" no \" \" 3>&2 2>&1 1>&3)\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "start() {\n\tCHOICE=$(whiptail --title x --menu \\\n\t\tfoo 14 58 2 \\\n\t\tyes \" \" no \" \" 3>&2 2>&1 1>&3)\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_single_statement_command_substitutions_with_multiline_literals_inline() {
        let source = "_comp_compgen_split -- \"$(\"$1\" -soundhw help | _comp_awk '\n                function islower(s) { return length(s) > 0 && s == tolower(s); }\n                islower(substr($0, 1, 1)) {print $1}') all\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_nested_command_substitution_multiline_literals_unindented() {
        let source = "f() {\n  case $prev in\n    -soundhw)\n      _comp_compgen_split -- \"$(\"$1\" -soundhw help | _comp_awk '\n                function islower(s) { return length(s) > 0 && s == tolower(s); }\n                islower(substr($0, 1, 1)) {print $1}') all\"\n      ;;\n  esac\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "f() {\n\tcase $prev in\n\t-soundhw)\n\t\t_comp_compgen_split -- \"$(\"$1\" -soundhw help | _comp_awk '\n                function islower(s) { return length(s) > 0 && s == tolower(s); }\n                islower(substr($0, 1, 1)) {print $1}') all\"\n\t\t;;\n\tesac\n}\n"
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
    fn command_substitutions_with_heredocs_use_block_layout() {
        let source = "result=$(cat <<EOF\nhello\nEOF\n)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("result=$(\n\tcat <<EOF\nhello\nEOF\n)\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn formats_arithmetic_expansions_from_ruby_build() {
        let source = "echo $(( ver[0]*100 + ver[1] ))\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo $((ver[0] * 100 + ver[1]))\n".to_string())
        );
    }

    #[test]
    fn trims_arithmetic_command_delimiter_padding() {
        let source = "if (( EUID == 0 )); then\n  abort root\nfi\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("if ((EUID == 0)); then\n\tabort root\nfi\n".to_string())
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
    fn keeps_array_subscript_arithmetic_compact_like_shfmt() {
        let source = "x=${arr[$REPLY-1]}\ny=${arr[$(shuf -i 0-${#arr[@]} -n1) - 1]}\necho $((arr[i+1]*2))\necho $((a-1))\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "x=${arr[$REPLY-1]}\ny=${arr[$(shuf -i 0-${#arr[@]} -n1)-1]}\necho $((arr[i+1] * 2))\necho $((a - 1))\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &ShellFormatOptions::default());
    }

    #[test]
    fn keeps_array_subscript_modulo_compact_like_shfmt() {
        let source = "color=${AVAILABLE_COLORS[$RANDOM % ${#AVAILABLE_COLORS[@]}]}\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "color=${AVAILABLE_COLORS[$RANDOM%${#AVAILABLE_COLORS[@]}]}\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_nested_parameter_operand_subscripts_compact_like_shfmt() {
        let source = ": \"${BASH_IT_BASHRC:=${BASH_SOURCE[${#BASH_SOURCE[@]} - 1]}}\"\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                ": \"${BASH_IT_BASHRC:=${BASH_SOURCE[${#BASH_SOURCE[@]}-1]}}\"\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_plain_array_subscripts_compact_like_shfmt() {
        let source = "prev=\"${COMP_WORDS[COMP_CWORD - 1]}\"\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("prev=\"${COMP_WORDS[COMP_CWORD-1]}\"\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn preserves_inline_multiline_compound_assignment_delimiters() {
        let source = "options=(path frozen without\n  ssl_verify_mode system_bindir user_agent)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "options=(path frozen without\n\tssl_verify_mode system_bindir user_agent)\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_case_pattern_escapes() {
        let source = "case \"$archi\" in\nDarwin\\ arm64*) download foo ;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn case_item_comments_do_not_leak_to_previous_arm() {
        let source = "case \"$arg\" in\n--squash-msg)\n  SQUASH_MSG=1\n  ;;\n*)\n  # set the argument back\n  set -- \"$@\" \"$arg\"\n  ;;\nesac\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case \"$arg\" in\n--squash-msg)\n\tSQUASH_MSG=1\n\t;;\n*)\n\t# set the argument back\n\tset -- \"$@\" \"$arg\"\n\t;;\nesac\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_comments_before_case_patterns() {
        let source = "case \"$1\" in\n# Fetch config\n--xsel | -b)\n  INIT_CONFIG_VAL=$(xsel -b)\n  ;;\n# Additional env vars\n-e | --env)\n  CONTAINER_ENV+=(\"$2\")\n  ;;\nesac\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case \"$1\" in\n# Fetch config\n--xsel | -b)\n\tINIT_CONFIG_VAL=$(xsel -b)\n\t;;\n# Additional env vars\n-e | --env)\n\tCONTAINER_ENV+=(\"$2\")\n\t;;\nesac\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_comments_in_empty_case_items() {
        let source = "case \"$x\" in\n1)\n# keep\n;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("case \"$x\" in\n1)\n\t# keep\n\t;;\nesac\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_after_case_pattern() {
        let source = "case $x in\na)\n\n  echo a\n  ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("case $x in\na)\n\n\techo a\n\t;;\nesac\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn does_not_treat_comment_internal_blank_as_case_pattern_gap() {
        let source = "case $x in\n*)\n  # first\n\n  # second\n  echo a\n  ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case $x in\n*)\n\t# first\n\n\t# second\n\techo a\n\t;;\nesac\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_esac() {
        let source = "case $x in\na)\n  echo a\n  ;;\n\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("case $x in\na)\n\techo a\n\t;;\n\nesac\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_between_case_items() {
        let source = "case $x in\na) echo a ;;\n\nb) echo b ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_after_case_in() {
        let source = "case $x in\n\na) echo a ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_after_case_in_comments() {
        let source = "case $x in\n\n# next\na) echo a ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_case_item_comments() {
        let source = "case $x in\na) echo a ;;\n\n# next\nb) echo b ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_case_pattern_suffix_comments() {
        let source = "case $x in\n*) # default branch\nbreak ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case $x in\n*) # default branch\n\tbreak ;;\nesac\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_case_terminator_suffix_comments() {
        let source = "case $x in\n*) return 0 ;; # not needed\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn case_terminator_suffix_scan_handles_utf8_prefixes() {
        let source = "# 不支持\ncase $x in\n*) echo ok ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_case_suffix_comments_before_commented_compound_body() {
        let source = "case $x in\n*) # default branch\n# explain\nif test -n \"$x\"; then\n  echo \"$x\"\nfi\n;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case $x in\n*) # default branch\n\t# explain\n\tif test -n \"$x\"; then\n\t\techo \"$x\"\n\tfi\n\t;;\nesac\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_comment_after_case_in_keyword() {
        let source = "case \"$( cut -d';' -f5 \"$FILE\" | md5sum )\" in # hash over costs\n\"$forced_hash\"*)\n  _log ok\n;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case \"$(cut -d';' -f5 \"$FILE\" | md5sum)\" in # hash over costs\n\"$forced_hash\"*)\n\t_log ok\n\t;;\nesac\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn does_not_duplicate_leading_comments_inside_brace_groups() {
        let source = "f() {\n  # before group\n  {\n    # inside group\n    echo ok\n  }\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "f() {\n\t# before group\n\t{\n\t\t# inside group\n\t\techo ok\n\t}\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn preserves_multiline_subshell_open_and_close_placement() {
        let source = "if ok; then\n  (mkdir -p -- \"$cachedir\" &&\n    echo \"$cache_id_line\"$'\\n'\"$output\" >\"$cachefile\") 2>/dev/null\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if ok; then\n\t(mkdir -p -- \"$cachedir\" &&\n\t\techo \"$cache_id_line\"$'\\n'\"$output\" >\"$cachefile\") 2>/dev/null\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_subshell_around_loop() {
        let source = "f() {\n  (while sudo -v; do\n    sleep 50\n  done) &\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "f() {\n\t(while sudo -v; do\n\t\tsleep 50\n\tdone) &\n}\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_single_line_subshell_with_background_body() {
        let source = "if ready; then\n  ($REGEN_CMD &)\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("if ready; then\n\t($REGEN_CMD &)\nfi\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn preserves_concatenated_ansi_c_quoted_assignment_values() {
        let source = "local excluded=$'\\ndefault\\n'${prefix//:/foo}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_concatenated_ansi_c_quoted_arguments() {
        let source = "echo \"$cache_id_line\"$'\\n'\"$output\" >\"$cachefile\"\n";
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
    fn preserves_leading_comments_inside_function_bodies() {
        let source = "function f() {\n  # parse all defined shortcuts ${BASH_IT_DIRS_BKS}\n  if [[ -s x ]]; then\n    echo yes\n  fi\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "function f() {\n\t# parse all defined shortcuts ${BASH_IT_DIRS_BKS}\n\tif [[ -s x ]]; then\n\t\techo yes\n\tfi\n}\n"
                    .to_string()
            )
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
    fn formats_redirect_spacing_inside_process_substitution() {
        let source = "read -ra candidates < <(complete words 2> /dev/null)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "read -ra candidates < <(complete words 2>/dev/null)\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_process_substitution_attached_after_equals() {
        let source = "setfacl --restore=<(grep -E -v '^# (owner|group):' \"$tmp_file\")\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_inline_multiline_process_substitution_continuations() {
        let source = "while read -r line; do\n\techo \"$line\"\ndone < <(comm -23 <(printf \"%s\\n\" \"${left[@]}\" | sort) \\\n\t<(printf \"%s\\n\" \"${right[@]}\" | sort))\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_block_process_substitution_source_indentation() {
        let source = "while read -r line; do\n\techo \"$line\"\ndone < <(\n\tprintf \"%s\\n\" \"${items[@]}\"\n)\n";
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
    fn preserves_fd_close_redirect_targets() {
        let source = "cmd 2>&-\nexec <&-\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_explicit_stdout_fd_on_dup_redirects() {
        let source = "cat 1>&2 <<EOF\nhi\nEOF\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_simple_command_redirect_positions() {
        let source = "echo >&2 \"bad news\"\n";
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
    fn preserves_explicit_line_break_when_list_operator_starts_continued_line() {
        let source =
            "_command_exists goenv \\\n  || [[ -x \"$GOENV_ROOT/bin/goenv\" ]] \\\n  || return 0\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "_command_exists goenv ||\n\t[[ -x \"$GOENV_ROOT/bin/goenv\" ]] ||\n\treturn 0\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_explicit_multiline_pipeline_by_default() {
        let source = "kubectl get secrets |\n  grep -v '^NAME[[:space:]]' |\n  awk '{print $1}'\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "kubectl get secrets |\n\tgrep -v '^NAME[[:space:]]' |\n\tawk '{print $1}'\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_explicit_multiline_pipeline_when_operator_starts_continued_line() {
        let source = "find $PKG -print0 | xargs -0 file | grep ELF \\\n  | cut -f 1 -d : | xargs strip --strip-unneeded 2> /dev/null || true\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "find $PKG -print0 | xargs -0 file | grep ELF |\n\tcut -f 1 -d : | xargs strip --strip-unneeded 2>/dev/null || true\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_continued_redirect_targets() {
        let source = "sed s/x/y/ in > \\\n  out\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("sed s/x/y/ in > \\\n\tout\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_for_in_first_word_continuation() {
        let source = "for net_mount in \\\n  ${HOST_MOUNTS_RO} ${HOST_MOUNTS} \\\n  '/dev' '/proc'; do\n  echo \"$net_mount\"\ndone\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "for net_mount in \\\n\t${HOST_MOUNTS_RO} ${HOST_MOUNTS} \\\n\t'/dev' '/proc'; do\n\techo \"$net_mount\"\ndone\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_for_targets_inside_inline_command_substitutions() {
        let source = "pass=\"$(for i in $(eval \"echo {1..$length}\"); do pickfrom /usr/share/dict/words; done)\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_comment_indentation_inside_inline_command_substitutions() {
        let source = "if ok; then\n\tfor item in $(printenv |\n\t\t# keep env names\n\t\tgrep '^APP_'); do\n\t\techo \"$item\"\n\tdone\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn indents_block_command_substitution_comments_inside_assignments() {
        let source = "if ok; then\n\titems=$(\n\t\t# keep generated names\n\t\tfind . -type f |\n\t\t\tsort\n\t)\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_command_substitution_assignment_continuation_alignment() {
        let source = "LIBS=\"$(pkg-config --libs openssl)\" \\\nCFLAGS=\"$SLKCFLAGS -Wl,-s -I$(pwd)/lib\" \\\n./configure \\\n--prefix=/usr\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "LIBS=\"$(pkg-config --libs openssl)\" \\\nCFLAGS=\"$SLKCFLAGS -Wl,-s -I$(pwd)/lib\" \\\n\t./configure \\\n\t--prefix=/usr\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_decl_compound_assignment_lines() {
        let source = "case $prev in\n--warnings)\n  local cats=(cross gnu obsolete override portability syntax\n    unsupported)\n  return\n  ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case $prev in\n--warnings)\n\tlocal cats=(cross gnu obsolete override portability syntax\n\t\tunsupported)\n\treturn\n\t;;\nesac\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_expanded_decl_compound_assignment_delimiters() {
        let source = "f() {\n  local commands=(\n    build\n    version\n  )\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "f() {\n\tlocal commands=(\n\t\tbuild\n\t\tversion\n\t)\n}\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn aligns_runs_of_trailing_comments() {
        let source = "short=1 # first\nmuch_longer=2 # second\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "short=1       # first\nmuch_longer=2 # second\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn aligns_trailing_comments_after_normalized_arithmetic_assignments() {
        let source = "border=$(( $(_system uptime days) * 3 )) # normally\nborder=$(( border + basecount ))         # later\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "border=$(($(_system uptime days) * 3)) # normally\nborder=$((border + basecount))         # later\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn aligns_trailing_comments_after_normalized_command_substitutions() {
        let source = "SPACER1=\"$(_sanitizer run \"$MAX1 $LOCAL\"  add_length_diff_with_spaces)\" # one\nSPACER2=\"$(_sanitizer run \"$MAX2 $REMOTE\" add_length_diff_with_spaces)\" # two\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "SPACER1=\"$(_sanitizer run \"$MAX1 $LOCAL\" add_length_diff_with_spaces)\"  # one\nSPACER2=\"$(_sanitizer run \"$MAX2 $REMOTE\" add_length_diff_with_spaces)\" # two\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn aligns_trailing_comments_after_parameter_replacements() {
        let source = "while read line; do\n  line=${line%%#*}   # Remove comments\n  line=${line//:/ }  # Change colon delimiter to space\n  line=${line//,/ }  # Change comma delimiter to space\ndone\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "while read line; do\n\tline=${line%%#*}  # Remove comments\n\tline=${line//:/ } # Change colon delimiter to space\n\tline=${line//,/ } # Change comma delimiter to space\ndone\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn aligns_trailing_comments_after_parameter_replacements_inside_functions() {
        let source = "read_conf() {\n  while read line; do\n    line=${line%%#*}   # Remove comments\n    line=${line//:/ }  # Change colon delimiter to space\n    line=${line//,/ }  # Change comma delimiter to space\n  done\n}\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, Some(Path::new("ldconfig.in-r3")), &options).unwrap(),
            FormattedSource::Formatted(
                "read_conf() {\n\twhile read line; do\n\t\tline=${line%%#*}  # Remove comments\n\t\tline=${line//:/ } # Change colon delimiter to space\n\t\tline=${line//,/ } # Change comma delimiter to space\n\tdone\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, Some(Path::new("ldconfig.in-r3")), &options);
    }

    #[test]
    fn aligns_trailing_comments_after_adjacent_redirect_commands() {
        let source = "if ok; then\n  rm -f /tmp/OLSR/meshrdf_neighs* 2>/dev/null    # enforce rewrite some lines later\n  echo >>$SCHEDULER \"_wifi speed check $gateway\" # will only test once\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if ok; then\n\trm -f /tmp/OLSR/meshrdf_neighs* 2>/dev/null    # enforce rewrite some lines later\n\techo >>$SCHEDULER \"_wifi speed check $gateway\" # will only test once\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_if_condition_on_own_line() {
        let source = "case $mode in\nprompt)\n  if\n    [[ -n ${ZSH_VERSION:-} ]]\n  then\n    echo zsh\n  fi\n  ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case $mode in\nprompt)\n\tif\n\t\t[[ -n ${ZSH_VERSION:-} ]]\n\tthen\n\t\techo zsh\n\tfi\n\t;;\nesac\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn aligns_trailing_comments_across_tab_indented_if_body() {
        let source = "check_restart() {\n\tif [ $percent -gt 300 -a $OPENWRT_REV -gt 0 ]; then\t# seems busy\n\t\treturn 1\t\t# sometimes high\n\tfi\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "check_restart() {\n\tif [ $percent -gt 300 -a $OPENWRT_REV -gt 0 ]; then # seems busy\n\t\treturn 1                                           # sometimes high\n\tfi\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_after_if_then() {
        let source = "if true; then\n\n  echo yes\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("if true; then\n\n\techo yes\nfi\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_if_fi() {
        let source =
            "if true; then\n  if other; then\n    echo yes\n  else\n    echo no\n  fi\n\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\tif other; then\n\t\techo yes\n\telse\n\t\techo no\n\tfi\n\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_simple_fi() {
        let source = "if true; then\n  echo yes\n\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("if true; then\n\techo yes\n\nfi\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_if_branches() {
        let source =
            "if true; then\n  echo yes\n\nelif false; then\n  echo no\n\nelse\n  echo maybe\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\techo yes\n\nelif false; then\n\techo no\n\nelse\n\techo maybe\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_commented_if_branch() {
        let source =
            "if true; then\n  echo yes\n\n# try the fallback\nelif false; then\n  echo no\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\techo yes\n\n# try the fallback\nelif false; then\n\techo no\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_fi_after_elif_branch() {
        let source = "if true; then\n  echo yes\nelif false; then\n  echo no\n\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\techo yes\nelif false; then\n\techo no\n\nfi\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn does_not_preserve_branch_blanks_from_inline_keywords() {
        let source = "# setup\n\nif true; then yes; else\n  no\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "# setup\n\nif true; then yes; else\n\tno\nfi\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_after_while_do() {
        let source = "while read -r dep; do\n\n  ver=${dep#*=}\ndone\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "while read -r dep; do\n\n\tver=${dep#*=}\ndone\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_done() {
        let source = "while true; do\n  echo yes\n\ndone\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("while true; do\n\techo yes\n\ndone\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_after_brace_group_open() {
        let source = "if true; then\n  [ -n \"$x\" ] && {\n\n    echo yes\n  }\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\t[ -n \"$x\" ] && {\n\n\t\techo yes\n\t}\nfi\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_after_brace_group_open_suffix_comment() {
        let source = "if true; then\n  [ -n \"$x\" ] || { # note\n\n    echo yes\n  }\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\t[ -n \"$x\" ] || { # note\n\n\t\techo yes\n\t}\nfi\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_inline_do_brace_close() {
        let source = "while read -r line; do {\n  echo \"$line\"\n\n} done <file\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "while read -r line; do {\n\techo \"$line\"\n\n}; done <file\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_blank_line_before_inline_do_brace_close_after_nested_group() {
        let source = "while read -r line; do {\n  [ -n \"$line\" ] && {\n    echo \"$line\"\n  }\n\n} done <file\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "while read -r line; do {\n\t[ -n \"$line\" ] && {\n\t\techo \"$line\"\n\t}\n\n}; done <file\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn formats_multiline_arithmetic_commands_with_continuations() {
        let source = "if true; then\n  ((\n  I++,\n  IDX = 16\n  + R * 5\n  + G * 6\n  ))\nelse\n  echo no\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\t((\\\n\tI++, \\\n\tIDX = 16 + \\\n\tR * 5 + \\\n\tG * 6))\n\nelse\n\techo no\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_continued_semicolon_terminators() {
        let source = "ln -s foo bar \\\n  ;\nrm bar\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("ln -s foo bar \\\n\t;\nrm bar\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_single_quoted_argument_payload_indentation() {
        let source = "cat \"$@\" |\n  python -c '\nfrom __future__ import print_function\nprint(\"ok\")\n'\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "cat \"$@\" |\n\tpython -c '\nfrom __future__ import print_function\nprint(\"ok\")\n'\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_assignment_payload_indentation() {
        let source = "if true; then\n  section+=\"\n$line\"\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted("if true; then\n\tsection+=\"\n$line\"\nfi\n".to_string())
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_assignment_continuation_payload_indentation() {
        let source = "if true; then\n  INCLUDE_TESTS=\"boot_services kernel \\\n                           filesystems usb \\\n                           hardening\"\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if true; then\n\tINCLUDE_TESTS=\"boot_services kernel \\\n                           filesystems usb \\\n                           hardening\"\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_quoted_assignment_payloads_with_nested_expansions() {
        let source = "result_command=\"${result_command}\n\t--label \\\"manager=distrobox\\\"\n\t--env \\\"SHELL=$(basename \"${SHELL:-\"/bin/bash\"}\")\\\"\n\t--env \\\"HOME=${container_user_home}\\\"\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_inline_multiline_command_substitution_source_indentation() {
        let source = "result_command=\"${result_command}\n\t\t$(printenv | grep '=' |\n\t\tgrep -Ev '^_' |\n\t\tsed 's/x/y/')\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn normalizes_multiline_command_substitution_arguments() {
        let source = "_comp_compgen_split -- \"$(\"$1\" -watchdog help 2>&1 |\n                _comp_awk '{print $1}')\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "_comp_compgen_split -- \"$(\"$1\" -watchdog help 2>&1 |\n\t_comp_awk '{print $1}')\"\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn normalizes_redirect_spacing_in_inline_multiline_command_substitutions() {
        let source = "binary_files=\"$(grep -rl \"# distrobox_binary\" \"${HOME}/.local/bin\" 2> /dev/null | sed 's/./\\\\&/g' |\n\txargs -I{} grep -le \"# name: ${container_name}$\" \"{}\" | sed 's/./\\\\&/g' |\n\txargs -I{} printf \"%s¤\" \"{}\" 2> /dev/null || :)\"\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "binary_files=\"$(grep -rl \"# distrobox_binary\" \"${HOME}/.local/bin\" 2>/dev/null | sed 's/./\\\\&/g' |\n\txargs -I{} grep -le \"# name: ${container_name}$\" \"{}\" | sed 's/./\\\\&/g' |\n\txargs -I{} printf \"%s¤\" \"{}\" 2>/dev/null || :)\"\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn trims_source_indent_from_block_command_substitutions() {
        let source = "f() {\n\tdesktop_files=$(\n\t\t# keep this with the nested command\n\t\tfind \"$dir\" -type f 2> /dev/null | sed 's/./\\\\&/g' |\n\t\t\txargs printf '%s\\n'\n\t)\n}\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "f() {\n\tdesktop_files=$(\n\t\t# keep this with the nested command\n\t\tfind \"$dir\" -type f 2>/dev/null | sed 's/./\\\\&/g' |\n\t\t\txargs printf '%s\\n'\n\t)\n}\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn preserves_multiline_compound_assignment_literal_shape() {
        let source = "case $mode in\ndocs)\n  CMD=(zsh -ilsc\n    'sudo chown /src &&\n     make -C /src doc')\n  ;;\nesac\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case $mode in\ndocs)\n\tCMD=(zsh -ilsc\n\t\t'sudo chown /src &&\n     make -C /src doc')\n\t;;\nesac\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn tab_stripping_heredocs_indent_body_with_context() {
        let source = "case $mode in\nnew)\n  cat >$file <<-EOF\nbody\nEOF\n  ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case $mode in\nnew)\n\tcat >$file <<-EOF\n\t\tbody\n\tEOF\n\t;;\nesac\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
    fn keeps_inline_case_inside_if_command_lists() {
        let source = "if case \"${icon_name}\" in \"/\"*) true ;; *) false ;; esac &&\n  [ -e \"${icon_name}\" ]; then\n  echo yes\nfi\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "if case \"${icon_name}\" in \"/\"*) true ;; *) false ;; esac &&\n\t[ -e \"${icon_name}\" ]; then\n\techo yes\nfi\n"
                    .to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
    }

    #[test]
    fn keeps_case_items_multiline_when_terminator_was_multiline() {
        let source = "case \"$x\" in\n-h|--help)  usage\n            ;;\nesac\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Formatted(
                "case \"$x\" in\n-h | --help)\n\tusage\n\t;;\nesac\n".to_string()
            )
        );
        assert_source_and_ast_paths_match(source, None, &options);
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
