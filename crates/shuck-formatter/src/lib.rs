#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]
//! Shell formatting entrypoints built on top of `shuck-parser` and `shuck-format`.
//!
//! Most callers will use [`format_source`] for source text or [`format_file_ast`] when they
//! already have a parsed shell AST.
#![recursion_limit = "256"]

//! Shell script formatter with configurable style options.

#[allow(missing_docs)]
mod command;
#[allow(missing_docs)]
mod comments;
#[allow(missing_docs)]
mod context;
#[allow(missing_docs)]
mod facts;
#[allow(missing_docs)]
mod options;
#[allow(missing_docs)]
mod scan;
#[allow(missing_docs)]
mod simplify;
#[allow(missing_docs)]
mod streaming;
#[allow(missing_docs)]
mod word;

use std::path::Path;
use std::process::{Command, Stdio};

use shuck_ast::File;
use shuck_format::LineEnding;
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
    while let Some(start) = trailing_line_ending_start(output)
        .filter(|start| trailing_line_ending_start(&output[..*start]).is_some())
    {
        output.truncate(start);
    }
    if trailing_line_ending_start(output).is_none() {
        if trailing_backslash_count(output) % 2 == 1 && !trailing_backslash_is_in_comment(output) {
            output.push('\\');
        }
        output.push_str(line_ending_str(line_ending));
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

fn trailing_backslash_is_in_comment(text: &str) -> bool {
    let line = text.rsplit_once('\n').map_or(text, |(_, line)| line);
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;
    let mut previous = None;

    for ch in line.chars() {
        if escaped {
            escaped = false;
            previous = Some(ch);
            continue;
        }
        match ch {
            '\\' if !in_single_quotes => {
                escaped = true;
            }
            '\'' if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
            }
            '"' if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
            }
            '#' if !in_single_quotes
                && !in_double_quotes
                && previous.is_none_or(char::is_whitespace) =>
            {
                return true;
            }
            _ => {}
        }
        previous = Some(ch);
    }

    false
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

    #[track_caller]
    fn assert_default_idempotent(source: &str, filename: &str) {
        assert_idempotent(
            source,
            Some(Path::new(filename)),
            &ShellFormatOptions::default(),
        );
    }

    #[track_caller]
    fn assert_formats_to(
        source: &str,
        path: Option<&Path>,
        options: &ShellFormatOptions,
        expected: impl Into<String>,
    ) {
        assert_eq!(
            format_source(source, path, options).unwrap(),
            FormattedSource::Formatted(expected.into())
        );
    }

    #[track_caller]
    fn assert_formats(source: &str, expected: impl Into<String>) {
        assert_formats_to(source, None, &ShellFormatOptions::default(), expected);
    }

    #[track_caller]
    fn assert_formats_to_with_ast(
        source: &str,
        path: Option<&Path>,
        options: &ShellFormatOptions,
        expected: impl Into<String>,
    ) {
        assert_formats_to(source, path, options, expected);
        assert_source_and_ast_paths_match(source, path, options);
    }

    #[track_caller]
    fn assert_unchanged(source: &str, path: Option<&Path>, options: &ShellFormatOptions) {
        assert_eq!(
            format_source(source, path, options).unwrap(),
            FormattedSource::Unchanged
        );
    }

    #[track_caller]
    fn assert_unchanged_default(source: &str) {
        assert_unchanged(source, None, &ShellFormatOptions::default());
    }

    #[track_caller]
    fn assert_unchanged_with_ast(source: &str, path: Option<&Path>, options: &ShellFormatOptions) {
        assert_unchanged(source, path, options);
        assert_source_and_ast_paths_match(source, path, options);
    }

    macro_rules! formatter_cases {
        ($($name:ident $body:block)+) => {
            $(
                #[test]
                fn $name() $body
            )+
        };
    }

    macro_rules! default_format_ast_cases {
        ($($name:ident: $source:expr => $expected:expr;)+) => {
            formatter_cases! { $($name { assert_formats_to_with_ast($source, None, &ShellFormatOptions::default(), $expected); })+ }
        };
    }

    macro_rules! bash_format_ast_cases {
        ($($name:ident: $source:expr => $expected:expr;)+) => {
            formatter_cases! { $($name {
                assert_formats_to_with_ast($source, None, &ShellFormatOptions::default().with_dialect(ShellDialect::Bash), $expected);
            })+ }
        };
    }

    macro_rules! default_format_cases {
        ($($name:ident: $source:expr => $expected:expr;)+) => {
            formatter_cases! { $($name { assert_formats($source, $expected); })+ }
        };
    }

    macro_rules! default_unchanged_ast_cases {
        ($($name:ident: $source:expr;)+) => {
            formatter_cases! { $($name { assert_unchanged_with_ast($source, None, &ShellFormatOptions::default()); })+ }
        };
    }

    macro_rules! bash_unchanged_ast_cases {
        ($($name:ident: $source:expr;)+) => {
            formatter_cases! { $($name {
                assert_unchanged_with_ast($source, None, &ShellFormatOptions::default().with_dialect(ShellDialect::Bash));
            })+ }
        };
    }

    macro_rules! default_unchanged_cases {
        ($($name:ident: $source:expr;)+) => {
            formatter_cases! { $($name { assert_unchanged_default($source); })+ }
        };
    }

    macro_rules! default_idempotent_cases {
        ($($name:ident: $source:expr, $filename:expr;)+) => {
            formatter_cases! { $($name { assert_default_idempotent($source, $filename); })+ }
        };
    }

    macro_rules! format_cases_with_options {
        ($($name:ident: $options:expr, $source:expr $(,)? => $expected:expr;)+) => {
            formatter_cases! {
                $($name {
                    let options = $options;
                    assert_formats_to($source, None, &options, $expected);
                })+
            }
        };
    }

    macro_rules! unchanged_cases_with_options {
        ($($name:ident: $options:expr, $source:expr $(,)?;)+) => {
            formatter_cases! {
                $($name {
                    let options = $options;
                    assert_unchanged($source, None, &options);
                })+
            }
        };
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

    fn stable_formatter_fixture_cases() -> Vec<(&'static str, &'static str, ShellFormatOptions)> {
        let same_name = |fixture, options| (fixture, fixture, options);
        let with_name = |fixture, filename, options| (fixture, filename, options);

        vec![
            same_name(
                "function_next_line.sh",
                ShellFormatOptions::default().with_function_next_line(true),
            ),
            same_name("case_default.sh", ShellFormatOptions::default()),
            same_name(
                "space_redirects.sh",
                ShellFormatOptions::default().with_space_redirects(true),
            ),
            same_name(
                "keep_padding.sh",
                ShellFormatOptions::default().with_keep_padding(true),
            ),
            same_name(
                "never_split.sh",
                ShellFormatOptions::default().with_never_split(true),
            ),
            same_name("nested_heredoc.sh", ShellFormatOptions::default()),
            with_name(
                "simplify.sh",
                "simplify.bash",
                ShellFormatOptions::default().with_simplify(true),
            ),
            same_name("minify.sh", ShellFormatOptions::default().with_minify(true)),
            with_name(
                "mksh_select.sh",
                "script.mksh",
                ShellFormatOptions::default().with_dialect(ShellDialect::Mksh),
            ),
        ]
    }

    const TONKA_PROMPT_ASSIGNMENT: &str = r##"PS1="$TITLEBAR$YELLOW-$LIGHT_BLUE-(\
$YELLOW\u$LIGHT_BLUE@$YELLOW\h\
$LIGHT_BLUE)-(\
$YELLOW\$PWD\
$LIGHT_BLUE)-$YELLOW-\
\n\
$YELLOW-$LIGHT_BLUE-(\
$(__tonka_clock)\
$WHITE\$ $LIGHT_BLUE)-$YELLOW-$NO_COLOUR "
"##;

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
        let source = "#!/bin/bash\n echo   hi\n";

        assert_formats(source, "#!/bin/bash\necho hi\n");
        assert!(!source_is_formatted(source, None, &ShellFormatOptions::default()).unwrap());
    }

    default_format_cases! {
        preserves_inline_comments:
            "echo hi    # note\n"
            => "echo hi # note\n";
        keeps_if_close_suffix_comment_on_outer_close:
            "if outer; then\n  if inner; then\n    :\n  fi\nfi # outer\n"
            => "if outer; then\n\tif inner; then\n\t\t:\n\tfi\nfi # outer\n";
        keeps_inline_if_close_suffix_comment_on_fi:
            "if ok; then good; fi    # done\n"
            => "if ok; then good; fi # done\n";
        keeps_loop_and_case_close_suffix_comments_on_close_keywords:
            "while ok; do\n  case $cmd in\n    run) : ;;\n  esac # command\n  :\ndone # loop\n"
            => "while ok; do\n\tcase $cmd in\n\trun) : ;;\n\tesac # command\n\t:\ndone # loop\n";
        keeps_inline_case_close_suffix_comment_on_esac:
            "case \"$IP\" in fe80::*) exit 0 ;; esac\t# ignore IPv6 linklocal, ip2dev() does not work here reliable anyway\n"
            => "case \"$IP\" in fe80::*) exit 0 ;; esac # ignore IPv6 linklocal, ip2dev() does not work here reliable anyway\n";
        aligns_nested_close_suffix_comments_by_column:
            "if outer; then\n\tif inner; then\n\t\tcase $cmd in\n\t\t*) : ;;\n\t\tesac # case\n\tfi # inner\nfi # outer\n"
            => "if outer; then\n\tif inner; then\n\t\tcase $cmd in\n\t\t*) : ;;\n\t\tesac # case\n\tfi    # inner\nfi     # outer\n";
        aligns_space_indented_close_suffix_comments_by_column:
            "if outer; then\n  if inner; then\n    :\n  fi # inner\nfi # outer\n"
            => "if outer; then\n\tif inner; then\n\t\t:\n\tfi # inner\nfi  # outer\n";
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

    default_format_cases! {
        formats_heredoc_command_heads_structurally:
            "cat<<EOF\nhello\nEOF\n"
            => "cat <<EOF\nhello\nEOF\n";
        formats_nested_heredoc_commands_without_indenting_body:
            "if true; then\ncat<<EOF\nhello\nEOF\nfi\n"
            => "if true; then\n\tcat <<EOF\nhello\nEOF\nfi\n";
    }

    default_format_ast_cases! {
        preserves_tab_stripped_heredoc_body_source:
            "if true; then\n  cat >run <<-EOF\n\t\t#!/bin/sh\n\n\t\texec 2>&1\n\tEOF\nfi\n"
            => "if true; then\n\tcat >run <<-EOF\n\t\t#!/bin/sh\n\n\t\texec 2>&1\n\tEOF\nfi\n";
        indents_tab_stripped_heredoc_body_to_context_depth:
            "if true; then\n\tcat >&2 <<-EOF\n\t* package moved\n\tEOF\nfi\n"
            => "if true; then\n\tcat >&2 <<-EOF\n\t\t* package moved\n\tEOF\nfi\n";
        tab_stripped_heredoc_closer_follows_context_indent:
            "if true; then\n  if ok; then\n\tcat <<-EOF\n\tbody\n\tEOF\n  fi\nfi\n"
            => "if true; then\n\tif ok; then\n\t\tcat <<-EOF\n\t\t\tbody\n\t\tEOF\n\tfi\nfi\n";
        preserves_relative_tabs_inside_tab_stripped_heredoc_body:
            "build() {\n\tcat <<-EOF >./prerm\n\t#!$PREFIX/bin/bash\n\tif [ -d $PREFIX/etc ]; then\n\t\techo ok\n\t\trm -f file\n\tfi\n\texit 0\n\tEOF\n}\n"
            => "build() {\n\tcat <<-EOF >./prerm\n\t\t#!$PREFIX/bin/bash\n\t\tif [ -d $PREFIX/etc ]; then\n\t\t\techo ok\n\t\t\trm -f file\n\t\tfi\n\t\texit 0\n\tEOF\n}\n";
    }

    default_format_cases! {
        preserves_multiline_if_body_comments:
            "if true; then\n# note\necho hi\nfi\n"
            => "if true; then\n\t# note\n\techo hi\nfi\n";
    }

    default_unchanged_ast_cases! {
        keeps_simple_if_else_inline:
            "if [ -n \"$REPORTFILE\" ]; then PREQS_MET=\"YES\"; else PREQS_MET=\"NO\"; fi\n";
    }

    default_format_ast_cases! {
        keeps_inline_then_arm_before_multiline_else:
            "if [ -n \"$REPORTFILE\" ]; then PREQS_MET=\"YES\"; else\n  PREQS_MET=\"NO\"\nfi\n"
            => "if [ -n \"$REPORTFILE\" ]; then PREQS_MET=\"YES\"; else\n\tPREQS_MET=\"NO\"\nfi\n";
    }

    default_format_cases! {
        preserves_comments_inside_elif_bodies:
            "foo() {\nif a; then\none\nelif b; then\n# note\n two\nfi\n}\n"
            => "foo() {\n\tif a; then\n\t\tone\n\telif b; then\n\t\t# note\n\t\ttwo\n\tfi\n}\n";
    }

    bash_unchanged_ast_cases! {
        multiline_if_conditions_do_not_capture_later_body_comments:
            "f() {\n\tif\n\t\t[[ -n \"${GEM_HOME:-}\" ]]\n\tthen\n\t\tcase \"$PATH:\" in\n\t\t$GEM_HOME/bin:*) true ;; # all fine\n\t\t*)\n\t\t\t# body note\n\t\t\twarn\n\t\t\t;;\n\t\tesac\n\tfi\n}\n\n# marker\ng() { :; }\n";
    }

    bash_format_ast_cases! {
        keeps_inline_else_arm_after_multiline_then:
            "if [ $size != scalable ]; then\n  ex=png\n  size=${size}x${size}\nelse ex=svg; fi\n"
            => "if [ $size != scalable ]; then\n\tex=png\n\tsize=${size}x${size}\nelse ex=svg; fi\n";
        aligns_else_suffix_comments_with_nested_multiline_header_suffix_comments:
            "if foo; then\n  :\nelse # branch\n  if [[ \"$x\" =~ y ]]\n  then # nested\n    :\n  fi\nfi\n"
            => "if foo; then\n\t:\nelse                      # branch\n\tif [[ \"$x\" =~ y ]]; then # nested\n\t\t:\n\tfi\nfi\n";
    }

    default_format_ast_cases! {
        aligns_comments_before_elif_and_else_with_branch_keywords:
            "if a; then\none\n# next branch\n# still next branch\nelif b; then\ntwo\n# final branch\nelse\nthree\nfi\n"
            => "if a; then\n\tone\n# next branch\n# still next branch\nelif b; then\n\ttwo\n# final branch\nelse\n\tthree\nfi\n";
        ignores_commented_branch_keywords_when_finding_else:
            "if a; then\n  one\nelse\n# disabled pre\n#if b; then\n#else\n  two\nfi\n"
            => "if a; then\n\tone\nelse\n\t# disabled pre\n\t#if b; then\n\t#else\n\ttwo\nfi\n";
        keeps_body_indented_comments_before_elif_inside_previous_branch:
            "if a; then\none\n  # still body context\nelif b; then\ntwo\nfi\n"
            => "if a; then\n\tone\n\t# still body context\nelif b; then\n\ttwo\nfi\n";
        keeps_disabled_elif_comment_block_before_real_elif:
            "if a; then\none\n\n#elif disabled; then\n    #cmd one\n    # note\n    #cmd two\n\nelif b; then\ntwo\nfi\n"
            => "if a; then\n\tone\n\n\t#elif disabled; then\n\t#cmd one\n\t# note\n\t#cmd two\n\nelif b; then\n\ttwo\nfi\n";
        keeps_explanatory_if_comment_before_elif_at_branch_indent:
            "if [ -d \"$source_dir\" ]; then\n  if ! mkdir -p \"$target_dir\"; then\n    return 1\n  fi\n# if instead it is a file\nelif [ -f \"$source_dir\" ]; then\n  touch \"$target_dir\"\nfi\n"
            => "if [ -d \"$source_dir\" ]; then\n\tif ! mkdir -p \"$target_dir\"; then\n\t\treturn 1\n\tfi\n# if instead it is a file\nelif [ -f \"$source_dir\" ]; then\n\ttouch \"$target_dir\"\nfi\n";
    }

    bash_format_ast_cases! {
        preserves_comments_between_elif_and_condition:
            "if a; then\none\nelif\n# explain\n [[ b ]]; then\ntwo\nfi\n"
            => "if a; then\n\tone\nelif\n\t# explain\n\t[[ b ]]\nthen\n\ttwo\nfi\n";
    }

    default_format_cases! {
        preserves_comments_after_if_blocks:
            "if true; then\necho hi\nfi\n# after\necho bye\n"
            => "if true; then\n\techo hi\nfi\n# after\necho bye\n";
    }

    default_format_ast_cases! {
        preserves_dangling_comment_inside_binary_brace_group_once:
            "if true; then\n  ls today && {\n    log done\n#\t\tcontinue\n  }\n\n  rm next\nfi\n"
            => "if true; then\n\tls today && {\n\t\tlog done\n\t\t#\t\tcontinue\n\t}\n\n\trm next\nfi\n";
    }

    bash_format_ast_cases! {
        binary_brace_group_does_not_gain_blank_before_next_command:
            "main() {\n  [[ ! -f $ok ]] && {\n    err missing\n  }\n  next\n}\n"
            => "main() {\n\t[[ ! -f $ok ]] && {\n\t\terr missing\n\t}\n\tnext\n}\n";
        preserves_leading_comments_inside_redirected_brace_group:
            "if [[ -n $DEBUG ]]; then\n  {\n    # one\n    # two\n    echo hi\n  } >&2\nfi\n"
            => "if [[ -n $DEBUG ]]; then\n\t{\n\t\t# one\n\t\t# two\n\t\techo hi\n\t} >&2\nfi\n";
    }

    default_format_cases! {
        preserves_comments_after_function_blocks:
            "foo() {\necho hi\n}\n# after\nbar\n"
            => "foo() {\n\techo hi\n}\n# after\nbar\n";
    }

    default_format_ast_cases! {
        normalizes_opening_brace_comment_spacing:
            "[ $ok ] && {\t\t# ready\n  echo hi\n}\n"
            => "[ $ok ] && { # ready\n\techo hi\n}\n";
        preserves_trailing_function_header_comment_when_brace_moves_up:
            "foo() # header comment\n{\n  echo hi\n}\n"
            => "foo() { # header comment\n\techo hi\n}\n";
        preserves_function_header_comment_spacing_when_brace_moves_up:
            "foo()\t\t# header comment\n{\n  echo hi\n}\n"
            => "foo() { # header comment\n\techo hi\n}\n";
        aligns_moved_function_header_comments_with_first_body_comment:
            "_olsr_uptime()\t\t\t# in seconds\n{\n  local option=\"$1\"\t# string option\n  local funcname='olsr_uptime'\n}\n"
            => "_olsr_uptime() {   # in seconds\n\tlocal option=\"$1\" # string option\n\tlocal funcname='olsr_uptime'\n}\n";
        body_comment_stops_moved_function_header_comment_alignment:
            "foo()\t\t# header\n{\n#\tlocal mac=\"$1\"\n\tlocal minute=\"${MINUTE:-$( date +%H )}\"\t\t# built during taskplanner: 00...23\n\tlocal hour=\"${HOUR:-$( date +%M )}\"\t\t# built during taskplanner: 00...59\n}\n"
            => "foo() { # header\n\t#\tlocal mac=\"$1\"\n\tlocal minute=\"${MINUTE:-$(date +%H)}\" # built during taskplanner: 00...23\n\tlocal hour=\"${HOUR:-$(date +%M)}\"     # built during taskplanner: 00...59\n}\n";
        aligns_old_style_function_header_comments_like_shfmt:
            "foo () # header\n{\n  a=1 # body\n}\n"
            => "foo() { # header\n\ta=1    # body\n}\n";
        preserves_header_and_opening_brace_comments_when_brace_moves_up:
            "foo() # header comment\n{ # body comment\n  echo hi\n}\n"
            => "foo() { # header comment\n\t# body comment\n\techo hi\n}\n";
        preserves_inline_function_keyword_opening_brace_comments:
            "function is_integer() { # helper function for todo-txt-count\n  [ \"$1\" -eq \"$1\" ] > /dev/null 2>&1\n  return $?\n}\n"
            => "function is_integer() { # helper function for todo-txt-count\n\t[ \"$1\" -eq \"$1\" ] >/dev/null 2>&1\n\treturn $?\n}\n";
        opening_brace_comment_stops_following_body_comment_alignment:
            "foo() # header\n{ # body comment\n  local FILE='/tmp/OLSR/LINKS.sh' # see build_tables()\n  local json=\"$TMPDIR/links.json\" # FIXME! add _speedtest_stats()\n}\n"
            => "foo() { # header\n\t# body comment\n\tlocal FILE='/tmp/OLSR/LINKS.sh' # see build_tables()\n\tlocal json=\"$TMPDIR/links.json\" # FIXME! add _speedtest_stats()\n}\n";
        preserves_blank_line_before_trailing_file_comment:
            "foo() {\necho hi\n}\n\n# ex: filetype=sh\n"
            => "foo() {\n\techo hi\n}\n\n# ex: filetype=sh\n";
    }

    default_unchanged_cases! {
        preserves_heredoc_trailing_comments_without_duplication:
            "cat <<EOF # note\nhi\nEOF\n";
    }

    bash_format_ast_cases! {
        formats_heredoc_pipeline_with_trailing_comment_structurally:
            "f(){\n    cat <<EOF |\nbody\n# heredoc comment\nEOF\n    python #|\n    #sed x\n}\n"
            => "f() {\n\tcat <<EOF |\nbody\n# heredoc comment\nEOF\n\t\tpython #|\n\t#sed x\n}\n";
        defers_heredoc_body_until_continued_pipeline_head_finishes:
            "if true; then\ncat << EOF | openssl req -new -key \"$key\" \\\n -x509 \\\n -out \"$cert\"\nbody\nEOF\nfi\n"
            => "if true; then\n\tcat <<EOF | openssl req -new -key \"$key\" \\\n\t\t-x509 \\\n\t\t-out \"$cert\"\nbody\nEOF\nfi\n";
    }

    default_idempotent_cases! {
        preserves_quoted_heredoc_delimiters_idempotently:
            "cat <<'EOF_264'\ndelta\nEOF_264\n",
            "quoted_heredoc.sh";
    }

    default_format_cases! {
        formats_decl_heredoc_heads_structurally:
            "declare -x foo=1<<EOF\nhi\nEOF\n"
            => "declare -x foo=1 <<EOF\nhi\nEOF\n";
    }

    default_unchanged_cases! {
        standalone_assignments_do_not_gain_trailing_spaces:
            "x=1\n";
        preserves_blank_lines_between_commands:
            "set -u\n\nfoo\n";
    }

    default_format_cases! {
        collapses_extra_blank_lines_between_items:
            "set -u\n\n\n# ready\n\n\nfoo\n"
            => "set -u\n\n# ready\n\nfoo\n";
    }

    #[test]
    fn preserves_blank_lines_after_functions() {
        let source = "foo() {\n  echo hi\n}\n\nbar\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_formats_to(
            source,
            Some(Path::new("function_gap.bash")),
            &options,
            "foo() {\n\techo hi\n}\n\nbar\n",
        );
    }

    default_unchanged_cases! {
        preserves_blank_lines_after_leading_comments:
            "#!/usr/bin/env bash\n\nset -u\n";
    }

    default_format_cases! {
        trims_trailing_comment_whitespace:
            "# note \nfoo # bar\t\n"
            => "# note\nfoo # bar\n";
    }

    bash_format_ast_cases! {
        preserves_final_comment_backslash_when_adding_trailing_newline:
            "aws logs filter-log-events \\\n                           \"$@\"\n                           #--max-items 1 \\\n                           #--end-time \"$(date '+%s')000\" \\\n"
            => "aws logs filter-log-events \\\n\t\"$@\"\n#--max-items 1 \\\n#--end-time \"$(date '+%s')000\" \\\n";
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

    default_unchanged_cases! {
        preserves_escaped_quotes_in_double_quoted_assignments:
            "fzf_completion=\"source \\\"$fzf_base/shell/completion.${shell}\\\"\"\n";
        preserves_prompt_escapes_in_double_quoted_assignments:
            "PS1=\"\\u:\\W \\$ \"\n";
    }

    default_unchanged_ast_cases! {
        preserves_escaped_dollar_literals_after_command_substitutions:
            "RUNTIME_CLASSPATH=$(echo $ALL_JARS | xargs printf -- \"\\$this_dir/%s:\"):\\$this_dir\n";
        preserves_escaped_dollar_literals_inside_quoted_command_substitutions:
            "XDGPATH=$(echo \"foreach dir [split [::tcl::tm::path list]] {puts \\$dir}\" | tclsh | tail -n1)\n";
    }

    bash_unchanged_ast_cases! {
        preserves_escaped_html_closing_tags_in_double_quoted_assignments:
            "_link=\"<a href=\\\"${target//' '/%20}\\\">[[${label:-}]]</a>\"\n";
    }

    default_format_ast_cases! {
        normalizes_backtick_escaped_dollar_literals_once:
            "XDGPATH=`echo \"foreach dir [split [::tcl::tm::path list]] {puts \\\\$dir}\" | tclsh | tail -n1`\n"
            => "XDGPATH=$(echo \"foreach dir [split [::tcl::tm::path list]] {puts \\$dir}\" | tclsh | tail -n1)\n";
    }

    #[test]
    fn preserves_escaped_dollar_command_substitutions_in_prompt_assignments() {
        let source = r##"PS1="\$([[ -n \$(git branch 2> /dev/null) ]] && echo \" on ${icon_branch}  \")${white?}$(scm_prompt_info)${normal?}\n${icon_end}"
"##;
        assert_unchanged_with_ast(source, None, &ShellFormatOptions::default());
    }

    #[test]
    fn preserves_multiline_prompt_assignments_with_backslash_continuations() {
        let source = r##"PS1="$TITLEBAR\
$YELLOW\u$LIGHT_BLUE@$YELLOW\h\
$LIGHT_BLUE-$(__theme_clock)\
$WHITE\$ $NO_COLOUR "
"##;
        assert_unchanged_with_ast(source, None, &ShellFormatOptions::default());
    }

    #[test]
    fn preserves_multiline_prompt_assignments_with_leading_continuation_lines() {
        assert_unchanged_with_ast(
            TONKA_PROMPT_ASSIGNMENT,
            None,
            &ShellFormatOptions::default(),
        );
    }

    #[test]
    fn preserves_indented_multiline_prompt_assignments_with_leading_continuation_lines() {
        let source = format!("prompt() {{\n  {TONKA_PROMPT_ASSIGNMENT}}}\n");
        assert_formats_to_with_ast(
            &source,
            None,
            &ShellFormatOptions::default(),
            format!("prompt() {{\n\t{TONKA_PROMPT_ASSIGNMENT}}}\n"),
        );
    }

    default_format_ast_cases! {
        preserves_multiline_double_quoted_argument_alignment:
            "if true; then\n  gcloud secrets list \\\n      --filter=\"labels.kubernetes-cluster=$current_cluster \\\n                AND NOT \\\n                labels.foo ~ .\" |\n  while read -r secret; do\n    echo \"$secret\"\n  done\nfi\n"
            => "if true; then\n\tgcloud secrets list \\\n\t\t--filter=\"labels.kubernetes-cluster=$current_cluster \\\n                AND NOT \\\n                labels.foo ~ .\" |\n\t\twhile read -r secret; do\n\t\t\techo \"$secret\"\n\t\tdone\nfi\n";
        preserves_background_terminator_at_if_branch_boundary:
            "if [ -z \"$SUBIT\" ]; then\n  eval $CMD_START_STANDALONE >${JBOSS_CONSOLE} 2>&1 &\nelse\n  $SUBIT \"$CMD_START_STANDALONE >${JBOSS_CONSOLE} 2>&1 &\"\nfi\n"
            => "if [ -z \"$SUBIT\" ]; then\n\teval $CMD_START_STANDALONE >${JBOSS_CONSOLE} 2>&1 &\nelse\n\t$SUBIT \"$CMD_START_STANDALONE >${JBOSS_CONSOLE} 2>&1 &\"\nfi\n";
        preserves_comment_after_then_without_raw_body_fallback:
            "if ! type -P wget &>/dev/null ||\n  type -P apk; then # Alpine built-in wget is not enough\n  \"$srcdir/../packages/install_packages.sh\" wget\nfi\n"
            => "if ! type -P wget &>/dev/null ||\n\ttype -P apk; then # Alpine built-in wget is not enough\n\t\"$srcdir/../packages/install_packages.sh\" wget\nfi\n";
        ignores_branch_keywords_inside_leading_comments:
            "f() {\n  if [ -f .iterate ]; then\n    #ls ./*/.git &>/dev/null; then  # note\n    hr\n  fi\n}\n"
            => "f() {\n\tif [ -f .iterate ]; then\n\t\t#ls ./*/.git &>/dev/null; then  # note\n\t\thr\n\tfi\n}\n";
        keeps_inline_case_conditions_before_then:
            "if case \"$@\" in *--usecwd*) true ;; *) false ;; esac then\n  USE_CWD=1\nfi\n"
            => "if case \"$@\" in *--usecwd*) true ;; *) false ;; esac then\n\tUSE_CWD=1\nfi\n";
    }

    bash_format_ast_cases! {
        splits_redirect_only_statement_after_background:
            "if ok; then\n  run --flag & 2>/dev/null\nfi\n"
            => "if ok; then\n\trun --flag &\n\t2>/dev/null\nfi\n";
        splits_following_statement_after_background:
            "if ok; then\n  run --flag 2>&1 & echo $! >\"$PIDFILE\"\nfi\n"
            => "if ok; then\n\trun --flag 2>&1 &\n\techo $! >\"$PIDFILE\"\nfi\n";
        preserves_eval_conditional_syntax_as_arguments:
            "if eval ! [[ \"$env_var\" =~ ^[[:digit:]]+$ ]]; then\n  echo ok\nfi\n"
            => "if eval ! [[ \"$env_var\" =~ ^[[:digit:]]+$ ]]; then\n\techo ok\nfi\n";
        keeps_assignment_command_substitution_condition_suffix_comment_after_then:
            "if ! out=\"$(\n     stat -c %Y \"$path\" 2>/dev/null\n   )\" # GNU\nthen\n  :\nfi\n"
            => "if ! out=\"$(\n\tstat -c %Y \"$path\" 2>/dev/null\n)\"; then # GNU\n\t:\nfi\n";
        collapses_negated_condition_continuation_before_pipeline:
            "while read -r module; do\n    if ! \\\n        git grep \"needle\" |\n        grep -v requirements.txt |\n        grep -q .; then\n        echo \"$module\"\n    fi\ndone\n"
            => "while read -r module; do\n\tif ! git grep \"needle\" |\n\t\tgrep -v requirements.txt |\n\t\tgrep -q .; then\n\t\techo \"$module\"\n\tfi\ndone\n";
        preserves_comment_between_list_operator_and_rhs:
            "if [ -z \"$jar\" ] ||\n  # incomplete download, resume it\n  ! jar tf \"$jar\" &>/dev/null; then\n  echo fetch\nfi\n"
            => "if [ -z \"$jar\" ] ||\n\t# incomplete download, resume it\n\t! jar tf \"$jar\" &>/dev/null; then\n\techo fetch\nfi\n";
        preserves_comment_between_list_operator_and_brace_group:
            "docker-compose exec -T jenkins-server install-plugins.sh ||\n  # New: later switch to\n  {\n    docker-compose cp plugins.txt jenkins-server:/\n  } || :\n"
            => "docker-compose exec -T jenkins-server install-plugins.sh ||\n\t# New: later switch to\n\t{\n\t\tdocker-compose cp plugins.txt jenkins-server:/\n\t} || :\n";
        keeps_command_list_rhs_brace_group_body_comments_inside_group:
            "if { true; } &&\n   command &&\n   {\n     # inside group\n     [[ -t 1 ]] ||\n     true\n   }\nthen\n  :\nfi\n"
            => "if { true; } &&\n\tcommand &&\n\t{\n\t\t# inside group\n\t\t[[ -t 1 ]] ||\n\t\t\ttrue\n\t}; then\n\t:\nfi\n";
    }

    default_unchanged_ast_cases! {
        preserves_grouped_if_conditions_before_then:
            "if {\n\t[ -n \"${SUDO_USER}\" ] || [ -n \"${DOAS_USER}\" ]\n} && [ \"$(id -ru)\" -eq 0 ]; then\n\tprintf '%s\\n' denied\nfi\n";
        normalizes_nested_grouped_if_condition_indentation:
            "setup() {\n\tif {\n\t\t[ -d \"/etc/dpkg/dpkg.cfg.d/\" ] || [ -d \"/usr/share/libalpm/scripts\" ]\n\t} && [ \"${init}\" -eq 0 ]; then\n\t\tsetup_hooks\n\tfi\n}\n";
    }

    default_format_ast_cases! {
        preserves_inline_brace_group_loop_bodies:
            "while read -r line; do {\n  echo \"$line\"\n}; done\n"
            => "while read -r line; do {\n\techo \"$line\"\n}; done\n";
    }

    bash_format_ast_cases! {
        collapses_then_after_multiline_grouped_if_conditions:
            "if {\n     [[ \"$group\" -eq 2 ]] &&\n       contains first\n   } || {\n     [[ \"$group\" -eq 3 ]] &&\n     ! contains second\n   }\nthen\n  return 0\nfi\n"
            => "if {\n\t[[ \"$group\" -eq 2 ]] &&\n\t\tcontains first\n} || {\n\t[[ \"$group\" -eq 3 ]] &&\n\t\t! contains second\n}; then\n\treturn 0\nfi\n";
        keeps_wrapped_inline_brace_group_conditions_attached:
            "if ! { [[ -d \"${status_file%/*}\" ]] \\\n  && [[ -r \"${status_file}\" ]]; }; then\n  echo \"\"\nfi\n"
            => "if ! { [[ -d \"${status_file%/*}\" ]] &&\n\t[[ -r \"${status_file}\" ]]; }; then\n\techo \"\"\nfi\n";
    }

    default_format_ast_cases! {
        preserves_legacy_inline_do_brace_group_without_semicolon_before_done:
            "for item in $items; do {\n  case \"$item\" in\n  a)\n    echo a\n    ;;\n  esac\n} done\n"
            => "for item in $items; do {\n\tcase \"$item\" in\n\ta)\n\t\techo a\n\t\t;;\n\tesac\n} done\n";
        preserves_legacy_inline_do_brace_group_ending_in_binary_compound:
            "for item in $items; do {\n  ok && {\n    case \"$item\" in\n    a)\n      echo a\n      ;;\n    esac\n  }\n} done\n"
            => "for item in $items; do {\n\tok && {\n\t\tcase \"$item\" in\n\t\ta)\n\t\t\techo a\n\t\t\t;;\n\t\tesac\n\t}\n} done\n";
        terminates_legacy_inline_do_brace_group_ending_in_if:
            "while read -r line; do {\n  if ok; then\n    :\n  fi\n} done <file\n"
            => "while read -r line; do {\n\tif ok; then\n\t\t:\n\tfi\n}; done <file\n";
        preserves_legacy_inline_do_brace_group_ending_in_loop_case:
            "for dev in $devs; do {\n  scan \"$dev\" | while read -r line; do {\n    case \"$line\" in\n    a)\n      echo a\n      ;;\n    esac\n  } done\n} done\n"
            => "for dev in $devs; do {\n\tscan \"$dev\" | while read -r line; do {\n\t\tcase \"$line\" in\n\t\ta)\n\t\t\techo a\n\t\t\t;;\n\t\tesac\n\t} done\n} done\n";
        preserves_explicit_breaks_inside_conditional_binaries:
            "[[ $a -le 255 && $b -le 255 &&\n  $c -le 255 && $d -le 255 ]]\n"
            => "[[ $a -le 255 && $b -le 255 &&\n\t$c -le 255 && $d -le 255 ]]\n";
        preserves_backslash_breaks_inside_conditional_binaries:
            "[[ $a -le 255 && $b -le 255 \\\n  && $c -le 255 && $d -le 255 ]]\n"
            => "[[ $a -le 255 && $b -le 255 &&\n\t$c -le 255 && $d -le 255 ]]\n";
    }

    bash_format_ast_cases! {
        collapses_backslash_breaks_inside_conditional_comparisons:
            "rename() {\n  if [[ -n  \"${_remote_head_branch:-}\"                            ]] &&\n     [[     \"${_remote_branch_name:-\"${_current_branch}\"}\" ==     \\\n              \"${_remote_head_branch:-}\"                          ]]\n  then\n    _exit_1 printf \"Only orphan branches can be renamed.\\\\n\"\n  fi\n}\n"
            => "rename() {\n\tif [[ -n \"${_remote_head_branch:-}\" ]] &&\n\t\t[[ \"${_remote_branch_name:-\"${_current_branch}\"}\" == \"${_remote_head_branch:-}\" ]]; then\n\t\t_exit_1 printf \"Only orphan branches can be renamed.\\\\n\"\n\tfi\n}\n";
    }

    default_unchanged_cases! {
        preserves_nested_parameter_expansions_inside_quoted_strings:
            "nvm_err \"N/A: version \\\"${PREFIXED_VERSION:-$PROVIDED_VERSION}\\\" is not yet installed.\"\n";
    }

    default_format_ast_cases! {
        preserves_default_redirect_spacing_without_space_redirects:
            "archi=$(uname -smo 2> /dev/null || uname -sm)\n"
            => "archi=$(uname -smo 2>/dev/null || uname -sm)\n";
        normalizes_redirect_spacing_inside_raw_multiline_command_substitutions:
            "host_sockets=\"$(find /run/host/run \\\n\t-xdev \\\n\t2> /dev/null || :)\"\n"
            => "host_sockets=\"$(find /run/host/run \\\n\t-xdev \\\n\t2>/dev/null || :)\"\n";
        keeps_command_substitution_condition_continuation_at_block_indent:
            "if true; then\n  if [[ $a != \"$(cat x)\" ||\n  $b == c ]]; then\n    echo yes\n  fi\nfi\n"
            => "if true; then\n\tif [[ $a != \"$(cat x)\" ||\n\t$b == c ]]; then\n\t\techo yes\n\tfi\nfi\n";
        normalizes_redirect_spacing_inside_parameter_default_commands:
            "[[ -t 1 && \"${CLICOLOR:=$(tput colors 2> /dev/null)}\" -ge 8 ]]\n"
            => "[[ -t 1 && \"${CLICOLOR:=$(tput colors 2>/dev/null)}\" -ge 8 ]]\n";
        normalizes_pipeline_spacing_inside_parameter_default_commands:
            "value=${value:-$(printf x|tr x y)}\n"
            => "value=${value:-$(printf x | tr x y)}\n";
    }

    bash_format_ast_cases! {
        normalizes_output_redirect_spacing_inside_raw_command_substitutions:
            "if $(! /sbin/pidof $PRGNAM > /dev/null 2>&1 ) ; then\n  echo stale\nfi\n"
            => "if $(! /sbin/pidof $PRGNAM >/dev/null 2>&1); then\n\techo stale\nfi\n";
        normalizes_leading_pipe_continuations_inside_raw_command_substitutions:
            "value=\"$(declare -f list_all \\\n\t| sed 's/list_all/list_all_without_hub/')\"\n"
            => "value=\"$(declare -f list_all |\n\tsed 's/list_all/list_all_without_hub/')\"\n";
        normalizes_trailing_pipe_continuations_inside_raw_command_substitutions:
            "value=\"$(\n  # note\n  foo | \\\n  bar | \\\n  baz\n)\"\n"
            => "value=\"$(\n\t# note\n\tfoo |\n\tbar |\n\tbaz\n)\"\n";
        carries_normalized_pipeline_indent_inside_raw_command_substitutions:
            "f() {\n    value=\"$(\n        # note\n        docker-compose \\\n            logs service | \\\n        grep token | \\\n        awk '{print $1}' || :\n    )\"\n}\n"
            => "f() {\n\tvalue=\"$(\n\t\t# note\n\t\tdocker-compose \\\n\t\t\tlogs service |\n\t\t\tgrep token |\n\t\t\tawk '{print $1}' || :\n\t)\"\n}\n";
        normalizes_leading_pipe_continuations_inside_process_substitutions:
            "while read -r line; do :; done < <(\n\tcat clean_files.txt \\\n\t\t| grep -v '^#'\n)\n"
            => "while read -r line; do :; done < <(\n\tcat clean_files.txt |\n\t\tgrep -v '^#'\n)\n";
        normalizes_here_string_spacing_inside_command_substitutions:
            "[[ $versions = \"$(sort -V <<< \"$versions\")\" ]]\n"
            => "[[ $versions = \"$(sort -V <<<\"$versions\")\" ]]\n";
        normalizes_here_string_spacing_in_raw_comment_command_substitutions:
            "value=\"$(\n\t# keep comment\n\tcat <<< \"$payload\"\n)\"\n"
            => "value=\"$(\n\t# keep comment\n\tcat <<<\"$payload\"\n)\"\n";
        normalizes_here_string_spacing:
            "sort -V <<< \"$versions\"\n"
            => "sort -V <<<\"$versions\"\n";
        preserves_multiline_here_string_literal_payload_indent:
            "case $kind in\n  service)\n    cat >$unit <<<\"\n[Unit]\nDescription=$name\n\n[Service]\nExecStart=$bin\n\"\n    ;;\nesac\n"
            => "case $kind in\nservice)\n\tcat >$unit <<<\"\n[Unit]\nDescription=$name\n\n[Service]\nExecStart=$bin\n\"\n\t;;\nesac\n";
        indents_multiline_here_string_command_substitution_targets:
            "f() {\n  IFS=' ' read -ra tags <<<\"$(\n    get_tags \"$1\" \"$2\"\n  )\"\n}\n"
            => "f() {\n\tIFS=' ' read -ra tags <<<\"$(\n\t\tget_tags \"$1\" \"$2\"\n\t)\"\n}\n";
    }

    default_unchanged_ast_cases! {
        preserves_empty_command_substitutions:
            "result=$()\n";
    }

    bash_format_ast_cases! {
        formats_commented_inline_subshell_here_string_targets:
            "f() {\n\tIFS=\" \" read -r -a COMPREPLY <<< \"$( (\n\t\twhile read -r -d ' ' i; do\n\t\t\t[[ -z \"$i\" ]] && continue\n\t\t\t# flatten array with spaces on either side,\n\t\t\t# otherwise we cannot grep on word boundaries of\n\t\t\t# first and last word\n\t\t\tCOMPREPLYSTR=\" ${COMPREPLY[*]} \"\n\t\t\t# remove word from list of completions\n\t\t\tIFS=\" \" read -r -a COMPREPLY <<< \"${COMPREPLYSTR/ ${i%% *} / }\"\n\t\tdone\n\t\tprintf '%s ' \"${COMPREPLY[@]}\"\n\t) <<< \"${COMP_WORDS[@]}\")\"\n}\n"
            => "f() {\n\tIFS=\" \" read -r -a COMPREPLY <<<\"$( (\n\t\twhile read -r -d ' ' i; do\n\t\t\t[[ -z \"$i\" ]] && continue\n\t\t\t# flatten array with spaces on either side,\n\t\t\t# otherwise we cannot grep on word boundaries of\n\t\t\t# first and last word\n\t\t\tCOMPREPLYSTR=\" ${COMPREPLY[*]} \"\n\t\t\t# remove word from list of completions\n\t\t\tIFS=\" \" read -r -a COMPREPLY <<<\"${COMPREPLYSTR/ ${i%% *} / }\"\n\t\tdone\n\t\tprintf '%s ' \"${COMPREPLY[@]}\"\n\t) <<<\"${COMP_WORDS[@]}\")\"\n}\n";
    }

    bash_unchanged_ast_cases! {
        preserves_unmodeled_command_substitution_bodies:
            "themes=$(grep \\{EXTRA_THEMES install.sh | cut -d= -f2 | cut -d} -f1)\n";
    }

    bash_format_ast_cases! {
        normalizes_redirect_spacing_in_raw_command_substitution_fallbacks:
            "find_dig=$(which dig 2> /dev/null | grep -v \"no [^ ]* in\")\ncontext_id=\"$(jq_debug_pipe_dump <<< \"$output\" | jq -r \".items[] | select(.name == \\\"$context_name\\\") | .id\")\"\nurl=\"$(jq -r \"limit(1; .[] | select(.title == \\\"$selected\\\" or .animal == \\\"$selected\\\") ) | .cover_src\" < \"$json\")\"\nCOMPREPLY=($(compgen -W \"$(awk -F ':' 'BEGIN {print_line = 0}; /^[^ ]/ {print_line = 0}' < ${MASTER_CONFIG})\" -- \"${cur}\"))\n"
            => "find_dig=$(which dig 2>/dev/null | grep -v \"no [^ ]* in\")\ncontext_id=\"$(jq_debug_pipe_dump <<<\"$output\" | jq -r \".items[] | select(.name == \\\"$context_name\\\") | .id\")\"\nurl=\"$(jq -r \"limit(1; .[] | select(.title == \\\"$selected\\\" or .animal == \\\"$selected\\\") ) | .cover_src\" <\"$json\")\"\nCOMPREPLY=($(compgen -W \"$(awk -F ':' 'BEGIN {print_line = 0}; /^[^ ]/ {print_line = 0}' <${MASTER_CONFIG})\" -- \"${cur}\"))\n";
        trims_inline_command_substitution_padding:
            "echo \"MD5SUM=\\\"$( md5sum file | cut -d' ' -f1 )\\\"\"\nlocal minute=\"${MINUTE:-$( date +%H )}\"\noutput=$( ls packages 2> /dev/null | grep pattern )\n"
            => "echo \"MD5SUM=\\\"$(md5sum file | cut -d' ' -f1)\\\"\"\nlocal minute=\"${MINUTE:-$(date +%H)}\"\noutput=$(ls packages 2>/dev/null | grep pattern)\n";
        trims_nested_inline_command_substitution_padding:
            "_pre=\"$( echo $( du -hs \"$directory/\" ) )\"\n"
            => "_pre=\"$(echo $(du -hs \"$directory/\"))\"\n";
        keeps_subshell_command_substitution_from_becoming_arithmetic:
            "id=$( (echo hi ; echo there) | checksum )\n"
            => "id=$( (\n\techo hi\n\techo there\n) | checksum)\n";
        keeps_inline_nested_subshells_from_becoming_arithmetic_commands:
            "run() {\n  ( ( echo hi ) 2>&1 | ( cat ) ) 5>&1\n}\n"
            => "run() {\n\t( (echo hi) 2>&1 | (cat)) 5>&1\n}\n";
        normalizes_inline_command_substitution_internal_spacing:
            "nlq=\"$(  _sanitizer run \"$nlq\"  numeric )\"\nline=$( head -n 2 $file|tail -n 1 )\nfile2patch=\"$( echo \"$line\" | cut -d' ' -f2 |cut -f1 )\"\nmsg=\"Welcome Hari - your last access was $(last|head -n2|tail -n1|sed 's/[^ ]\\+ \\+[^ ]\\+ \\+[^ ]\\+ \\+//;s/ *$//')\"\n"
            => "nlq=\"$(_sanitizer run \"$nlq\" numeric)\"\nline=$(head -n 2 $file | tail -n 1)\nfile2patch=\"$(echo \"$line\" | cut -d' ' -f2 | cut -f1)\"\nmsg=\"Welcome Hari - your last access was $(last | head -n2 | tail -n1 | sed 's/[^ ]\\+ \\+[^ ]\\+ \\+[^ ]\\+ \\+//;s/ *$//')\"\n";
    }

    default_format_ast_cases! {
        formats_multiline_command_substitutions:
            "result=$(\necho foo\necho bar\n)\n"
            => "result=$(\n\techo foo\n\techo bar\n)\n";
        formats_block_command_substitutions_with_trailing_comments:
            "size=$(\nstat -f\"%z\" \"$tmpFile\" 2> /dev/null; # OS X `stat`\nstat -c\"%s\" \"$tmpFile\" 2> /dev/null # GNU `stat`\n)\n"
            => "size=$(\n\tstat -f\"%z\" \"$tmpFile\" 2>/dev/null # OS X `stat`\n\tstat -c\"%s\" \"$tmpFile\" 2>/dev/null # GNU `stat`\n)\n";
        preserves_command_substitutions_with_closing_paren_on_own_line:
            "output=\"$(foo |\n          bar\n         )\"\n"
            => "output=\"$(\n\tfoo |\n\t\tbar\n)\"\n";
        formats_multiline_command_substitutions_with_compound_commands:
            "result=$(\nif foo; then\necho hi\nelse\necho bye\nfi\n)\n"
            => "result=$(\n\tif foo; then\n\t\techo hi\n\telse\n\t\techo bye\n\tfi\n)\n";
        preserves_inline_continued_command_substitution_assignments:
            "start() {\n  CHOICE=$(whiptail --title x --menu \\\n    foo 14 58 2 \\\n    yes \" \" no \" \" 3>&2 2>&1 1>&3)\n}\n"
            => "start() {\n\tCHOICE=$(whiptail --title x --menu \\\n\t\tfoo 14 58 2 \\\n\t\tyes \" \" no \" \" 3>&2 2>&1 1>&3)\n}\n";
        keeps_nested_command_substitution_multiline_literals_unindented:
            "f() {\n  case $prev in\n    -soundhw)\n      _comp_compgen_split -- \"$(\"$1\" -soundhw help | _comp_awk '\n                function islower(s) { return length(s) > 0 && s == tolower(s); }\n                islower(substr($0, 1, 1)) {print $1}') all\"\n      ;;\n  esac\n}\n"
            => "f() {\n\tcase $prev in\n\t-soundhw)\n\t\t_comp_compgen_split -- \"$(\"$1\" -soundhw help | _comp_awk '\n                function islower(s) { return length(s) > 0 && s == tolower(s); }\n                islower(substr($0, 1, 1)) {print $1}') all\"\n\t\t;;\n\tesac\n}\n";
    }

    bash_format_ast_cases! {
        trims_command_substitution_close_line_continuations:
            "tag=\"$(\n  grep '\"tag_name.*\"'\".*$version\" \"$json\" \\\n  | head -1 \\\n  | sed 's,.*\"\\(gm'\"$version\"'[^\\\"]*\\)\".*,\\1,'\\\n  )\"\n"
            => "tag=\"$(\n\tgrep '\"tag_name.*\"'\".*$version\" \"$json\" |\n\t\thead -1 |\n\t\tsed 's,.*\"\\(gm'\"$version\"'[^\\\"]*\\)\".*,\\1,'\n)\"\n";
        keeps_quoted_command_substitution_continuation_indent_stable:
            "icons() {\n  icon_files=\"${icon_files}¤$(find \\\n    /usr/share/icons \\\n    /usr/share/pixmaps \\\n    /var/lib/flatpak/exports/share/icons -iname \"*${icon}*\" \\\n    -printf \"%p¤\" 2> /dev/null || :)\"\n}\n"
            => "icons() {\n\ticon_files=\"${icon_files}¤$(find \\\n\t\t/usr/share/icons \\\n\t\t/usr/share/pixmaps \\\n\t\t/var/lib/flatpak/exports/share/icons -iname \"*${icon}*\" \\\n\t\t-printf \"%p¤\" 2>/dev/null || :)\"\n}\n";
        indents_inline_continued_command_substitution_assignments:
            "_npm_completion() {\n  compadd -- $(COMP_CWORD=$((CURRENT-1)) \\\n               COMP_LINE=$BUFFER \\\n               COMP_POINT=0 \\\n               npm completion -- \"${words[@]}\" \\\n               2>/dev/null)\n}\n"
            => "_npm_completion() {\n\tcompadd -- $(COMP_CWORD=$((CURRENT - 1)) \\\n\t\tCOMP_LINE=$BUFFER \\\n\t\tCOMP_POINT=0 \\\n\t\tnpm completion -- \"${words[@]}\" \\\n\t\t2>/dev/null)\n}\n";
        command_substitution_assignment_continuations_do_not_double_context_indent:
            "get_pr_url(){\n    local existing_pr\n    existing_pr=\"$(gh pr list -R \"$owner/$repo\" \\\n        --json baseRefName,changedFiles \\\n        -q \".[] |\n            select(.baseRefName == \\\"$base\\\")\n    \")\"\n}\n"
            => "get_pr_url() {\n\tlocal existing_pr\n\texisting_pr=\"$(gh pr list -R \"$owner/$repo\" \\\n\t\t--json baseRefName,changedFiles \\\n\t\t-q \".[] |\n            select(.baseRefName == \\\"$base\\\")\n    \")\"\n}\n";
        formats_commented_if_command_substitutions_structurally:
            "_SCOPED=\"$(\n  # selected notebook flag\n  if [[ \"$a\" != \"$b\" ]]\n  then\n    printf \"1\\\\n\"\n  else\n    printf \"0\\\\n\"\n  fi\n)\"\n"
            => "_SCOPED=\"$(\n\t# selected notebook flag\n\tif [[ \"$a\" != \"$b\" ]]; then\n\t\tprintf \"1\\\\n\"\n\telse\n\t\tprintf \"0\\\\n\"\n\tfi\n)\"\n";
    }

    default_unchanged_ast_cases! {
        keeps_single_statement_command_substitutions_with_multiline_literals_inline:
            "_comp_compgen_split -- \"$(\"$1\" -soundhw help | _comp_awk '\n                function islower(s) { return length(s) > 0 && s == tolower(s); }\n                islower(substr($0, 1, 1)) {print $1}') all\"\n";
        command_substitutions_with_comments_fall_back_to_raw_source:
            "result=$(echo foo # keep comment\necho bar)\n";
    }

    default_format_ast_cases! {
        command_substitutions_with_heredocs_use_block_layout:
            "result=$(cat <<EOF\nhello\nEOF\n)\n"
            => "result=$(\n\tcat <<EOF\nhello\nEOF\n)\n";
    }

    bash_format_ast_cases! {
        formats_multiline_command_substitutions_with_escaped_quotes_structurally:
            "response=\"$(\n  download --flag \\\n    \"https://example.test?url=${target}\" |\n    LC_ALL=C sed -E \"s/.*\\\"url\\\": \\\"([^\\\"]+)\\\".*/\\1/g\" || printf \"\"\n)\"\n"
            => "response=\"$(\n\tdownload --flag \\\n\t\t\"https://example.test?url=${target}\" |\n\t\tLC_ALL=C sed -E \"s/.*\\\"url\\\": \\\"([^\\\"]+)\\\".*/\\1/g\" || printf \"\"\n)\"\n";
        formats_commented_brace_group_pipeline_command_substitutions_structurally:
            "content=\"$(\n  {\n    cat \"$file\"\n  } | {\n    if [[ \"$tool\" =~ readab ]] &&\n       command -v readable; then # readability-cli\n      readable \\\n        --base \"$url\" \\\n        --quiet \\\n        2>/dev/null || cat\n    else\n      cat\n    fi\n  }\n)\"\n"
            => "content=\"$(\n\t{\n\t\tcat \"$file\"\n\t} | {\n\t\tif [[ \"$tool\" =~ readab ]] &&\n\t\t\tcommand -v readable; then # readability-cli\n\t\t\treadable \\\n\t\t\t\t--base \"$url\" \\\n\t\t\t\t--quiet \\\n\t\t\t\t2>/dev/null || cat\n\t\telse\n\t\t\tcat\n\t\tfi\n\t}\n)\"\n";
        command_substitutions_with_comments_and_own_line_close_use_block_layout:
            "result=\"$(grep -En pattern \"$script\" |\n                     grep -Ev -e skip \\\n                              # keep this filter documented\n                    )\"\n"
            => "result=\"$(\n\tgrep -En pattern \"$script\" |\n\t\tgrep -Ev -e skip\n\t# keep this filter documented\n)\"\n";
        normalizes_raw_block_command_substitution_short_space_indent:
            "version=$(\n  # keep the sourced version local\n  source ./version.sh\n  echo \"$VERSION\"\n)\n"
            => "version=$(\n\t# keep the sourced version local\n\tsource ./version.sh\n\techo \"$VERSION\"\n)\n";
        command_substitution_heredoc_arguments_keep_body_unindented:
            "if ok; then\n    upload --policy \"$(cat <<EOF\n{\n  \"items\": [\n$(\n    for item in \"${items[@]}\"; do\n        printf '\"%s\",\\n' \"$item\"\n    done |\n    sed '$ s/,$//'\n)\n  ]\n}\nEOF\n)\"\nfi\n"
            => "if ok; then\n\tupload --policy \"$(\n\t\tcat <<EOF\n{\n  \"items\": [\n$(\n\t\t\t\tfor item in \"${items[@]}\"; do\n\t\t\t\t\tprintf '\"%s\",\\n' \"$item\"\n\t\t\t\tdone |\n\t\t\t\t\tsed '$ s/,$//'\n\t\t\t)\n  ]\n}\nEOF\n\t)\"\nfi\n";
        command_substitution_heredocs_strip_wrapper_indent:
            "if true; then\n\tjson+=$(\n\t\tcat << EOF\n\t\t\t\t,\nEOF\n\t)\nfi\n"
            => "if true; then\n\tjson+=$(\n\t\tcat <<EOF\n\t\t\t\t,\nEOF\n\t)\nfi\n";
        command_substitution_heredocs_normalize_top_level_operator_spacing:
            "json=$(\n\tcat << EOF\n{\n\t\"ok\": true\n}\nEOF\n)\n"
            => "json=$(\n\tcat <<EOF\n{\n\t\"ok\": true\n}\nEOF\n)\n";
        command_substitution_stripped_heredoc_closer_follows_command_indent:
            "x=\"$(\n    if ok; then\n        cat <<-EOF\nbody\nEOF\n    fi\n)\"\n"
            => "x=\"$(\n\tif ok; then\n\t\tcat <<-EOF\n\t\t\tbody\n\t\tEOF\n\tfi\n)\"\n";
        quoted_command_substitution_with_escaped_replacements_formats_structurally:
            "sed_script=\"$(\n        for prefix in $prefixes; do\n            echo \"s|${prefix}\\\\>|$prefix|g;\"\n        done\n)\"\n"
            => "sed_script=\"$(\n\tfor prefix in $prefixes; do\n\t\techo \"s|${prefix}\\\\>|$prefix|g;\"\n\tdone\n)\"\n";
        raw_block_command_substitution_strips_wrapper_indent_with_comments:
            "sed_script=\"$(\n        while read -r directory prefix; do\n            if [ -z \"$directory\" ]; then\n                continue\n            fi\n            # catch whole scripts\n            echo \"s|${prefix}\\\\>|$directory/${prefix}|g;\"\n        done <<< \"$mappings\"\n)\"\n"
            => "sed_script=\"$(\n\twhile read -r directory prefix; do\n\t\tif [ -z \"$directory\" ]; then\n\t\t\tcontinue\n\t\tfi\n\t\t# catch whole scripts\n\t\techo \"s|${prefix}\\\\>|$directory/${prefix}|g;\"\n\tdone <<<\"$mappings\"\n)\"\n";
        assignment_command_substitution_heredoc_keeps_literal_tail_unindented:
            "if ok; then\n    response=\"$(\n        nc <<EOF || :\nHTTP/1.1 200 OK\n\naccepted\nEOF\n    )\"\nfi\n"
            => "if ok; then\n\tresponse=\"$(\n\t\tnc <<EOF || :\nHTTP/1.1 200 OK\n\naccepted\nEOF\n\t)\"\nfi\n";
        command_position_command_substitution_heredoc_keeps_literal_tail_unindented:
            "(\n$(\ncat << HERE\nHERE\n)\n)\n"
            => "(\n\t$(\n\t\tcat <<HERE\nHERE\n\t)\n)\n";
        process_substitution_heredoc_in_assignment_keeps_literal_tail_unindented:
            "if ok; then\n  result=$(OPENSSL_CONF=<(cat <<EOF\nbody\nEOF\n) curl url)\nfi\n"
            => "if ok; then\n\tresult=$(OPENSSL_CONF=<(\n\t\tcat <<EOF\nbody\nEOF\n\t) curl url)\nfi\n";
        raw_block_command_substitution_indents_backslash_continuations_before_comment:
            "if ok; then\n    output=\"$(\n        NO_TOKEN_AUTH=1 \\\n        USERNAME=\"$SPOTIFY_ID\" \\\n        PASSWORD=\"$SPOTIFY_SECRET\" \\\n        -d code=\"$code\" \\\n        #-d code_verifier=\"$code_verifier\"\n    )\"\nfi\n"
            => "if ok; then\n\toutput=\"$(\n\t\tNO_TOKEN_AUTH=1 \\\n\t\t\tUSERNAME=\"$SPOTIFY_ID\" \\\n\t\t\tPASSWORD=\"$SPOTIFY_SECRET\" \\\n\t\t\t-d code=\"$code\"\n\t\t#-d code_verifier=\"$code_verifier\"\n\t)\"\nfi\n";
        rendered_heredoc_bodies_preserve_escaped_variables:
            "cat <<EOF > script\n#!/bin/bash\nexec $(which dart) \"\\$@\"\nEOF\n"
            => "cat <<EOF >script\n#!/bin/bash\nexec $(which dart) \"\\$@\"\nEOF\n";
    }

    bash_unchanged_ast_cases! {
        rendered_heredoc_bodies_preserve_escaped_backslashes:
            "cat <<EOF\nline \\\\\nEOF\n";
    }

    bash_format_ast_cases! {
        heredoc_command_substitution_continuations_follow_shell_indent:
            "if ok; then\n  cat <<EOF\nx $(date +%F |\n      # comment\n      sed 's/-/--/g') y\nEOF\nfi\n"
            => "if ok; then\n\tcat <<EOF\nx $(date +%F |\n\t\t# comment\n\t\tsed 's/-/--/g') y\nEOF\nfi\n";
    }

    default_unchanged_ast_cases! {
        command_substitution_bounds_do_not_capture_following_comments:
            "value=$(pwd)\n# after\nnext\n";
    }

    bash_format_ast_cases! {
        command_substitution_bounds_do_not_capture_following_list_operators:
            "value=$(cmd \\\n  arg) || echo no\n"
            => "value=$(cmd \\\n\targ) || echo no\n";
    }

    default_unchanged_ast_cases! {
        dirname_command_substitution_bounds_do_not_capture_following_comments:
            "cd \"$(dirname \"${BASH_SOURCE[0]}\")\"\n# after\nnext\n";
        nested_command_substitution_bounds_do_not_capture_following_comments:
            "INFO=\"$(which \"${COMMAND}\") ($(type \"${COMMAND}\" | command awk '{print $4}'))\"\n# after\nnext\n";
        preserves_conditional_command_substitutions_with_nested_quoted_arguments:
            "[[ \"$(get_permission \"$1\")\" != \"$(id -u)\" ]]\n";
        preserves_file_not_grpowned_command_substitution_shape:
            "[[ \" $(id -G \"${USER}\") \" != *\" $(get_group \"$1\") \"* ]]\n";
        preserves_long_suffix_trim_operators_in_words:
            "package_url=\"${package_url%%#*}\"\necho \"${1%%.*}\"\n";
    }

    bash_unchanged_ast_cases! {
        preserves_escaped_quote_words_with_nested_quoted_command_substitutions:
            "echo \"\\\"$BUILDSCRIPT\\\" -a \\\"$TERMUX_ARCH\\\" $TERMUX_DEBUG_BUILD --format \\\"$TERMUX_FORMAT\\\" --library $(test \"${PKG_DIR%/*}\" = \"gpkg\" && echo \"glibc\" || echo \"bionic\") ${TERMUX_OUTPUT_DIR+-o $TERMUX_OUTPUT_DIR} $TERMUX_INSTALL_DEPS \\\"$PKG_DIR\\\"\"\n";
    }

    default_format_ast_cases! {
        preserves_parameter_replacements_and_slice_offsets:
            "if [ \"$package_url\" != \"${package_url/\\#}\" ]; then\n  echo \"${arg:$index:1}\"\n  local fetch_args=(\"$package_name\" \"${@:1:$package_type_nargs}\")\n  local y=${charmap:$((RANDOM%${#charmap})):1}\n  for arg in \"${@:$(( $package_type_nargs + 1 ))}\"; do\n    echo \"$arg\"\n  done\nfi\n"
            => "if [ \"$package_url\" != \"${package_url/\\#/}\" ]; then\n\techo \"${arg:$index:1}\"\n\tlocal fetch_args=(\"$package_name\" \"${@:1:$package_type_nargs}\")\n\tlocal y=${charmap:$((RANDOM % ${#charmap})):1}\n\tfor arg in \"${@:$(($package_type_nargs + 1))}\"; do\n\t\techo \"$arg\"\n\tdone\nfi\n";
        preserves_replacement_patterns_that_need_raw_delimiters:
            "title=\"${title//\\\"}\"\nlocal profile=\"${1// }\"\n"
            => "title=\"${title//\\\"/}\"\nlocal profile=\"${1// /}\"\n";
    }

    default_unchanged_ast_cases! {
        preserves_quoted_replacements_with_escaped_delimiters:
            "query=\"${query//\\\"/\\\\\\\"}\"\nurl_path=\"${url_path//https:\\\\/\\\\/api.openai.com\\/v1}\"\n";
    }

    default_format_ast_cases! {
        inserts_empty_replacement_delimiter_after_escaped_quote_replacements:
            "playlist=\"${playlist//\\\\\"/\\\\\\\\\"}\"\nplaylist=\"${playlist//'/\\\\'}\"\n"
            => "playlist=\"${playlist//\\\\\"/\\\\\\\\\"/}\"\nplaylist=\"${playlist//'/\\\\'/}\"\n";
        preserves_negative_parameter_slice_offset_spacing:
            "if [ \"${filename: -5}\" != .orig ]; then\n  echo no\nfi\n"
            => "if [ \"${filename: -5}\" != .orig ]; then\n\techo no\nfi\n";
    }

    bash_unchanged_ast_cases! {
        compacts_parameter_slice_arithmetic_operands:
            "region=\"${zone::${#zone}-1}\"\nindex=\"${items:1+2:count-1}\"\n";
    }

    default_format_ast_cases! {
        formats_major_minor_multiline_command_substitution_like_shfmt:
            "major_minor() {\n  echo \"${1%%.*}.$(\n    x=\"${1#*.}\"\n    echo \"${x%%.*}\"\n  )\"\n}\n"
            => "major_minor() {\n\techo \"${1%%.*}.$(\n\t\tx=\"${1#*.}\"\n\t\techo \"${x%%.*}\"\n\t)\"\n}\n";
        formats_nvm_args_command_substitution_without_losing_sed_body_layout:
            "ARGS=$(\n  nvm_echo \"$@\" | command sed \"\n    s/--progress-bar /--progress=bar /\n    s/-s /-q /\n  \"\n)\n"
            => "ARGS=$(\n\tnvm_echo \"$@\" | command sed \"\n    s/--progress-bar /--progress=bar /\n    s/-s /-q /\n  \"\n)\n";
    }

    bash_format_ast_cases! {
        formats_quoted_block_command_substitution_conditions_like_shfmt:
            "f() {\n  declare -i -r test_jobs_effective=\"$(\n    if [[ \"${TEST_JOBS:-detect}\" = \"detect\" ]] \\\n      && command -v nproc &> /dev/null; then\n      nproc\n    fi\n  )\"\n}\n"
            => "f() {\n\tdeclare -i -r test_jobs_effective=\"$(\n\t\tif [[ \"${TEST_JOBS:-detect}\" = \"detect\" ]] &&\n\t\t\tcommand -v nproc &>/dev/null; then\n\t\t\tnproc\n\t\tfi\n\t)\"\n}\n";
        formats_quoted_continued_command_substitution_lists_like_shfmt:
            "f() {\n  branchName=\"$(git symbolic-ref --quiet --short HEAD 2> /dev/null \\\n    || git rev-parse --short HEAD 2> /dev/null \\\n    || echo '(unknown)')\"\n}\n"
            => "f() {\n\tbranchName=\"$(git symbolic-ref --quiet --short HEAD 2>/dev/null ||\n\t\tgit rev-parse --short HEAD 2>/dev/null ||\n\t\techo '(unknown)')\"\n}\n";
        indents_assignment_command_substitution_leading_pipe_continuations:
            "f() {\n  certText=$(echo \"${tmp}\" \\\n    | openssl x509 -text -certopt \"no_header, no_serial, \\\n    no_signame\")\n}\n"
            => "f() {\n\tcertText=$(echo \"${tmp}\" |\n\t\topenssl x509 -text -certopt \"no_header, no_serial, \\\n\t\tno_signame\")\n}\n";
        indents_inline_command_substitution_backslash_continuations:
            "f() {\n  providers=\"$(find . |\n    sed -e 's/^a/b/' \\\n      -e 's/^c/d/')\"\n}\n"
            => "f() {\n\tproviders=\"$(find . |\n\t\tsed -e 's/^a/b/' \\\n\t\t\t-e 's/^c/d/')\"\n}\n";
        indents_literal_assignment_command_substitution_pipeline_continuations:
            "f() {\n  while ok; do\n    protected_branches=\"$protected_branches\n                            $(jq_debug_pipe_dump <<< \"$output\" |\n                              jq -r '.[] | select(.protected == true)')\"\n  done\n}\n"
            => "f() {\n\twhile ok; do\n\t\tprotected_branches=\"$protected_branches\n                            $(jq_debug_pipe_dump <<<\"$output\" |\n\t\t\tjq -r '.[] | select(.protected == true)')\"\n\tdone\n}\n";
        indents_command_substitution_continuations_after_multiline_literals:
            "f() {\n  allowed=\"$(sed 's/#.*//;\n                        s/^[[:space:]]*//;\n                        /^[[:space:]]*$/d;' \\\n                        \"$file\")\"\n}\n"
            => "f() {\n\tallowed=\"$(sed 's/#.*//;\n                        s/^[[:space:]]*//;\n                        /^[[:space:]]*$/d;' \\\n\t\t\"$file\")\"\n}\n";
        indents_assignment_command_substitution_pipelines_after_multiline_literals:
            "if [ \"$version\" = latest ]; then\n    version=\"$(gh api \"repos/$owner_repo/tags\" \\\n                --jq '\n                    .[] |\n                    select(.name | test(\"^go[0-9]\")) |\n                    .name\n                ' --paginate |\n                head -n1 |\n                sed 's/^go//' || :)\"\nfi\n"
            => "if [ \"$version\" = latest ]; then\n\tversion=\"$(gh api \"repos/$owner_repo/tags\" \\\n\t\t--jq '\n                    .[] |\n                    select(.name | test(\"^go[0-9]\")) |\n                    .name\n                ' --paginate |\n\t\thead -n1 |\n\t\tsed 's/^go//' || :)\"\nfi\n";
        block_command_substitution_pipelines_keep_nested_indent:
            "backups=\"$(\n    while read -r mountpoint; do\n        ls -t \"$mountpoint\" |\n        sed '\n            s|\\.backup/*$||;\n        '\n    done <<< \"$mountpoints\" |\n    sort -r\n)\"\n"
            => "backups=\"$(\n\twhile read -r mountpoint; do\n\t\tls -t \"$mountpoint\" |\n\t\t\tsed '\n            s|\\.backup/*$||;\n        '\n\tdone <<<\"$mountpoints\" |\n\t\tsort -r\n)\"\n";
    }

    bash_format_ast_cases! {
        block_command_substitution_pipeline_stage_after_multiline_literal_keeps_stage_indent:
            "versions=\"$(\n    grep rpm <<< \"$downloads_page\" |\n    sed '\n        s/^.*basic[[:alpha:]]*-//;\n        s/linuxx64//;\n    ' |\n    sort -Vur\n)\"\n"
            => "versions=\"$(\n\tgrep rpm <<<\"$downloads_page\" |\n\t\tsed '\n        s/^.*basic[[:alpha:]]*-//;\n        s/linuxx64//;\n    ' |\n\t\tsort -Vur\n)\"\n";
        block_command_substitution_pipeline_after_command_continuations_keeps_stage_indent:
            "artist_id=\"$(\n    SEARCH_TYPE=artist \\\n    SEARCH_LIMIT=50 \\\n    \"$srcdir/search.sh\" \"$artist\" |\n    jq -r \"\n        .items[] |\n        .id\n    \" |\n    head -n1\n)\"\n"
            => "artist_id=\"$(\n\tSEARCH_TYPE=artist \\\n\t\tSEARCH_LIMIT=50 \\\n\t\t\"$srcdir/search.sh\" \"$artist\" |\n\t\tjq -r \"\n        .items[] |\n        .id\n    \" |\n\t\thead -n1\n)\"\n";
        inline_assignment_command_substitution_pipeline_after_multiline_literal_keeps_body_indent:
            "f() {\n  packages=\"$(sed 's/#.*//;\n         s/[<>=].*//;\n         /^[[:space:]]*$/d;' $package_files |\n        sort |\n        uniq -d\n    )\"\n}\n"
            => "f() {\n\tpackages=\"$(\n\t\tsed 's/#.*//;\n         s/[<>=].*//;\n         /^[[:space:]]*$/d;' $package_files |\n\t\t\tsort |\n\t\t\tuniq -d\n\t)\"\n}\n";
        preserves_quoted_command_substitution_multiline_literals:
            "f() {\n  _comp_compgen_split -- \"$(cmd | _comp_awk '\n                function islower(s) { return length(s) > 0 && s == tolower(s); }\n                islower(substr($0, 1, 1)) {print $1}')\"\n}\n"
            => "f() {\n\t_comp_compgen_split -- \"$(cmd | _comp_awk '\n                function islower(s) { return length(s) > 0 && s == tolower(s); }\n                islower(substr($0, 1, 1)) {print $1}')\"\n}\n";
        indents_block_command_substitution_assignments_with_multiline_literals:
            "f() {\n  gw=\"$(\n    netstat -rn |\n    awk '\n            /^default/ { print $2 }\n        '\n  )\"\n}\n"
            => "f() {\n\tgw=\"$(\n\t\tnetstat -rn |\n\t\t\tawk '\n            /^default/ { print $2 }\n        '\n\t)\"\n}\n";
        keeps_multiline_arithmetic_if_condition_then_attached:
            "if ! (( BASH_VERSINFO[0] > 4 ||\n        BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 2 )); then\n  exit 1\nfi\n"
            => "if ! ((\\\nBASH_VERSINFO[0] > 4 || \\\nBASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 2)); then\n\texit 1\nfi\n";
        formats_binary_spacing_around_command_substitution_arithmetic_operands:
            "printf \"%s\\n\" \"$(($(foo)-bar))\"\n"
            => "printf \"%s\\n\" \"$(($(foo) - bar))\"\n";
    }

    bash_unchanged_ast_cases! {
        preserves_inline_if_elif_command_substitutions:
            "color=\"$(if [ \"$status\" = ok ]; then echo GREEN; elif [ \"$status\" = bad ]; then echo RED; else echo WHITE; fi)\"\n";
    }

    default_format_cases! {
        formats_arithmetic_expansions_from_ruby_build:
            "echo $(( ver[0]*100 + ver[1] ))\n"
            => "echo $((ver[0] * 100 + ver[1]))\n";
        trims_arithmetic_command_delimiter_padding:
            "if (( EUID == 0 )); then\n  abort root\nfi\n"
            => "if ((EUID == 0)); then\n\tabort root\nfi\n";
        formats_shell_style_variables_inside_arithmetic_expansions_like_shfmt:
            "index=$(($index+1))\n"
            => "index=$(($index + 1))\n";
    }

    default_format_ast_cases! {
        formats_arithmetic_for_init_assignment_spacing:
            "for ((i=1;i<limit;++i)); do\n  echo \"$i\"\ndone\nfor ((j = 1; ; j++)); do\n  echo \"$j\"\ndone\n"
            => "for ((i = 1; i < limit; ++i)); do\n\techo \"$i\"\ndone\nfor ((j = 1; ; j++)); do\n\techo \"$j\"\ndone\n";
        formats_arithmetic_command_assignment_spacing:
            "((count+=1))\n((total = count + 1))\n((y=x+1))\nif ((${value:=0} == 1)); then\n  return 0\nfi\n"
            => "((count += 1))\n((total = count + 1))\n((y = x + 1))\nif ((${value:=0} == 1)); then\n\treturn 0\nfi\n";
        trims_command_substitution_padding_inside_arithmetic_expansions:
            "echo $(( $( echo \"$speed\" | cut -d'k' -f1 ) * 1024 ))\nborder=$(( $( _system uptime days ) * 3 )) # daily\n"
            => "echo $(($(echo \"$speed\" | cut -d'k' -f1) * 1024))\nborder=$(($(_system uptime days) * 3)) # daily\n";
        trims_arithmetic_expansion_padding_inside_double_quotes:
            "echo \"$(( $(_system date unixtime) - DIFF ))\"\necho \"lasts $(( $t2 - $t1 )) seconds ($(( ($t2 - $t1) / 60 )) minutes)\"\n"
            => "echo \"$(($(_system date unixtime) - DIFF))\"\necho \"lasts $(($t2 - $t1)) seconds ($((($t2 - $t1) / 60)) minutes)\"\n";
    }

    default_unchanged_ast_cases! {
        preserves_shell_style_variables_inside_arithmetic_expansions:
            "index=$(($index + 1))\n";
        preserves_command_substitutions_inside_arithmetic_expansions:
            "echo $(($(echo \"$speed\" | cut -d'k' -f1) * 1024))\nborder=$(($(_system uptime days) * 3)) # daily\n";
    }

    default_format_ast_cases! {
        keeps_array_subscript_arithmetic_compact_like_shfmt:
            "x=${arr[$REPLY-1]}\ny=${arr[$(shuf -i 0-${#arr[@]} -n1) - 1]}\necho $((arr[i+1]*2))\necho $((a-1))\n"
            => "x=${arr[$REPLY-1]}\ny=${arr[$(shuf -i 0-${#arr[@]} -n1)-1]}\necho $((arr[i+1] * 2))\necho $((a - 1))\n";
    }

    bash_format_ast_cases! {
        keeps_array_subscript_modulo_compact_like_shfmt:
            "color=${AVAILABLE_COLORS[$RANDOM % ${#AVAILABLE_COLORS[@]}]}\n"
            => "color=${AVAILABLE_COLORS[$RANDOM%${#AVAILABLE_COLORS[@]}]}\n";
        formats_arithmetic_expansion_array_subscripts_like_shfmt:
            "echo ${options[$((choice*2+1))]}\n"
            => "echo ${options[$((choice * 2 + 1))]}\n";
        formats_parenthesized_array_subscripts_like_shfmt:
            "echo ${arr[(($i+1))]}\necho ${arr[((i+1))]}\necho ${arr[(i+1)]}\necho ${arr[($i+1)]}\necho ${arr[$i+1]}\n"
            => "echo ${arr[(($i + 1))]}\necho ${arr[((i + 1))]}\necho ${arr[(i + 1)]}\necho ${arr[($i + 1)]}\necho ${arr[$i+1]}\n";
        keeps_nested_parameter_operand_subscripts_compact_like_shfmt:
            ": \"${BASH_IT_BASHRC:=${BASH_SOURCE[${#BASH_SOURCE[@]} - 1]}}\"\n"
            => ": \"${BASH_IT_BASHRC:=${BASH_SOURCE[${#BASH_SOURCE[@]}-1]}}\"\n";
        keeps_plain_array_subscripts_compact_like_shfmt:
            "prev=\"${COMP_WORDS[COMP_CWORD - 1]}\"\n"
            => "prev=\"${COMP_WORDS[COMP_CWORD-1]}\"\n";
        keeps_identifier_array_subscript_arithmetic_compact_like_shfmt:
            "source \"${_files[_file - __array_offset]}\"\n"
            => "source \"${_files[_file-__array_offset]}\"\n";
    }

    default_format_cases! {
        formats_braced_shell_style_variables_inside_arithmetic_expansions_like_shfmt:
            "echo $(( ${ver[0]}*100 + ${ver[1]} ))\n"
            => "echo $((${ver[0]} * 100 + ${ver[1]}))\n";
        formats_arithmetic_expansions_from_pyenv_python_build:
            "for arg in \"${@:$(( $package_type_nargs + 1 ))}\"; do\n  echo \"$arg\"\ndone\n"
            => "for arg in \"${@:$(($package_type_nargs + 1))}\"; do\n\techo \"$arg\"\ndone\n";
        normalizes_backtick_command_substitutions_to_dollar_paren:
            "local computed_checksum=`echo \"$($checksum_command < \"$filename\")\" | tr [A-Z] [a-z]`\n"
            => "local computed_checksum=$(echo \"$($checksum_command <\"$filename\")\" | tr [A-Z] [a-z])\n";
    }

    bash_format_ast_cases! {
        normalizes_backticks_inside_preserved_escaped_quote_words:
            "eval printf \"\\\"$name -> $`echo \"${env_var}_DEFAULT\"` => \\\"\"\n"
            => "eval printf \"\\\"$name -> $$(echo \"${env_var}_DEFAULT\") => \\\"\"\n";
    }

    #[test]
    fn backtick_in_redirect_is_idempotent() {
        let source = "declare -x foo=1<\\`OF\nhi\nEF\n";
        assert_idempotent(source, None, &ShellFormatOptions::default());
    }

    default_format_cases! {
        preserves_backslash_continued_simple_commands_from_fzf_install:
            "create_file \"$bind_file\" \\\n  'function fish_user_key_bindings' \\\n  '  fzf --fish | source' \\\n  'end'\n"
            => "create_file \"$bind_file\" \\\n\t'function fish_user_key_bindings' \\\n\t'  fzf --fish | source' \\\n\t'end'\n";
    }

    default_format_ast_cases! {
        normalizes_indented_word_continuations_like_shfmt:
            "cp -a \\\n  docs README LICENSE\\\n  $PKG/usr/doc\n"
            => "cp -a \\\n\tdocs README LICENSE $PKG/usr/doc\n";
        preserves_word_continuation_without_space_before_backslash:
            "printf '%s\\n' \\\n  'ime' 'desc'\\\n  'help' ''\n"
            => "printf '%s\\n' \\\n\t'ime' 'desc' \\\n\t'help' ''\n";
    }

    bash_unchanged_ast_cases! {
        preserves_escaped_trailing_space_arguments:
            "ARCH=$(uname -a | cut -f12 -d\\ )\n";
    }

    default_format_cases! {
        preserves_backslash_continued_simple_commands_from_homebrew_install:
            "\"$1\" --enable-frozen-string-literal --disable=gems,did_you_mean,rubyopt -rrubygems -e \\\n    \"abort if Gem::Version.new(RUBY_VERSION) < \\\n              Gem::Version.new('${REQUIRED_RUBY_VERSION}')\" 2>/dev/null\n"
            => "\"$1\" --enable-frozen-string-literal --disable=gems,did_you_mean,rubyopt -rrubygems -e \\\n\t\"abort if Gem::Version.new(RUBY_VERSION) < \\\n              Gem::Version.new('${REQUIRED_RUBY_VERSION}')\" 2>/dev/null\n";
        preserves_leading_redirect_placement_in_nvm_err_helpers:
            "nvm_err() {\n  >&2 nvm_echo \"$@\"\n}\n\nnvm_err_with_colors() {\n  >&2 nvm_echo_with_colors \"$@\"\n}\n"
            => "nvm_err() {\n\t>&2 nvm_echo \"$@\"\n}\n\nnvm_err_with_colors() {\n\t>&2 nvm_echo_with_colors \"$@\"\n}\n";
    }

    default_unchanged_cases! {
        preserves_inline_negated_subshell_conditions:
            "if ! (try_curl \"$url\" || try_wget \"$url\"); then\n\treturn 1\nfi\n";
    }

    default_format_cases! {
        negated_subshell_conditions_do_not_capture_later_file_comments:
            "download() {\n  local url\n  url=https://github.com/junegunn/fzf/releases/download/v$version/${1}\n  set -o pipefail\n  if ! (try_curl $url || try_wget $url); then\n    set +o pipefail\n    binary_error=\"Failed to download with curl and wget\"\n    return\n  fi\n  set +o pipefail\n}\n\n# Try to download binary executable\narchi=$(uname -smo 2> /dev/null || uname -sm)\n"
            => "download() {\n\tlocal url\n\turl=https://github.com/junegunn/fzf/releases/download/v$version/${1}\n\tset -o pipefail\n\tif ! (try_curl $url || try_wget $url); then\n\t\tset +o pipefail\n\t\tbinary_error=\"Failed to download with curl and wget\"\n\t\treturn\n\tfi\n\tset +o pipefail\n}\n\n# Try to download binary executable\narchi=$(uname -smo 2>/dev/null || uname -sm)\n";
        preserves_else_branch_comments_inside_the_branch:
            "if foo; then\n  bar\nelse\n  # branch comment\n  baz\nfi\n"
            => "if foo; then\n\tbar\nelse\n\t# branch comment\n\tbaz\nfi\n";
        preserves_outdented_else_branch_leading_comments:
            "if foo; then\n  bar\nelse\n  baz=\n# disabled\nfi\n"
            => "if foo; then\n\tbar\nelse\n\tbaz=\n# disabled\nfi\n";
    }

    default_unchanged_cases! {
        preserves_outdented_dangling_comments_before_fi:
            "if outer; then\n\tif inner; then\n\t\tok\n\telse\n\t\tfallback\n\t# disabled\n\t# exit\n\tfi\nfi\n";
    }

    default_format_cases! {
        preserves_space_indented_dangling_comments_before_fi:
            "add_keys() {\n    if [ \"$file\" = - ]; then\n        file=/dev/stdin\n    # sed reports this already\n    #elif ! [ -f \"$file\" ]; then\n    #    die \"missing: $file\"\n    fi\n}\n"
            => "add_keys() {\n\tif [ \"$file\" = - ]; then\n\t\tfile=/dev/stdin\n\t# sed reports this already\n\t#elif ! [ -f \"$file\" ]; then\n\t#    die \"missing: $file\"\n\tfi\n}\n";
        normalizes_underindented_dangling_comments_inside_case_bodies:
            "case $x in\na)\nif outer; then\n\tif inner; then\n\t\tok\n\telse\n\t\t:\n\t\t# disabled\n\tfi\nfi\n;;\nesac\n"
            => "case $x in\na)\n\tif outer; then\n\t\tif inner; then\n\t\t\tok\n\t\telse\n\t\t\t:\n\t\t\t# disabled\n\t\tfi\n\tfi\n\t;;\nesac\n";
    }

    default_format_cases! {
        formats_multiline_compound_assignments_structurally:
            "directories=(\n  bin\n  etc\n  Frameworks\n)\n"
            => "directories=(\n\tbin\n\tetc\n\tFrameworks\n)\n";
    }

    default_format_ast_cases! {
        preserves_inline_multiline_compound_assignment_delimiters:
            "options=(path frozen without\n  ssl_verify_mode system_bindir user_agent)\n"
            => "options=(path frozen without\n\tssl_verify_mode system_bindir user_agent)\n";
        aligns_compound_assignment_command_substitution_close_suffixes:
            "f() {\n  case \"$prev\" in\n  -a)\n    COMPREPLY=($(compgen -W \"$(\n      salt-key -l un --no-color\n      salt-key -l rej --no-color\n    )\" -- \"${cur}\"))\n    ;;\n  esac\n}\n"
            => "f() {\n\tcase \"$prev\" in\n\t-a)\n\t\tCOMPREPLY=($(compgen -W \"$(\n\t\t\tsalt-key -l un --no-color\n\t\t\tsalt-key -l rej --no-color\n\t\t)\" -- \"${cur}\"))\n\t\t;;\n\tesac\n}\n";
    }

    bash_format_ast_cases! {
        removes_multiline_compound_assignment_line_continuations:
            "if ok; then\n    params+=(-Done=true -Dtwo=false \\\n               -Dthree=false \\\n               -Dfour=true)\nfi\n"
            => "if ok; then\n\tparams+=(-Done=true -Dtwo=false\n\t\t-Dthree=false\n\t\t-Dfour=true)\nfi\n";
        preserves_escaped_multiline_double_quoted_compound_items:
            "show() {\n  items+=(\"\\\n    $(printf \" ---------\")\n     Text.\n\n     $(\n  for   x in a b\n  do\n    echo \"$x\"\n  done\n)\")\n}\n"
            => "show() {\n\titems+=(\"\\\n    $(printf \" ---------\")\n     Text.\n\n     $(\n\t\tfor x in a b; do\n\t\t\techo \"$x\"\n\t\tdone\n\t)\")\n}\n";
        strips_residual_indent_from_continued_array_rows:
            "cmd=(\n  grep -s                         \\\n    -e \"^<${url}>\"                 \\\n    -e \"^##\"\n)\n"
            => "cmd=(\n\tgrep -s\n\t-e \"^<${url}>\"\n\t-e \"^##\"\n)\n";
        preserves_array_command_substitution_argument_continuations:
            "x=( $(find . -not \\( -path ./x -prune \\) -not -name lib \\\n  -not -name other | sort) )\n"
            => "x=($(find . -not \\( -path ./x -prune \\) -not -name lib \\\n\t-not -name other | sort))\n";
        removes_decl_array_assignment_line_continuations:
            "local cmd=(dialog --title \"Select\" --default-item \"$default\" \\\n    --menu \"Choose\" 18 50 9)\n"
            => "local cmd=(dialog --title \"Select\" --default-item \"$default\"\n\t--menu \"Choose\" 18 50 9)\n";
        normalizes_multiline_compound_assignment_row_spacing:
            "options=(\n  1 \"1080p\"  \"Set 1080p\"\n  2 \"720p\"   \"Set 720p\"\n)\n"
            => "options=(\n\t1 \"1080p\" \"Set 1080p\"\n\t2 \"720p\" \"Set 720p\"\n)\n";
        ignores_comments_for_multiline_compound_assignment_body_indent:
            "versions=(1.16.0\n# Match the server package.\n                    21.1.16)\n"
            => "versions=(1.16.0\n\t# Match the server package.\n\t21.1.16)\n";
        preserves_blank_lines_inside_multiline_compound_assignments:
            "args=(\n  one\n\n  # group\n  two\n\n  three\n)\n"
            => "args=(\n\tone\n\n\t# group\n\ttwo\n\n\tthree\n)\n";
        normalizes_keyed_compound_assignment_row_indent:
            "declare -A map=(\n        [up]=one\n   [down]=two\n\n        [left]=three\n)\n"
            => "declare -A map=(\n\t[up]=one\n\t[down]=two\n\n\t[left]=three\n)\n";
        normalizes_multiline_command_substitution_array_elements:
            "items=(\nfirst\n    $(\n        for item in $items; do\n            echo \"$item\"\n        done\n    )\n\n)\n"
            => "items=(\n\tfirst\n\t$(\n\t\tfor item in $items; do\n\t\t\techo \"$item\"\n\t\tdone\n\t)\n\n)\n";
        normalizes_array_command_substitution_elements_like_shfmt:
            "options=( \n  config_file \"$(\n     [[ \"$config\" == *.cfg ]] && echo ok\n  )\"\n  enabled \"$( [[ -n \"$flag\" ]] && echo true || echo false)\"\n)\n"
            => "options=(\n\tconfig_file \"$(\n\t\t[[ \"$config\" == *.cfg ]] && echo ok\n\t)\"\n\tenabled \"$([[ -n \"$flag\" ]] && echo true || echo false)\"\n)\n";
        indents_array_append_command_substitution_elements_like_shfmt:
            "f() {\n  if ok; then\n    opts+=(\"$(\n      get x\n    )\")\n  fi\n}\n"
            => "f() {\n\tif ok; then\n\t\topts+=(\"$(\n\t\t\tget x\n\t\t)\")\n\tfi\n}\n";
        preserves_compound_assignment_command_substitution_body_indent:
            "f() {\n  _files=($(\n    while [[ \"$PWD\" != \"/\" ]]; do\n      _file=\"$PWD/.env\"\n      if [[ -e \"${_file}\" ]]; then\n        echo \"${_file}\"\n      fi\n      builtin cd .. || true\n    done\n  ))\n}\n"
            => "f() {\n\t_files=($(\n\t\twhile [[ \"$PWD\" != \"/\" ]]; do\n\t\t\t_file=\"$PWD/.env\"\n\t\t\tif [[ -e \"${_file}\" ]]; then\n\t\t\t\techo \"${_file}\"\n\t\t\tfi\n\t\t\tbuiltin cd .. || true\n\t\tdone\n\t))\n}\n";
        preserves_compound_assignment_command_substitution_command_continuations:
            "f() {\n  _remote_branches=($(\n    git -C \"$path\" ls-remote --heads \"$url\" 2>/dev/null \\\n      | LC_ALL=C sed \"s/.*\\///g\"\n  ))\n  _diff=($(\n    printf \"%s\\n\" \\\n      \"${_index_list[@]:-}\" \\\n      \"${_file_list[@]:-}\" \\\n      | sort \\\n      | uniq -u\n  ))\n}\n"
            => "f() {\n\t_remote_branches=($(\n\t\tgit -C \"$path\" ls-remote --heads \"$url\" 2>/dev/null |\n\t\t\tLC_ALL=C sed \"s/.*\\///g\"\n\t))\n\t_diff=($(\n\t\tprintf \"%s\\n\" \\\n\t\t\t\"${_index_list[@]:-}\" \\\n\t\t\t\"${_file_list[@]:-}\" |\n\t\t\tsort |\n\t\t\tuniq -u\n\t))\n}\n";
        normalizes_array_command_substitution_leading_list_operators:
            "f() {\n  x=(\\\n    $(printf \"%s\\n\" \"${xs[@]}\" \\\n      | sort \\\n      | cut -d: -f1 \\\n    || true))\n}\n"
            => "f() {\n\tx=(\n\t\t$(printf \"%s\\n\" \"${xs[@]}\" |\n\t\t\tsort |\n\t\t\tcut -d: -f1 ||\n\t\t\ttrue))\n}\n";
        strips_redirect_residual_indent_in_compound_assignment_command_substitutions:
            "f() {\n  _remote_branches=($(\n    git -C \"$path\" ls-remote        \\\n      --heads \"$url\"        \\\n       2>/dev/null                              \\\n      | LC_ALL=C sed \"s/.*\\///g\" || :\n  ))\n}\n"
            => "f() {\n\t_remote_branches=($(\n\t\tgit -C \"$path\" ls-remote \\\n\t\t\t--heads \"$url\" \\\n\t\t\t2>/dev/null |\n\t\t\tLC_ALL=C sed \"s/.*\\///g\" || :\n\t))\n}\n";
        expands_multistatement_command_substitutions_in_array_values:
            "f() {\n  local -A ver=(\n    [libx11]=\"$(. \"${TERMUX_SCRIPTDIR}/packages/libx11/build.sh\"; echo \"${TERMUX_PKG_VERSION}\")\"\n  )\n}\n"
            => "f() {\n\tlocal -A ver=(\n\t\t[libx11]=\"$(\n\t\t\t. \"${TERMUX_SCRIPTDIR}/packages/libx11/build.sh\"\n\t\t\techo \"${TERMUX_PKG_VERSION}\"\n\t\t)\"\n\t)\n}\n";
        expands_multistatement_assignment_command_substitutions:
            "x=$(cd /tmp ; ls | wc -l )\n"
            => "x=$(\n\tcd /tmp\n\tls | wc -l\n)\n";
        indents_quoted_block_command_substitution_loop_close:
            "f() {\n  eval \"$(\n    for key in a b; do\n      awk -F= \"/$key/\" <<< \"$profile_data\"\n    done\n  )\"\n}\n"
            => "f() {\n\teval \"$(\n\t\tfor key in a b; do\n\t\t\tawk -F= \"/$key/\" <<<\"$profile_data\"\n\t\tdone\n\t)\"\n}\n";
        normalizes_raw_block_command_substitution_comment_indent:
            "f() {\n    value=\"$(\n        docker-compose -f file.yml \\\n            exec -T service cat secret </dev/null\n            # keep this note with the command\n    )\"\n}\n"
            => "f() {\n\tvalue=\"$(\n\t\tdocker-compose -f file.yml \\\n\t\t\texec -T service cat secret </dev/null\n\t\t# keep this note with the command\n\t)\"\n}\n";
        normalizes_raw_block_command_substitution_shell_indent:
            "f() {\n    value=\"$(\n        aws service call \\\n            --query 'Items[]{\n                        \"Name\": Name\n                    }' \\\n            --output json |\n        jq -r \"\n            .[]\n        \"\n    )\"\n}\n"
            => "f() {\n\tvalue=\"$(\n\t\taws service call \\\n\t\t\t--query 'Items[]{\n                        \"Name\": Name\n                    }' \\\n\t\t\t--output json |\n\t\t\tjq -r \"\n            .[]\n        \"\n\t)\"\n}\n";
        indents_raw_block_command_substitution_pipeline_after_compound_close:
            "regions=\"$(\n    # choose enabled regions by default\n    if [ -n \"${ALL_REGIONS:-}\" ]; then\n        list_regions --all\n    else\n        list_regions\n    fi |\n    jq -r '.Regions[] | .Name'\n)\"\n"
            => "regions=\"$(\n\t# choose enabled regions by default\n\tif [ -n \"${ALL_REGIONS:-}\" ]; then\n\t\tlist_regions --all\n\telse\n\t\tlist_regions\n\tfi |\n\t\tjq -r '.Regions[] | .Name'\n)\"\n";
    }

    default_unchanged_cases! {
        preserves_case_pattern_escapes:
            "case \"$archi\" in\nDarwin\\ arm64*) download foo ;;\nesac\n";
    }

    default_format_ast_cases! {
        preserves_comments_in_empty_case_items:
            "case \"$x\" in\n1)\n# keep\n;;\nesac\n"
            => "case \"$x\" in\n1)\n\t# keep\n\t;;\nesac\n";
        preserves_comments_after_final_case_terminator:
            "case $key in\nfoo)\n  echo foo\n  ;;\n\n  #if TestValue --function equals --value \"$value\" --search \"1\"; then\n  #     echo \"Found $value\"\n  #else\n  #     echo \"Not found\"\n  #fi\nesac\n"
            => "case $key in\nfoo)\n\techo foo\n\t;;\n\n\t#if TestValue --function equals --value \"$value\" --search \"1\"; then\n\t#     echo \"Found $value\"\n\t#else\n\t#     echo \"Not found\"\n\t#fi\nesac\n";
    }

    bash_format_ast_cases! {
        case_item_comments_do_not_leak_to_previous_arm:
            "case \"$arg\" in\n--squash-msg)\n  SQUASH_MSG=1\n  ;;\n*)\n  # set the argument back\n  set -- \"$@\" \"$arg\"\n  ;;\nesac\n"
            => "case \"$arg\" in\n--squash-msg)\n\tSQUASH_MSG=1\n\t;;\n*)\n\t# set the argument back\n\tset -- \"$@\" \"$arg\"\n\t;;\nesac\n";
        preserves_comments_before_case_patterns:
            "case \"$1\" in\n# Fetch config\n--xsel | -b)\n  INIT_CONFIG_VAL=$(xsel -b)\n  ;;\n# Additional env vars\n-e | --env)\n  CONTAINER_ENV+=(\"$2\")\n  ;;\nesac\n"
            => "case \"$1\" in\n# Fetch config\n--xsel | -b)\n\tINIT_CONFIG_VAL=$(xsel -b)\n\t;;\n# Additional env vars\n-e | --env)\n\tCONTAINER_ENV+=(\"$2\")\n\t;;\nesac\n";
        preserves_body_indented_comments_before_case_patterns:
            "f() {\n  case \"$prev\" in\n  -G)\n    echo grains\n    ;;\n    # FIXME\n  -R)\n    echo range\n    ;;\n  esac\n}\n"
            => "f() {\n\tcase \"$prev\" in\n\t-G)\n\t\techo grains\n\t\t;;\n\t\t# FIXME\n\t-R)\n\t\techo range\n\t\t;;\n\tesac\n}\n";
        preserves_body_indented_comments_before_overindented_case_patterns:
            "if [ -z \"$ARCH\" ]; then\n  case \"$( uname -m )\" in\n    arm*) ARCH=arm\n          NO_ASM=1 ;;\n    # comment\n       *) ARCH=$( uname -m ) ;;\n  esac\nfi\n"
            => "if [ -z \"$ARCH\" ]; then\n\tcase \"$(uname -m)\" in\n\tarm*)\n\t\tARCH=arm\n\t\tNO_ASM=1\n\t\t;;\n\t\t# comment\n\t*) ARCH=$(uname -m) ;;\n\tesac\nfi\n";
        indents_disabled_case_arm_comments_before_next_pattern:
            "f() {\n  case \"$mode\" in\n  client)\n    echo client\n    ;;\n#\t\thybrid)\n#\t\t\techo hybrid\n#\t\t;;\n  *)\n    echo default\n    ;;\n  esac\n}\n"
            => "f() {\n\tcase \"$mode\" in\n\tclient)\n\t\techo client\n\t\t;;\n\t\t#\t\thybrid)\n\t\t#\t\t\techo hybrid\n\t\t#\t\t;;\n\t*)\n\t\techo default\n\t\t;;\n\tesac\n}\n";
        keeps_space_disabled_case_pattern_comments_at_pattern_indent:
            "if [ -z \"$ARCH\" ]; then\n  case \"$( uname -m )\" in\n#    i?86) ARCH=i586 ;;\n    arm*) ARCH=arm ;;\n  esac\nfi\n"
            => "if [ -z \"$ARCH\" ]; then\n\tcase \"$(uname -m)\" in\n\t#    i?86) ARCH=i586 ;;\n\tarm*) ARCH=arm ;;\n\tesac\nfi\n";
        indents_space_disabled_case_pattern_comments_between_arms:
            "if [ -z \"$ARCH\" ]; then\n  case \"$( uname -m )\" in\n    i?86) ARCH=i586 ;;\n#    arm*) ARCH=arm ;;\n    *)    ARCH=$( uname -m ) ;;\n  esac\nfi\n"
            => "if [ -z \"$ARCH\" ]; then\n\tcase \"$(uname -m)\" in\n\ti?86) ARCH=i586 ;;\n\t\t#    arm*) ARCH=arm ;;\n\t*) ARCH=$(uname -m) ;;\n\tesac\nfi\n";
        indents_aligned_disabled_case_arm_comments_before_next_pattern:
            "case \"$ext\" in\n          #.envrc)  cd \"$dirname\" && direnv allow .\n           .envrc)  shellcheck \"$basename\"\n                    ;;\nesac\n"
            => "case \"$ext\" in\n#.envrc)  cd \"$dirname\" && direnv allow .\n.envrc)\n\tshellcheck \"$basename\"\n\t;;\nesac\n";
        keeps_disabled_case_comments_with_explanatory_prefix_comments:
            "f() {\n  case \"$ext\" in\n               # this command does not fail when missing\n               #.vimrc)  if ! vim -c \"source $basename\" -c \"q\"; then\n               .vimrc)  echo ok\n                        ;;\n  esac\n}\n"
            => "f() {\n\tcase \"$ext\" in\n\t# this command does not fail when missing\n\t#.vimrc)  if ! vim -c \"source $basename\" -c \"q\"; then\n\t.vimrc)\n\t\techo ok\n\t\t;;\n\tesac\n}\n";
        keeps_disabled_case_arm_body_comments_at_pattern_indent_without_hash_tabs:
            "case \"$GUI\" in\n    # disabled for now\n    #QT) UI=Qt4\n         #sed -i x\n         #;;\n    QT5) UI=Qt5 ;;\nesac\n"
            => "case \"$GUI\" in\n# disabled for now\n#QT) UI=Qt4\n#sed -i x\n#;;\nQT5) UI=Qt5 ;;\nesac\n";
        keeps_aligned_disabled_case_patterns_at_pattern_indent:
            "case ${FUNCTION} in\n      \"equals\")\n          CMP1=$(echo ${SEARCH} | tr '[:upper:]' '[:lower:]')\n          if [ \"${CMP1}\" = \"${CMP1}\" ]; then RETVAL=0; else RETVAL=1; fi\n      ;;\n      #\"not-equal\")   COLOR=$WHITE   ;;\n      #\"lt\" | \"less-than\")  COLOR=$YELLOW  ;;\n      *) echo \"INVALID OPTION USED\"; exit 1 ;;\nesac\n"
            => "case ${FUNCTION} in\n\"equals\")\n\tCMP1=$(echo ${SEARCH} | tr '[:upper:]' '[:lower:]')\n\tif [ \"${CMP1}\" = \"${CMP1}\" ]; then RETVAL=0; else RETVAL=1; fi\n\t;;\n#\"not-equal\")   COLOR=$WHITE   ;;\n#\"lt\" | \"less-than\")  COLOR=$YELLOW  ;;\n*)\n\techo \"INVALID OPTION USED\"\n\texit 1\n\t;;\nesac\n";
        preserves_prefix_comments_before_empty_case_items:
            "case \"$LUA\" in\n  # LUA=no: accept and do nothing\n  no) ;;\n  # Anything else is a fail\n  *) echo fail ;;\nesac\n"
            => "case \"$LUA\" in\n# LUA=no: accept and do nothing\nno) ;;\n# Anything else is a fail\n*) echo fail ;;\nesac\n";
        keeps_case_header_comments_before_inline_first_pattern:
            "# For 15.0\n# otherwise cater to current\n#\ncase $(cmake --version | head -1) in 3.2*.*)\n  echo old\n  ;;\nesac\n"
            => "# For 15.0\n# otherwise cater to current\n#\ncase $(cmake --version | head -1) in 3.2*.*)\n\techo old\n\t;;\nesac\n";
    }

    default_format_ast_cases! {
        preserves_blank_line_after_case_pattern:
            "case $x in\na)\n\n  echo a\n  ;;\nesac\n"
            => "case $x in\na)\n\n\techo a\n\t;;\nesac\n";
        preserves_blank_line_after_case_pattern_with_prefix_comments:
            "case $x in\na)\n  echo a\n  ;;\n# disabled *)\n# note\n*)\n\n  echo default\n  ;;\nesac\n"
            => "case $x in\na)\n\techo a\n\t;;\n# disabled *)\n# note\n*)\n\n\techo default\n\t;;\nesac\n";
        does_not_treat_comment_internal_blank_as_case_pattern_gap:
            "case $x in\n*)\n  # first\n\n  # second\n  echo a\n  ;;\nesac\n"
            => "case $x in\n*)\n\t# first\n\n\t# second\n\techo a\n\t;;\nesac\n";
        preserves_blank_line_before_esac:
            "case $x in\na)\n  echo a\n  ;;\n\nesac\n"
            => "case $x in\na)\n\techo a\n\t;;\n\nesac\n";
        preserves_blank_line_before_esac_after_missing_terminator:
            "case $x in\n*) echo \"$x\"\n\nesac\n"
            => "case $x in\n*) echo \"$x\" ;;\n\nesac\n";
        preserves_blank_line_before_case_item_terminator:
            "case $x in\na)\n  echo a\n\n  ;;\nesac\n"
            => "case $x in\na)\n\techo a\n\n\t;;\nesac\n";
        does_not_treat_comment_internal_blank_as_case_terminator_gap:
            "case $x in\na)\n  echo a\n\n  # note\n  ;;\nesac\n"
            => "case $x in\na)\n\techo a\n\n\t# note\n\t;;\nesac\n";
        preserves_case_pattern_suffix_comments:
            "case $x in\n*) # default branch\nbreak ;;\nesac\n"
            => "case $x in\n*) # default branch\n\tbreak ;;\nesac\n";
    }

    bash_format_ast_cases! {
        adds_blank_line_after_multiline_case_patterns:
            "case \"$1\" in\n  --disable \\\n  | --disable-http \\\n  | --disable-https \\\n  )\n    apache_args+=(\"$1\")\n    ;;\nesac\n"
            => "case \"$1\" in\n--disable | \\\n\t--disable-http | \\\n\t--disable-https)\n\n\tapache_args+=(\"$1\")\n\t;;\nesac\n";
        trims_final_case_pattern_continuation_before_close_paren:
            "case \"$1\" in\n  *.xsl|\\\n  *.[ch]\\\n      ) pygmentize -f 256 \"$1\"\n      ;;\nesac\n"
            => "case \"$1\" in\n*.xsl | \\\n\t*.[ch])\n\tpygmentize -f 256 \"$1\"\n\t;;\nesac\n";
        keeps_attached_multiline_case_patterns_compact:
            "case \"$1\" in\n  --nginx-additional-configuration \\\n  | --nginx-external-configuration)\n    nginx_args+=(\"$1\")\n    ;;\nesac\n"
            => "case \"$1\" in\n--nginx-additional-configuration | \\\n\t--nginx-external-configuration)\n\tnginx_args+=(\"$1\")\n\t;;\nesac\n";
    }

    default_unchanged_ast_cases! {
        preserves_blank_line_between_case_items:
            "case $x in\na) echo a ;;\n\nb) echo b ;;\nesac\n";
        preserves_blank_line_after_case_in:
            "case $x in\n\na) echo a ;;\nesac\n";
        preserves_blank_line_after_case_in_comments:
            "case $x in\n\n# next\na) echo a ;;\nesac\n";
        preserves_blank_line_before_case_item_comments:
            "case $x in\na) echo a ;;\n\n# next\nb) echo b ;;\nesac\n";
    }

    #[test]
    fn aligns_case_pattern_suffix_comments_with_body_comments() {
        let source = "case \"$NODENUMBER\" in\n\t100)\t# j4\n\t\t$IPT -I FORWARD -i $LANDEV -o $WIFIDEV\t# 2nd rule = up\n\t\t$IPT -I FORWARD -i $WIFIDEV -o $LANDEV\t# 1st rule = down\n\t;;\nesac\n";
        let options = ShellFormatOptions::default();
        let expected = format!(
            "case \"$NODENUMBER\" in\n100){}# j4\n\t$IPT -I FORWARD -i $LANDEV -o $WIFIDEV # 2nd rule = up\n\t$IPT -I FORWARD -i $WIFIDEV -o $LANDEV # 1st rule = down\n\t;;\nesac\n",
            " ".repeat(36)
        );

        assert_formats_to_with_ast(source, None, &options, expected);
    }

    default_unchanged_ast_cases! {
        preserves_case_terminator_suffix_comments:
            "case $x in\n*) return 0 ;; # not needed\nesac\n";
    }

    default_format_ast_cases! {
        preserves_case_terminator_suffix_comment_alignment:
            "case ${PAGE} in\n    \"Folio\") W=612; H=936;;      # 8.5 x 13 in.\n    \"Quarto\") W=612, H=780;;     # 8.5 x 10.8 in.\nesac\n"
            => "case ${PAGE} in\n\"Folio\")\n\tW=612\n\tH=936\n\t;;                       # 8.5 x 13 in.\n\"Quarto\") W=612, H=780 ;; # 8.5 x 10.8 in.\nesac\n";
    }

    #[test]
    fn aligns_case_terminator_comments_by_contiguous_runs() {
        let source = "case \"$match\" in\n\tolsr1) \t\techo \"u32 match $udp match ip dport 698 0xffff\" ;;\t# UDP dport 698\n\tolsr2) \t\techo \"u32 match $udp match ip dport 269 0xffff\" ;;\t# UDP dport 269\n\ttcp_with_ack) \techo \"u32 match $tcp match u8 0x10 0xff at nexthdr+13\" ;;\n\ttcp_with_ack2)\techo \"u32 match $tcp match u8 0x05 0x0f at 0 match u16 0x0000 0xffc0 at 2 match u8 0x10 0xff at 33\" ;; # wondershaper\n\tvoip1)\t\t_netfilter tc_match_voip_codec '00' ;;\t# PCMU\n\tvoip2)\t\t_netfilter tc_match_voip_codec '04' ;;\t# G723\nesac\n";
        let options = ShellFormatOptions::default();
        let tcp_with_ack2 = "tcp_with_ack2) echo \"u32 match $tcp match u8 0x05 0x0f at 0 match u16 0x0000 0xffc0 at 2 match u8 0x10 0xff at 33\" ;;";
        let voip1 = "voip1) _netfilter tc_match_voip_codec '00' ;;";
        let voip2 = "voip2) _netfilter tc_match_voip_codec '04' ;;";
        let target_column = tcp_with_ack2.len() + 1;

        assert_formats_to_with_ast(
            source,
            None,
            &options,
            format!(
                "case \"$match\" in\nolsr1) echo \"u32 match $udp match ip dport 698 0xffff\" ;; # UDP dport 698\nolsr2) echo \"u32 match $udp match ip dport 269 0xffff\" ;; # UDP dport 269\ntcp_with_ack) echo \"u32 match $tcp match u8 0x10 0xff at nexthdr+13\" ;;\ntcp_with_ack2) echo \"u32 match $tcp match u8 0x05 0x0f at 0 match u16 0x0000 0xffc0 at 2 match u8 0x10 0xff at 33\" ;; # wondershaper\n{voip1}{}# PCMU\n{voip2}{}# G723\nesac\n",
                " ".repeat(target_column - voip1.len()),
                " ".repeat(target_column - voip2.len())
            ),
        );
    }

    default_format_ast_cases! {
        aligns_case_terminator_comments_after_pattern_pipe_spacing:
            "case \"$mac\" in\n  19|'6470028b2260') PORT=7534 ;; # first\n  16|'6470028b1ba2') PORT= ;; # second\n  8|'f4ec38c9c32c') PORT=7783 ;; # third\nesac\n"
            => "case \"$mac\" in\n19 | '6470028b2260') PORT=7534 ;; # first\n16 | '6470028b1ba2') PORT= ;;     # second\n8 | 'f4ec38c9c32c') PORT=7783 ;;  # third\nesac\n";
        preserves_case_suffix_comments_before_commented_compound_body:
            "case $x in\n*) # default branch\n# explain\nif test -n \"$x\"; then\n  echo \"$x\"\nfi\n;;\nesac\n"
            => "case $x in\n*) # default branch\n\t# explain\n\tif test -n \"$x\"; then\n\t\techo \"$x\"\n\tfi\n\t;;\nesac\n";
        preserves_comment_after_case_in_keyword:
            "case \"$( cut -d';' -f5 \"$FILE\" | md5sum )\" in # hash over costs\n\"$forced_hash\"*)\n  _log ok\n;;\nesac\n"
            => "case \"$(cut -d';' -f5 \"$FILE\" | md5sum)\" in # hash over costs\n\"$forced_hash\"*)\n\t_log ok\n\t;;\nesac\n";
        preserves_case_in_comment_containing_in_words:
            "case $NETWORK in\t\t# new nodes start at $I, with registering until old nodes are in database\nffweimar) I=500 ;;\nesac\n"
            => "case $NETWORK in # new nodes start at $I, with registering until old nodes are in database\nffweimar) I=500 ;;\nesac\n";
    }

    default_unchanged_ast_cases! {
        case_terminator_suffix_scan_handles_utf8_prefixes:
            "# 不支持\ncase $x in\n*) echo ok ;;\nesac\n";
    }

    default_format_cases! {
        preserves_brace_group_wrapper_comments:
            "# c\n{ # note\na=1\n}\n"
            => "# c\n{ # note\n\ta=1\n}\n";
    }

    bash_format_ast_cases! {
        preserves_nested_brace_group_after_case_body_open_comment:
            "case \"$x\" in\na)\n  grep x f && { # note\n    {\n      echo\n    } >>\"$file\"\n  } # note\n  ;;\nesac\n"
            => "case \"$x\" in\na)\n\tgrep x f && { # note\n\t\t{\n\t\t\techo\n\t\t} >>\"$file\"\n\t}\n\t;;\nesac\n";
    }

    default_format_ast_cases! {
        does_not_duplicate_leading_comments_inside_brace_groups:
            "f() {\n  # before group\n  {\n    # inside group\n    echo ok\n  }\n}\n"
            => "f() {\n\t# before group\n\t{\n\t\t# inside group\n\t\techo ok\n\t}\n}\n";
        preserves_leading_comments_before_brace_group_pipelines:
            "f() {\n  # before group\n  {\n    echo \"$@\"\n  } |\n  cat\n}\n"
            => "f() {\n\t# before group\n\t{\n\t\techo \"$@\"\n\t} |\n\t\tcat\n}\n";
        keeps_pipeline_rhs_brace_group_body_comments_inside_group:
            "f() {\n  {\n    echo left\n  } |\n  {\n  # inside group\n  echo right\n  }\n}\n"
            => "f() {\n\t{\n\t\techo left\n\t} |\n\t\t{\n\t\t\t# inside group\n\t\t\techo right\n\t\t}\n}\n";
        keeps_same_line_pipeline_rhs_brace_group_attached:
            "f() {\n  {\n    echo body\n  } | {\n    # Header\n    cat\n  } | {\n    # Footer\n    cat\n  }\n}\n"
            => "f() {\n\t{\n\t\techo body\n\t} | {\n\t\t# Header\n\t\tcat\n\t} | {\n\t\t# Footer\n\t\tcat\n\t}\n}\n";
        keeps_nested_same_line_pipeline_rhs_brace_group_attached:
            "f() {\n  {\n    {\n      echo body\n    } || {\n      echo fallback\n    }\n  } | {\n    # Header\n    cat\n  } | {\n    # Footer\n    cat\n  }\n}\n"
            => "f() {\n\t{\n\t\t{\n\t\t\techo body\n\t\t} || {\n\t\t\techo fallback\n\t\t}\n\t} | {\n\t\t# Header\n\t\tcat\n\t} | {\n\t\t# Footer\n\t\tcat\n\t}\n}\n";
    }

    default_format_cases! {
        standalone_brace_groups_do_not_consume_later_file_comments:
            "[ -n \"$x\" ] && {\nset -x\n}\n# later\nnext\n"
            => "[ -n \"$x\" ] && {\n\tset -x\n}\n# later\nnext\n";
    }

    default_unchanged_cases! {
        preserves_single_line_function_bodies:
            "tty_escape() { printf \"\\\\033[%sm\" \"$1\"; }\n";
        preserves_single_line_nested_function_bodies:
            "setup() { shellspec_type_name() { eval echo type_name ${1+'\"$@\"'}; }; }\n";
        preserves_single_line_subshells:
            "(cd \"$fzf_base\"/bin && rm -f fzf && ln -sf \"$which_fzf\" fzf)\n";
    }

    default_format_ast_cases! {
        preserves_multiline_subshell_open_and_close_placement:
            "if ok; then\n  (mkdir -p -- \"$cachedir\" &&\n    echo \"$cache_id_line\"$'\\n'\"$output\" >\"$cachefile\") 2>/dev/null\nfi\n"
            => "if ok; then\n\t(mkdir -p -- \"$cachedir\" &&\n\t\techo \"$cache_id_line\"$'\\n'\"$output\" >\"$cachefile\") 2>/dev/null\nfi\n";
        preserves_multiline_subshell_around_loop:
            "f() {\n  (while sudo -v; do\n    sleep 50\n  done) &\n}\n"
            => "f() {\n\t(while sudo -v; do\n\t\tsleep 50\n\tdone) &\n}\n";
        preserves_single_line_subshell_with_background_body:
            "if ready; then\n  ($REGEN_CMD &)\nfi\n"
            => "if ready; then\n\t($REGEN_CMD &)\nfi\n";
    }

    bash_format_ast_cases! {
        expands_subshells_with_line_continuation_headers:
            "(cd samples/ && \\\n  find . -name \"build.sh\" -exec chmod 0755 {} \\;\n)\n"
            => "(\n\tcd samples/ &&\n\t\tfind . -name \"build.sh\" -exec chmod 0755 {} \\;\n)\n";
        preserves_compound_redirect_continuations:
            "(\n  echo '#!/usr/bin/wish -f'\n  cat completion.tcl\n  sed '1,5d' $PRGNAM\n) \\\n  >$PKG/usr/bin/$PRGNAM\n"
            => "(\n\techo '#!/usr/bin/wish -f'\n\tcat completion.tcl\n\tsed '1,5d' $PRGNAM\n) \\\n\t>$PKG/usr/bin/$PRGNAM\n";
    }

    default_format_cases! {
        preserves_single_line_subshells_inside_case_bodies:
            "build_package_pyston() {\n  build_package_copy\n  mkdir -p \"${PREFIX_PATH}/bin\" \"${PREFIX_PATH}/lib\"\n  local bin\n  shopt -s nullglob\n  for bin in \"bin/\"*; do\n    if [ -f \"${bin}\" ] && [ -x \"${bin}\" ] && [ ! -L \"${bin}\" ]; then\n      case \"${bin##*/}\" in\n      \"pyston\"* )\n        ( cd \"${PREFIX_PATH}/bin\" && ln -fs \"${bin##*/}\" \"python\" )\n        ;;\n      esac\n    fi\n  done\n  shopt -u nullglob\n}\n"
            => "build_package_pyston() {\n\tbuild_package_copy\n\tmkdir -p \"${PREFIX_PATH}/bin\" \"${PREFIX_PATH}/lib\"\n\tlocal bin\n\tshopt -s nullglob\n\tfor bin in \"bin/\"*; do\n\t\tif [ -f \"${bin}\" ] && [ -x \"${bin}\" ] && [ ! -L \"${bin}\" ]; then\n\t\t\tcase \"${bin##*/}\" in\n\t\t\t\"pyston\"*)\n\t\t\t\t(cd \"${PREFIX_PATH}/bin\" && ln -fs \"${bin##*/}\" \"python\")\n\t\t\t\t;;\n\t\t\tesac\n\t\tfi\n\tdone\n\tshopt -u nullglob\n}\n";
        preserves_inline_group_redirect_suffixes:
            "build_package_activepython() {\n  local package_name=\"$1\"\n  { bash \"install.sh\" --install-dir \"${PREFIX_PATH}\"\n  } >&4 2>&1\n}\n"
            => "build_package_activepython() {\n\tlocal package_name=\"$1\"\n\t{\n\t\tbash \"install.sh\" --install-dir \"${PREFIX_PATH}\"\n\t} >&4 2>&1\n}\n";
        trailing_comments_on_function_closing_braces_do_not_poison_following_layout:
            "foo() {\necho hi\n} # trailing\nbar\n"
            => "foo() {\n\techo hi\n} # trailing\nbar\n";
    }

    default_unchanged_ast_cases! {
        preserves_escaped_command_names:
            "\\grep -q foo file\n";
        preserves_ansi_c_quoted_assignment_values:
            "x=$'\\n'\n";
        preserves_concatenated_ansi_c_quoted_assignment_values:
            "local excluded=$'\\ndefault\\n'${prefix//:/foo}\n";
        preserves_concatenated_ansi_c_quoted_arguments:
            "echo \"$cache_id_line\"$'\\n'\"$output\" >\"$cachefile\"\n";
        preserves_concatenated_ansi_c_and_escaped_double_quoted_arguments:
            "echo $'\\n'\"TERMUX_APP_PACKAGE: \\\"$TERMUX_APP_PACKAGE\\\"\"\n";
        preserves_ansi_c_quoted_condition_patterns:
            "[[ \"$c\" == $'\\r' || \"$c\" == $'\\n' ]]\n";
        preserves_multiline_function_bodies:
            "version_gt() {\n\t[[ \"${1%.*}\" -gt \"${2%.*}\" ]]\n}\n";
    }

    bash_unchanged_ast_cases! {
        heredoc_function_close_suffix_keeps_closing_brace:
            "foo() {\ncat <<EOF\nbody\nEOF\n} # trailing\nbar() { :; }\n";
        preserves_alias_expanded_simple_commands_verbatim:
            "shopt -s expand_aliases\nalias die='EXIT=$? LINE=$LINENO error_exit'\ndie \"A problem occured.\"\n";
        preserves_alias_expanded_commands_before_case_terminators:
            "shopt -s expand_aliases\nalias die='EXIT=$? LINE=$LINENO error_exit'\ncase $CLASS in\n*) false || die \"Invalid storage class.\" ;;\nesac\n";
    }

    default_format_ast_cases! {
        preserves_leading_comments_inside_function_bodies:
            "function f() {\n  # parse all defined shortcuts ${BASH_IT_DIRS_BKS}\n  if [[ -s x ]]; then\n    echo yes\n  fi\n}\n"
            => "function f() {\n\t# parse all defined shortcuts ${BASH_IT_DIRS_BKS}\n\tif [[ -s x ]]; then\n\t\techo yes\n\tfi\n}\n";
    }

    default_unchanged_ast_cases! {
        preserves_process_substitution_redirect_spacing:
            "cat < <(which -a foo)\n";
    }

    bash_format_ast_cases! {
        preserves_process_substitution_redirect_spacing_inside_command_substitutions:
            "output=\"$(\n  exec > >(tee /dev/fd/2) 2>&1\n)\"\nlatest=\"$(grep x < <(jq -r '.body' <<< \"$response\"))\"\n"
            => "output=\"$(\n\texec > >(tee /dev/fd/2) 2>&1\n)\"\nlatest=\"$(grep x < <(jq -r '.body' <<<\"$response\"))\"\n";
    }

    default_format_ast_cases! {
        formats_redirect_spacing_inside_process_substitution:
            "read -ra candidates < <(complete words 2> /dev/null)\n"
            => "read -ra candidates < <(complete words 2>/dev/null)\n";
    }

    default_unchanged_ast_cases! {
        preserves_process_substitution_attached_after_equals:
            "setfacl --restore=<(grep -E -v '^# (owner|group):' \"$tmp_file\")\n";
        preserves_inline_multiline_process_substitution_continuations:
            "while read -r line; do\n\techo \"$line\"\ndone < <(comm -23 <(printf \"%s\\n\" \"${left[@]}\" | sort) \\\n\t<(printf \"%s\\n\" \"${right[@]}\" | sort))\n";
    }

    default_format_ast_cases! {
        normalizes_inline_process_substitution_source_indent:
            "unsetall() {\n    while read -r env_var; do\n        unset \"$env_var\"\n    done < <( env |\n        grep -i \"$match\" |\n        sed 's/=.*//' )\n}\n"
            => "unsetall() {\n\twhile read -r env_var; do\n\t\tunset \"$env_var\"\n\tdone < <(env |\n\t\tgrep -i \"$match\" |\n\t\tsed 's/=.*//')\n}\n";
    }

    bash_unchanged_ast_cases! {
        aligns_multiline_single_quoted_process_substitution_words:
            "_sqlmap() {\n\tif [[ \"$cur\" == * ]]; then\n\t\twhile IFS='' read -r line; do COMPREPLY+=(\"$line\"); done < <(\n\t\t\tcompgen -W '-h --help \\\n\t\t\t--data --cookie \\\n\t\t\t--wizard' -- \"$cur\"\n\t\t)\n\tfi\n}\n";
    }

    default_format_ast_cases! {
        formats_process_substitution_with_own_line_close_as_block:
            "while read x; do\n  :\ndone < <(cmd | \\\n        awk 'BEGIN {x=0} /Sink/ {\n                 x=$1\n             }'\n        )\n"
            => "while read x; do\n\t:\ndone < <(\n\tcmd |\n\t\tawk 'BEGIN {x=0} /Sink/ {\n                 x=$1\n             }'\n)\n";
    }

    bash_format_ast_cases! {
        formats_process_substitution_heredocs_as_blocks:
            "curl -d @<(cat <<EOF\nbody\nEOF\n)\n"
            => "curl -d @<(\n\tcat <<EOF\nbody\nEOF\n)\n";
    }

    default_unchanged_ast_cases! {
        preserves_block_process_substitution_source_indentation:
            "while read -r line; do\n\techo \"$line\"\ndone < <(\n\tprintf \"%s\\n\" \"${items[@]}\"\n)\n";
    }

    default_format_ast_cases! {
        normalizes_block_process_substitution_source_indent:
            "while read -r line; do\n    echo \"$line\"\ndone < <(\n    produce_items\n)\n"
            => "while read -r line; do\n\techo \"$line\"\ndone < <(\n\tproduce_items\n)\n";
    }

    bash_format_ast_cases! {
        expands_multistatement_process_substitutions_as_blocks:
            "while read game; do\n    echo \"$game\"\ndone < <(_get_opts; echo -e \"a\\nb\")\n"
            => "while read game; do\n\techo \"$game\"\ndone < <(\n\t_get_opts\n\techo -e \"a\\nb\"\n)\n";
        normalizes_process_substitution_block_indent_from_partial_source_indent:
            "if ok; then\n   cat < <(produce; consume)\nfi\n"
            => "if ok; then\n\tcat < <(\n\t\tproduce\n\t\tconsume\n\t)\nfi\n";
        keeps_inline_process_substitution_brace_group_attached:
            "while read -r line; do\n    menu+=(\"$line\")\ndone < <( { echo \"$a\"; echo \"$b\"; } | sort -u )\n"
            => "while read -r line; do\n\tmenu+=(\"$line\")\ndone < <({\n\techo \"$a\"\n\techo \"$b\"\n} | sort -u)\n";
        does_not_duplicate_process_substitution_comments_before_pipeline_rhs:
            "while read -r item; do\n    echo \"$item\"\ndone < <(\n    # note\n    produce_items\n) |\nconsume_items\n"
            => "while read -r item; do\n\techo \"$item\"\ndone < <(\n\t# note\n\tproduce_items\n) |\n\tconsume_items\n";
        indents_process_substitution_pipeline_comments:
            "cat < <(\n    produce_items |\n    # keep this filter documented\n    filter_items |\n    sort_items\n)\n"
            => "cat < <(\n\tproduce_items |\n\t\t# keep this filter documented\n\t\tfilter_items |\n\t\tsort_items\n)\n";
        aligns_raw_pipeline_comments_after_continuation_stages:
            "cat < <(\n    produce_items |\n    filter_items |\n    # keep this filter documented\n    normalize_items |\n    sort_items\n)\n"
            => "cat < <(\n\tproduce_items |\n\t\tfilter_items |\n\t\t# keep this filter documented\n\t\tnormalize_items |\n\t\tsort_items\n)\n";
        compacts_raw_process_substitution_semicolon_terminators:
            "cat < <(\n    produce_items |\n    # keep this filter documented\n    { filter_items || : ; } |\n    sort_items\n)\n"
            => "cat < <(\n\tproduce_items |\n\t\t# keep this filter documented\n\t\t{ filter_items || :; } |\n\t\tsort_items\n)\n";
    }

    bash_format_ast_cases! {
        keeps_nested_process_substitution_close_indent_in_raw_command_substitutions:
            "urls=\"$(\n    while read -r filename; do\n        # keep comment with raw block path\n        { grep -E \"$url_regex\" \"$filename\" || : ; } |\n        if [ -n \"${URL_LINKS_IGNORED:-}\" ]; then\n            grep -Eivf <(\n                tr '[:space:]' '\\n' <<< \"$URL_LINKS_IGNORED\" |\n                sed 's/^[[:space:]]*//;\n                     s/[[:space:]]*$//;\n                     /^[[:space:]]*$/d'\n            )\n        else\n            cat\n        fi\n    done |\n    sort -uf\n)\"\n"
            => "urls=\"$(\n\twhile read -r filename; do\n\t\t# keep comment with raw block path\n\t\t{ grep -E \"$url_regex\" \"$filename\" || :; } |\n\t\t\tif [ -n \"${URL_LINKS_IGNORED:-}\" ]; then\n\t\t\t\tgrep -Eivf <(\n\t\t\t\t\ttr '[:space:]' '\\n' <<<\"$URL_LINKS_IGNORED\" |\n\t\t\t\t\t\tsed 's/^[[:space:]]*//;\n                     s/[[:space:]]*$//;\n                     /^[[:space:]]*$/d'\n\t\t\t\t)\n\t\t\telse\n\t\t\t\tcat\n\t\t\tfi\n\tdone |\n\t\tsort -uf\n)\"\n";
        indents_continued_process_substitution_comments_once:
            "cmd \\\n<(\n    produce |\n    sort #|\n    # keep the sorted stream documented\n    # before the process substitution closes\n) \\\n<(\n    # describe target stream\n    consume\n) |\nsed 's/x/y/'\n"
            => "cmd \\\n\t<(\n\t\tproduce |\n\t\t\tsort #|\n\t\t# keep the sorted stream documented\n\t\t# before the process substitution closes\n\t) \\\n\t<(\n\t\t# describe target stream\n\t\tconsume\n\t) |\n\tsed 's/x/y/'\n";
        preserves_raw_block_multiline_literal_payloads:
            "value=\"$(\n    produce_items |\n    sed '\n        s/a/b ;\n    ' |\n    consume_items\n)\"\n"
            => "value=\"$(\n\tproduce_items |\n\t\tsed '\n        s/a/b ;\n    ' |\n\t\tconsume_items\n)\"\n";
        shifts_raw_pipeline_compound_bodies_with_continuation:
            "value=\"$(\n    produce_items |\n    # keep this filter documented\n    while read -r item; do\n        consume_item \"$item\"\n    done || :\n)\"\n"
            => "value=\"$(\n\tproduce_items |\n\t\t# keep this filter documented\n\t\twhile read -r item; do\n\t\t\tconsume_item \"$item\"\n\t\tdone || :\n)\"\n";
    }

    default_unchanged_ast_cases! {
        preserves_fd_duplication_redirect_targets:
            "cmd 2>&$fd\ncmd 1>&/dev/null\ncmd >&file\n";
        preserves_append_both_redirect_spelling:
            "cmd &>>/dev/null\ncmd &>>log <input\n";
    }

    bash_unchanged_ast_cases! {
        preserves_adjacent_numeric_fd_heredoc_redirects:
            "exec \"${SHELL:-sh}\" -i 3<<EOF 4<&0 <&3\n  set +e\nEOF\n";
        preserves_fd_close_redirect_targets:
            "cmd 2>&-\nexec <&-\nexec {ACCEPT_FD}>&-\n";
        preserves_multi_digit_fd_duplication_redirect_prefixes:
            "exec 99>&1\nexec 99>&-\nread 42<&0\n";
    }

    default_unchanged_ast_cases! {
        preserves_explicit_stdout_fd_on_dup_redirects:
            "cat 1>&2 <<EOF\nhi\nEOF\n";
        preserves_simple_command_redirect_positions:
            "echo >&2 \"bad news\"\n";
    }

    bash_format_ast_cases! {
        separates_numeric_word_before_output_both_redirects:
            "echo \"Usage\" 1&>2\n"
            => "echo \"Usage\" 1 &>2\n";
        moves_interspersed_simple_command_redirects_after_arguments:
            "curl -sSf >\"$jar\" \"$url\"\ncmd a >out b 2>err c\ncmd >out a 2>err b\n"
            => "curl -sSf \"$url\" >\"$jar\"\ncmd a b c >out 2>err\ncmd >out a b 2>err\n";
        preserves_time_command_trailing_comment_after_redirect:
            "time nice ffmpeg -i \"$filepath\" \"$mp4_filepath\" < /dev/null  # note\n"
            => "time nice ffmpeg -i \"$filepath\" \"$mp4_filepath\" </dev/null # note\n";
    }

    default_unchanged_ast_cases! {
        preserves_regex_operands_in_conditionals:
            "[[ \"$x\" =~ \"git version \"([^ ]*).* ]]\n";
    }

    bash_format_ast_cases! {
        preserves_regex_alternation_operands_in_conditionals:
            "if [[ $line =~ \\<(target|extension-point)[[:space:]].*name=[\\\"\\']([^\\\"\\']+) ]]; then\n  echo \"${BASH_REMATCH[2]}\"\nfi\n"
            => "if [[ $line =~ \\<(target|extension-point)[[:space:]].*name=[\\\"\\']([^\\\"\\']+) ]]; then\n\techo \"${BASH_REMATCH[2]}\"\nfi\n";
        preserves_escaped_spaces_in_conditional_regex_operands:
            "if [[ \"$line\" =~ ^=\\  ]]; then\n  echo ok\nfi\nif [[ ! \"$line\" =~ ^=\\  ]] && [[ \"$n\" -gt 20 ]]; then\n  break\nfi\n"
            => "if [[ \"$line\" =~ ^=\\  ]]; then\n\techo ok\nfi\nif [[ ! \"$line\" =~ ^=\\  ]] && [[ \"$n\" -gt 20 ]]; then\n\tbreak\nfi\n";
    }

    #[test]
    fn preserves_explicit_line_break_after_list_operator() {
        let source = "command -v curl >/dev/null &&\n  if [[ $1 =~ tar.gz$ ]]; then\n    curl -fL $1 | tar $tar_opts\n  else\n    echo nope\n  fi\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_formats_to(
            source,
            Some(Path::new("list_operator_if.bash")),
            &options,
            "command -v curl >/dev/null &&\n\tif [[ $1 =~ tar.gz$ ]]; then\n\t\tcurl -fL $1 | tar $tar_opts\n\telse\n\t\techo nope\n\tfi\n",
        );
    }

    bash_format_ast_cases! {
        preserves_list_break_after_multiline_condition:
            "f() {\n  [[ ! -f \"$cert_file\" ||\n    \"$cert_file\" -ot /one ||\n    \"$cert_file\" -ot /two\n  ]] || (( ${force:-0} > 0 ))\n}\n"
            => "f() {\n\t[[ ! -f \"$cert_file\" ||\n\t\t\"$cert_file\" -ot /one ||\n\t\t\"$cert_file\" -ot /two ]] ||\n\t\t((${force:-0} > 0))\n}\n";
        keeps_inline_list_rhs_after_wrapped_conditional:
            "f() {\n  [[ \"${show:-}\" != true ||\n    -z \"$(which todo.sh)\" ]] && return\n}\n"
            => "f() {\n\t[[ \"${show:-}\" != true ||\n\t\t-z \"$(which todo.sh)\" ]] && return\n}\n";
        preserves_explicit_line_break_when_list_operator_starts_continued_line:
            "_command_exists goenv \\\n  || [[ -x \"$GOENV_ROOT/bin/goenv\" ]] \\\n  || return 0\n"
            => "_command_exists goenv ||\n\t[[ -x \"$GOENV_ROOT/bin/goenv\" ]] ||\n\treturn 0\n";
    }

    bash_unchanged_ast_cases! {
        keeps_inline_list_operator_before_multiline_brace_group:
            r#"function S() {
	about 'save a bookmark'
	param '1: bookmark name'
	example '$ S mybkmrk'
	group 'dirs'

	[[ $# -eq 1 ]] || {
		echo "${FUNCNAME[0]} function requires 1 argument"
		return 1
	}

	echo "$1"=\""${PWD}"\" >>"${BASH_IT_DIRS_BKS?}"
}

function R() {
	about 'remove a bookmark'
	param '1: bookmark name'
	example '$ R mybkmrk'
	group 'dirs'

	[[ $# -eq 1 ]] || {
		echo "${FUNCNAME[0]} function requires 1 argument"
		return 1
	}
}
"#;
    }

    bash_format_ast_cases! {
        keeps_multiline_condition_list_operator_before_brace_group:
            "while [[ -n \"$x\" ]] &&\n  ! {\n    [[ -d \"$x\" ]] &&\n      [[ -f \"$x\" ]]\n  } && {\n    {\n      [[ \"$x\" =~ ^/ ]] &&\n        [[ \"$x\" != / ]]\n    } || {\n      [[ \"$x\" != /tmp ]]\n    }\n  }; do\n  x=\"${x%/*}\"\ndone\n"
            => "while [[ -n \"$x\" ]] &&\n\t! {\n\t\t[[ -d \"$x\" ]] &&\n\t\t\t[[ -f \"$x\" ]]\n\t} && {\n\t{\n\t\t[[ \"$x\" =~ ^/ ]] &&\n\t\t\t[[ \"$x\" != / ]]\n\t} || {\n\t\t[[ \"$x\" != /tmp ]]\n\t}\n}; do\n\tx=\"${x%/*}\"\ndone\n";
    }

    format_cases_with_options! {
        formats_completion_function_subscripts_and_case_indent_like_shfmt:
            ShellFormatOptions::default().with_dialect(ShellDialect::Bash),
            r#"_saltkey() {
	local cur prev opts prev pprev
	COMPREPLY=()
	cur="${COMP_WORDS[COMP_CWORD]}"
	prev="${COMP_WORDS[COMP_CWORD - 1]}"
	if [ "${COMP_CWORD}" -gt 2 ]; then
		pprev="${COMP_WORDS[COMP_CWORD - 2]}"
	fi

	case "${prev}" in
		-a | --accept)
			COMPREPLY=($(compgen -W "$(
				salt-key -l un --no-color
				salt-key -l rej --no-color
			)" -- "${cur}"))
			return 0
			;;
	esac
	return 0
}
"#
            => r#"_saltkey() {
	local cur prev opts prev pprev
	COMPREPLY=()
	cur="${COMP_WORDS[COMP_CWORD]}"
	prev="${COMP_WORDS[COMP_CWORD-1]}"
	if [ "${COMP_CWORD}" -gt 2 ]; then
		pprev="${COMP_WORDS[COMP_CWORD-2]}"
	fi

	case "${prev}" in
	-a | --accept)
		COMPREPLY=($(compgen -W "$(
			salt-key -l un --no-color
			salt-key -l rej --no-color
		)" -- "${cur}"))
		return 0
		;;
	esac
	return 0
}
"#;
    }

    bash_format_ast_cases! {
        formats_case_command_substitution_array_assignment_like_shfmt:
            r#"_genkernel() {
	declare args rhs
	args=( $(case $args in
	('<0-5>') compgen -W "$(echo {1..5})" -- "$rhs" ;;
	('<outfile>'|'<file>') compgen -A file -o plusdirs -- "$rhs" ;;

	(*) compgen -o bashdefault -- "$rhs" ;; # punt
    esac) )
}
"#
            => r#"_genkernel() {
	declare args rhs
	args=($(case $args in
		'<0-5>') compgen -W "$(echo {1..5})" -- "$rhs" ;;
		'<outfile>' | '<file>') compgen -A file -o plusdirs -- "$rhs" ;;

		*) compgen -o bashdefault -- "$rhs" ;; # punt
		esac))
}
"#;
    }

    default_format_ast_cases! {
        preserves_explicit_multiline_pipeline_by_default:
            "kubectl get secrets |\n  grep -v '^NAME[[:space:]]' |\n  awk '{print $1}'\n"
            => "kubectl get secrets |\n\tgrep -v '^NAME[[:space:]]' |\n\tawk '{print $1}'\n";
        keeps_pipeline_blank_before_disabled_comment_block:
            "produce_json |\n\n#if disabled; then\n#  old_filter\n#fi\n\njq -r '.items[]'\n"
            => "produce_json |\n\n\t#if disabled; then\n\t#  old_filter\n\t#fi\n\tjq -r '.items[]'\n";
        preserves_comments_between_pipeline_and_compound_command:
            "while read -r value; do\n  echo \"$value\"\ndone |\n# keep alternate implementation note\nif type -P helper >/dev/null; then\n  helper\nelse\n  cat\nfi\n"
            => "while read -r value; do\n\techo \"$value\"\ndone |\n\t# keep alternate implementation note\n\tif type -P helper >/dev/null; then\n\t\thelper\n\telse\n\t\tcat\n\tfi\n";
        preserves_explicit_multiline_pipeline_when_operator_starts_continued_line:
            "find $PKG -print0 | xargs -0 file | grep ELF \\\n  | cut -f 1 -d : | xargs strip --strip-unneeded 2> /dev/null || true\n"
            => "find $PKG -print0 | xargs -0 file | grep ELF |\n\tcut -f 1 -d : | xargs strip --strip-unneeded 2>/dev/null || true\n";
        preserves_continued_redirect_targets:
            "sed s/x/y/ in > \\\n  out\n"
            => "sed s/x/y/ in > \\\n\tout\n";
        preserves_for_in_first_word_continuation:
            "for net_mount in \\\n  ${HOST_MOUNTS_RO} ${HOST_MOUNTS} \\\n  '/dev' '/proc'; do\n  echo \"$net_mount\"\ndone\n"
            => "for net_mount in \\\n\t${HOST_MOUNTS_RO} ${HOST_MOUNTS} \\\n\t'/dev' '/proc'; do\n\techo \"$net_mount\"\ndone\n";
        preserves_comment_after_loop_do_without_raw_body_fallback:
            "for J in \"${I}\"/*; do  # iterate over folders in a safe way\n  FIND=$(echo \"${J}\")\n  if [ -f \"${J}\" ]; then\n    echo \"${FIND}\"\n  fi\ndone\n"
            => "for J in \"${I}\"/*; do # iterate over folders in a safe way\n\tFIND=$(echo \"${J}\")\n\tif [ -f \"${J}\" ]; then\n\t\techo \"${FIND}\"\n\tfi\ndone\n";
        preserves_inline_do_if_body_layout:
            "for ITEM in ${LIST}; do if DirectoryExists ${ITEM}; then FOUND=1; break; fi; done\n"
            => "for ITEM in ${LIST}; do if DirectoryExists ${ITEM}; then\n\tFOUND=1\n\tbreak\nfi; done\n";
        preserves_brace_group_attached_after_pipeline_operator:
            "link=$(cat \"${postdetailslog}\" | {\n  nc -w 3 termbin.com 9999\n  echo $? > /tmp/nc_exit_status\n} | tr -d '\\n\\0')\n"
            => "link=$(cat \"${postdetailslog}\" | {\n\tnc -w 3 termbin.com 9999\n\techo $? >/tmp/nc_exit_status\n} | tr -d '\\n\\0')\n";
    }

    bash_format_ast_cases! {
        keeps_pipeline_continuation_at_list_rhs_indent:
            "if true; then\n  ffmpeg \\\n    && convert GIF:- \\\n    | gifsicle > out || return 2\nfi\n"
            => "if true; then\n\tffmpeg &&\n\t\tconvert GIF:- |\n\t\tgifsicle >out || return 2\nfi\n";
        expands_inline_command_substitution_pipeline_brace_groups:
            "f() {\n  title=\"$(curl -sS --fail \"$url\" | { head -n1 | sed 's/^#*//'; cat >/dev/null; } )\"\n}\n"
            => "f() {\n\ttitle=\"$(curl -sS --fail \"$url\" | {\n\t\thead -n1 | sed 's/^#*//'\n\t\tcat >/dev/null\n\t})\"\n}\n";
    }

    default_format_cases! {
        preserves_comments_between_pipeline_commands:
            "dat() {\n  find . -type f |\n    # keep this filter\n    grep -v patch |\n    sort\n}\n"
            => "dat() {\n\tfind . -type f |\n\t\t# keep this filter\n\t\tgrep -v patch |\n\t\tsort\n}\n";
    }

    default_unchanged_ast_cases! {
        preserves_for_targets_inside_inline_command_substitutions:
            "pass=\"$(for i in $(eval \"echo {1..$length}\"); do pickfrom /usr/share/dict/words; done)\"\n";
        preserves_inline_then_if_body_layout:
            "if [ -n \"${TMPFILE}\" ]; then if [ -f ${TMPFILE} ]; then rm -f ${TMPFILE}; fi; fi\n";
    }

    bash_format_ast_cases! {
        indents_raw_command_substitution_brace_group_bodies:
            "items=\"$(\n    {\n    # primary items\n    produce_items |\n    sort_items\n    }\n)\"\n"
            => "items=\"$(\n\t{\n\t\t# primary items\n\t\tproduce_items |\n\t\t\tsort_items\n\t}\n)\"\n";
        indents_raw_command_substitution_pipeline_continuations:
            "url=\"$(\n    git remote -v |\n    awk '{print $2}' |\n    head -n 1\n)\"\n"
            => "url=\"$(\n\tgit remote -v |\n\t\tawk '{print $2}' |\n\t\thead -n 1\n)\"\n";
        indents_raw_command_substitution_pipeline_before_multiline_literal_command:
            "if ok; then\n    url=\"$(\n        git remote -v |\n        awk '{print $2}' |\n        perl -pe \"\n            s/foo/bar/\n        \"\n    )\"\nfi\n"
            => "if ok; then\n\turl=\"$(\n\t\tgit remote -v |\n\t\t\tawk '{print $2}' |\n\t\t\tperl -pe \"\n            s/foo/bar/\n        \"\n\t)\"\nfi\n";
        expands_nested_inline_command_substitutions_inside_raw_blocks:
            "value=\"$(\n    {\n    sort |\n    uniq -d\n    } |\n    grep -vi $(IFS=$'\\n'; for line in $ignored_lines_regex; do [[ \"$line\" =~ ^[[:space:]]*$ ]] && continue; printf \"%s\" \" -e '$line'\"; done)\n)\"\n"
            => "value=\"$(\n\t{\n\t\tsort |\n\t\t\tuniq -d\n\t} |\n\t\tgrep -vi $(\n\t\t\tIFS=$'\\n'\n\t\t\tfor line in $ignored_lines_regex; do\n\t\t\t\t[[ \"$line\" =~ ^[[:space:]]*$ ]] && continue\n\t\t\t\tprintf \"%s\" \" -e '$line'\"\n\t\t\tdone\n\t\t)\n)\"\n";
        normalizes_nested_command_substitution_argument_indent:
            "f() {\n  result=\"$(\n    add --content \"$(\n      printf \"%b\\\\n\" \"$body\" \\\n        | tr -d $'\\r'\n    )\" --skip\n  )\"\n}\n"
            => "f() {\n\tresult=\"$(\n\t\tadd --content \"$(\n\t\t\tprintf \"%b\\\\n\" \"$body\" |\n\t\t\t\ttr -d $'\\r'\n\t\t)\" --skip\n\t)\"\n}\n";
        indents_raw_command_substitution_compound_pipeline_bodies:
            "urls=\"$(\n    find files |\n    if [ -n \"$filter\" ]; then\n        grep \"$filter\" || :\n    else\n        cat\n    fi |\n    while read -r file; do\n        [ -f \"$file\" ] || continue\n        grep \"$file\" |\n        if [ -n \"$ignored\" ]; then\n            grep -v \"$ignored\"\n        else\n            cat\n        fi\n    done |\n    sort -u\n)\"\n"
            => "urls=\"$(\n\tfind files |\n\t\tif [ -n \"$filter\" ]; then\n\t\t\tgrep \"$filter\" || :\n\t\telse\n\t\t\tcat\n\t\tfi |\n\t\twhile read -r file; do\n\t\t\t[ -f \"$file\" ] || continue\n\t\t\tgrep \"$file\" |\n\t\t\t\tif [ -n \"$ignored\" ]; then\n\t\t\t\t\tgrep -v \"$ignored\"\n\t\t\t\telse\n\t\t\t\t\tcat\n\t\t\t\tfi\n\t\tdone |\n\t\tsort -u\n)\"\n";
        indents_inline_raw_command_substitution_compound_pipeline_bodies:
            "playlist_id=\"$(producer |\n    if [ \"$x\" ]; then\n        # keep exact match\n        while read -r id name; do\n            if [[ \"$name\" = \"$playlist_name\" ]]; then\n               echo \"$id\"\n               break\n            fi\n        done\n    else\n        grep -Fi \"$playlist_name\" |\n        awk '{print $1}'\n    fi || :\n)\"\n"
            => "playlist_id=\"$(\n\tproducer |\n\t\tif [ \"$x\" ]; then\n\t\t\t# keep exact match\n\t\t\twhile read -r id name; do\n\t\t\t\tif [[ \"$name\" = \"$playlist_name\" ]]; then\n\t\t\t\t\techo \"$id\"\n\t\t\t\t\tbreak\n\t\t\t\tfi\n\t\t\tdone\n\t\telse\n\t\t\tgrep -Fi \"$playlist_name\" |\n\t\t\t\tawk '{print $1}'\n\t\tfi || :\n)\"\n";
        preserves_loop_body_brace_group_background_before_done:
            "for workflow_name in $workflows; do\n  {\n    output=\"$(printf '%s\\n' \"$workflow_name\")\"\n    echo \"$output\"\n  } &\ndone |\nsort\n"
            => "for workflow_name in $workflows; do\n\t{\n\t\toutput=\"$(printf '%s\\n' \"$workflow_name\")\"\n\t\techo \"$output\"\n\t} &\ndone |\n\tsort\n";
        indents_block_command_substitution_loop_body_comments:
            "tests=\"$(\n    for filename in $filelist; do\n        # expensive filter\n        echo \"check $filename\"\n    done\n)\"\n"
            => "tests=\"$(\n\tfor filename in $filelist; do\n\t\t# expensive filter\n\t\techo \"check $filename\"\n\tdone\n)\"\n";
        indents_block_command_substitution_pipeline_comments_inside_assignments:
            "snapshots=\"$(\n    tmutil listlocalsnapshots \"$path\" |\n    tail -n +2 |\n    # update snapshots can't be deleted so just take the date timestamped ones:\n    #\n    #                  2026-02-14-041148\n    command ggrep -oP '\\d{4}-\\d\\d-\\d\\d-\\d+'\n)\"\n"
            => "snapshots=\"$(\n\ttmutil listlocalsnapshots \"$path\" |\n\t\ttail -n +2 |\n\t\t# update snapshots can't be deleted so just take the date timestamped ones:\n\t\t#\n\t\t#                  2026-02-14-041148\n\t\tcommand ggrep -oP '\\d{4}-\\d\\d-\\d\\d-\\d+'\n)\"\n";
        normalizes_raw_block_command_substitution_inline_comment_padding:
            "resources=\"$(\n    kubectl api-resources |\n    tail -n +2 || :  # ignore incomplete API discovery\n)\"\n"
            => "resources=\"$(\n\tkubectl api-resources |\n\t\ttail -n +2 || : # ignore incomplete API discovery\n)\"\n";
        normalizes_raw_command_substitution_leading_list_operators:
            "matches=\"$(git grep -Ei \\\n    -e a \\\n    | grep -Fv x \\\n    || :\n    # note\n)\"\n"
            => "matches=\"$(\n\tgit grep -Ei \\\n\t\t-e a |\n\t\tgrep -Fv x ||\n\t\t:\n\t# note\n)\"\n";
        preserves_repeated_inline_substitution_continuation_indent:
            "matches=\"$(git grep -Ei \\\n    -e first \\\n    -e second \\\n    -e third \\\n    | grep -Fv skip \\\n    || :\n    # note\n)\"\n"
            => "matches=\"$(\n\tgit grep -Ei \\\n\t\t-e first \\\n\t\t-e second \\\n\t\t-e third |\n\t\tgrep -Fv skip ||\n\t\t:\n\t# note\n)\"\n";
    }

    default_unchanged_ast_cases! {
        preserves_comment_indentation_inside_inline_command_substitutions:
            "if ok; then\n\tfor item in $(printenv |\n\t\t# keep env names\n\t\tgrep '^APP_'); do\n\t\techo \"$item\"\n\tdone\nfi\n";
        indents_block_command_substitution_comments_inside_assignments:
            "if ok; then\n\titems=$(\n\t\t# keep generated names\n\t\tfind . -type f |\n\t\t\tsort\n\t)\nfi\n";
    }

    default_format_ast_cases! {
        preserves_command_substitution_assignment_continuation_alignment:
            "LIBS=\"$(pkg-config --libs openssl)\" \\\nCFLAGS=\"$SLKCFLAGS -Wl,-s -I$(pwd)/lib\" \\\n./configure \\\n--prefix=/usr\n"
            => "LIBS=\"$(pkg-config --libs openssl)\" \\\nCFLAGS=\"$SLKCFLAGS -Wl,-s -I$(pwd)/lib\" \\\n\t./configure \\\n\t--prefix=/usr\n";
        keeps_leading_command_substitution_assignment_continuations_flush_left:
            "A=$(pwd) \\\nB=1 \\\nC=2 \\\ncmd\n"
            => "A=$(pwd) \\\nB=1 \\\nC=2 \\\n\tcmd\n";
        indents_assignment_continuations_after_nonleading_command_substitution:
            "A=1 \\\nB=$(pwd) \\\nC=2 \\\ncmd\n"
            => "A=1 \\\n\tB=$(pwd) \\\n\tC=2 \\\n\tcmd\n";
        preserves_multiline_decl_compound_assignment_lines:
            "case $prev in\n--warnings)\n  local cats=(cross gnu obsolete override portability syntax\n    unsupported)\n  return\n  ;;\nesac\n"
            => "case $prev in\n--warnings)\n\tlocal cats=(cross gnu obsolete override portability syntax\n\t\tunsupported)\n\treturn\n\t;;\nesac\n";
        preserves_expanded_decl_compound_assignment_delimiters:
            "f() {\n  local commands=(\n    build\n    version\n  )\n}\n"
            => "f() {\n\tlocal commands=(\n\t\tbuild\n\t\tversion\n\t)\n}\n";
        aligns_runs_of_trailing_comments:
            "short=1 # first\nmuch_longer=2 # second\n"
            => "short=1       # first\nmuch_longer=2 # second\n";
        aligns_trailing_comments_after_empty_array_assignments:
            "x=() # first\nyyy=() # second\n"
            => "x=()   # first\nyyy=() # second\n";
        aligns_trailing_comments_after_normalized_arithmetic_assignments:
            "border=$(( $(_system uptime days) * 3 )) # normally\nborder=$(( border + basecount ))         # later\n"
            => "border=$(($(_system uptime days) * 3)) # normally\nborder=$((border + basecount))         # later\n";
        aligns_trailing_comments_after_normalized_command_substitutions:
            "SPACER1=\"$(_sanitizer run \"$MAX1 $LOCAL\"  add_length_diff_with_spaces)\" # one\nSPACER2=\"$(_sanitizer run \"$MAX2 $REMOTE\" add_length_diff_with_spaces)\" # two\n"
            => "SPACER1=\"$(_sanitizer run \"$MAX1 $LOCAL\" add_length_diff_with_spaces)\"  # one\nSPACER2=\"$(_sanitizer run \"$MAX2 $REMOTE\" add_length_diff_with_spaces)\" # two\n";
        aligns_trailing_comments_after_parameter_replacements:
            "while read line; do\n  line=${line%%#*}   # Remove comments\n  line=${line//:/ }  # Change colon delimiter to space\n  line=${line//,/ }  # Change comma delimiter to space\ndone\n"
            => "while read line; do\n\tline=${line%%#*}  # Remove comments\n\tline=${line//:/ } # Change colon delimiter to space\n\tline=${line//,/ } # Change comma delimiter to space\ndone\n";
    }

    bash_format_ast_cases! {
        aligns_trailing_comments_after_normalized_array_assignments:
            "args=( \"${args[@]/%/ }\" )\t\t\t# add space to all\nargs=( \"${args[@]/%$slash /$slash}\" )\t# remove space from dirs\n"
            => "args=(\"${args[@]/%/ }\")             # add space to all\nargs=(\"${args[@]/%$slash /$slash}\") # remove space from dirs\n";
        aligns_trailing_comments_after_empty_parameter_replacements:
            "MAINVER=\"${VERSION//_*}\"  # e.g. 1.8.0_9 => 1.8.0\nDEBVER=\"${VERSION//*_}\"   # e.g. 1.8.0_9 => 9\n"
            => "MAINVER=\"${VERSION//_*/}\" # e.g. 1.8.0_9 => 1.8.0\nDEBVER=\"${VERSION//*_/}\"  # e.g. 1.8.0_9 => 9\n";
    }

    #[test]
    fn aligns_trailing_comments_after_parameter_replacements_inside_functions() {
        let source = "read_conf() {\n  while read line; do\n    line=${line%%#*}   # Remove comments\n    line=${line//:/ }  # Change colon delimiter to space\n    line=${line//,/ }  # Change comma delimiter to space\n  done\n}\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);

        assert_formats_to_with_ast(
            source,
            Some(Path::new("ldconfig.in-r3")),
            &options,
            "read_conf() {\n\twhile read line; do\n\t\tline=${line%%#*}  # Remove comments\n\t\tline=${line//:/ } # Change colon delimiter to space\n\t\tline=${line//,/ } # Change comma delimiter to space\n\tdone\n}\n",
        );
    }

    default_format_ast_cases! {
        aligns_trailing_comments_after_adjacent_redirect_commands:
            "if ok; then\n  rm -f /tmp/OLSR/meshrdf_neighs* 2>/dev/null    # enforce rewrite some lines later\n  echo >>$SCHEDULER \"_wifi speed check $gateway\" # will only test once\nfi\n"
            => "if ok; then\n\trm -f /tmp/OLSR/meshrdf_neighs* 2>/dev/null    # enforce rewrite some lines later\n\techo >>$SCHEDULER \"_wifi speed check $gateway\" # will only test once\nfi\n";
        aligns_trailing_comments_after_normalized_redirect_spacing:
            "netint=$(${ipcommand} -o addr | grep \"${ip}\" | awk '{print $2}')                      # e.g eth0\nnetlink=$(${ethtoolcommand} \"${netint}\" 2> /dev/null | grep Speed | awk '{print $2}') # e.g 1000Mb/s\n"
            => "netint=$(${ipcommand} -o addr | grep \"${ip}\" | awk '{print $2}')                     # e.g eth0\nnetlink=$(${ethtoolcommand} \"${netint}\" 2>/dev/null | grep Speed | awk '{print $2}') # e.g 1000Mb/s\n";
        preserves_if_condition_on_own_line:
            "case $mode in\nprompt)\n  if\n    [[ -n ${ZSH_VERSION:-} ]]\n  then\n    echo zsh\n  fi\n  ;;\nesac\n"
            => "case $mode in\nprompt)\n\tif\n\t\t[[ -n ${ZSH_VERSION:-} ]]\n\tthen\n\t\techo zsh\n\tfi\n\t;;\nesac\n";
    }

    bash_format_ast_cases! {
        aligns_trailing_comments_after_removed_semicolons:
            "BUILD_TNC=${BUILD_TNC:-true}           ; # build tnc XML validator module\nBUILD_TDOMHTML=${BUILD_TDOMHTML:-true} ; # build tdomhtml html generation module\n"
            => "BUILD_TNC=${BUILD_TNC:-true}           # build tnc XML validator module\nBUILD_TDOMHTML=${BUILD_TDOMHTML:-true} # build tdomhtml html generation module\n";
        aligns_trailing_comments_after_bare_redirect_spacing:
            "cp -ar SlackBuild $PKG/opt/$PRGNAM/          # Copy the SlackBuild script\ncat $PRGNAM.sh > $PKG/opt/$PRGNAM/$PRGNAM.sh # Copy the launcher script\n"
            => "cp -ar SlackBuild $PKG/opt/$PRGNAM/         # Copy the SlackBuild script\ncat $PRGNAM.sh >$PKG/opt/$PRGNAM/$PRGNAM.sh # Copy the launcher script\n";
        splits_multistatement_if_conditions_like_shfmt:
            "f() {\n  if curl -X PUT -k \"${@:2}\"\n    \"$url\" \\\n      -H x \\\n      -d y; then\n    echo ok\n  fi\n}\n"
            => "f() {\n\tif\n\t\tcurl -X PUT -k \"${@:2}\"\n\t\t\"$url\" \\\n\t\t\t-H x \\\n\t\t\t-d y\n\tthen\n\t\techo ok\n\tfi\n}\n";
        collapses_then_on_next_line_after_simple_if_conditions_like_shfmt:
            "f() {\n  if [ -z \"${EDITOR:-}\" ]\n  then\n    EDITOR=vi\n  elif grep -q \"$cur\" <<<'-g'\n  then\n    COMPREPLY+=(\"-g\")\n  fi\n  if ! ContainsString \"lock\" \"$value\"\n  then\n    FOUND=1\n  fi\n}\n"
            => "f() {\n\tif [ -z \"${EDITOR:-}\" ]; then\n\t\tEDITOR=vi\n\telif grep -q \"$cur\" <<<'-g'; then\n\t\tCOMPREPLY+=(\"-g\")\n\tfi\n\tif ! ContainsString \"lock\" \"$value\"; then\n\t\tFOUND=1\n\tfi\n}\n";
    }

    bash_format_ast_cases! {
        keeps_multiline_literal_if_conditions_inline_like_shfmt:
            "case \"$ext\" in\n.vimrc) if vim -c \"\n    if !filereadable('$basename') |\n        cquit 1\n    endif\n    \" -c \"q\"; then\n  echo ok\nfi\n;;\nesac\n"
            => "case \"$ext\" in\n.vimrc)\n\tif vim -c \"\n    if !filereadable('$basename') |\n        cquit 1\n    endif\n    \" -c \"q\"; then\n\t\techo ok\n\tfi\n\t;;\nesac\n";
        splits_multistatement_elif_conditions_like_shfmt:
            "f() {\n  if type -p perl >/dev/null; then\n    perl -pe decode\n  elif type -p python3 >/dev/null &&\n    log \"using python\"\n    python3 -c 'import html' >/dev/null; then\n    python3 -c decode\n  elif type -p xmlstarlet >/dev/null; then\n    xmlstarlet unesc\n  fi\n}\n"
            => "f() {\n\tif type -p perl >/dev/null; then\n\t\tperl -pe decode\n\telif\n\t\ttype -p python3 >/dev/null &&\n\t\t\tlog \"using python\"\n\t\tpython3 -c 'import html' >/dev/null\n\tthen\n\t\tpython3 -c decode\n\telif type -p xmlstarlet >/dev/null; then\n\t\txmlstarlet unesc\n\tfi\n}\n";
        splits_multistatement_loop_conditions_like_shfmt:
            "while read mac; read name; do\n  printf '%s\\n' \"$mac:$name\"\ndone\nuntil poll; sleep 1; do\n  :\ndone\n"
            => "while\n\tread mac\n\tread name\ndo\n\tprintf '%s\\n' \"$mac:$name\"\ndone\nuntil\n\tpoll\n\tsleep 1\ndo\n\t:\ndone\n";
    }

    bash_unchanged_ast_cases! {
        preserves_if_chain_condition_on_own_line:
            "f() {\n\tif\n\t\t[[ -z \"${remote:-}\" ]]\n\tthen\n\t\techo missing\n\telif\n\t\tfile_exists_at_url \"$remote\"\n\tthen\n\t\techo remote\n\telse\n\t\techo none\n\tfi\n}\n";
    }

    default_format_ast_cases! {
        aligns_trailing_comments_across_tab_indented_if_body:
            "check_restart() {\n\tif [ $percent -gt 300 -a $OPENWRT_REV -gt 0 ]; then\t# seems busy\n\t\treturn 1\t\t# sometimes high\n\tfi\n}\n"
            => "check_restart() {\n\tif [ $percent -gt 300 -a $OPENWRT_REV -gt 0 ]; then # seems busy\n\t\treturn 1                                           # sometimes high\n\tfi\n}\n";
    }

    bash_format_ast_cases! {
        aligns_comments_across_space_indented_multiline_headers:
            "search() {\n        if ok\n        then\n          ag                      \\\n            --filename            \\\n            --hidden              \\\n            --ignore \".git\"       \\\n            --ignore-case         \\\n            --noheading           \\\n            \"${_search_args[@]}\"  \\\n            \"${_query}\"           \\\n            \"${_search_paths[@]}\" \\\n              || return 0 # Don't fail out within a single scope.\n        elif _search_with \"ack\" \"${_search_utility:-}\"\n        then # ack is available.\n          :\n        fi\n}\n"
            => "search() {\n\tif ok; then\n\t\tag \\\n\t\t\t--filename \\\n\t\t\t--hidden \\\n\t\t\t--ignore \".git\" \\\n\t\t\t--ignore-case \\\n\t\t\t--noheading \\\n\t\t\t\"${_search_args[@]}\" \\\n\t\t\t\"${_query}\" \\\n\t\t\t\"${_search_paths[@]}\" ||\n\t\t\treturn 0                                           # Don't fail out within a single scope.\n\telif _search_with \"ack\" \"${_search_utility:-}\"; then # ack is available.\n\t\t:\n\tfi\n}\n";
        aligns_list_rhs_comments_with_following_branch_header_comments:
            "search() {\n  for __target_path in \"${_target_paths[@]:-}\"\n  do\n    {\n      if _search_with \"ag\" \"${_search_utility:-}\"\n      then\n        ag                      \\\n          --filename            \\\n          --hidden              \\\n          --ignore \".git\"       \\\n          --ignore-case         \\\n          --noheading           \\\n          \"${_search_args[@]}\"  \\\n          \"${_query}\"           \\\n          \"${_search_paths[@]}\" \\\n            || return 0 # Don't fail out within a single scope.\n      elif _search_with \"ack\" \"${_search_utility:-}\"\n      then # ack is available.\n        :\n      fi\n    }\n  done\n}\n"
            => "search() {\n\tfor __target_path in \"${_target_paths[@]:-}\"; do\n\t\t{\n\t\t\tif _search_with \"ag\" \"${_search_utility:-}\"; then\n\t\t\t\tag \\\n\t\t\t\t\t--filename \\\n\t\t\t\t\t--hidden \\\n\t\t\t\t\t--ignore \".git\" \\\n\t\t\t\t\t--ignore-case \\\n\t\t\t\t\t--noheading \\\n\t\t\t\t\t\"${_search_args[@]}\" \\\n\t\t\t\t\t\"${_query}\" \\\n\t\t\t\t\t\"${_search_paths[@]}\" ||\n\t\t\t\t\treturn 0                                           # Don't fail out within a single scope.\n\t\t\telif _search_with \"ack\" \"${_search_utility:-}\"; then # ack is available.\n\t\t\t\t:\n\t\t\tfi\n\t\t}\n\tdone\n}\n";
        aligns_else_comments_with_space_indented_nested_then_comments:
            "show() {\n  if outer\n  then\n      if ok\n      then\n        rm -f \"${_rendered_temp_file_path:?}\"\n    else # default\n      if ((_print_output))\n      then # `show --print [--no-color]`\n        if ((_COLOR_ENABLED))\n        then # `show --print`\n          _highlight_syntax_if_available \"${_target_path}\"\n        else # `show --print --no-color`\n          cat \"${_target_path}\"\n        fi\n      fi\n    fi\n  fi\n}\n"
            => "show() {\n\tif outer; then\n\t\tif ok; then\n\t\t\trm -f \"${_rendered_temp_file_path:?}\"\n\t\telse                          # default\n\t\t\tif ((_print_output)); then   # `show --print [--no-color]`\n\t\t\t\tif ((_COLOR_ENABLED)); then # `show --print`\n\t\t\t\t\t_highlight_syntax_if_available \"${_target_path}\"\n\t\t\t\telse # `show --print --no-color`\n\t\t\t\t\tcat \"${_target_path}\"\n\t\t\t\tfi\n\t\t\tfi\n\t\tfi\n\tfi\n}\n";
    }

    #[test]
    fn aligns_trailing_comments_with_following_outdented_branch() {
        let source = "cell_ram() {\n\tcase \"$ram_size\" in\n\t\t12*|13*)\n\t\t\tif [ ${#ram_size} -eq 5 ]; then\t\t\t# size\n\t\t\t\tif   [ -z \"$zram_memusage\" ]; then\n\t\t\t\t\tbgcolor=\"$color_alarm\"\t\t# disabled\n\t\t\t\telif [ \"$zram_memusage\" -lt 320000 ]; then\t# pppoe\n\t\t\t\t\tbgcolor=\"$color_lightgreen\"\n\t\t\t\tfi\n\t\t\tfi\n\t\t;;\n\tesac\n}\n";
        let options = ShellFormatOptions::default();
        let alarm_prefix = "\t\t\t\tbgcolor=\"$color_alarm\"";
        let elif_prefix = "\t\t\telif [ \"$zram_memusage\" -lt 320000 ]; then ";
        let alarm_padding = " ".repeat(elif_prefix.len() - alarm_prefix.len());

        assert_formats_to_with_ast(
            source,
            None,
            &options,
            format!(
                "cell_ram() {{\n\tcase \"$ram_size\" in\n\t12* | 13*)\n\t\tif [ ${{#ram_size}} -eq 5 ]; then # size\n\t\t\tif [ -z \"$zram_memusage\" ]; then\n{alarm_prefix}{alarm_padding}# disabled\n\t\t\telif [ \"$zram_memusage\" -lt 320000 ]; then # pppoe\n\t\t\t\tbgcolor=\"$color_lightgreen\"\n\t\t\tfi\n\t\tfi\n\t\t;;\n\tesac\n}}\n"
            ),
        );
    }

    #[test]
    fn preserves_simple_elif_condition_on_own_line_after_heredoc_branch() {
        let source = "#!/bin/sh\n\nif [ \"$1\" = --query ]; then\n\n  cat <<EOF\nquery\nEOF\n\nelif\n  [ \"$1\" = --listmonitors ]\nthen\n\n  cat <<EOF\nmonitors\nEOF\nfi\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Posix);

        assert_formats_to_with_ast(
            source,
            None,
            &options,
            "#!/bin/sh\n\nif [ \"$1\" = --query ]; then\n\n\tcat <<EOF\nquery\nEOF\n\nelif\n\t[ \"$1\" = --listmonitors ]\nthen\n\n\tcat <<EOF\nmonitors\nEOF\nfi\n",
        );
    }

    default_format_ast_cases! {
        aligns_inline_if_close_comments_after_reindent:
            "scan() {\n       if IsRunning \"sentineld\"; then SENTINELONE_SCANNER_RUNNING=1; fi # macOS\n       if IsRunning \"s1-agent\"; then SENTINELONE_SCANNER_RUNNING=1; fi # Linux\n       if IsRunning \"SentinelAgent\"; then SENTINELONE_SCANNER_RUNNING=1; fi # Windows\n}\n"
            => "scan() {\n\tif IsRunning \"sentineld\"; then SENTINELONE_SCANNER_RUNNING=1; fi     # macOS\n\tif IsRunning \"s1-agent\"; then SENTINELONE_SCANNER_RUNNING=1; fi      # Linux\n\tif IsRunning \"SentinelAgent\"; then SENTINELONE_SCANNER_RUNNING=1; fi # Windows\n}\n";
        preserves_blank_line_after_if_then:
            "if true; then\n\n  echo yes\nfi\n"
            => "if true; then\n\n\techo yes\nfi\n";
        preserves_blank_line_before_if_fi:
            "if true; then\n  if other; then\n    echo yes\n  else\n    echo no\n  fi\n\nfi\n"
            => "if true; then\n\tif other; then\n\t\techo yes\n\telse\n\t\techo no\n\tfi\n\nfi\n";
        preserves_blank_line_before_simple_fi:
            "if true; then\n  echo yes\n\nfi\n"
            => "if true; then\n\techo yes\n\nfi\n";
        does_not_treat_comment_internal_blank_as_fi_gap:
            "if true; then\n  echo yes\n\n  # disabled\nfi\n"
            => "if true; then\n\techo yes\n\n\t# disabled\nfi\n";
        preserves_blank_line_before_if_branches:
            "if true; then\n  echo yes\n\nelif false; then\n  echo no\n\nelse\n  echo maybe\nfi\n"
            => "if true; then\n\techo yes\n\nelif false; then\n\techo no\n\nelse\n\techo maybe\nfi\n";
        preserves_blank_line_before_commented_if_branch:
            "if true; then\n  echo yes\n\n# try the fallback\nelif false; then\n  echo no\nfi\n"
            => "if true; then\n\techo yes\n\n# try the fallback\nelif false; then\n\techo no\nfi\n";
        preserves_blank_line_before_fi_after_elif_branch:
            "if true; then\n  echo yes\nelif false; then\n  echo no\n\nfi\n"
            => "if true; then\n\techo yes\nelif false; then\n\techo no\n\nfi\n";
        does_not_preserve_branch_blanks_from_inline_keywords:
            "# setup\n\nif true; then yes; else\n  no\nfi\n"
            => "# setup\n\nif true; then yes; else\n\tno\nfi\n";
        preserves_blank_line_after_while_do:
            "while read -r dep; do\n\n  ver=${dep#*=}\ndone\n"
            => "while read -r dep; do\n\n\tver=${dep#*=}\ndone\n";
        preserves_blank_line_before_done:
            "while true; do\n  echo yes\n\ndone\n"
            => "while true; do\n\techo yes\n\ndone\n";
        preserves_blank_line_after_brace_group_open:
            "if true; then\n  [ -n \"$x\" ] && {\n\n    echo yes\n  }\nfi\n"
            => "if true; then\n\t[ -n \"$x\" ] && {\n\n\t\techo yes\n\t}\nfi\n";
        preserves_blank_line_after_brace_group_open_suffix_comment:
            "if true; then\n  [ -n \"$x\" ] || { # note\n\n    echo yes\n  }\nfi\n"
            => "if true; then\n\t[ -n \"$x\" ] || { # note\n\n\t\techo yes\n\t}\nfi\n";
    }

    bash_format_ast_cases! {
        preserves_blank_line_after_if_then_suffix_comment:
            "if [[ -s ./bin/rails ]]; then # binstub\n\n  ruby ./bin/rails\nfi\n"
            => "if [[ -s ./bin/rails ]]; then # binstub\n\n\truby ./bin/rails\nfi\n";
        does_not_insert_blank_after_then_suffix_comment_before_body_comment:
            "if [[ \"${#_test_line}\" -gt \"${_COLUMNS}\" ]]\nthen # wrap to next line\n  # Use the existing value.\n  echo yes\nfi\n"
            => "if [[ \"${#_test_line}\" -gt \"${_COLUMNS}\" ]]; then # wrap to next line\n\t# Use the existing value.\n\techo yes\nfi\n";
        indents_dangling_comments_before_done_like_shfmt:
            "while true; do\n  echo ok\n# buffered input\ndone\n"
            => "while true; do\n\techo ok\n\t# buffered input\ndone\n";
    }

    bash_unchanged_ast_cases! {
        preserves_while_condition_on_own_line:
            "f() {\n\twhile\n\t\t[[ ! -r \"$target\" && \"$target\" != \"\" ]]\n\tdo\n\t\tchmod ugo+rX \"$target\"\n\t\ttarget=\"${target%/*}\"\n\tdone\n}\n";
    }

    #[test]
    fn aligns_brace_group_open_suffix_comments_with_body_comments() {
        let source = "if true; then\n\t[ $LASTSEEN -gt 350000 ] && {\t\t# 97 hours\n\t\tLASTSEEN=\"$(( $LOCALUNIXTIME - $( stat -c \"%Y\" \"$FILE\" ) ))\"\t\t# Y = last modification time\n\t}\nfi\n";
        let options = ShellFormatOptions::default();
        let open_line = "[ $LASTSEEN -gt 350000 ] && {";
        let assignment_line = "LASTSEEN=\"$(($LOCALUNIXTIME - $(stat -c \"%Y\" \"$FILE\")))\"";
        let target_column = assignment_line.len() + 2;

        assert_formats_to_with_ast(
            source,
            None,
            &options,
            format!(
                "if true; then\n\t{open_line}{}# 97 hours\n\t\t{assignment_line} # Y = last modification time\n\t}}\nfi\n",
                " ".repeat(target_column - open_line.len())
            ),
        );
    }

    default_format_ast_cases! {
        does_not_insert_blank_before_body_leading_brace_pipeline:
            "if ok; then\n  {\n    echo yes\n  } | cat\nelse\n  {\n    echo no\n  } | cat\nfi\n"
            => "if ok; then\n\t{\n\t\techo yes\n\t} | cat\nelse\n\t{\n\t\techo no\n\t} | cat\nfi\n";
        preserves_blank_line_before_inline_do_brace_close:
            "while read -r line; do {\n  echo \"$line\"\n\n} done <file\n"
            => "while read -r line; do {\n\techo \"$line\"\n\n}; done <file\n";
        preserves_blank_line_before_inline_do_brace_close_after_nested_group:
            "while read -r line; do {\n  [ -n \"$line\" ] && {\n    echo \"$line\"\n  }\n\n} done <file\n"
            => "while read -r line; do {\n\t[ -n \"$line\" ] && {\n\t\techo \"$line\"\n\t}\n\n}; done <file\n";
        formats_multiline_arithmetic_commands_with_continuations:
            "if true; then\n  ((\n  I++,\n  IDX = 16\n  + R * 5\n  + G * 6\n  ))\nelse\n  echo no\nfi\n"
            => "if true; then\n\t((\\\n\tI++, \\\n\tIDX = 16 + \\\n\tR * 5 + \\\n\tG * 6))\n\nelse\n\techo no\nfi\n";
    }

    bash_format_ast_cases! {
        formats_multiline_arithmetic_expansions_with_continuations:
            "_auto_limit_amount=\"$((
  ${_available_lines:-1}                -
    ${_header_and_footer_line_count:-0} +
    ${_auto_limit_adjustment:-0}
))\"\n"
            => "_auto_limit_amount=\"$((\\\n\t${_available_lines:-1} - \\\n\t${_header_and_footer_line_count:-0} + \\\n\t${_auto_limit_adjustment:-0}))\"\n";
    }

    default_format_ast_cases! {
        preserves_continued_semicolon_terminators:
            "ln -s foo bar \\\n  ;\nrm bar\n"
            => "ln -s foo bar \\\n\t;\nrm bar\n";
        preserves_multiline_single_quoted_argument_payload_indentation:
            "cat \"$@\" |\n  python -c '\nfrom __future__ import print_function\nprint(\"ok\")\n'\n"
            => "cat \"$@\" |\n\tpython -c '\nfrom __future__ import print_function\nprint(\"ok\")\n'\n";
        preserves_multiline_assignment_payload_indentation:
            "if true; then\n  section+=\"\n$line\"\nfi\n"
            => "if true; then\n\tsection+=\"\n$line\"\nfi\n";
        removes_unquoted_assignment_continuation_backslashes:
            "packages=$one\\\n$two\\\n$three\n"
            => "packages=$one$two$three\n";
        preserves_multiline_assignment_continuation_payload_indentation:
            "if true; then\n  INCLUDE_TESTS=\"boot_services kernel \\\n                           filesystems usb \\\n                           hardening\"\nfi\n"
            => "if true; then\n\tINCLUDE_TESTS=\"boot_services kernel \\\n                           filesystems usb \\\n                           hardening\"\nfi\n";
        normalizes_multiline_command_substitution_arguments:
            "_comp_compgen_split -- \"$(\"$1\" -watchdog help 2>&1 |\n                _comp_awk '{print $1}')\"\n"
            => "_comp_compgen_split -- \"$(\"$1\" -watchdog help 2>&1 |\n\t_comp_awk '{print $1}')\"\n";
        indents_inline_command_substitution_pipeline_words:
            "f() {\n  for fl in \"$HOME/.ssh/config\" \\\n    $(grep \"^\\s*Include\" \"$HOME/.ssh/config\" |\n      awk '{for (i=2; i<=NF; i++) print $i}' |\n      sed -Ee \"s|^([^/~])|$HOME/.ssh/\\1|\"); do\n    echo \"$fl\"\n  done\n}\n"
            => "f() {\n\tfor fl in \"$HOME/.ssh/config\" \\\n\t\t$(grep \"^\\s*Include\" \"$HOME/.ssh/config\" |\n\t\t\tawk '{for (i=2; i<=NF; i++) print $i}' |\n\t\t\tsed -Ee \"s|^([^/~])|$HOME/.ssh/\\1|\"); do\n\t\techo \"$fl\"\n\tdone\n}\n";
    }

    default_unchanged_ast_cases! {
        preserves_command_substitution_padding_inside_single_quotes:
            "echo >>$TOOLS 'x=$( uptime_in_seconds )'\n";
        preserves_multiline_quoted_assignment_payloads_with_nested_expansions:
            "result_command=\"${result_command}\n\t--label \\\"manager=distrobox\\\"\n\t--env \\\"SHELL=$(basename \"${SHELL:-\"/bin/bash\"}\")\\\"\n\t--env \\\"HOME=${container_user_home}\\\"\"\n";
        preserves_inline_multiline_command_substitution_source_indentation:
            "result_command=\"${result_command}\n\t\t$(printenv | grep '=' |\n\t\tgrep -Ev '^_' |\n\t\tsed 's/x/y/')\"\n";
    }

    default_format_ast_cases! {
        normalizes_redirect_spacing_in_inline_multiline_command_substitutions:
            "binary_files=\"$(grep -rl \"# distrobox_binary\" \"${HOME}/.local/bin\" 2> /dev/null | sed 's/./\\\\&/g' |\n\txargs -I{} grep -le \"# name: ${container_name}$\" \"{}\" | sed 's/./\\\\&/g' |\n\txargs -I{} printf \"%s¤\" \"{}\" 2> /dev/null || :)\"\n"
            => "binary_files=\"$(grep -rl \"# distrobox_binary\" \"${HOME}/.local/bin\" 2>/dev/null | sed 's/./\\\\&/g' |\n\txargs -I{} grep -le \"# name: ${container_name}$\" \"{}\" | sed 's/./\\\\&/g' |\n\txargs -I{} printf \"%s¤\" \"{}\" 2>/dev/null || :)\"\n";
        trims_source_indent_from_block_command_substitutions:
            "f() {\n\tdesktop_files=$(\n\t\t# keep this with the nested command\n\t\tfind \"$dir\" -type f 2> /dev/null | sed 's/./\\\\&/g' |\n\t\t\txargs printf '%s\\n'\n\t)\n}\n"
            => "f() {\n\tdesktop_files=$(\n\t\t# keep this with the nested command\n\t\tfind \"$dir\" -type f 2>/dev/null | sed 's/./\\\\&/g' |\n\t\t\txargs printf '%s\\n'\n\t)\n}\n";
    }

    bash_format_ast_cases! {
        preserves_multiline_compound_assignment_literal_shape:
            "case $mode in\ndocs)\n  CMD=(zsh -ilsc\n    'sudo chown /src &&\n     make -C /src doc')\n  ;;\nesac\n"
            => "case $mode in\ndocs)\n\tCMD=(zsh -ilsc\n\t\t'sudo chown /src &&\n     make -C /src doc')\n\t;;\nesac\n";
        preserves_decl_multiline_compound_assignment_literal_shape:
            "f() {\n  local options=(\n    1 \"Short\"\n    \"First line\n\nliteral continuation\"\n  )\n}\n"
            => "f() {\n\tlocal options=(\n\t\t1 \"Short\"\n\t\t\"First line\n\nliteral continuation\"\n\t)\n}\n";
    }

    format_cases_with_options! {
        binary_next_line_pipeline_keeps_heredoc_body_unindented:
            ShellFormatOptions::default().with_binary_next_line(true),
            "cat foo | \\\ncat<<EOF\nhello\nEOF\n",
            => "cat foo \\\n\t| cat <<EOF\nhello\nEOF\n";
    }

    default_format_ast_cases! {
        tab_stripping_heredocs_indent_body_with_context:
            "case $mode in\nnew)\n  cat >$file <<-EOF\nbody\nEOF\n  ;;\nesac\n"
            => "case $mode in\nnew)\n\tcat >$file <<-EOF\n\t\tbody\n\tEOF\n\t;;\nesac\n";
    }

    unchanged_cases_with_options! {
        binary_next_line_does_not_force_single_line_pipelines_to_wrap:
            ShellFormatOptions::default().with_binary_next_line(true),
            "cat foo | cat bar\n";
    }

    format_cases_with_options! {
        honors_function_next_line_option:
            ShellFormatOptions::default().with_function_next_line(true),
            "foo(){\necho hi\n}\n"
            => "foo()\n{\n\techo hi\n}\n";
        minify_drops_comments_but_preserves_shebang:
            ShellFormatOptions::default().with_minify(true),
            "#!/bin/bash\necho hi # note\n",
            => "#!/bin/bash\necho hi\n";
    }

    default_format_cases! {
        formats_case_items_with_expected_indentation:
            "case $x in\na) echo a;;\nb) echo b;;\nesac\n"
            => "case $x in\na) echo a ;;\nb) echo b ;;\nesac\n";
    }

    default_format_ast_cases! {
        keeps_inline_case_inside_if_command_lists:
            "if case \"${icon_name}\" in \"/\"*) true ;; *) false ;; esac &&\n  [ -e \"${icon_name}\" ]; then\n  echo yes\nfi\n"
            => "if case \"${icon_name}\" in \"/\"*) true ;; *) false ;; esac &&\n\t[ -e \"${icon_name}\" ]; then\n\techo yes\nfi\n";
        keeps_standalone_inline_case_commands:
            "for src in $source; do\n  case \"$src\" in */*) continue ;; esac\n  echo \"$src\"\ndone\n"
            => "for src in $source; do\n\tcase \"$src\" in */*) continue ;; esac\n\techo \"$src\"\ndone\n";
        keeps_inline_case_commands_with_missing_terminators:
            "shellspec_is_number() {\n  case ${1:-} in ( '' | *[!0-9]* ) return 1; esac\n  return 0\n}\n"
            => "shellspec_is_number() {\n\tcase ${1:-} in '' | *[!0-9]*) return 1 ;; esac\n\treturn 0\n}\n";
        keeps_inline_case_arms_inside_command_substitutions:
            "value=\"$(\n  while read -r key; do\n    case \"$key\" in\n    A) echo A ;;\n    B) echo B ;;\n    esac\n  done\n)\"\n\n# later comment\nnext() { :; }\n"
            => "value=\"$(\n\twhile read -r key; do\n\t\tcase \"$key\" in\n\t\tA) echo A ;;\n\t\tB) echo B ;;\n\t\tesac\n\tdone\n)\"\n\n# later comment\nnext() { :; }\n";
        keeps_case_items_multiline_when_terminator_was_multiline:
            "case \"$x\" in\n-h|--help)  usage\n            ;;\nesac\n"
            => "case \"$x\" in\n-h | --help)\n\tusage\n\t;;\nesac\n";
        keeps_inline_case_item_when_terminator_is_missing:
            "case \"$x\" in\n*)  usage\nesac\n"
            => "case \"$x\" in\n*) usage ;;\nesac\n";
        keeps_case_header_items_inline_when_later_body_wraps:
            "case \"$mode\" in a) ;; b) ;; c)\n  echo c\nesac\n"
            => "case \"$mode\" in a) ;; b) ;; c)\n\techo c\n\t;;\nesac\n";
        keeps_missing_terminator_case_item_body_on_pattern_line:
            "case \"$x\" in\n*) value= && for item in $items; do {\n  echo \"$item\"\n} done\nesac\n"
            => "case \"$x\" in\n*) value= && for item in $items; do {\n\techo \"$item\"\n}; done ;;\nesac\n";
    }

    bash_format_ast_cases! {
        keeps_inline_case_arms_with_if_bodies:
            "case \"$name\" in\nFastfile) if [[ \"$path\" =~ /fastlane/Fastfile ]]; then\n  ruby -c \"$name\"\nfi ;;\nesac\n"
            => "case \"$name\" in\nFastfile) if [[ \"$path\" =~ /fastlane/Fastfile ]]; then\n\truby -c \"$name\"\nfi ;;\nesac\n";
        keeps_inline_case_arms_with_if_else_bodies:
            "case \"$RETROARCH\" in\n*) if [ -x /usr/share/games/retroarch ]; then\n    build_ra=yes\nelse\n    build_ra=no\nfi ;;\nesac\n"
            => "case \"$RETROARCH\" in\n*) if [ -x /usr/share/games/retroarch ]; then\n\tbuild_ra=yes\nelse\n\tbuild_ra=no\nfi ;;\nesac\n";
        splits_nested_case_body_when_outer_terminator_was_on_next_line:
            "case $x in\na) case $y in\nb) echo b ;; esac # note\n;;\nesac\n"
            => "case $x in\na)\n\tcase $y in\n\tb) echo b ;; esac # note\n\t;;\nesac\n";
        keeps_empty_case_terminators_after_pattern_suffix_comments_parseable:
            "case \"$line\" in\n\"status-filtered \"*) # ignore other status-filtered lines\n  ;;\n\"#\"*) # allow for comments\n  ;;\nesac\n"
            => "case \"$line\" in\n\"status-filtered \"*) # ignore other status-filtered lines\n\t;;\n\"#\"*) # allow for comments\n\t;;\nesac\n";
    }

    default_unchanged_ast_cases! {
        keeps_inline_case_commands_with_multiple_patterns:
            "case ${1:-} in '' | *[!0-9]*) return 1 ;; esac\n";
    }

    format_cases_with_options! {
        switch_case_indent_indents_patterns_and_bodies:
            ShellFormatOptions::default().with_switch_case_indent(true),
            "case $x in\na) echo a;;\nesac\n"
            => "case $x in\n\ta)\n\t\techo a\n\t\t;;\nesac\n";
        space_redirects_insert_spaces_between_operator_and_target:
            ShellFormatOptions::default().with_space_redirects(true),
            "echo hi>out\n"
            => "echo hi > out\n";
    }

    unchanged_cases_with_options! {
        keep_padding_preserves_safe_verbatim_regions:
            ShellFormatOptions::default().with_keep_padding(true),
            "a=1  b=2\n";
    }

    #[test]
    fn keep_padding_still_formats_unpadded_syntax() {
        let options = ShellFormatOptions::default().with_keep_padding(true);

        assert_formats_to(
            "#!/bin/bash\n echo hi\n",
            None,
            &options,
            "#!/bin/bash\necho hi\n",
        );
        assert!(!source_is_formatted("#!/bin/bash\n echo hi\n", None, &options).unwrap());
    }

    #[test]
    fn normalizes_extra_crlf_trailing_newlines() {
        let source = "echo hi\r\n\r\n";
        let options = ShellFormatOptions::default();

        assert_formats_to(source, Some(Path::new("test.sh")), &options, "echo hi\r\n");
        assert!(!source_is_formatted(source, Some(Path::new("test.sh")), &options).unwrap());
    }

    format_cases_with_options! {
        never_split_prefers_compact_layouts:
            ShellFormatOptions::default().with_never_split(true),
            "if true; then\necho hi\nfi\n",
            => "if true; then echo hi; fi\n";
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
        assert_unchanged(
            "print ${(m)foo}\n",
            Some(Path::new("script.zsh")),
            &ShellFormatOptions::default(),
        );

        assert_unchanged_default("#!/usr/bin/env zsh\nprint ${(m)foo}\n");
        assert_unchanged_default("#!/usr/bin/env -S zsh -f\nprint ${(m)foo}\n");
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

        assert_unchanged_with_ast(source, Some(Path::new("script.zsh")), &options);
    }

    unchanged_cases_with_options! {
        mksh_dialect_formats_select_commands:
            ShellFormatOptions::default().with_dialect(ShellDialect::Mksh),
            "select name in foo; do echo \"$name\"; done\n";
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
            assert_unchanged_with_ast(source, None, &options);
        }
    }

    default_unchanged_ast_cases! {
        preserves_prefix_match_selector_kind_when_formatting:
            "printf '%s\\n' \"${!prefix@}\" \"${!prefix*}\"\n";
    }

    #[test]
    fn format_file_ast_matches_format_source_for_formatter_fixtures() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle-fixtures");
        for (fixture, filename, options) in stable_formatter_fixture_cases() {
            let source = fs::read_to_string(fixture_root.join(fixture)).unwrap();
            assert_source_and_ast_paths_match(&source, Some(Path::new(filename)), &options);
        }
        let source = fs::read_to_string(fixture_root.join("binary_next_line.sh")).unwrap();
        let options = ShellFormatOptions::default().with_binary_next_line(true);
        assert_source_and_ast_paths_match(
            &source,
            Some(Path::new("binary_next_line.sh")),
            &options,
        );

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
        for (fixture, filename, options) in stable_formatter_fixture_cases() {
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

    default_idempotent_cases! {
        preserves_group_spacing_idempotently_for_nested_subshells:
            "foo(foo()) \n",
            "nested_subshell_spacing.sh";
        preserves_blank_lines_after_multiline_subshells_idempotently:
            "(\n\techo hi\n)\n\nfoo() {\n\techo bye\n}\n",
            "multiline_subshell_gap.sh";
        preserves_blank_lines_after_multiline_brace_groups_idempotently:
            "{\n\techo hi\n}\n\nfoo() {\n\techo bye\n}\n",
            "multiline_brace_gap.sh";
        preserves_spacing_after_nested_multiline_subshells_before_simple_commands:
            "(\n\t(\n\t\tfalse\n\t)\n)\ngrep \"name delta\"\n",
            "nested_multiline_subshell_then_stmt.sh";
        preserves_spacing_after_nested_multiline_subshells_before_other_groups:
            "(\n\t(\n\t\tfalse\n\t)\n)\n(\n\twhile true; do\n\t\t:\n\tdone\n)\n",
            "nested_multiline_subshell_then_group.bash";
        preserves_spacing_after_subshells_that_end_with_function_definitions:
            "foo() {\n\t(\n\t\tbar() {\n\t\t\techo hi\n\t\t}\n\t)\n\n\tprintf x\n}\n",
            "subshell_function_tail_gap.bash";
        preserves_shebang_spacing_before_nested_multiline_groups_idempotently:
            "#!/usr/bin/env bash\n\n(\n\t(\n\t\techo hi\n\t)\n)\n",
            "shebang_nested_groups.bash";
        preserves_legacy_bracket_arithmetic_idempotently:
            "#!/bin/sh\n\ni=$[$i+1]\n",
            "legacy_bracket_arithmetic.sh";
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
        assert_default_idempotent("[[ -n]]\n", "fuzz.bash");
        assert_default_idempotent("[[ ! -n]]\n", "fuzz.bash");
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
        assert_default_idempotent(source, "fuzz.sh");
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
        assert_default_idempotent(source, "fuzz.sh");
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
