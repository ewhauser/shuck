//! Cached source and path signals shared by ambient contract providers.
//!
//! Ambient providers are intentionally heuristic: they answer questions such as
//! "does this zsh-shaped file mention `$history`?" or "does this config file
//! assign `POWERLEVEL9K_...` names?" The old provider code answered those
//! questions with repeated raw `contains()` and line scans. `AmbientSignals`
//! centralizes the one-file facts that are cheap to compute once and expensive
//! to rediscover for each provider.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use super::source_scan::{code_before_shell_comment, parse_shell_name_at};

pub(super) struct AmbientSignals<'a> {
    source: SourceSignals<'a>,
    path: Option<PathSignals>,
}

impl<'a> AmbientSignals<'a> {
    pub(super) fn new(source: &'a str, path: Option<&Path>) -> Self {
        Self {
            source: SourceSignals::new(source),
            path: path.map(PathSignals::new),
        }
    }

    pub(super) fn source(&self) -> &SourceSignals<'a> {
        &self.source
    }

    pub(super) fn path(&self) -> Option<&PathSignals> {
        self.path.as_ref()
    }
}

pub(super) struct PathSignals {
    path: PathBuf,
}

impl PathSignals {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

pub(super) struct SourceSignals<'a> {
    source: &'a str,
    mentioned_names: BTreeSet<String>,
    assigned_names: BTreeSet<String>,
    code_lines: Vec<&'a str>,
    zmodload_lines: Vec<&'a str>,
    has_probable_function_definition: bool,
    has_source_command: bool,
    loads_zsh_colors: bool,
}

impl<'a> SourceSignals<'a> {
    fn new(source: &'a str) -> Self {
        let mut code_lines = Vec::new();
        let mut zmodload_lines = Vec::new();
        let mut has_probable_function_definition = false;
        let mut has_source_command = false;
        let mut loads_zsh_colors = false;

        for line in source.lines() {
            let code = code_before_shell_comment(line);
            let trimmed = code.trim();
            let trim_start = code.trim_start();
            code_lines.push(trimmed);
            has_probable_function_definition |= probable_function_definition(trimmed);
            has_source_command |= source_command_line(trimmed);
            loads_zsh_colors |= line_autoloads_zsh_colors(trim_start);
            if code.contains("zmodload") {
                zmodload_lines.push(code);
            }
        }

        Self {
            source,
            mentioned_names: collect_parameter_mentions(source),
            assigned_names: collect_assignment_names(source),
            code_lines,
            zmodload_lines,
            has_probable_function_definition,
            has_source_command,
            loads_zsh_colors,
        }
    }

    pub(super) fn contains(&self, pattern: &str) -> bool {
        self.source.contains(pattern)
    }

    pub(super) fn mentions_name(&self, name: &str) -> bool {
        self.mentioned_names.contains(name)
    }

    pub(super) fn assigns_name(&self, name: &str) -> bool {
        self.assigned_names.contains(name)
    }

    pub(super) fn assigns_name_with_prefix(&self, prefix: &str) -> bool {
        self.assigned_names
            .iter()
            .any(|name| name.starts_with(prefix))
    }

    pub(super) fn has_probable_function_definition(&self) -> bool {
        self.has_probable_function_definition
    }

    pub(super) fn has_source_command(&self) -> bool {
        self.has_source_command
    }

    pub(super) fn loads_zsh_module(&self, module: &str) -> bool {
        self.zmodload_lines.iter().any(|code| code.contains(module))
    }

    pub(super) fn loads_zsh_colors(&self) -> bool {
        self.loads_zsh_colors
    }

    pub(super) fn static_assignment_value(&self, name: &str) -> Option<String> {
        for code in &self.code_lines {
            let Some(rest) = code.strip_prefix(name) else {
                continue;
            };
            let rest = rest.trim_start();
            let Some(value) = rest.strip_prefix('=') else {
                continue;
            };
            let value = value.trim_start();
            if value.is_empty() {
                continue;
            }

            let (raw_value, quoted) = if let Some(rest) = value.strip_prefix('"') {
                (rest.split('"').next()?, true)
            } else if let Some(rest) = value.strip_prefix('\'') {
                (rest.split('\'').next()?, true)
            } else {
                (
                    value
                        .split(|ch: char| ch.is_whitespace() || ch == ';')
                        .next()?,
                    false,
                )
            };

            if raw_value.is_empty() || raw_value.contains('$') {
                continue;
            }
            if quoted || is_shell_variable_name(raw_value) {
                return Some(raw_value.to_owned());
            }
        }

        None
    }

    pub(super) fn defines_function(&self, name: &str) -> bool {
        self.code_lines.iter().enumerate().any(|(index, line)| {
            let Some(candidate) = source_function_definition_candidate(line, name) else {
                return false;
            };

            function_definition_rest_opens_body(candidate.rest)
                || (candidate.allows_next_line_body
                    && self
                        .code_lines
                        .iter()
                        .skip(index + 1)
                        .find(|next| !next.is_empty())
                        .is_some_and(|next| next.starts_with('{')))
        })
    }
}

