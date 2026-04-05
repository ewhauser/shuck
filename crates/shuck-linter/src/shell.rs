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
        Self::infer_from_shebang(source).unwrap_or_else(|| {
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
        let first_line = source.lines().next()?.trim();
        let line = first_line.strip_prefix("#!")?.trim();

        let mut parts = line.split_whitespace();
        let first = parts.next()?;
        let interpreter = if Path::new(first).file_name()?.to_str()? == "env" {
            parts.next()?
        } else {
            Path::new(first).file_name()?.to_str()?
        };

        Some(Self::from_name(interpreter))
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
    fn infers_from_extension_when_shebang_is_missing() {
        let inferred = ShellDialect::infer("local foo=bar\n", Some(Path::new("/tmp/example.bash")));
        assert_eq!(inferred, ShellDialect::Bash);
    }
}
