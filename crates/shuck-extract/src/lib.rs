#![warn(missing_docs)]

//! Extract embedded shell scripts from non-shell host files.

use std::ops::Range;
use std::path::Path;

use anyhow::{Result, anyhow};
use marked_yaml::{Node, parse_yaml};

const GITHUB_ACTIONS_SOURCE_ID: usize = 0;

/// A shell snippet extracted from a host file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedScript {
    /// The shell source code after placeholder substitution.
    pub source: String,
    /// Byte offset of the snippet's first character within the host file.
    pub host_offset: usize,
    /// 1-based line number of the snippet's first character within the host file.
    pub host_start_line: usize,
    /// 1-based column of the snippet's first character within the host file.
    pub host_start_column: usize,
    /// Per-line host positions for decoded snippet lines.
    pub host_line_starts: Vec<HostLineStart>,
    /// Host column expansions for decoded characters that came from YAML escapes.
    pub host_column_mappings: Vec<HostColumnMapping>,
    /// The shell dialect for this snippet.
    pub dialect: ExtractedDialect,
    /// Human-readable location label inside the host file.
    pub label: String,
    /// Which embedded format produced this snippet.
    pub format: EmbeddedFormat,
    /// Placeholder mappings produced during expression substitution.
    pub placeholders: Vec<PlaceholderMapping>,
    /// Shell flags implied by the host environment.
    pub implicit_flags: ImplicitShellFlags,
}

/// Shell dialect inferred for an extracted snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractedDialect {
    /// Bash syntax.
    Bash,
    /// POSIX `sh` syntax.
    Sh,
    /// A shell that shuck does not currently lint.
    Unsupported,
}

/// Host format that produced an embedded shell snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedFormat {
    /// GitHub Actions workflows and composite actions.
    GitHubActions,
}

/// Host-file position of an extracted snippet line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostLineStart {
    /// 1-based line number in the host file.
    pub line: usize,
    /// 1-based column number in the host file.
    pub column: usize,
}

/// Host-file position where a decoded snippet segment begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostColumnMapping {
    /// 1-based decoded snippet line number.
    pub line: usize,
    /// 1-based decoded snippet column number where this host segment begins.
    pub column: usize,
    /// 1-based host-file line number for the segment start.
    pub host_line: usize,
    /// 1-based host-file column number for the segment start.
    pub host_column: usize,
}

/// Mapping between a synthetic placeholder and the original template expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderMapping {
    /// Placeholder variable name without the leading `$`.
    pub name: String,
    /// Original expression including `${{` / `}}`.
    pub original: String,
    /// Inner expression text.
    pub expression: String,
    /// Taint classification for the expression.
    pub taint: ExpressionTaint,
    /// Span of the substituted `${NAME}` text within the extracted source.
    pub substituted_span: Range<usize>,
    /// Approximate span of the original `${{ ... }}` text within the host file.
    pub host_span: Range<usize>,
}

/// Trust level for a GitHub Actions expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionTaint {
    /// Value is influenced by untrusted user input at runtime.
    UserControlled,
    /// Value contains a secret.
    Secret,
    /// Value is repository or workflow controlled.
    Trusted,
    /// Value could not be classified confidently.
    Unknown,
}

/// Shell flags injected by the host environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImplicitShellFlags {
    /// Whether `errexit` is active.
    pub errexit: bool,
    /// Whether `pipefail` is active.
    pub pipefail: bool,
    /// Effective shell template when known.
    pub template: Option<String>,
}

/// Extracts embedded shell snippets from a host file.
pub trait Extractor {
    /// Returns true when this extractor should inspect the given path.
    fn matches(&self, path: &Path) -> bool;

    /// Returns true when the given source looks like the extractor's format.
    fn probe(&self, source: &str) -> bool;

    /// Extracts embedded shell snippets from the given source.
    fn extract(&self, source: &str) -> Result<Vec<EmbeddedScript>>;
}

/// Returns true when any registered extractor can handle the path.
pub fn is_extractable(path: &Path) -> bool {
    extractors().iter().any(|extractor| extractor.matches(path))
}

/// Runs all matching extractors for a host path and source.
pub fn extract_all(path: &Path, source: &str) -> Result<Vec<EmbeddedScript>> {
    let mut scripts = Vec::new();
    for extractor in extractors() {
        if extractor.matches(path) {
            scripts.extend(extractor.extract(source)?);
        }
    }
    Ok(scripts)
}

fn extractors() -> [GitHubActionsExtractor; 1] {
    [GitHubActionsExtractor]
}

#[derive(Debug, Clone, Copy)]
struct GitHubActionsExtractor;

impl Extractor for GitHubActionsExtractor {
    fn matches(&self, path: &Path) -> bool {
        gha_path_matches(path)
    }

    fn probe(&self, source: &str) -> bool {
        parse_yaml(GITHUB_ACTIONS_SOURCE_ID, source)
            .ok()
            .and_then(|node| node.as_mapping().cloned())
            .is_some_and(|mapping| is_github_actions_mapping(&mapping))
    }

    fn extract(&self, source: &str) -> Result<Vec<EmbeddedScript>> {
        let root = parse_yaml(GITHUB_ACTIONS_SOURCE_ID, source)
            .map_err(|err| anyhow!("parse GitHub Actions YAML: {err}"))?;
        let Some(root) = root.as_mapping() else {
            return Ok(Vec::new());
        };
        if !is_github_actions_mapping(root) {
            return Ok(Vec::new());
        }

        if is_composite_action(root) {
            extract_composite_action(root, source)
        } else {
            extract_workflow(root, source)
        }
    }
}

fn gha_path_matches(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    if !matches!(extension.to_ascii_lowercase().as_str(), "yml" | "yaml") {
        return false;
    }

    if path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "action.yml" | "action.yaml"
            )
        })
    {
        return true;
    }

    let parts = path
        .iter()
        .filter_map(|part| part.to_str())
        .collect::<Vec<_>>();
    parts
        .windows(2)
        .any(|window| matches!(window, [".github", "workflows"]))
}

