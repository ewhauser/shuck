use std::sync::Arc;

use anyhow::Result;
use shuck_extract::{EmbeddedScript, ExtractedDialect, HostLineStart, extract_all};
use shuck_linter::{AmbientShellOptions, LinterSettings, Rule, ShellCheckCodeMap, ShellDialect};
use shuck_parser::{Error as ParseError, parser::Parser};

use super::analyze::{FileCheckResult, collect_lint_diagnostics, read_shared_source};
use super::cache::CheckCacheData;
use super::display::display_parse_error;
use crate::commands::check_output::{
    DisplayPosition, DisplaySpan, DisplayedDiagnostic, DisplayedDiagnosticKind,
};
use crate::commands::project_runner::PendingProjectFile;

pub(super) fn analyze_embedded_file(
    pending: PendingProjectFile,
    base_linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    include_source: bool,
) -> Result<FileCheckResult> {
    let host_source = read_shared_source(&pending.file.absolute_path)?;
    let host_display_source = include_source.then_some(host_source.clone());
    let extracted = match extract_all(&pending.file.absolute_path, host_source.as_ref()) {
        Ok(extracted) => extracted,
        Err(err) => {
            let diagnostics = vec![display_parse_error(
                &pending.file.display_path,
                &pending.file.relative_path,
                &pending.file.absolute_path,
                1,
                1,
                err.to_string(),
                host_display_source,
            )];
            return Ok(FileCheckResult {
                file: pending.file,
                file_key: pending.file_key,
                cache_data: CheckCacheData::from_displayed(&diagnostics, true),
                diagnostics,
                fixes_applied: 0,
                parse_failed: true,
            });
        }
    };

    let mut displayed = Vec::new();
    let mut parse_failed = false;

    for embedded in extracted.into_iter().filter(embedded_supported_dialect) {
        let Some((shell_dialect, parse_dialect)) = embedded_dialects(embedded.dialect) else {
            continue;
        };

        let snippet_source: Arc<str> = Arc::from(embedded.source.clone());
        let parse_result = Parser::with_dialect(&snippet_source, parse_dialect).parse();
        let snippet_parse_failed = parse_result.is_err();
        parse_failed |= snippet_parse_failed;
        let linter_settings = base_linter_settings
            .clone()
            .with_shell(shell_dialect)
            .with_ambient_shell_options(AmbientShellOptions {
                errexit: embedded.implicit_flags.errexit,
                pipefail: embedded.implicit_flags.pipefail,
            });
        let diagnostics = collect_lint_diagnostics(
            &pending,
            &snippet_source,
            &parse_result,
            &linter_settings,
            shellcheck_map,
            &pending.file.absolute_path,
        )
        .into_iter()
        .filter(|diagnostic| embedded_rule_allowed(diagnostic.rule))
        .collect::<Vec<_>>();

        if snippet_parse_failed && diagnostics.is_empty() {
            let ParseError::Parse {
                message,
                line,
                column,
            } = parse_result.strict_error();
            displayed.push(remap_embedded_parse_error(
                &pending,
                &embedded,
                line,
                column,
                prefixed_embedded_message(&embedded, &message),
                host_display_source.clone(),
            ));
            continue;
        }

        displayed.extend(remap_embedded_lint_diagnostics(
            &pending,
            &embedded,
            &diagnostics,
            host_display_source.clone(),
        ));
    }

    Ok(FileCheckResult {
        file: pending.file,
        file_key: pending.file_key,
        cache_data: CheckCacheData::from_displayed(&displayed, parse_failed),
        diagnostics: displayed,
        fixes_applied: 0,
        parse_failed,
    })
}
fn embedded_supported_dialect(embedded: &EmbeddedScript) -> bool {
    !matches!(embedded.dialect, ExtractedDialect::Unsupported)
}

fn embedded_dialects(
    dialect: ExtractedDialect,
) -> Option<(ShellDialect, shuck_parser::ShellDialect)> {
    match dialect {
        ExtractedDialect::Bash => Some((ShellDialect::Bash, shuck_parser::ShellDialect::Bash)),
        ExtractedDialect::Sh => Some((ShellDialect::Sh, shuck_parser::ShellDialect::Posix)),
        ExtractedDialect::Unsupported => None,
    }
}

