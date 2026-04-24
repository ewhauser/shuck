//! Helpers for reading interpreter names from shebang lines.

use std::path::Path;

/// Returns the interpreter command name from a shebang line.
///
/// For `/usr/bin/env` shebangs, this skips `env -S` and returns the first command
/// in the split string, so `#!/usr/bin/env -S bash -e` reports `bash`.
#[must_use]
pub fn interpreter_name(line: &str) -> Option<&str> {
    let line = line.trim();
    let line = line.strip_prefix("#!")?.trim();

    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    let first_name = token_basename(first)?;
    if first_name == "env" {
        return env_interpreter_name(&mut parts);
    }

    Some(first_name)
}

fn env_interpreter_name<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut skip_next = false;
    while let Some(part) = parts.next() {
        if skip_next {
            skip_next = false;
            continue;
        }

        if part == "-S" || part == "--split-string" {
            return parts.next().and_then(token_basename);
        }

        if let Some(split_string) = part
            .strip_prefix("-S")
            .filter(|split_string| !split_string.is_empty())
        {
            return token_basename(split_string);
        }

        if let Some(split_string) = part.strip_prefix("--split-string=") {
            return split_string
                .split_whitespace()
                .next()
                .and_then(token_basename);
        }

        if looks_like_env_assignment(part) {
            continue;
        }

        if env_option_consumes_value(part) {
            skip_next = env_option_uses_separate_value(part);
            continue;
        }

        if part.starts_with('-') {
            continue;
        }

        return token_basename(part);
    }

    None
}

fn env_option_consumes_value(token: &str) -> bool {
    matches!(token, "-u" | "-C" | "--unset" | "--chdir")
        || token.starts_with("-u")
        || token.starts_with("-C")
        || token.starts_with("--unset=")
        || token.starts_with("--chdir=")
}

fn env_option_uses_separate_value(token: &str) -> bool {
    matches!(token, "-u" | "-C" | "--unset" | "--chdir")
}

fn looks_like_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn token_basename(token: &str) -> Option<&str> {
    Path::new(token).file_name()?.to_str()
}

#[cfg(test)]
mod tests {
    use super::interpreter_name;

    #[test]
    fn reads_direct_interpreter_name() {
        assert_eq!(interpreter_name("#!/bin/dash"), Some("dash"));
    }

    #[test]
    fn reads_env_interpreter_name() {
        assert_eq!(interpreter_name("#!/usr/bin/env bash"), Some("bash"));
    }

    #[test]
    fn skips_env_split_string_flag() {
        assert_eq!(interpreter_name("#!/usr/bin/env -S bash -e"), Some("bash"));
        assert_eq!(
            interpreter_name("#!/usr/bin/env -S /bin/zsh -f"),
            Some("zsh")
        );
    }

    #[test]
    fn skips_env_options_and_assignments() {
        assert_eq!(
            interpreter_name("#!/usr/bin/env -i FOO=1 -u BAR bash"),
            Some("bash")
        );
    }
}
