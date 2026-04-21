#![recursion_limit = "256"]

mod ast_format;
mod command;
mod comments;
mod context;
mod facts;
mod generated;
mod options;
mod prelude;
mod redirect;
mod script;
mod shared_traits;
mod simplify;
mod streaming;
mod word;

use std::path::Path;

use shuck_ast::File;
use shuck_format::FormatResult;
use shuck_parser::{Error as ParseError, parser::Parser};

#[cfg(feature = "benchmarking")]
use crate::facts::FormatterFacts;

pub use crate::options::{ResolvedShellFormatOptions, ShellDialect, ShellFormatOptions};
pub use shuck_format::IndentStyle;

pub type ShellFormatter<'source, 'buf> =
    shuck_format::Formatter<context::ShellFormatContext<'source>>;

pub(crate) trait FormatNodeRule<N> {
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

impl From<shuck_format::FormatError> for FormatError {
    fn from(error: shuck_format::FormatError) -> Self {
        Self::Internal(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, FormatError>;

pub fn format_source(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    let dialect = options.resolve(source, path).dialect();
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
    let dialect = options.resolve(source, path).dialect();
    let parsed = Parser::with_dialect(source, dialect).parse();
    if parsed.is_err() {
        return Err(map_parse_error(parsed.strict_error()));
    }

    let resolved = options.resolve(source, path);
    check_file(source, parsed.file, resolved)
}

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
    mut file: File,
    resolved: ResolvedShellFormatOptions,
) -> Result<FormattedSource> {
    if resolved.keep_padding() {
        return Ok(FormattedSource::Unchanged);
    }

    if resolved.simplify() || resolved.minify() {
        simplify::simplify_file(&mut file, source);
    }

    let mut output = streaming::format_file_streaming(source, &file, &resolved)?;
    ensure_single_trailing_newline(&mut output);

    if output == source {
        Ok(FormattedSource::Unchanged)
    } else {
        Ok(FormattedSource::Formatted(output))
    }
}

fn check_file(source: &str, mut file: File, resolved: ResolvedShellFormatOptions) -> Result<bool> {
    if resolved.keep_padding() {
        return Ok(true);
    }

    if resolved.simplify() || resolved.minify() {
        simplify::simplify_file(&mut file, source);
    }

    streaming::format_file_streaming_matches_source(source, &file, &resolved)
}

#[cfg(feature = "benchmarking")]
#[doc(hidden)]
#[must_use]
pub fn build_formatter_facts(source: &str, file: &File) -> usize {
    let resolved = ShellFormatOptions::default().resolve(source, None);
    FormatterFacts::build(source, file, &resolved).len()
}

fn ensure_single_trailing_newline(output: &mut String) {
    while output.ends_with("\n\n") {
        output.pop();
    }
    if !output.ends_with('\n') {
        if trailing_backslash_count(output) % 2 == 1 {
            output.push('\\');
        }
        output.push('\n');
    }
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
    use shuck_benchmark::TEST_FILES;
    use shuck_indexer::Indexer;
    use shuck_linter::{Diagnostic, LinterSettings, lint_file_at_path_with_parse_result};
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
        let indexer = Indexer::new(source, &parse_result);
        let settings = LinterSettings::default().with_analyzed_paths([path.to_path_buf()]);
        lint_file_at_path_with_parse_result(
            &parse_result,
            source,
            &indexer,
            &settings,
            None,
            Some(path),
        )
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
    fn command_substitutions_with_heredocs_fall_back_to_raw_source() {
        let source = "result=$(cat <<EOF\nhello\nEOF\n)\n";
        let options = ShellFormatOptions::default();

        assert_eq!(
            format_source(source, None, &options).unwrap(),
            FormattedSource::Unchanged
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
    fn standalone_brace_groups_do_not_consume_later_file_comments() {
        let source = "[ -n \"$x\" ] && {\nset -x\n}\n# later\nnext\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "[ -n \"$x\" ] && {\n\tset -x\n}\n\n# later\nnext\n".to_string()
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
    fn format_file_ast_matches_format_source_for_benchmark_corpus() {
        let options = ShellFormatOptions::default();

        for file in TEST_FILES.iter() {
            let filename = std::format!("{}.bash", file.name);
            assert_source_and_ast_paths_match(file.source, Some(Path::new(&filename)), &options);
        }
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
