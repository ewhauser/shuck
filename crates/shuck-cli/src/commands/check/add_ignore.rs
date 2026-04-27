use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use shuck_linter::add_ignores_to_path;

use super::analyze::read_shared_source;
use super::cache::CheckCacheData;
use super::diagnostics_exit_status;
use super::display::{display_parse_error, push_lint_diagnostics};
use super::settings::{ResolvedCheckSettings, resolve_project_check_settings};
use crate::ExitStatus;
use crate::args::CheckCommand;
use crate::commands::check_output::DisplayedDiagnostic;
use crate::commands::project_runner::prepare_project_runs;
use crate::config::ConfigArguments;
use crate::discover::{DiscoveryOptions, FileKind};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct AddIgnoreReport {
    pub(super) diagnostics: Vec<DisplayedDiagnostic>,
    pub(super) directives_added: usize,
}

impl AddIgnoreReport {
    pub(super) fn exit_status(&self, exit_zero: bool) -> ExitStatus {
        diagnostics_exit_status(&self.diagnostics, exit_zero)
    }
}
pub(super) fn run_add_ignore_with_cwd(
    args: &CheckCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
    cache_root: &Path,
    reason: Option<&str>,
) -> Result<AddIgnoreReport> {
    let include_source = matches!(args.output_format, crate::args::CheckOutputFormatArg::Full);
    let mut runs = prepare_project_runs::<CheckCacheData, ResolvedCheckSettings, _>(
        &args.paths,
        cwd,
        &DiscoveryOptions {
            exclude_patterns: args.file_selection.exclude.clone(),
            extend_exclude_patterns: args.file_selection.extend_exclude.clone(),
            respect_gitignore: args.respect_gitignore(),
            force_exclude: args.force_exclude(),
            parallel: false,
            cache_root: Some(cache_root.to_path_buf()),
            use_config_roots: config_arguments.use_config_roots(),
        },
        cache_root,
        true,
        b"project-cache-key",
        |project_root| {
            resolve_project_check_settings(project_root, config_arguments, &args.rule_selection)
        },
    )?;

    let mut report = AddIgnoreReport::default();

    for run in &mut runs {
        run.files.retain(|file| file.kind == FileKind::Shell);
    }

    for run in runs {
        let per_file_shell = Arc::clone(&run.settings.per_file_shell);
        let analyzed_paths = run
            .files
            .iter()
            .map(|file| file.absolute_path.clone())
            .collect::<Vec<_>>();
        let linter_settings = run
            .settings
            .linter_settings
            .clone()
            .with_analyzed_paths(analyzed_paths);

        for file in run.files {
            let file_linter_settings =
                if let Some(shell) = per_file_shell.shell_for_path(&file.absolute_path) {
                    linter_settings.clone().with_shell(shell)
                } else {
                    linter_settings.clone()
                };
            let result = add_ignores_to_path(&file.absolute_path, &file_linter_settings, reason)?;
            report.directives_added += result.directives_added;
            if result.parse_error.is_none() && result.diagnostics.is_empty() {
                continue;
            }

            let raw_source = read_shared_source(&file.absolute_path)?;
            let source = include_source.then_some(raw_source.clone());
            if let Some(error) = result.parse_error {
                report.diagnostics.push(display_parse_error(
                    &file.display_path,
                    &file.relative_path,
                    &file.absolute_path,
                    error.line,
                    error.column,
                    error.message,
                    source.clone(),
                ));
            }
            push_lint_diagnostics(
                &mut report.diagnostics,
                &file.display_path,
                &file.relative_path,
                &file.absolute_path,
                &result.diagnostics,
                &raw_source,
                source,
            );
        }
    }

    report.diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.span.start.line.cmp(&right.span.start.line))
            .then(left.span.start.column.cmp(&right.span.start.column))
            .then(left.message.cmp(&right.message))
    });

    Ok(report)
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
    fn add_ignore_respects_per_file_shell_overrides() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("bashy.sh");
        fs::write(&script, "source helper.sh\n").unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            select: Some(vec![RuleSelector::Rule(Rule::SourceBuiltinInSh)]),
            per_file_shell: Some(vec![PatternShellPair {
                pattern: "bashy.sh".to_owned(),
                shell: ShellDialect::Bash,
            }]),
            ..RuleSelectionArgs::default()
        };

        let report = run_add_ignore_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
            None,
        )
        .unwrap();

        assert_eq!(report.directives_added, 0);
        assert!(report.diagnostics.is_empty());
        assert_eq!(fs::read_to_string(script).unwrap(), "source helper.sh\n");
    }
}
