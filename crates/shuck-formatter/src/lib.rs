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
    let parsed = Parser::with_dialect(source, dialect)
        .parse()
        .map_err(map_parse_error)?;

    format_file_ast(source, parsed.file, path, options)
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

    use shuck_ast::{AssignmentValue, Command};
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
        let from_ast = format_file_ast(source, parsed.file, path, options).unwrap();
        assert_eq!(from_source, from_ast);
    }

    fn assert_idempotent(source: &str, path: Option<&Path>, options: &ShellFormatOptions) {
        let once = match format_source(source, path, options).unwrap() {
            FormattedSource::Unchanged => source.to_string(),
            FormattedSource::Formatted(formatted) => formatted,
        };
        let twice = match format_source(&once, path, options).unwrap() {
            FormattedSource::Unchanged => once.clone(),
            FormattedSource::Formatted(formatted) => formatted,
        };
        assert_eq!(once, twice);
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
    fn formats_arithmetic_expansions_from_ruby_build() {
        let source = "echo $(( ver[0]*100 + ver[1] ))\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo $((ver[0] * 100 + ver[1]))\n".to_string())
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
                "for arg in \"${@:$((package_type_nargs + 1))}\"; do\n\techo \"$arg\"\ndone\n"
                    .to_string()
            )
        );
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

        for file in TEST_FILES {
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
}