fn collect_parameter_mentions(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let bytes = source.as_bytes();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find('$') {
        let dollar = cursor + relative;
        let after_dollar = dollar + 1;
        if bytes.get(after_dollar) == Some(&b'{') {
            let name_start = after_dollar + 1;
            if let Some((name, after_name)) = parse_shell_name_at(source, name_start) {
                if matches!(bytes.get(after_name), Some(b'}' | b'[' | b':')) {
                    names.insert(name.to_owned());
                }
                cursor = after_name;
            } else {
                cursor = name_start;
            }
        } else if let Some((name, after_name)) = parse_shell_name_at(source, after_dollar) {
            insert_unbraced_name_prefixes(name, &mut names);
            cursor = after_name;
        } else {
            cursor = after_dollar;
        }
    }
    names
}

fn insert_unbraced_name_prefixes(name: &str, names: &mut BTreeSet<String>) {
    for (offset, ch) in name.char_indices() {
        let end = if offset == 0 {
            ch.len_utf8()
        } else {
            offset + ch.len_utf8()
        };
        names.insert(name[..end].to_owned());
    }
}

fn collect_assignment_names(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (offset, ch) in source.char_indices() {
        if !(ch == '_' || ch.is_ascii_alphabetic()) {
            continue;
        }
        if source[..offset]
            .chars()
            .next_back()
            .is_some_and(is_shell_name_char)
        {
            continue;
        }
        let Some((name, after_name)) = parse_shell_name_at(source, offset) else {
            continue;
        };
        let after = source[after_name..].chars().next();
        let assignment_like = matches!(after, Some('=') | Some('['))
            || (after == Some('+') && source[after_name..].chars().nth(1) == Some('='));
        if assignment_like {
            names.insert(name.to_owned());
        }
    }
    names
}

fn probable_function_definition(trimmed: &str) -> bool {
    if trimmed.starts_with('#') || trimmed.is_empty() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix("function ") {
        return rest.contains('{');
    }

    trimmed.contains("() {") || trimmed.contains("(){")
}

fn source_command_line(trimmed: &str) -> bool {
    trimmed.starts_with("source ")
        || trimmed.starts_with(". ")
        || trimmed.starts_with("\\source ")
        || trimmed.starts_with("\\. ")
}

fn line_autoloads_zsh_colors(code: &str) -> bool {
    let code = first_shell_command_segment(code);
    let mut words = code.split_whitespace();
    let mut command = words.next();
    while matches!(command, Some("builtin" | "command")) {
        command = words.next();
    }
    if command != Some("autoload") {
        return false;
    }

    words.any(|word| word == "colors")
}

fn first_shell_command_segment(code: &str) -> &str {
    ["&&", "||", ";", "|"]
        .iter()
        .filter_map(|separator| code.find(separator))
        .min()
        .map_or(code, |index| &code[..index])
}

struct FunctionDefinitionCandidate<'a> {
    rest: &'a str,
    allows_next_line_body: bool,
}

fn source_function_definition_candidate<'a>(
    source: &'a str,
    name: &str,
) -> Option<FunctionDefinitionCandidate<'a>> {
    let (source, has_function_keyword) = source
        .strip_prefix("function ")
        .map_or((source, false), |rest| (rest, true));
    let (candidate, after_name) = parse_shell_name_at(source, 0)?;
    if candidate != name {
        return None;
    }

    let rest = source[after_name..].trim();
    Some(FunctionDefinitionCandidate {
        rest,
        allows_next_line_body: (has_function_keyword && rest.is_empty()) || rest == "()",
    })
}

fn function_definition_rest_opens_body(rest: &str) -> bool {
    rest.starts_with('{') || (rest.starts_with("()") && rest.contains('{'))
}

fn is_shell_variable_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_shell_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::SourceSignals;

    #[test]
    fn unbraced_mentions_preserve_prefix_matching() {
        let signals = SourceSignals::new("printf '%s\\n' \"$fg_bold\"\n");

        assert!(signals.mentions_name("fg"));
        assert!(signals.mentions_name("fg_bold"));
    }

    #[test]
    fn braced_mentions_remain_exact() {
        let signals = SourceSignals::new("printf '%s\\n' \"${fg_bold}\"\n");

        assert!(!signals.mentions_name("fg"));
        assert!(signals.mentions_name("fg_bold"));
    }

    #[test]
    fn assignment_signals_match_shell_name_boundaries() {
        let signals =
            SourceSignals::new("XHISTFILE=bad\n$HISTFILE=odd-but-old-shape\nHISTSIZE+=1\n");

        assert!(!signals.assigns_name("XHIST"));
        assert!(signals.assigns_name("HISTFILE"));
        assert!(signals.assigns_name("HISTSIZE"));
    }

    #[test]
    fn assignment_signals_match_prefix_queries() {
        let signals = SourceSignals::new("VCS_STATUS_RESULT=ok\nVCS_STATUS_HAS_STAGED=1\n");

        assert!(signals.assigns_name_with_prefix("VCS_STATUS_"));
        assert!(!signals.assigns_name_with_prefix("P9K_"));
    }

    #[test]
    fn zmodload_signal_ignores_separator_comments() {
        let signals = SourceSignals::new("true;# zmodload zsh/parameter\n");

        assert!(!signals.loads_zsh_module("zsh/parameter"));
    }

    #[test]
    fn zmodload_signal_keeps_executed_segments_after_separators() {
        let signals = SourceSignals::new("true; zmodload zsh/parameter\n");

        assert!(signals.loads_zsh_module("zsh/parameter"));
    }
}
