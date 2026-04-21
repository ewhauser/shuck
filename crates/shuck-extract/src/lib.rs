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
    let mut tokens = template.split_whitespace();
    let Some(first) = tokens.next() else {
        return ExtractedDialect::Unsupported;
    };

    let first = shell_token_basename(first);
    if first == "env" {
        for token in tokens {
            if token == "{0}" || token.starts_with('-') {
                continue;
            }
            return shell_name_dialect(&shell_token_basename(token));
        }
        return ExtractedDialect::Unsupported;
    }

    shell_name_dialect(&first)
}

fn shell_token_basename(token: &str) -> String {
    Path::new(token)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(token)
        .to_ascii_lowercase()
}

fn shell_name_dialect(name: &str) -> ExtractedDialect {
    match name {
        "bash" => ExtractedDialect::Bash,
        "sh" => ExtractedDialect::Sh,
        "pwsh" | "powershell" | "cmd" | "python" => ExtractedDialect::Unsupported,
        _ => ExtractedDialect::Unsupported,
    }
}

fn parse_template_flags(template: &str) -> ImplicitShellFlags {
    let mut errexit = false;
    let mut pipefail = false;
    let mut tokens = template.split_whitespace().peekable();
    let _ = tokens.next();

    while let Some(token) = tokens.next() {
        match token {
            "{0}" => {}
            "-e" | "--errexit" => errexit = true,
            "-o" => match tokens.next() {
                Some("errexit") => errexit = true,
                Some("pipefail") => pipefail = true,
                _ => {}
            },
            token if token.starts_with('-') && !token.starts_with("--") => {
                let flags = token.trim_start_matches('-');
                if flags.contains('e') {
                    errexit = true;
                }
                if flags.contains('o')
                    && let Some(next) = tokens.peek().copied()
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

    if node_contains_github_expression(runs_on) {
        return RunnerKind::Unknown;
    }

    RunnerKind::Unix
}

fn node_contains_runner_label(node: &Node, label: &str) -> bool {
    let label = label.to_ascii_lowercase();
    if node
        .as_scalar()
        .is_some_and(|scalar| scalar.as_str().to_ascii_lowercase().contains(&label))
    {
        return true;
    }

    node.as_sequence().is_some_and(|sequence| {
        sequence.iter().any(|item| {
            item.as_scalar()
                .is_some_and(|scalar| scalar.as_str().to_ascii_lowercase().contains(&label))
        })
    })
}

fn node_contains_github_expression(node: &Node) -> bool {
    if node
        .as_scalar()
        .is_some_and(|scalar| scalar.as_str().contains("${{"))
    {
        return true;
    }

    node.as_sequence().is_some_and(|sequence| {
        sequence.iter().any(|item| {
            item.as_scalar()
                .is_some_and(|scalar| scalar.as_str().contains("${{"))
        })
    })
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
    let host_offset = adjust_offset_to_scalar_content(host_source, start_offset, raw_source);
    let (host_start_line, host_start_column) = line_column_for_offset(host_source, host_offset);
    let (source, placeholders) = substitute_github_actions_expressions(raw_source, host_offset);

    EmbeddedScript {
        source,
        host_offset,
        host_start_line,
        host_start_column,
        dialect: shell.dialect,
        label: label.to_owned(),
        format,
        placeholders,
        implicit_flags: shell.implicit_flags,
    }
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
      - shell: /bin/sh -e {0}
        run: echo hi
"#;

        let scripts = extract_all(Path::new(".github/workflows/ci.yml"), source).unwrap();
        assert_eq!(scripts.len(), 3);
        assert_eq!(scripts[0].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[1].dialect, ExtractedDialect::Bash);
        assert_eq!(scripts[2].dialect, ExtractedDialect::Sh);
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
