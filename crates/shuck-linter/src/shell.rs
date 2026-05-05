use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShellDialect {
    #[default]
    Unknown,
    Sh,
    Bash,
    Dash,
    Ksh,
    Mksh,
    Zsh,
}

impl ShellDialect {
    pub fn parser_dialect(self) -> shuck_parser::ShellDialect {
        match self {
            Self::Mksh => shuck_parser::ShellDialect::Mksh,
            Self::Zsh => shuck_parser::ShellDialect::Zsh,
            Self::Unknown | Self::Sh | Self::Bash | Self::Dash | Self::Ksh => {
                shuck_parser::ShellDialect::Bash
            }
        }
    }

    pub fn semantic_dialect(self) -> shuck_parser::ShellDialect {
        match self {
            Self::Sh | Self::Dash | Self::Ksh => shuck_parser::ShellDialect::Posix,
            Self::Mksh => shuck_parser::ShellDialect::Mksh,
            Self::Zsh => shuck_parser::ShellDialect::Zsh,
            Self::Unknown | Self::Bash => shuck_parser::ShellDialect::Bash,
        }
    }

    pub fn shell_profile(self) -> shuck_parser::ShellProfile {
        shuck_parser::ShellProfile::native(self.semantic_dialect())
    }

    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "sh" => Self::Sh,
            "bash" => Self::Bash,
            "dash" => Self::Dash,
            "ksh" => Self::Ksh,
            "mksh" => Self::Mksh,
            "zsh" => Self::Zsh,
            _ => Self::Unknown,
        }
    }

    pub fn infer(source: &str, path: Option<&Path>) -> Self {
        Self::infer_from_shellcheck_header(source)
            .or_else(|| Self::infer_from_shebang(source))
            .or_else(|| Self::infer_from_source_markers(source))
            .unwrap_or_else(|| {
                path.map_or(Self::Unknown, |path| {
                    match path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.to_ascii_lowercase())
                        .as_deref()
                    {
                        Some("sh") => Self::Sh,
                        Some("bash") => Self::Bash,
                        Some("dash") => Self::Dash,
                        Some("ksh") => Self::Ksh,
                        Some("mksh") => Self::Mksh,
                        Some("zsh") => Self::Zsh,
                        _ => Self::Unknown,
                    }
                })
            })
    }

    fn infer_from_shebang(source: &str) -> Option<Self> {
        let interpreter = shuck_parser::shebang::interpreter_name(source.lines().next()?)?;
        Some(Self::from_name(interpreter))
    }

    fn infer_from_shellcheck_header(source: &str) -> Option<Self> {
        for line in source.lines() {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with("#!") {
                continue;
            }

            let Some(comment) = trimmed.strip_prefix('#') else {
                break;
            };
            let body = comment.trim_start().to_ascii_lowercase();
            let Some(shell_name) = body.strip_prefix("shellcheck shell=") else {
                continue;
            };

            let dialect = Self::from_name(shell_name.split_whitespace().next().unwrap_or_default());
            if dialect != Self::Unknown {
                return Some(dialect);
            }
        }

        None
    }

    fn infer_from_source_markers(source: &str) -> Option<Self> {
        let mut saw_bash_marker = false;
        let mut saw_zsh_marker = false;
        let mut at_directive_prefix = true;
        let mut heredoc_delimiter: Option<(String, bool)> = None;

        for line in source.lines() {
            let trimmed = line.trim_start();
            if let Some((delimiter, strip_tabs)) = heredoc_delimiter.as_ref() {
                let candidate = if *strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if candidate.trim_end() == delimiter {
                    heredoc_delimiter = None;
                }
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with("#!") {
                continue;
            }
            if trimmed.starts_with('#') {
                let comment = trimmed.strip_prefix('#').unwrap_or_default();
                if at_directive_prefix
                    && (comment.starts_with("compdef") || comment.starts_with("autoload"))
                {
                    saw_zsh_marker = true;
                }
                at_directive_prefix = false;
                continue;
            }

            let code = trimmed.split('#').next().unwrap_or(trimmed);
            saw_bash_marker |= line_has_bash_marker(code);
            saw_zsh_marker |= line_has_zsh_marker(code);
            heredoc_delimiter = line_heredoc_delimiter(code);
            at_directive_prefix = false;

            if saw_bash_marker && saw_zsh_marker {
                return None;
            }
        }

        match (saw_bash_marker, saw_zsh_marker) {
            (true, false) => Some(Self::Bash),
            (false, true) => Some(Self::Zsh),
            _ => None,
        }
    }
}

