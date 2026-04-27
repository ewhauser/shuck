use std::io::{self, BufWriter};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use shuck_ast::TextSize;
use shuck_linter::Applicability;

use super::CheckReport;
use crate::commands::check_output::{
    DisplayPosition, DisplaySpan, DisplayedApplicability, DisplayedDiagnostic,
    DisplayedDiagnosticKind, DisplayedEdit, DisplayedFix, print_report_to,
};
use crate::commands::project_runner::PendingProjectFile;

pub(super) fn print_report(
    report: &CheckReport,
    output_format: crate::args::CheckOutputFormatArg,
) -> Result<()> {
    print_diagnostics(&report.diagnostics, output_format)
}

pub(super) fn print_diagnostics(
    diagnostics: &[DisplayedDiagnostic],
    output_format: crate::args::CheckOutputFormatArg,
) -> Result<()> {
    let mut stdout = BufWriter::new(io::stdout().lock());
    print_report_to(
        &mut stdout,
        diagnostics,
        output_format,
        colored::control::SHOULD_COLORIZE.should_colorize(),
    )?;
    Ok(())
}
pub(super) fn display_lint_diagnostics(
    pending: &PendingProjectFile,
    source: &Arc<str>,
    diagnostics: &[shuck_linter::Diagnostic],
    include_source: bool,
) -> Vec<DisplayedDiagnostic> {
    let diagnostic_source = (!diagnostics.is_empty() && include_source).then(|| source.clone());

    diagnostics
        .iter()
        .map(|diagnostic| DisplayedDiagnostic {
            path: pending.file.display_path.clone(),
            relative_path: pending.file.relative_path.clone(),
            absolute_path: pending.file.absolute_path.clone(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.span.start.line, diagnostic.span.start.column),
                DisplayPosition::new(diagnostic.span.end.line, diagnostic.span.end.column),
            ),
            message: diagnostic.message.clone(),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code().to_owned(),
                severity: diagnostic.severity.as_str().to_owned(),
            },
            fix: displayed_fix_from_diagnostic(diagnostic, source),
            source: diagnostic_source.clone(),
        })
        .collect()
}

pub(super) fn display_parse_error(
    display_path: &Path,
    relative_path: &Path,
    absolute_path: &Path,
    line: usize,
    column: usize,
    message: String,
    source: Option<Arc<str>>,
) -> DisplayedDiagnostic {
    DisplayedDiagnostic {
        path: display_path.to_path_buf(),
        relative_path: relative_path.to_path_buf(),
        absolute_path: absolute_path.to_path_buf(),
        span: DisplaySpan::point(line, column),
        message,
        kind: DisplayedDiagnosticKind::ParseError,
        fix: None,
        source,
    }
}

fn displayed_fix_from_diagnostic(
    diagnostic: &shuck_linter::Diagnostic,
    source: &str,
) -> Option<DisplayedFix> {
    let fix = diagnostic.fix.as_ref()?;
    let line_index = shuck_indexer::LineIndex::new(source);

    Some(DisplayedFix {
        applicability: match fix.applicability() {
            Applicability::Safe => DisplayedApplicability::Safe,
            Applicability::Unsafe => DisplayedApplicability::Unsafe,
        },
        message: diagnostic.fix_title.clone(),
        edits: fix
            .edits()
            .iter()
            .map(|edit| displayed_edit_from_fix(edit, &line_index, source))
            .collect(),
    })
}

fn displayed_edit_from_fix(
    edit: &shuck_linter::Edit,
    line_index: &shuck_indexer::LineIndex,
    source: &str,
) -> DisplayedEdit {
    let range = edit.range();
    let start_offset = floor_char_boundary(source, usize::from(range.start()));
    let end_offset = ceil_char_boundary(source, usize::from(range.end()));

    DisplayedEdit {
        location: display_position_at_offset(source, line_index, start_offset),
        end_location: display_position_at_offset(source, line_index, end_offset),
        content: edit.content().to_owned(),
    }
}

fn display_position_at_offset(
    source: &str,
    line_index: &shuck_indexer::LineIndex,
    target_offset: usize,
) -> DisplayPosition {
    let line = line_index.line_number(TextSize::new(target_offset as u32));
    let line_start = line_index
        .line_start(line)
        .map(usize::from)
        .unwrap_or_default();

    DisplayPosition::new(line, source[line_start..target_offset].chars().count() + 1)
}

fn floor_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn ceil_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset < source.len() && !source.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}
pub(super) fn push_lint_diagnostics(
    displayed: &mut Vec<DisplayedDiagnostic>,
    path: &Path,
    relative_path: &Path,
    absolute_path: &Path,
    diagnostics: &[shuck_linter::Diagnostic],
    raw_source: &Arc<str>,
    source: Option<Arc<str>>,
) {
    for diagnostic in diagnostics {
        displayed.push(DisplayedDiagnostic {
            path: path.to_path_buf(),
            relative_path: relative_path.to_path_buf(),
            absolute_path: absolute_path.to_path_buf(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.span.start.line, diagnostic.span.start.column),
                DisplayPosition::new(diagnostic.span.end.line, diagnostic.span.end.column),
            ),
            message: diagnostic.message.clone(),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code().to_owned(),
                severity: diagnostic.severity.as_str().to_owned(),
            },
            fix: displayed_fix_from_diagnostic(diagnostic, raw_source),
            source: source.clone(),
        });
    }
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
    use crate::config::ConfigArguments;
    use crate::discover::{FileKind, normalize_path};

    #[test]
    fn report_output_includes_ansi_styles_when_enabled() {
        colored::control::set_override(true);

        let report = CheckReport {
            diagnostics: vec![DisplayedDiagnostic {
                source: Some(Arc::<str>::from("echo ok\nvalue=$foo\nprintf '%s' $bar\n")),
                ..lint_displayed_diagnostic(
                    "script.sh",
                    DisplaySpan::new(DisplayPosition::new(3, 14), DisplayPosition::new(3, 18)),
                    "example message",
                    "C014",
                    "warning",
                )
            }],
            cache_hits: 0,
            cache_misses: 0,
            fixes_applied: 0,
            parse_failed: false,
        };

        let mut output = Vec::new();
        print_report_to(
            &mut output,
            &report.diagnostics,
            CheckOutputFormatArg::Full,
            true,
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\u{1b}["));
        assert!(output.contains("warning"));
        assert!(output.contains("C014"));

        colored::control::unset_override();
    }

    #[test]
    fn report_output_stays_plain_when_colors_are_disabled() {
        let report = CheckReport {
            diagnostics: vec![parse_displayed_diagnostic(
                "script.sh",
                DisplaySpan::point(2, 7),
                "unterminated construct",
            )],
            cache_hits: 0,
            cache_misses: 0,
            fixes_applied: 0,
            parse_failed: false,
        };

        let mut output = Vec::new();
        print_report_to(
            &mut output,
            &report.diagnostics,
            CheckOutputFormatArg::Concise,
            false,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "script.sh:2:7: parse error unterminated construct\n"
        );
    }
}
