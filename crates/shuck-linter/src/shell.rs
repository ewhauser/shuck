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

        for line in source.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                let comment = trimmed.trim_start_matches('#').trim_start();
                if comment.starts_with("compdef") || comment.starts_with("autoload") {
                    saw_zsh_marker = true;
                }
                continue;
            }

            let code = trimmed.split('#').next().unwrap_or(trimmed);
            saw_bash_marker |= line_has_bash_marker(code);
            saw_zsh_marker |= line_has_zsh_marker(code);

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
    contains_shell_word(line, "BASH_SOURCE")
        || contains_shell_word(line, "BASH_VERSION")
        || contains_shell_word(line, "PROMPT_COMMAND")
        || starts_with_shell_word(line, "shopt")
}

fn line_has_zsh_marker(line: &str) -> bool {
    contains_shell_word(line, "ZSH_VERSION")
        || contains_shell_word(line, "ZSH_EVAL_CONTEXT")
        || starts_with_shell_word(line, "zstyle")
        || starts_with_shell_word(line, "zmodload")
        || line_has_zsh_emulate_marker(line)
        || line_has_zsh_autoload_marker(line)
        || line.contains("${${")
        || line.contains("${(%):-%x}")
        || line.contains("${+commands[")
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

fn starts_with_shell_word(line: &str, needle: &str) -> bool {
    shell_words(line)
        .first()
        .is_some_and(|word| *word == needle)
}

fn contains_shell_word(line: &str, needle: &str) -> bool {
    shell_words(line).iter().any(|word| *word == needle)
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