fn line_has_bash_marker(line: &str) -> bool {
    contains_unquoted_parameter(line, "BASH_SOURCE")
        || contains_unquoted_parameter(line, "BASH_VERSION")
        || contains_unquoted_parameter(line, "PROMPT_COMMAND")
        || starts_with_assignment(line, "PROMPT_COMMAND")
        || starts_with_shell_word(line, "shopt")
}

fn line_has_zsh_marker(line: &str) -> bool {
    contains_unquoted_parameter(line, "ZSH_VERSION")
        || contains_unquoted_parameter(line, "ZSH_EVAL_CONTEXT")
        || starts_with_shell_word(line, "zstyle")
        || starts_with_shell_word(line, "zmodload")
        || line_has_zsh_emulate_marker(line)
        || line_has_zsh_autoload_marker(line)
        || contains_unquoted_literal(line, "${${")
        || contains_unquoted_literal(line, "${(%):-%x}")
        || contains_unquoted_literal(line, "${+commands[")
}

fn line_has_zsh_emulate_marker(line: &str) -> bool {
    let words = shell_words(line);
    words.first().is_some_and(|word| *word == "emulate")
        && words.iter().skip(1).any(|word| *word == "zsh")
}

fn line_has_zsh_autoload_marker(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("autoload ") && trimmed.split_whitespace().any(|word| word.starts_with('-'))
}

fn line_heredoc_delimiter(line: &str) -> Option<(String, bool)> {
    let redirect_start = heredoc_redirect_start(line)?;
    let mut rest = &line[redirect_start + 2..];
    let strip_tabs = rest.starts_with('-');
    if strip_tabs {
        rest = &rest[1..];
    }
    let delimiter = rest.split_whitespace().next()?;
    let delimiter = delimiter
        .trim_matches('\'')
        .trim_matches('"')
        .trim_matches('\\');
    (!delimiter.is_empty()).then(|| (delimiter.to_owned(), strip_tabs))
}

fn heredoc_redirect_start(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;
    let mut arithmetic_depth = 0usize;

    while index + 1 < bytes.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }

        let byte = bytes[index];
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }

        if arithmetic_depth > 0 {
            if bytes.get(index..index + 2) == Some(b"))") {
                arithmetic_depth -= 1;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if byte == b'\'' && !in_double_quotes {
            in_single_quotes = !in_single_quotes;
            index += 1;
            continue;
        }
        if byte == b'"' && !in_single_quotes {
            in_double_quotes = !in_double_quotes;
            index += 1;
            continue;
        }
        if in_single_quotes || in_double_quotes {
            index += 1;
            continue;
        }

        if bytes.get(index..index + 3) == Some(b"$((") {
            arithmetic_depth += 1;
            index += 3;
            continue;
        }
        if bytes.get(index..index + 2) == Some(b"((") && arithmetic_command_start(bytes, index) {
            arithmetic_depth += 1;
            index += 2;
            continue;
        }

        if bytes.get(index..index + 2) == Some(b"<<") {
            if bytes.get(index + 2) != Some(&b'<') {
                return Some(index);
            }
            index += 3;
            continue;
        }

        index += 1;
    }

    None
}

fn arithmetic_command_start(bytes: &[u8], index: usize) -> bool {
    bytes[..index]
        .iter()
        .rev()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_none_or(|byte| matches!(*byte, b';' | b'&' | b'|' | b'('))
}

fn starts_with_shell_word(line: &str, needle: &str) -> bool {
    shell_words(line)
        .first()
        .is_some_and(|word| *word == needle)
}

fn starts_with_assignment(line: &str, name: &str) -> bool {
    let Some(suffix) = line.trim_start().strip_prefix(name) else {
        return false;
    };
    suffix.starts_with('=') || suffix.starts_with("+=")
}

fn contains_unquoted_parameter(line: &str, name: &str) -> bool {
    let name_bytes = name.as_bytes();
    contains_unquoted_marker(line, |bytes, index| {
        if bytes.get(index) != Some(&b'$') {
            return false;
        }
        let braced_start = index + 2;
        let braced_end = braced_start + name_bytes.len();
        if bytes.get(index + 1) == Some(&b'{')
            && bytes
                .get(braced_start..braced_end)
                .is_some_and(|candidate| candidate == name_bytes)
            && bytes
                .get(braced_end)
                .is_none_or(|byte| !is_shell_name_byte(*byte))
        {
            return true;
        }
        let plain_start = index + 1;
        let plain_end = plain_start + name_bytes.len();
        bytes
            .get(plain_start..plain_end)
            .is_some_and(|candidate| candidate == name_bytes)
            && bytes
                .get(plain_end)
                .is_none_or(|byte| !is_shell_name_byte(*byte))
    })
}