fn is_github_actions_mapping(root: &marked_yaml::types::MarkedMappingNode) -> bool {
    is_workflow(root) || is_composite_action(root)
}

fn is_workflow(root: &marked_yaml::types::MarkedMappingNode) -> bool {
    root.get_node("on").is_some() && root.get_mapping("jobs").is_some()
}

fn is_composite_action(root: &marked_yaml::types::MarkedMappingNode) -> bool {
    root.get_mapping("runs")
        .and_then(|runs| runs.get_scalar("using"))
        .is_some_and(|using| using.as_str().eq_ignore_ascii_case("composite"))
}

fn extract_workflow(
    root: &marked_yaml::types::MarkedMappingNode,
    host_source: &str,
) -> Result<Vec<EmbeddedScript>> {
    let mut scripts = Vec::new();
    let workflow_default_shell = nested_scalar(root, &["defaults", "run", "shell"]);
    let Some(jobs) = root.get_mapping("jobs") else {
        return Ok(scripts);
    };

    for (job_name, job_node) in jobs.iter() {
        let Some(job) = job_node.as_mapping() else {
            continue;
        };
        let job_default_shell = nested_scalar(job, &["defaults", "run", "shell"])
            .or_else(|| workflow_default_shell.clone());
        let runner_kind = runner_kind(job.get_node("runs-on"));
        let Some(steps) = job.get_sequence("steps") else {
            continue;
        };

        for (index, step_node) in steps.iter().enumerate() {
            let Some(step) = step_node.as_mapping() else {
                continue;
            };
            let Some(run) = step.get_scalar("run") else {
                continue;
            };

            let shell = step
                .get_scalar("shell")
                .map(|scalar| scalar.as_str().to_owned())
                .or_else(|| job_default_shell.clone());
            let label = format!("jobs.{}.steps[{index}].run", job_name.as_str());
            scripts.push(build_embedded_script(
                run,
                host_source,
                &label,
                EmbeddedFormat::GitHubActions,
                resolve_shell(shell.as_deref(), runner_kind),
            ));
        }
    }

    Ok(scripts)
}

fn extract_composite_action(
    root: &marked_yaml::types::MarkedMappingNode,
    host_source: &str,
) -> Result<Vec<EmbeddedScript>> {
    let mut scripts = Vec::new();
    let Some(steps) = root
        .get_mapping("runs")
        .and_then(|runs| runs.get_sequence("steps"))
    else {
        return Ok(scripts);
    };

    for (index, step_node) in steps.iter().enumerate() {
        let Some(step) = step_node.as_mapping() else {
            continue;
        };
        let Some(run) = step.get_scalar("run") else {
            continue;
        };
        let shell = step
            .get_scalar("shell")
            .map(|scalar| scalar.as_str().to_owned());
        let label = format!("runs.steps[{index}].run");
        scripts.push(build_embedded_script(
            run,
            host_source,
            &label,
            EmbeddedFormat::GitHubActions,
            resolve_shell(shell.as_deref(), RunnerKind::Unix),
        ));
    }

    Ok(scripts)
}

