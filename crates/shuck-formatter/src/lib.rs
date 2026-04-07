mod command;
mod comments;
mod context;
mod generated;
mod options;
mod prelude;
mod redirect;
mod script;
mod shared_traits;
mod simplify;
mod word;

use std::path::Path;

use shuck_ast::{Comment, Script};
use shuck_format::{FormatResult, format};
use shuck_parser::{Error as ParseError, parser::Parser};

use crate::shared_traits::AsFormat;

pub use crate::options::{ResolvedShellFormatOptions, ShellDialect, ShellFormatOptions};
pub use shuck_format::IndentStyle;

pub type ShellFormatter<'source, 'buf> =
    shuck_format::Formatter<context::ShellFormatContext<'source>>;

pub trait FormatNodeRule<N> {
    fn fmt(&self, node: &N, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormattedSource {
    Unchanged,
    Formatted(String),
}

impl FormattedSource {
    #[must_use]
    pub fn is_changed(&self) -> bool {
        matches!(self, Self::Formatted(_))
    }
}

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

pub type Result<T> = std::result::Result<T, FormatError>;

pub fn format_source(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    let dialect = options.resolve(source, path).dialect();
    let parsed = Parser::with_dialect(source, dialect)
        .parse()
        .map_err(map_parse_error)?;

    format_script_ast(source, &parsed.script, &parsed.comments, path, options)
}

pub fn format_script_ast(
    source: &str,
    script: &Script,
    comments: &[Comment],
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    let resolved = options.resolve(source, path);
    if resolved.simplify() || resolved.minify() {
        let mut rewritten = script.clone();
        let simplify_report = simplify::simplify_script(&mut rewritten, source);
        debug_assert!(simplify_report.total_changes() >= simplify_report.applied().len());
        return format_script(source, &rewritten, comments, resolved);
    }

    format_script(source, script, comments, resolved)
}

fn format_script(
    source: &str,
    script: &Script,
    comment_ranges: &[Comment],
    resolved: ResolvedShellFormatOptions,
) -> Result<FormattedSource> {
    let comments = comments::Comments::from_ast(source, comment_ranges);
    let context = context::ShellFormatContext::new(resolved, source, comments);
    let formatted = format!(context, [script.format()])
        .map_err(|error| FormatError::Internal(error.to_string()))?;
    let mut output = formatted
        .print()
        .map_err(|error| FormatError::Internal(error.to_string()))?
        .into_code();
    ensure_single_trailing_newline(&mut output);

    if output == source {
        Ok(FormattedSource::Unchanged)
    } else {
        Ok(FormattedSource::Formatted(output))
    }
}

#[cfg(feature = "benchmarking")]
#[doc(hidden)]
#[must_use]
pub fn build_comment_index(source: &str, comments: &[Comment]) -> usize {
    comments::Comments::from_ast(source, comments).len()
}

fn ensure_single_trailing_newline(output: &mut String) {
    while output.ends_with("\n\n") {
        output.pop();
    }
    if !output.ends_with('\n') {
        output.push('\n');
    }
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

    use shuck_benchmark::TEST_FILES;

    use super::*;

    fn parse_for_ast_format(
        source: &str,
        path: Option<&Path>,
        options: &ShellFormatOptions,
    ) -> shuck_parser::parser::ParseOutput {
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
        let from_ast =
            format_script_ast(source, &parsed.script, &parsed.comments, path, options).unwrap();
        assert_eq!(from_source, from_ast);
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
    }

    #[test]
    fn preserves_inline_comments() {
        let formatted =
            format_source("echo hi    # note\n", None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo hi  # note\n".to_string())
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
    fn minify_drops_comments() {
        let options = ShellFormatOptions::default().with_minify(true);
        let formatted = format_source("#!/bin/bash\necho hi # note\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo hi\n".to_string())
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
    fn format_script_ast_matches_format_source_for_benchmark_corpus() {
        let options = ShellFormatOptions::default();

        for file in TEST_FILES {
            let filename = std::format!("{}.bash", file.name);
            assert_source_and_ast_paths_match(file.source, Some(Path::new(&filename)), &options);
        }
    }

    #[test]
    fn format_script_ast_matches_format_source_for_formatter_fixtures() {
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
}