fn contains_unquoted_literal(line: &str, literal: &str) -> bool {
    contains_unquoted_marker(line, |bytes, index| {
        bytes
            .get(index..index + literal.len())
            .is_some_and(|candidate| candidate == literal.as_bytes())
    })
}

fn contains_unquoted_marker(line: &str, mut matches_at: impl FnMut(&[u8], usize) -> bool) -> bool {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut in_single_quotes = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if byte == b'\'' {
            in_single_quotes = !in_single_quotes;
            index += 1;
            continue;
        }
        if !in_single_quotes && matches_at(bytes, index) {
            return true;
        }
        index += 1;
    }

    false
}

fn is_shell_name_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn shell_words(line: &str) -> Vec<&str> {
    line.split(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_from_shebang_before_extension() {
        let inferred = ShellDialect::infer("#!/usr/bin/env bash\nlocal foo=bar\n", None);
        assert_eq!(inferred, ShellDialect::Bash);
    }

    #[test]
    fn infers_from_env_split_shebang_before_extension() {
        let inferred = ShellDialect::infer("#!/usr/bin/env -S bash -e\nlocal foo=bar\n", None);
        assert_eq!(inferred, ShellDialect::Bash);
    }

    #[test]
    fn infers_from_extension_when_shebang_is_missing() {
        let inferred = ShellDialect::infer("local foo=bar\n", Some(Path::new("/tmp/example.bash")));
        assert_eq!(inferred, ShellDialect::Bash);
    }

    #[test]
    fn infers_zsh_from_source_markers_before_sh_extension() {
        let source = r#"
[[ -n "$ZSH" ]] || export ZSH="${${(%):-%x}:a:h}"
zstyle -s ':omz:update' mode update_mode
autoload -U compaudit compinit
"#;
        let inferred = ShellDialect::infer(source, Some(Path::new("/tmp/oh-my-zsh.sh")));
        assert_eq!(inferred, ShellDialect::Zsh);
    }

    #[test]
    fn infers_zsh_from_compdef_comment_before_sh_extension() {
        let inferred = ShellDialect::infer(
            "#compdef git\n_arguments '*:: :->args'\n",
            Some(Path::new("/tmp/_git.sh")),
        );
        assert_eq!(inferred, ShellDialect::Zsh);
    }

    #[test]
    fn ignores_late_compdef_comment_markers_after_real_content() {
        let inferred = ShellDialect::infer(
            "printf '%s\\n' ok\n#compdef git\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn ignores_free_form_comments_that_mention_zsh_directive_words() {
        let inferred = ShellDialect::infer(
            "# autoload helper cache\n# compdef examples live elsewhere\nprintf '%s\\n' ok\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn ignores_quoted_dialect_marker_names_without_shell_usage() {
        let inferred = ShellDialect::infer(
            "printf '%s\\n' \"ZSH_VERSION\" \"BASH_VERSION\" \"PROMPT_COMMAND\"\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn ignores_single_literal_dialect_marker_names_without_dollar_prefix() {
        let inferred =
            ShellDialect::infer("echo ZSH_VERSION\n", Some(Path::new("/tmp/example.sh")));
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn ignores_literal_or_escaped_dialect_parameter_markers() {
        let inferred = ShellDialect::infer(
            "printf '%s\\n' '${ZSH_VERSION}' \"\\$BASH_VERSION\" \"$ZSH_VERSIONED\"\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn ignores_literal_or_escaped_zsh_expansion_markers() {
        let inferred = ShellDialect::infer(
            "printf '%s\\n' '${${(%):-%x}:a:h}' \"\\${+commands[git]}\"\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn ignores_dialect_markers_inside_heredoc_bodies() {
        let source = "\
cat <<'EOF'
$ZSH_VERSION
${BASH_SOURCE[0]}
EOF
";
        let inferred = ShellDialect::infer(source, Some(Path::new("/tmp/example.sh")));
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn here_strings_do_not_hide_later_source_markers() {
        let source = "\
cat <<< \"$value\"
zstyle -s ':omz:update' mode update_mode
";
        let inferred = ShellDialect::infer(source, Some(Path::new("/tmp/example.sh")));
        assert_eq!(inferred, ShellDialect::Zsh);
    }

    #[test]
    fn arithmetic_shifts_do_not_hide_later_source_markers() {
        let source = "\
((x<<1))
value=$((1<<2))
zstyle -s ':omz:update' mode update_mode
";
        let inferred = ShellDialect::infer(source, Some(Path::new("/tmp/example.sh")));
        assert_eq!(inferred, ShellDialect::Zsh);
    }

    #[test]
    fn infers_from_executed_dialect_parameter_markers() {
        let zsh = ShellDialect::infer(
            "printf '%s\\n' \"$ZSH_VERSION\"\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(zsh, ShellDialect::Zsh);

        let bash = ShellDialect::infer(
            "printf '%s\\n' \"${BASH_SOURCE[0]}\"\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(bash, ShellDialect::Bash);
    }

    #[test]
    fn infers_bash_from_source_markers_before_sh_extension() {
        let source = r#"
if [[ "${BASH_SOURCE[0]}" == */* ]]; then
  shopt -s promptvars
  PROMPT_COMMAND=update_prompt
fi
"#;
        let inferred = ShellDialect::infer(source, Some(Path::new("/tmp/gitstatus.plugin.sh")));
        assert_eq!(inferred, ShellDialect::Bash);
    }

    #[test]
    fn keeps_plain_sh_extension_as_sh_without_specific_markers() {
        let inferred = ShellDialect::infer(
            "local foo=bar\n[[ -n $foo ]] && echo \"$foo\"\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn ambiguous_bash_and_zsh_markers_fall_back_to_extension() {
        let inferred = ShellDialect::infer(
            "echo \"$BASH_VERSION $ZSH_VERSION\"\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn infers_from_shellcheck_shell_directive_without_shebang() {
        let inferred = ShellDialect::infer(
            "# shellcheck shell=sh\nprintf '%s\\n' \"${!arr[*]}\"\n",
            Some(Path::new("/tmp/example")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn shellcheck_shell_directive_overrides_shebang() {
        let inferred = ShellDialect::infer(
            "#!/bin/bash\n# shellcheck shell=sh\nprintf '%s\\n' \"${!arr[*]}\"\n",
            Some(Path::new("/tmp/example.sh")),
        );
        assert_eq!(inferred, ShellDialect::Sh);
    }

    #[test]
    fn parser_dialect_matches_linter_shell_policy() {
        assert_eq!(
            ShellDialect::Unknown.parser_dialect(),
            shuck_parser::ShellDialect::Bash
        );
        assert_eq!(
            ShellDialect::Bash.parser_dialect(),
            shuck_parser::ShellDialect::Bash
        );
        assert_eq!(
            ShellDialect::Sh.parser_dialect(),
            shuck_parser::ShellDialect::Bash
        );
        assert_eq!(
            ShellDialect::Dash.parser_dialect(),
            shuck_parser::ShellDialect::Bash
        );
        assert_eq!(
            ShellDialect::Ksh.parser_dialect(),
            shuck_parser::ShellDialect::Bash
        );
        assert_eq!(
            ShellDialect::Mksh.parser_dialect(),
            shuck_parser::ShellDialect::Mksh
        );
        assert_eq!(
            ShellDialect::Zsh.parser_dialect(),
            shuck_parser::ShellDialect::Zsh
        );
    }

    #[test]
    fn semantic_dialect_matches_linter_shell_policy() {
        assert_eq!(
            ShellDialect::Unknown.semantic_dialect(),
            shuck_parser::ShellDialect::Bash
        );
        assert_eq!(
            ShellDialect::Bash.semantic_dialect(),
            shuck_parser::ShellDialect::Bash
        );
        assert_eq!(
            ShellDialect::Sh.semantic_dialect(),
            shuck_parser::ShellDialect::Posix
        );
        assert_eq!(
            ShellDialect::Dash.semantic_dialect(),
            shuck_parser::ShellDialect::Posix
        );
        assert_eq!(
            ShellDialect::Ksh.semantic_dialect(),
            shuck_parser::ShellDialect::Posix
        );
        assert_eq!(
            ShellDialect::Mksh.semantic_dialect(),
            shuck_parser::ShellDialect::Mksh
        );
        assert_eq!(
            ShellDialect::Zsh.semantic_dialect(),
            shuck_parser::ShellDialect::Zsh
        );
    }
}