fn embedded_rule_allowed(rule: Rule) -> bool {
    !matches!(
        rule,
        Rule::NonAbsoluteShebang
            | Rule::IndentedShebang
            | Rule::SpaceAfterHashBang
            | Rule::ShebangNotOnFirstLine
            | Rule::MissingShebangLine
            | Rule::DuplicateShebangFlag
            | Rule::DynamicSourcePath
            | Rule::UntrackedSourceFile
    )
}

fn remap_embedded_lint_diagnostics(
    pending: &PendingProjectFile,
    embedded: &EmbeddedScript,
    diagnostics: &[shuck_linter::Diagnostic],
    source: Option<Arc<str>>,
) -> Vec<DisplayedDiagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| DisplayedDiagnostic {
            path: pending.file.display_path.clone(),
            relative_path: pending.file.relative_path.clone(),
            absolute_path: pending.file.absolute_path.clone(),
            span: remap_embedded_span(
                embedded,
                diagnostic.span.start.line,
                diagnostic.span.start.column,
                diagnostic.span.end.line,
                diagnostic.span.end.column,
            ),
            message: prefixed_embedded_message(embedded, &diagnostic.message),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code().to_owned(),
                severity: diagnostic.severity.as_str().to_owned(),
            },
            fix: None,
            source: source.clone(),
        })
        .collect()
}

fn remap_embedded_parse_error(
    pending: &PendingProjectFile,
    embedded: &EmbeddedScript,
    line: usize,
    column: usize,
    message: String,
    source: Option<Arc<str>>,
) -> DisplayedDiagnostic {
    let position = remap_embedded_position(embedded, line, column);
    DisplayedDiagnostic {
        path: pending.file.display_path.clone(),
        relative_path: pending.file.relative_path.clone(),
        absolute_path: pending.file.absolute_path.clone(),
        span: DisplaySpan::point(position.line, position.column),
        message,
        kind: DisplayedDiagnosticKind::ParseError,
        fix: None,
        source,
    }
}

fn remap_embedded_span(
    embedded: &EmbeddedScript,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
) -> DisplaySpan {
    DisplaySpan::new(
        remap_embedded_position(embedded, start_line, start_column),
        remap_embedded_position(embedded, end_line, end_column),
    )
}

pub(super) fn remap_embedded_position(
    embedded: &EmbeddedScript,
    line: usize,
    column: usize,
) -> DisplayPosition {
    let snippet_line = line.max(1);
    let host_line_start = embedded
        .host_line_starts
        .get(snippet_line.saturating_sub(1))
        .copied()
        .unwrap_or(HostLineStart {
            line: embedded.host_start_line + snippet_line.saturating_sub(1),
            column: embedded.host_start_column,
        });
    remap_embedded_column(embedded, snippet_line, host_line_start, column)
}

fn remap_embedded_column(
    embedded: &EmbeddedScript,
    snippet_line: usize,
    host_line_start: HostLineStart,
    snippet_column: usize,
) -> DisplayPosition {
    let decoded_column = remap_placeholder_column(embedded, snippet_line, snippet_column);
    remap_decoded_yaml_column(embedded, snippet_line, host_line_start, decoded_column)
}

fn remap_placeholder_column(
    embedded: &EmbeddedScript,
    snippet_line: usize,
    snippet_column: usize,
) -> usize {
    let local_column = snippet_column.saturating_sub(1);
    let mut cumulative_delta = 0isize;

    for placeholder in &embedded.placeholders {
        let (placeholder_line, placeholder_column) =
            source_line_column_for_offset(&embedded.source, placeholder.substituted_span.start);
        if placeholder_line != snippet_line {
            continue;
        }

        let substituted_start = placeholder_column.saturating_sub(1);
        let substituted_len = span_char_len(&embedded.source, &placeholder.substituted_span);
        let substituted_end = substituted_start + substituted_len;
        let host_len = placeholder.original.chars().count();

        if local_column >= substituted_end {
            cumulative_delta += host_len as isize - substituted_len as isize;
            continue;
        }

        if local_column >= substituted_start {
            let decoded_start = substituted_start as isize + cumulative_delta;
            let relative = local_column - substituted_start;
            let mapped = decoded_start + relative.min(host_len.saturating_sub(1)) as isize;
            return mapped.max(0) as usize + 1;
        }
    }

    (local_column as isize + cumulative_delta).max(0) as usize + 1
}

