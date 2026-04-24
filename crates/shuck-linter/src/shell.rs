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
}
