use std::path::{Path, PathBuf};

use anyhow::Result;
use rayon::prelude::*;
use shuck_config::ConfigArguments;
use shuck_linter::{Applicability, LinterSettings, RuleSelector, ShellCheckCodeMap};

use super::CheckReport;
use super::analyze::{analyze_file, read_shared_source};
use super::cache::{CheckCacheData, CheckCacheSettings, push_cached_diagnostics};
use super::settings::{ResolvedCheckSettings, resolve_project_check_settings};
use crate::args::{CheckCommand, FileSelectionArgs, RuleSelectionArgs};
use crate::commands::project_runner::{ProjectRunRequest, prepare_project_runs_with_cache_key};
use crate::discover::{DiscoveryOptions, FileKind};

pub(super) fn run_check_with_cwd(
    args: &CheckCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
    cache_root: &Path,
) -> Result<CheckReport> {
    let include_source = matches!(args.output_format, crate::args::CheckOutputFormatArg::Full);
    let fix_applicability = requested_fix_applicability(args);
    let mut runs = prepare_project_runs_with_cache_key::<
        CheckCacheData,
        ResolvedCheckSettings,
        CheckCacheSettings,
        _,
        _,
    >(
        ProjectRunRequest {
            inputs: &args.paths,
            cwd,
            discovery_options: &DiscoveryOptions {
                exclude_patterns: args.file_selection.exclude.clone(),
                extend_exclude_patterns: args.file_selection.extend_exclude.clone(),
                respect_gitignore: args.respect_gitignore(),
                force_exclude: args.force_exclude(),
                parallel: true,
                cache_root: Some(cache_root.to_path_buf()),
                use_config_roots: config_arguments.use_config_roots(),
            },
            cache_root,
            no_cache: args.no_cache || fix_applicability.is_some(),
            cache_tag: b"project-cache-key",
        },
        |project_root| {
            resolve_project_check_settings(project_root, config_arguments, &args.rule_selection)
        },
        |_, files, settings| Ok(CheckCacheSettings::new(settings, files)),
    )?;
    let shellcheck_map = ShellCheckCodeMap::default();

    let mut report = CheckReport::default();

    for run in &mut runs {
        if !run.settings.embedded_enabled {
            run.files.retain(|file| file.kind == FileKind::Shell);
        }
    }

    for mut run in runs {
        let project_settings = run.settings.clone();
        let analyzed_paths = LinterSettings::analyzed_path_set(
            run.files
                .iter()
                .filter(|file| file.kind == FileKind::Shell)
                .map(|file| file.absolute_path.clone()),
        );
        let linter_settings = project_settings
            .linter_settings
            .clone()
            .with_analyzed_path_set(analyzed_paths);
        let pending = run.take_pending_files(|file, cached| {
            report.cache_hits += 1;
            report.parse_failed |= cached.parse_failed;
            let source = (include_source && !cached.diagnostics.is_empty())
                .then(|| read_shared_source(&file.absolute_path))
                .transpose()?;
            push_cached_diagnostics(
                &mut report,
                &file.display_path,
                &file.relative_path,
                &file.absolute_path,
                &cached.diagnostics,
                source,
            );
            Ok(())
        })?;

        let results = pending
            .into_par_iter()
            .map(|pending| {
                analyze_file(
                    pending,
                    &linter_settings,
                    &project_settings.per_file_shell,
                    &shellcheck_map,
                    include_source,
                    fix_applicability,
                    &project_settings.fixable_rules,
                )
            })
            .collect::<Vec<_>>();

        for result in results {
            let result = result?;
            report.fixes_applied += result.fixes_applied;
            report.parse_failed |= result.parse_failed;
            report.diagnostics.extend(result.diagnostics);
            if let Some(cache) = run.cache.as_mut() {
                cache.insert(
                    result.file.relative_path.clone(),
                    result.file_key,
                    result.cache_data,
                );
            }
            report.cache_misses += 1;
        }

        run.persist_cache()?;
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
pub(crate) fn benchmark_check_paths(
    cwd: &Path,
    paths: &[PathBuf],
    output_format: crate::args::CheckOutputFormatArg,
) -> Result<usize> {
    let report = run_check_with_cwd(
        &CheckCommand {
            fix: false,
            unsafe_fixes: false,
            add_ignore: None,
            no_cache: true,
            output_format,
            watch: false,
            paths: paths.to_vec(),
            rule_selection: RuleSelectionArgs {
                select: Some(vec![RuleSelector::All]),
                ..RuleSelectionArgs::default()
            },
            file_selection: FileSelectionArgs::default(),
            exit_zero: false,
            exit_non_zero_on_fix: false,
        },
        &ConfigArguments::default(),
        cwd,
        &cwd.join("cache"),
    )?;

    Ok(report.diagnostics.len())
}
fn requested_fix_applicability(args: &CheckCommand) -> Option<Applicability> {
    if args.unsafe_fixes {
        Some(Applicability::Unsafe)
    } else if args.fix {
        Some(Applicability::Safe)
    } else {
        None
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
    use crate::discover::{FileKind, normalize_path};
    use shuck_config::ConfigArguments;

    #[test]
    fn parse_failure_with_warning_lint_stays_fatal_under_exit_zero() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("broken.sh"),
            "#!/bin/sh\nif true\n  echo hi\nfi\n",
        )
        .unwrap();

        let mut args = check_args(false);
        args.exit_zero = true;

        let first = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let second = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(first.exit_status(true, false), ExitStatus::Failure);
        assert_eq!(second.exit_status(true, false), ExitStatus::Failure);
        assert_eq!(first.cache_misses, 1);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(diagnostic_codes(&first), vec!["C064".to_owned()]);
        assert_eq!(diagnostic_codes(&second), vec!["C064".to_owned()]);
        assert!(first.parse_failed);
        assert!(second.parse_failed);
    }

    #[test]
    fn unfixable_rules_prevent_fix_application() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("warn.sh");
        let source = "#!/bin/bash\nprintf '%s\\n' x &;\n";
        fs::write(&script, source).unwrap();

        let mut args = check_args(true);
        args.fix = true;
        args.rule_selection = RuleSelectionArgs {
            extend_select: vec![RuleSelector::Rule(Rule::AmpersandSemicolon)],
            unfixable: vec![RuleSelector::Rule(Rule::AmpersandSemicolon)],
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.fixes_applied, 0);
        assert_eq!(fs::read_to_string(script).unwrap(), source);
        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::AmpersandSemicolon.code().to_owned()]
        );
    }

    #[test]
    fn no_cache_does_not_write_cache_files() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }

    #[test]
    fn sorts_diagnostics_deterministically_after_parallel_run() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("z.sh"), "#!/bin/bash\nif true\n").unwrap();
        fs::write(tempdir.path().join("a.bash"), "local foo=bar\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let paths = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.path.clone())
            .collect::<Vec<_>>();

        let mut sorted_paths = paths.clone();
        sorted_paths.sort();
        assert_eq!(paths, sorted_paths);
        assert_eq!(paths.first(), Some(&PathBuf::from("a.bash")));
        assert_eq!(paths.last(), Some(&PathBuf::from("z.sh")));
    }

    #[test]
    fn duplicate_explicit_file_and_directory_inputs_are_deduplicated() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("dup.sh"), "#!/bin/bash\nif true\n").unwrap();

        let args = CheckCommand {
            paths: vec![PathBuf::from("."), PathBuf::from("dup.sh")],
            ..check_args(true)
        };
        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
        assert_eq!(report.diagnostics.len(), 1);
    }

    #[test]
    fn skips_a_configured_cache_directory_inside_the_walked_tree() {
        let tempdir = tempdir().unwrap();
        let cache_root = tempdir.path().join("custom-cache");
        fs::create_dir_all(&cache_root).unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();
        fs::write(cache_root.join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root,
        )
        .unwrap();

        assert!(report.diagnostics.is_empty());
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }
}