fn nested_scalar(mapping: &marked_yaml::types::MarkedMappingNode, path: &[&str]) -> Option<String> {
    let (last, parents) = path.split_last()?;
    let mut current = mapping;
    for segment in parents {
        current = current.get_mapping(segment)?;
    }
    current
        .get_scalar(last)
        .map(|scalar| scalar.as_str().to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellResolution {
    dialect: ExtractedDialect,
    implicit_flags: ImplicitShellFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunnerKind {
    Unix,
    Windows,
    Unknown,
}

fn resolve_shell(shell: Option<&str>, runner_kind: RunnerKind) -> ShellResolution {
    match shell.map(str::trim).filter(|value| !value.is_empty()) {
        None => match runner_kind {
            RunnerKind::Unix => ShellResolution {
                dialect: ExtractedDialect::Bash,
                implicit_flags: ImplicitShellFlags {
                    errexit: true,
                    pipefail: true,
                    template: Some("bash --noprofile --norc -eo pipefail {0}".to_owned()),
                },
            },
            RunnerKind::Windows => ShellResolution {
                dialect: ExtractedDialect::Unsupported,
                implicit_flags: ImplicitShellFlags::default(),
            },
            RunnerKind::Unknown => ShellResolution {
                dialect: ExtractedDialect::Unsupported,
                implicit_flags: ImplicitShellFlags::default(),
            },
        },
        Some("bash") => ShellResolution {
            dialect: ExtractedDialect::Bash,
            implicit_flags: ImplicitShellFlags {
                errexit: true,
                pipefail: true,
                template: Some("bash --noprofile --norc -eo pipefail {0}".to_owned()),
            },
        },
        Some("sh") => ShellResolution {
            dialect: ExtractedDialect::Sh,
            implicit_flags: ImplicitShellFlags {
                errexit: true,
                pipefail: false,
                template: Some("sh -e {0}".to_owned()),
            },
        },
        Some(value) => ShellResolution {
            dialect: detect_shell_dialect(value),
            implicit_flags: parse_template_flags(value),
        },
    }
}

fn detect_shell_dialect(template: &str) -> ExtractedDialect {
    let mut tokens = template_tokens(template).into_iter();
    let Some(first) = tokens.next() else {
        return ExtractedDialect::Unsupported;
    };

    let first = shell_token_basename(&first);
    if first == "env" {
        let mut skip_next = false;
        for token in tokens {
            if skip_next {
                skip_next = false;
                continue;
            }
            if token == "{0}" || looks_like_env_assignment(&token) {
                continue;
            }
            if env_option_consumes_value(&token) {
                skip_next = env_option_uses_separate_value(&token);
                continue;
            }
            if token.starts_with('-') {
                continue;
            }
            return shell_name_dialect(&shell_token_basename(&token));
        }
        return ExtractedDialect::Unsupported;
    }

    shell_name_dialect(&first)
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

fn template_tokens(template: &str) -> Vec<String> {
    #[derive(Clone, Copy)]
    enum QuoteState {
        Unquoted,
        SingleQuoted,
        DoubleQuoted,
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = template.chars();
    let mut state = QuoteState::Unquoted;

    while let Some(ch) = chars.next() {
        match state {
            QuoteState::Unquoted => match ch {
                '\'' => state = QuoteState::SingleQuoted,
                '"' => state = QuoteState::DoubleQuoted,
                '\\' => match chars.next() {
                    Some(escaped)
                        if escaped.is_whitespace() || matches!(escaped, '"' | '\'' | '\\') =>
                    {
                        current.push(escaped);
                    }
                    Some(escaped) => {
                        current.push(ch);
                        current.push(escaped);
                    }
                    None => current.push(ch),
                },
                ch if ch.is_whitespace() => push_template_token(&mut tokens, &mut current),
                _ => current.push(ch),
            },
            QuoteState::SingleQuoted => {
                if ch == '\'' {
                    state = QuoteState::Unquoted;
                } else {
                    current.push(ch);
                }
            }
            QuoteState::DoubleQuoted => match ch {
                '"' => state = QuoteState::Unquoted,
                '\\' => match chars.next() {
                    Some(escaped) if matches!(escaped, '"' | '\\' | '$' | '`') => {
                        current.push(escaped);
                    }
                    Some(escaped) => {
                        current.push(ch);
                        current.push(escaped);
                    }
                    None => current.push(ch),
                },
                _ => current.push(ch),
            },
        }
    }

    push_template_token(&mut tokens, &mut current);
    tokens
}

fn push_template_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn shell_token_basename(token: &str) -> String {
    let basename = token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
        .to_ascii_lowercase();

    basename
        .strip_suffix(".exe")
        .or_else(|| basename.strip_suffix(".cmd"))
        .or_else(|| basename.strip_suffix(".bat"))
        .unwrap_or(&basename)
        .to_owned()
}

fn shell_name_dialect(name: &str) -> ExtractedDialect {
    match name {
        "bash" => ExtractedDialect::Bash,
        "sh" => ExtractedDialect::Sh,
        "pwsh" | "powershell" | "cmd" | "python" => ExtractedDialect::Unsupported,
        _ => ExtractedDialect::Unsupported,
    }
}

fn looks_like_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };

    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, 'A'..='Z' | 'a'..='z' | '_') {
        return false;
    }

    chars.all(|ch| matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_'))
}

fn parse_template_flags(template: &str) -> ImplicitShellFlags {
    let mut errexit = false;
    let mut pipefail = false;
    let mut tokens = template_tokens(template).into_iter().peekable();
    let _ = tokens.next();

    while let Some(token) = tokens.next() {
        match token.as_str() {
            "{0}" => {}
            "-e" | "--errexit" => errexit = true,
            "-o" => match tokens.next() {
                Some(value) if value == "errexit" => errexit = true,
                Some(value) if value == "pipefail" => pipefail = true,
                _ => {}
            },
            token if token.starts_with('-') && !token.starts_with("--") => {
                let flags = token.trim_start_matches('-');
                if flags.contains('e') {
                    errexit = true;
                }
                if flags.contains('o')
                    && let Some(next) = tokens.peek().map(String::as_str)
                {
                    match next {
                        "errexit" => {
                            errexit = true;
                            let _ = tokens.next();
                        }
                        "pipefail" => {
                            pipefail = true;
                            let _ = tokens.next();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    ImplicitShellFlags {
        errexit,
        pipefail,
        template: Some(template.to_owned()),
    }
}

fn runner_kind(runs_on: Option<&Node>) -> RunnerKind {
    let Some(runs_on) = runs_on else {
        return RunnerKind::Unix;
    };

    if node_contains_runner_label(runs_on, "windows") {
        return RunnerKind::Windows;
    }

    if node_contains_unix_runner_label(runs_on) {
        return RunnerKind::Unix;
    }

    if node_contains_github_expression(runs_on) {
        return RunnerKind::Unknown;
    }

    RunnerKind::Unknown
}

fn node_contains_runner_label(node: &Node, label: &str) -> bool {
    if node
        .as_scalar()
        .is_some_and(|scalar| scalar_matches_runner_label(scalar.as_str(), label))
    {
        return true;
    }

    node.as_sequence().is_some_and(|sequence| {
        sequence
            .iter()
            .any(|item| node_contains_runner_label(item, label))
    }) || node
        .as_mapping()
        .and_then(|mapping| mapping.get_node("labels"))
        .is_some_and(|labels| node_contains_runner_label(labels, label))
}

fn scalar_matches_runner_label(scalar: &str, label: &str) -> bool {
    let scalar = scalar.trim().to_ascii_lowercase();
    match label {
        "windows" => {
            scalar == "windows"
                || scalar.strip_prefix("windows-").is_some_and(|suffix| {
                    suffix == "latest"
                        || suffix.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                })
        }
        "ubuntu" => {
            scalar == "ubuntu"
                || scalar.strip_prefix("ubuntu-").is_some_and(|suffix| {
                    suffix == "latest"
                        || suffix == "slim"
                        || suffix.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                })
        }
        "macos" => {
            scalar == "macos"
                || scalar.strip_prefix("macos-").is_some_and(|suffix| {
                    suffix == "latest"
                        || suffix.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                })
        }
        "linux" => scalar == "linux",
        _ => scalar == label,
    }
}

fn node_contains_github_expression(node: &Node) -> bool {
    if node
        .as_scalar()
        .is_some_and(|scalar| scalar.as_str().contains("${{"))
    {
        return true;
    }

    if node
        .as_sequence()
        .is_some_and(|sequence| sequence.iter().any(node_contains_github_expression))
    {
        return true;
    }

    node.as_mapping().is_some_and(|mapping| {
        mapping
            .iter()
            .any(|(_, value)| node_contains_github_expression(value))
    })
}

fn node_contains_unix_runner_label(node: &Node) -> bool {
    ["ubuntu", "linux", "macos"]
        .into_iter()
        .any(|label| node_contains_runner_label(node, label))
}

fn build_embedded_script(
    run: &marked_yaml::types::MarkedScalarNode,
    host_source: &str,
    label: &str,
    format: EmbeddedFormat,
    shell: ShellResolution,
) -> EmbeddedScript {
    let raw_source = run.as_str();
    let marker = run.span().start().copied();
    let start_offset = marker
        .map(|marker| byte_offset_for_line_column(host_source, marker.line(), marker.column()))
        .unwrap_or_default();
    let source_mapping = source_mapping_for_scalar(host_source, start_offset, raw_source);
    let host_offset = source_mapping.host_offset;
    let host_start_line = source_mapping.host_line_starts[0].line;
    let host_start_column = source_mapping.host_line_starts[0].column;
    let (source, placeholders) = substitute_github_actions_expressions(raw_source, host_offset);

    EmbeddedScript {
        source,
        host_offset,
        host_start_line,
        host_start_column,
        host_line_starts: source_mapping.host_line_starts,
        host_column_mappings: source_mapping.host_column_mappings,
        dialect: shell.dialect,
        label: label.to_owned(),
        format,
        placeholders,
        implicit_flags: shell.implicit_flags,
    }
}

struct SourceMapping {
    host_offset: usize,
    host_line_starts: Vec<HostLineStart>,
    host_column_mappings: Vec<HostColumnMapping>,
}

fn source_mapping_for_scalar(source: &str, start_offset: usize, scalar: &str) -> SourceMapping {
    if let Some(mapping) = double_quoted_source_mapping(source, start_offset, scalar) {
        return mapping;
    }

    if let Some(mapping) = folded_block_source_mapping(source, start_offset, scalar) {
        return mapping;
    }

    let host_offset = adjust_offset_to_scalar_content(source, start_offset, scalar);
    let (host_start_line, host_start_column) = line_column_for_offset(source, host_offset);
    SourceMapping {
        host_offset,
        host_line_starts: default_host_line_starts(host_start_line, host_start_column, scalar),
        host_column_mappings: Vec::new(),
    }
}

fn double_quoted_source_mapping(
    source: &str,
    start_offset: usize,
    scalar: &str,
) -> Option<SourceMapping> {
    if source.get(start_offset..)?.chars().next()? != '"' {
        return None;
    }

    let content_offset = start_offset + '"'.len_utf8();
    let mut host_line_starts = vec![{
        let (line, column) = line_column_for_offset(source, content_offset);
        HostLineStart { line, column }
    }];
    let mut host_column_mappings = Vec::new();
    let expected_line_count = decoded_line_count(scalar);
    let mut decoded_line = 1usize;
    let mut decoded_column = 1usize;
    let mut decoded_offset = 0usize;
    let mut relative_offset = 0usize;
    let content = &source[content_offset..];

    while relative_offset < content.len() {
        let absolute_offset = content_offset + relative_offset;
        let ch = source[absolute_offset..].chars().next()?;
        match ch {
            '\\' => {
                let escape = parse_double_quoted_yaml_escape(source, absolute_offset)?;
                let decoded_char = consume_decoded_char(
                    scalar,
                    &mut decoded_offset,
                    &mut decoded_line,
                    &mut decoded_column,
                )?;
                let (line, column) =
                    line_column_for_offset(source, absolute_offset + escape.host_columns);
                if decoded_char == '\n' {
                    host_line_starts.push(HostLineStart { line, column });
                } else {
                    if escape.host_columns > 1 && decoded_offset < scalar.len() {
                        push_host_column_mapping(
                            &mut host_column_mappings,
                            HostColumnMapping {
                                line: decoded_line,
                                column: decoded_column,
                                host_line: line,
                                host_column: column,
                            },
                        );
                    }
                }
                relative_offset += escape.host_columns;
            }
            '"' => {
                if host_line_starts.len() == expected_line_count {
                    return Some(SourceMapping {
                        host_offset: content_offset,
                        host_line_starts,
                        host_column_mappings,
                    });
                }
                return None;
            }
            '\n' => {
                let folded =
                    scan_double_quoted_physical_newline(source, content_offset, relative_offset)?;
                let next_relative_offset = folded.next_relative_offset;
                consume_double_quoted_fold(
                    scalar,
                    &mut decoded_offset,
                    &mut decoded_line,
                    &mut decoded_column,
                    &mut host_line_starts,
                    &mut host_column_mappings,
                    folded,
                )?;
                relative_offset = next_relative_offset;
            }
            _ => {
                consume_decoded_char(
                    scalar,
                    &mut decoded_offset,
                    &mut decoded_line,
                    &mut decoded_column,
                )?;
                relative_offset += ch.len_utf8();
            }
        }
    }

    None
}

fn push_host_column_mapping(
    host_column_mappings: &mut Vec<HostColumnMapping>,
    mapping: HostColumnMapping,
) {
    if host_column_mappings.last().copied() == Some(mapping) {
        return;
    }
    host_column_mappings.push(mapping);
}

fn consume_decoded_char(
    scalar: &str,
    decoded_offset: &mut usize,
    decoded_line: &mut usize,
    decoded_column: &mut usize,
) -> Option<char> {
    let ch = scalar.get(*decoded_offset..)?.chars().next()?;
    *decoded_offset += ch.len_utf8();
    if ch == '\n' {
        *decoded_line += 1;
        *decoded_column = 1;
    } else {
        *decoded_column += 1;
    }
    Some(ch)
}

struct DoubleQuotedPhysicalNewline {
    next_relative_offset: usize,
    continuation: HostLineStart,
    continuation_char: char,
    blank_lines: Vec<usize>,
}

fn scan_double_quoted_physical_newline(
    source: &str,
    content_offset: usize,
    relative_offset: usize,
) -> Option<DoubleQuotedPhysicalNewline> {
    let mut scan_offset = relative_offset;
    let mut blank_lines = Vec::new();

    loop {
        let absolute_offset = content_offset + scan_offset;
        if source.get(absolute_offset..)?.chars().next()? != '\n' {
            return None;
        }
        scan_offset += '\n'.len_utf8();

        let line_start = content_offset + scan_offset;
        let mut content_start = line_start;
        while matches!(
            source.get(content_start..)?.chars().next(),
            Some(' ' | '\t')
        ) {
            content_start += 1;
        }

        let continuation_char = source.get(content_start..)?.chars().next()?;
        if continuation_char == '\n' {
            let (line, _) = line_column_for_offset(source, line_start);
            blank_lines.push(line);
            scan_offset = content_start - content_offset;
            continue;
        }

        let (line, column) = line_column_for_offset(source, content_start);
        return Some(DoubleQuotedPhysicalNewline {
            next_relative_offset: content_start - content_offset,
            continuation: HostLineStart { line, column },
            continuation_char,
            blank_lines,
        });
    }
}

fn consume_double_quoted_fold(
    scalar: &str,
    decoded_offset: &mut usize,
    decoded_line: &mut usize,
    decoded_column: &mut usize,
    host_line_starts: &mut Vec<HostLineStart>,
    host_column_mappings: &mut Vec<HostColumnMapping>,
    folded: DoubleQuotedPhysicalNewline,
) -> Option<()> {
    let mut folded_output = Vec::new();

    while let Some(ch) = scalar.get(*decoded_offset..)?.chars().next() {
        if ch == folded.continuation_char && folded.continuation_char != '"' {
            break;
        }
        if folded.continuation_char == '"' && *decoded_offset == scalar.len() {
            break;
        }
        if !matches!(ch, ' ' | '\n') {
            return None;
        }
        folded_output.push(ch);
        *decoded_offset += ch.len_utf8();
    }

    let newline_count = folded_output.iter().filter(|&&ch| ch == '\n').count();
    let mut newline_index = 0usize;

    for ch in folded_output {
        match ch {
            ' ' => {
                *decoded_column += 1;
            }
            '\n' => {
                newline_index += 1;
                let host_line_start = if newline_index == newline_count {
                    folded.continuation
                } else {
                    HostLineStart {
                        line: folded
                            .blank_lines
                            .get(newline_index.saturating_sub(1))
                            .copied()
                            .unwrap_or(folded.continuation.line),
                        column: folded.continuation.column,
                    }
                };
                host_line_starts.push(host_line_start);
                *decoded_line += 1;
                *decoded_column = 1;
            }
            _ => unreachable!(),
        }
    }

    if newline_count == 0 && folded.continuation_char != '"' && *decoded_offset < scalar.len() {
        push_host_column_mapping(
            host_column_mappings,
            HostColumnMapping {
                line: *decoded_line,
                column: *decoded_column,
                host_line: folded.continuation.line,
                host_column: folded.continuation.column,
            },
        );
    }

    Some(())
}

fn folded_block_source_mapping(
    source: &str,
    start_offset: usize,
    scalar: &str,
) -> Option<SourceMapping> {
    let host_offset = adjust_offset_to_scalar_content(source, start_offset, scalar);
    let (host_start_line, host_start_column) = line_column_for_offset(source, host_offset);
    let content_line_start = line_start_offset(source, host_offset);
    let header_line = previous_line_text(source, content_line_start)?;
    if !header_line_is_folded_block(header_line) {
        return None;
    }

    let content_indent = host_start_column.saturating_sub(1);
    let expected_line_count = decoded_line_count(scalar);
    let mut host_line_starts = vec![HostLineStart {
        line: host_start_line,
        column: host_start_column,
    }];
    let mut current_line_start = content_line_start;
    let mut current_line_number = host_start_line;
    let mut previous_nonblank = classify_block_scalar_line(
        source,
        current_line_start,
        current_line_number,
        content_indent,
    )?;
    let mut pending_blank_lines = Vec::new();

    while let Some(next_line_start) = next_line_start_offset(source, current_line_start) {
        current_line_number += 1;
        let next_line = classify_block_scalar_line(
            source,
            next_line_start,
            current_line_number,
            content_indent,
        )?;
        if next_line.ends_block {
            break;
        }
        current_line_start = next_line_start;

        if next_line.is_blank {
            pending_blank_lines.push(next_line.line);
            continue;
        }

        if !pending_blank_lines.is_empty() {
            for blank_line in pending_blank_lines
                .iter()
                .copied()
                .take(pending_blank_lines.len().saturating_sub(1))
            {
                host_line_starts.push(HostLineStart {
                    line: blank_line,
                    column: host_start_column,
                });
            }
            host_line_starts.push(HostLineStart {
                line: next_line.line,
                column: host_start_column,
            });
            pending_blank_lines.clear();
        } else if previous_nonblank.is_more_indented || next_line.is_more_indented {
            host_line_starts.push(HostLineStart {
                line: next_line.line,
                column: host_start_column,
            });
        }

        previous_nonblank = next_line;

        if host_line_starts.len() >= expected_line_count {
            break;
        }
    }

    while host_line_starts.len() < expected_line_count {
        let previous = host_line_starts.last().copied().unwrap_or(HostLineStart {
            line: host_start_line,
            column: host_start_column,
        });
        host_line_starts.push(HostLineStart {
            line: previous.line + 1,
            column: host_start_column,
        });
    }

    Some(SourceMapping {
        host_offset,
        host_line_starts,
        host_column_mappings: Vec::new(),
    })
}

#[derive(Clone, Copy)]
struct BlockScalarLine {
    line: usize,
    is_blank: bool,
    is_more_indented: bool,
    ends_block: bool,
}

fn classify_block_scalar_line(
    source: &str,
    line_start: usize,
    line_number: usize,
    content_indent: usize,
) -> Option<BlockScalarLine> {
    let line_end = source[line_start..]
        .find('\n')
        .map(|relative| line_start + relative)
        .unwrap_or(source.len());
    let line = source.get(line_start..line_end)?.trim_end_matches('\r');
    let indent = line.chars().take_while(|&ch| ch == ' ').count();
    let is_blank = line.trim().is_empty();
    Some(BlockScalarLine {
        line: line_number,
        is_blank,
        is_more_indented: !is_blank && indent > content_indent,
        ends_block: !is_blank && indent < content_indent,
    })
}

fn line_start_offset(source: &str, offset: usize) -> usize {
    source[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn previous_line_text(source: &str, line_start: usize) -> Option<&str> {
    if line_start == 0 {
        return None;
    }

    let previous_line_end = line_start - 1;
    let previous_line_start = source[..previous_line_end]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    source
        .get(previous_line_start..previous_line_end)
        .map(|line| line.trim_end_matches('\r'))
}

fn next_line_start_offset(source: &str, line_start: usize) -> Option<usize> {
    source[line_start..]
        .find('\n')
        .map(|relative| line_start + relative + 1)
        .filter(|offset| *offset <= source.len())
}

fn header_line_is_folded_block(line: &str) -> bool {
    line.rsplit_once(':')
        .map(|(_, value)| value.trim_start().starts_with('>'))
        .unwrap_or(false)
}

struct ParsedYamlEscape {
    host_columns: usize,
}

fn parse_double_quoted_yaml_escape(source: &str, offset: usize) -> Option<ParsedYamlEscape> {
    debug_assert_eq!(source[offset..].chars().next(), Some('\\'));
    let escape = source[offset + '\\'.len_utf8()..].chars().next()?;
    let host_columns = match escape {
        'x' => '\\'.len_utf8() + escape.len_utf8() + fixed_hex_escape_len(source, offset, 2)?,
        'u' => '\\'.len_utf8() + escape.len_utf8() + fixed_hex_escape_len(source, offset, 4)?,
        'U' => '\\'.len_utf8() + escape.len_utf8() + fixed_hex_escape_len(source, offset, 8)?,
        _ => '\\'.len_utf8() + escape.len_utf8(),
    };

    Some(ParsedYamlEscape { host_columns })
}

fn fixed_hex_escape_len(source: &str, offset: usize, digits: usize) -> Option<usize> {
    let start = offset + '\\'.len_utf8() + 1;
    let end = start + digits;
    source
        .get(start..end)?
        .chars()
        .all(|ch| ch.is_ascii_hexdigit())
        .then_some(digits)
}

fn default_host_line_starts(
    host_start_line: usize,
    host_start_column: usize,
    source: &str,
) -> Vec<HostLineStart> {
    let mut line_starts = vec![HostLineStart {
        line: host_start_line,
        column: host_start_column,
    }];

    let line_count = decoded_line_count(source);
    while line_starts.len() < line_count {
        let previous = line_starts.last().copied().unwrap_or(HostLineStart {
            line: host_start_line,
            column: host_start_column,
        });
        line_starts.push(HostLineStart {
            line: previous.line + 1,
            column: host_start_column,
        });
    }

    line_starts
}

fn decoded_line_count(source: &str) -> usize {
    source.chars().filter(|&ch| ch == '\n').count() + 1
}

fn adjust_offset_to_scalar_content(source: &str, offset: usize, scalar: &str) -> usize {
    if scalar.is_empty() || offset >= source.len() {
        return offset.min(source.len());
    }
    if source[offset..].starts_with(scalar) {
        return offset;
    }

    let probe_len = scalar
        .char_indices()
        .nth(16)
        .map(|(index, _)| index)
        .unwrap_or(scalar.len());
    let probe = &scalar[..probe_len];
    let search_end = source.len().min(offset.saturating_add(512));
    source[offset..search_end]
        .find(probe)
        .map(|relative| offset + relative)
        .unwrap_or(offset)
}

fn substitute_github_actions_expressions(
    source: &str,
    host_offset: usize,
) -> (String, Vec<PlaceholderMapping>) {
    let mut output = String::with_capacity(source.len());
    let mut placeholders = Vec::new();
    let mut cursor = 0usize;
    let mut counter = 1usize;

    while let Some(start_relative) = source[cursor..].find("${{") {
        let start = cursor + start_relative;
        output.push_str(&source[cursor..start]);
        let expression_start = start + 3;
        let Some(end) = find_github_actions_expression_end(source, expression_start) else {
            output.push_str(&source[start..]);
            cursor = source.len();
            break;
        };
        let original = &source[start..end];
        let expression = original
            .trim_start_matches("${{")
            .trim_end_matches("}}")
            .trim()
            .to_owned();
        let name = format!("_SHUCK_GHA_{counter}");
        let replacement = format!("${{{name}}}");
        let substituted_start = output.len();
        output.push_str(&replacement);
        let substituted_end = output.len();
        placeholders.push(PlaceholderMapping {
            name,
            original: original.to_owned(),
            expression: expression.clone(),
            taint: classify_expression_taint(&expression),
            substituted_span: substituted_start..substituted_end,
            host_span: host_offset + start..host_offset + end,
        });
        counter += 1;
        cursor = end;
    }

    if cursor < source.len() {
        output.push_str(&source[cursor..]);
    }

    (output, placeholders)
}

fn find_github_actions_expression_end(source: &str, expression_start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = expression_start;
    let mut in_single_quoted_string = false;

    while index + 1 < bytes.len() {
        if in_single_quoted_string {
            if bytes[index] == b'\'' {
                // GitHub Actions expressions escape `'` inside string literals as `''`.
                if index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                    index += 2;
                    continue;
                }
                in_single_quoted_string = false;
            }
            index += 1;
            continue;
        }

        if bytes[index] == b'\'' {
            in_single_quoted_string = true;
            index += 1;
            continue;
        }

        if bytes[index] == b'}' && bytes[index + 1] == b'}' {
            return Some(index + 2);
        }

        index += 1;
    }

    None
}

fn classify_expression_taint(expression: &str) -> ExpressionTaint {
    let expression = expression.trim().to_ascii_lowercase();
    if expression.starts_with("secrets.") || expression == "github.token" {
        return ExpressionTaint::Secret;
    }
    if expression == "github.head_ref"
        || matches!(
            expression.as_str(),
            "github.event.issue.title"
                | "github.event.issue.body"
                | "github.event.pull_request.title"
                | "github.event.pull_request.body"
                | "github.event.pull_request.head.ref"
                | "github.event.comment.body"
                | "github.event.review.body"
                | "github.event.discussion.title"
                | "github.event.discussion.body"
        )
        || (expression.starts_with("github.event.pages.") && expression.ends_with(".page_name"))
        || (expression.starts_with("github.event.commits.")
            && (expression.ends_with(".message")
                || expression.ends_with(".author.name")
                || expression.ends_with(".author.email")))
    {
        return ExpressionTaint::UserControlled;
    }
    if matches!(
        expression.as_str(),
        "github.repository"
            | "github.sha"
            | "github.ref"
            | "github.run_id"
            | "runner.os"
            | "runner.arch"
    ) || ["env.", "vars.", "matrix.", "needs.", "steps."]
        .iter()
        .any(|prefix| expression.starts_with(prefix))
    {
        return ExpressionTaint::Trusted;
    }
    if expression.starts_with("inputs.") || expression.contains('(') {
        return ExpressionTaint::Unknown;
    }

    ExpressionTaint::Unknown
}

fn byte_offset_for_line_column(source: &str, target_line: usize, target_column: usize) -> usize {
    if target_line <= 1 && target_column <= 1 {
        return 0;
    }

    let mut line = 1usize;
    let mut column = 1usize;
    for (offset, ch) in source.char_indices() {
        if line == target_line && column == target_column {
            return offset;
        }

        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    source.len()
}

fn line_column_for_offset(source: &str, target_offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;
    for (offset, ch) in source.char_indices() {
        if offset >= target_offset {
            break;
        }

        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_github_actions_paths() {
        assert!(is_extractable(Path::new(".github/workflows/ci.yml")));
        assert!(is_extractable(Path::new("action.yaml")));
        assert!(!is_extractable(Path::new("ci.yml")));
        assert!(!is_extractable(Path::new("script.sh")));
    }

    #[test]
    fn probes_workflows_and_composite_actions() {
        let extractor = GitHubActionsExtractor;
        assert!(
            extractor
                .probe("on: push\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps: []\n")
        );
        assert!(extractor.probe("name: test\nruns:\n  using: composite\n  steps: []\n"));
        assert!(!extractor.probe("name: config\nservices:\n  db: {}\n"));
    }

    #[test]
    fn extracts_workflow_steps_with_shell_hierarchy_and_placeholders() {
        let source = r#"
on: push
defaults:
  run:
    shell: sh
jobs:
  build:
    runs-on: ubuntu-latest
    defaults:
      run:
        shell: bash {0}
    steps:
      - run: echo ${{ github.event.pull_request.title }}
      - shell: sh
        run: echo hi
      - shell: pwsh
        run: Write-Host hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 3);

        assert_eq!(scripts[0].label, "jobs.build.steps[0].run");
        assert_eq!(scripts[0].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[0].source, "echo ${_SHUCK_GHA_1}");
        assert_eq!(scripts[0].placeholders.len(), 1);
        assert_eq!(
            scripts[0].placeholders[0].taint,
            ExpressionTaint::UserControlled
        );
        assert!(!scripts[0].implicit_flags.errexit);
        assert!(!scripts[0].implicit_flags.pipefail);

        assert_eq!(scripts[1].dialect, ExtractedDialect::Sh);
        assert!(scripts[1].implicit_flags.errexit);
        assert!(!scripts[1].implicit_flags.pipefail);

        assert_eq!(scripts[2].dialect, ExtractedDialect::Unsupported);
    }

    #[test]
    fn uses_default_shell_for_windows_and_unix_runners() {
        let source = r#"
on: push
jobs:
  unix:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
  windows:
    runs-on: windows-latest
    steps:
      - run: echo hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[0].dialect, ExtractedDialect::Bash);
        assert!(scripts[0].implicit_flags.errexit);
        assert!(scripts[0].implicit_flags.pipefail);
        assert_eq!(scripts[1].dialect, ExtractedDialect::Unsupported);
    }

    #[test]
    fn skips_default_shell_when_runner_is_dynamic() {
        let source = r#"
on: push
jobs:
  dynamic:
    runs-on: ${{ matrix.os }}
    steps:
      - run: echo hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].dialect, ExtractedDialect::Unsupported);
    }

    #[test]
    fn infers_default_shell_when_fixed_runner_labels_mix_with_expressions() {
        let source = r#"
on: push
jobs:
  build:
    runs-on:
      - ubuntu-latest
      - ${{ matrix.arch }}
    steps:
      - run: echo hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].dialect, ExtractedDialect::Bash);
    }

    #[test]
    fn prefers_unix_runner_when_custom_self_hosted_label_mentions_windows() {
        let source = r#"
on: push
jobs:
  build:
    runs-on:
      - self-hosted
      - linux
      - windows-tools
    steps:
      - run: echo hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].dialect, ExtractedDialect::Bash);
        assert!(scripts[0].implicit_flags.errexit);
        assert!(scripts[0].implicit_flags.pipefail);
    }

    #[test]
    fn recognizes_path_and_env_shell_templates() {
        let source = r#"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - shell: /bin/bash -e {0}
        run: echo hi
      - shell: /usr/bin/env bash -e {0}
        run: echo hi
      - shell: /usr/bin/env FOO=1 bash -e {0}
        run: echo hi
      - shell: /usr/bin/env -u FOO bash -e {0}
        run: echo hi
      - shell: /bin/sh -e {0}
        run: echo hi
      - shell: '"C:/Program Files/Git/bin/bash.exe" -e {0}'
        run: echo hi
      - shell: '"C:\Program Files\Git\bin\bash.exe" -e {0}'
        run: echo hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 7);
        assert_eq!(scripts[0].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[1].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[2].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[3].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[4].dialect, ExtractedDialect::Sh);
        assert_eq!(scripts[5].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[6].dialect, ExtractedDialect::Bash);
    }

    #[test]
    fn skips_default_shell_for_ambiguous_runner_labels() {
        let source = r#"
on: push
jobs:
  self_hosted:
    runs-on: self-hosted
    steps:
      - run: echo hi
  labeled:
    runs-on: [self-hosted, x64]
    steps:
      - run: echo hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[0].dialect, ExtractedDialect::Unsupported);
        assert_eq!(scripts[1].dialect, ExtractedDialect::Unsupported);
    }

    #[test]
    fn infers_default_shell_from_mapping_form_runner_labels() {
        let source = r#"
on: push
jobs:
  labeled:
    runs-on:
      group: hosted
      labels: ubuntu-latest
    steps:
      - run: echo hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].dialect, ExtractedDialect::Bash);
        assert!(scripts[0].implicit_flags.errexit);
        assert!(scripts[0].implicit_flags.pipefail);
    }

    #[test]
    fn extracts_composite_action_steps() {
        let source = r#"
name: demo
runs:
  using: composite
  steps:
    - run: |
        echo hi
        echo "${{ github.sha }}"
"#;

        let scripts = extract_all(Path::new("action.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].label, "runs.steps[0].run");
        assert_eq!(scripts[0].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[0].host_start_line, 7);
        assert_eq!(scripts[0].host_start_column, 9);
        assert_eq!(scripts[0].source, "echo hi\necho \"${_SHUCK_GHA_1}\"\n");
        assert_eq!(scripts[0].placeholders[0].taint, ExpressionTaint::Trusted);
    }

    #[test]
    fn preserves_host_line_starts_for_escaped_double_quoted_runs() {
        let source = r#"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: "echo hi\nif true\nfi"
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].source, "echo hi\nif true\nfi");
        assert_eq!(
            scripts[0].host_line_starts,
            vec![
                HostLineStart {
                    line: 7,
                    column: 15,
                },
                HostLineStart {
                    line: 7,
                    column: 24,
                },
                HostLineStart {
                    line: 7,
                    column: 33,
                },
            ]
        );
    }

    #[test]
    fn preserves_host_columns_for_non_newline_escaped_double_quoted_runs() {
        let source = r#"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: "echo\t\"hi\""
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].source, "echo\t\"hi\"");
        assert_eq!(
            scripts[0].host_column_mappings,
            vec![
                HostColumnMapping {
                    line: 1,
                    column: 6,
                    host_line: 7,
                    host_column: 21,
                },
                HostColumnMapping {
                    line: 1,
                    column: 7,
                    host_line: 7,
                    host_column: 23,
                },
            ]
        );
    }

    #[test]
    fn remaps_folded_double_quoted_runs_onto_later_host_lines() {
        let source = r#"on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: "echo ok
          ; unused=1"
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].source, "echo ok ; unused=1");
        assert_eq!(
            scripts[0].host_column_mappings,
            vec![HostColumnMapping {
                line: 1,
                column: 9,
                host_line: 7,
                host_column: 11,
            }]
        );
    }

    #[test]
    fn preserves_host_line_gaps_for_folded_block_runs() {
        let source = r#"on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: >
          if true

          then
            echo hi
          fi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].source, "if true\nthen\n  echo hi\nfi\n");
        assert_eq!(
            scripts[0].host_line_starts,
            vec![
                HostLineStart {
                    line: 7,
                    column: 11
                },
                HostLineStart {
                    line: 9,
                    column: 11
                },
                HostLineStart {
                    line: 10,
                    column: 11,
                },
                HostLineStart {
                    line: 11,
                    column: 11,
                },
                HostLineStart {
                    line: 12,
                    column: 11,
                },
            ]
        );
    }

    #[test]
    fn wraps_placeholder_expansions_to_preserve_identifier_boundaries() {
        let source = r#"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github.ref }}suffix
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].source, "echo ${_SHUCK_GHA_1}suffix");
        assert_eq!(scripts[0].placeholders[0].substituted_span, 5..20);
    }

    #[test]
    fn keeps_double_closing_braces_inside_expression_string_literals() {
        let source = r#"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ format('}}', github.ref) }}
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].source, "echo ${_SHUCK_GHA_1}");
        assert_eq!(scripts[0].placeholders.len(), 1);
        assert_eq!(
            scripts[0].placeholders[0].expression,
            "format('}}', github.ref)"
        );
    }

    #[test]
    fn classifies_taint_patterns() {
        assert_eq!(
            classify_expression_taint("github.event.comment.body"),
            ExpressionTaint::UserControlled
        );
        assert_eq!(
            classify_expression_taint("secrets.API_KEY"),
            ExpressionTaint::Secret
        );
        assert_eq!(
            classify_expression_taint("matrix.os"),
            ExpressionTaint::Trusted
        );
        assert_eq!(
            classify_expression_taint("format('{0}', github.ref)"),
            ExpressionTaint::Unknown
        );
    }
}