fn remap_decoded_yaml_column(
    embedded: &EmbeddedScript,
    snippet_line: usize,
    host_line_start: HostLineStart,
    decoded_column: usize,
) -> DisplayPosition {
    let mut segment = host_line_start;
    let mut segment_column = 1usize;

    for mapping in embedded
        .host_column_mappings
        .iter()
        .filter(|mapping| mapping.line == snippet_line && mapping.column <= decoded_column)
    {
        segment = HostLineStart {
            line: mapping.host_line,
            column: mapping.host_column,
        };
        segment_column = mapping.column;
    }

    DisplayPosition::new(
        segment.line,
        segment.column + decoded_column.saturating_sub(segment_column),
    )
}

fn source_line_column_for_offset(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;

    for (index, ch) in source.char_indices() {
        if index >= offset {
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

fn span_char_len(source: &str, span: &std::ops::Range<usize>) -> usize {
    source
        .get(span.clone())
        .map_or(0, |value| value.chars().count())
}

fn prefixed_embedded_message(embedded: &EmbeddedScript, message: &str) -> String {
    format!("{}: {message}", embedded.label)
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::mpsc::{TryRecvError, channel};

    use notify::event::{CreateKind, EventAttributes, ModifyKind, RemoveKind, RenameMode};
    use shuck_extract::{
        EmbeddedFormat, EmbeddedScript, ExtractedDialect, HostLineStart, ImplicitShellFlags,
    };
    use shuck_linter::{
        Category, LinterSettings, Rule, RuleSelector, RuleSet, ShellCheckCodeMap, ShellDialect,
    };
    use shuck_parser::parser::Parser;
    use tempfile::tempdir;

    use super::*;
    use crate::ExitStatus;
    use crate::args::{
        CheckCommand, CheckOutputFormatArg, FileSelectionArgs, PatternRuleSelectorPair,
        PatternShellPair, RuleSelectionArgs,
    };
    use crate::commands::check::add_ignore::run_add_ignore_with_cwd;
    use crate::commands::check::analyze::{
        analyze_file, collect_lint_diagnostics, read_shared_source,
    };
    use crate::commands::check::cache::CachedDisplayedDiagnosticKind;
    use crate::commands::check::display::display_lint_diagnostics;
    use crate::commands::check::embedded::remap_embedded_position;
    use crate::commands::check::run::run_check_with_cwd;
    use crate::commands::check::settings::{
        CompiledPerFileShellList, PerFileShell, parse_rule_selectors,
    };
    use crate::commands::check::test_support::*;
    use crate::commands::check::watch::{
        WatchTarget, collect_watch_targets, drain_watch_batch, should_clear_screen,
        watch_event_requires_rerun,
    };
    use crate::commands::check::{CheckReport, diagnostics_exit_status};
    use crate::commands::check_output::{
        DisplayPosition, DisplaySpan, DisplayedDiagnostic, DisplayedDiagnosticKind, print_report_to,
    };
    use crate::commands::project_runner::PendingProjectFile;
    use crate::discover::{FileKind, normalize_path};
    use shuck_config::ConfigArguments;

    #[test]
    fn checks_embedded_github_actions_workflows() {
        let tempdir = tempdir().unwrap();
        let workflows = tempdir.path().join(".github/workflows");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(
            workflows.join("ci.yml"),
            r#"on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: |
          unused=1
          echo ok
"#,
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::UnusedAssignment.code().to_owned()]
        );
        assert_eq!(
            report.diagnostics[0].path,
            PathBuf::from(".github/workflows/ci.yml")
        );
        assert_eq!(report.diagnostics[0].span.start.line, 7);
        assert_eq!(report.diagnostics[0].span.start.column, 11);
        assert!(
            report.diagnostics[0]
                .message
                .starts_with("jobs.test.steps[0].run:")
        );
        assert!(
            report.diagnostics[0]
                .source
                .as_deref()
                .is_some_and(|source| source.contains("on: push"))
        );
    }

    #[test]
    fn skips_default_windows_shell_steps() {
        let tempdir = tempdir().unwrap();
        let workflows = tempdir.path().join(".github/workflows");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(
            workflows.join("ci.yml"),
            r#"on: push
jobs:
  windows:
    runs-on: windows-latest
    steps:
      - run: |
          unused=1
          echo ok
"#,
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn remaps_embedded_columns_on_later_lines() {
        let embedded = EmbeddedScript {
            source: "echo hi\necho bye\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 9,
            host_line_starts: vec![
                HostLineStart { line: 7, column: 9 },
                HostLineStart { line: 8, column: 9 },
                HostLineStart { line: 9, column: 9 },
            ],
            host_column_mappings: Vec::new(),
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: Vec::new(),
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 2, 5);
        assert_eq!(position.line, 8);
        assert_eq!(position.column, 13);
    }

    #[test]
    fn remaps_columns_after_placeholder_expansion_on_the_same_line() {
        let embedded = EmbeddedScript {
            source: "echo ${_SHUCK_GHA_1}$FOO\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 9,
            host_line_starts: vec![HostLineStart { line: 7, column: 9 }],
            host_column_mappings: Vec::new(),
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: vec![shuck_extract::PlaceholderMapping {
                name: "_SHUCK_GHA_1".to_owned(),
                original: "${{ github.ref }}".to_owned(),
                expression: "github.ref".to_owned(),
                taint: shuck_extract::ExpressionTaint::Trusted,
                substituted_span: 5..20,
                host_span: 5..22,
            }],
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 1, 21);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 31);
    }

    #[test]
    fn remaps_columns_after_non_ascii_placeholder_expansion() {
        let embedded = EmbeddedScript {
            source: "echo ${_SHUCK_GHA_1}$FOO\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 9,
            host_line_starts: vec![HostLineStart { line: 7, column: 9 }],
            host_column_mappings: Vec::new(),
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: vec![shuck_extract::PlaceholderMapping {
                name: "_SHUCK_GHA_1".to_owned(),
                original: "${{ github.refé }}".to_owned(),
                expression: "github.refé".to_owned(),
                taint: shuck_extract::ExpressionTaint::Trusted,
                substituted_span: 5..20,
                host_span: 5..24,
            }],
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 1, 21);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 32);
    }

    #[test]
    fn remaps_positions_for_escaped_yaml_newlines() {
        let embedded = EmbeddedScript {
            source: "echo hi\nif true\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 15,
            host_line_starts: vec![
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
            ],
            host_column_mappings: Vec::new(),
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: Vec::new(),
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 2, 1);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 24);
    }

    #[test]
    fn remaps_columns_after_non_newline_yaml_escapes() {
        let embedded = EmbeddedScript {
            source: "echo\tif true\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 15,
            host_line_starts: vec![HostLineStart {
                line: 7,
                column: 15,
            }],
            host_column_mappings: vec![shuck_extract::HostColumnMapping {
                line: 1,
                column: 6,
                host_line: 7,
                host_column: 21,
            }],
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: Vec::new(),
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 1, 6);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 21);
    }

    #[test]
    fn remaps_columns_after_folded_double_quoted_yaml_newlines() {
        let embedded = EmbeddedScript {
            source: "echo ok ; unused=1\n".to_owned(),
            host_offset: 0,
            host_start_line: 6,
            host_start_column: 15,
            host_line_starts: vec![HostLineStart {
                line: 6,
                column: 15,
            }],
            host_column_mappings: vec![shuck_extract::HostColumnMapping {
                line: 1,
                column: 9,
                host_line: 7,
                host_column: 13,
            }],
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: Vec::new(),
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 1, 9);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 13);
    }
    #[test]
    fn can_disable_embedded_workflow_checks_in_config() {
        let tempdir = tempdir().unwrap();
        let workflows = tempdir.path().join(".github/workflows");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[check]\nembedded = false\n",
        )
        .unwrap();
        fs::write(
            workflows.join("ci.yml"),
            r#"on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: |
          unused=1
          echo ok
"#,
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert!(report.diagnostics.is_empty());
    }
}
