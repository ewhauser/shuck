use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use shuck_cache::{FileCacheKey, PackageCache};
use shuck_formatter::{
    FormatError, FormattedSource, ShellFormatOptions, format_source, source_is_formatted,
};
use similar::TextDiff;

use crate::ExitStatus;
use crate::args::FormatCommand;
use crate::cache::resolve_cache_root;
use crate::commands::project_runner::{PendingProjectFile, prepare_project_runs};
use crate::config::ConfigArguments;
use crate::discover::{DiscoveryOptions, FileKind};
use crate::format_settings::{ResolvedFormatSettings, resolve_project_format_settings};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplayedFormatError {
    path: PathBuf,
    line: usize,
    column: usize,
    message: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct FormatReport {
    errors: Vec<DisplayedFormatError>,
    changed_files: Vec<PathBuf>,
    cache_hits: usize,
    cache_misses: usize,
}

impl FormatReport {
    fn exit_status(&self, mode: FormatMode) -> ExitStatus {
        if !self.errors.is_empty() {
            return ExitStatus::Error;
        }

        if matches!(mode, FormatMode::Check | FormatMode::Diff) && !self.changed_files.is_empty() {
            ExitStatus::Failure
        } else {
            ExitStatus::Success
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum FormatMode {
    Write,
    Check,
    Diff,
}

impl FormatMode {
    pub(crate) fn from_cli(args: &FormatCommand) -> Self {
        if args.diff {
            Self::Diff
        } else if args.check {
            Self::Check
        } else {
            Self::Write
        }
    }

    pub(crate) fn is_write(self) -> bool {
        matches!(self, Self::Write)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum FormatCacheData {
    Unchanged,
    ParseError(ParseCacheFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ParseCacheFailure {
    message: String,
    line: usize,
    column: usize,
}

pub(crate) fn format(
    args: FormatCommand,
    config_arguments: &ConfigArguments,
    cache_dir: Option<&Path>,
) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let mode = FormatMode::from_cli(&args);
    let cache_root = resolve_cache_root(&cwd, cache_dir)?;
    let report = run_format_with_cwd(&args, config_arguments, &cwd, &cache_root, mode)?;
    print_report(&report)?;
    Ok(report.exit_status(mode))
}

pub(crate) fn write_parse_error_line(
    writer: &mut impl Write,
    path: &Path,
    line: usize,
    column: usize,
    message: &str,
) -> io::Result<()> {
    writeln!(
        writer,
        "{}:{}:{}: parse error {}",
        path.display(),
        line,
        column,
        message
    )
}

pub(crate) fn unified_diff(path: &Path, original: &str, formatted: &str) -> String {
    let old = format!("a/{}", path.display());
    let new = format!("b/{}", path.display());
    TextDiff::from_lines(original, formatted)
        .unified_diff()
        .header(&old, &new)
        .to_string()
}

fn print_report(report: &FormatReport) -> Result<()> {
    let mut stdout = BufWriter::new(io::stdout().lock());
    for error in &report.errors {
        write_parse_error_line(
            &mut stdout,
            &error.path,
            error.line,
            error.column,
            &error.message,
        )?;
    }
    Ok(())
}

fn run_format_with_cwd(
    args: &FormatCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
    cache_root: &Path,
    mode: FormatMode,
) -> Result<FormatReport> {
    let cli_settings = args.format_settings_patch();
    let options = DiscoveryOptions {
        exclude_patterns: args.file_selection.exclude.clone(),
        extend_exclude_patterns: args.file_selection.extend_exclude.clone(),
        respect_gitignore: args.respect_gitignore(),
        force_exclude: args.force_exclude(),
        parallel: false,
        cache_root: Some(cache_root.to_path_buf()),
        use_config_roots: config_arguments.use_config_roots(),
    };
    let runs = prepare_project_runs::<FormatCacheData, ResolvedFormatSettings, _>(
        &args.files,
        cwd,
        &options,
        cache_root,
        args.no_cache,
        b"project-format-cache-key",
        |project_root| {
            resolve_project_format_settings(
                &project_root.storage_root,
                config_arguments,
                cli_settings,
            )
        },
    )?;
    let mut report = FormatReport::default();

    for mut run in runs {
        run.files.retain(|file| file.kind == FileKind::Shell);
        let settings = run.settings.to_shell_format_options();
        let pending = run.take_pending_files(|file, cached| {
            report.cache_hits += 1;
            if let FormatCacheData::ParseError(error) = cached {
                report.errors.push(DisplayedFormatError {
                    path: file.display_path,
                    line: error.line,
                    column: error.column,
                    message: error.message,
                });
            }
            Ok(())
        })?;

        for pending in pending {
            handle_pending_file(pending, &settings, mode, &mut run.cache, &mut report)?;
        }

        run.persist_cache()?;
    }

    report.errors.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
            .then(left.message.cmp(&right.message))
    });

    Ok(report)
}

fn handle_pending_file(
    pending: PendingProjectFile,
    settings: &ShellFormatOptions,
    mode: FormatMode,
    cache: &mut Option<PackageCache<FormatCacheData>>,
    report: &mut FormatReport,
) -> Result<()> {
    let PendingProjectFile { file, file_key } = pending;
    let source = fs::read_to_string(&file.absolute_path)?;
    let (cached_result, cached_key) = if matches!(mode, FormatMode::Check) {
        match source_is_formatted(&source, Some(&file.absolute_path), settings) {
            Ok(true) => (Some(FormatCacheData::Unchanged), file_key.clone()),
            Ok(false) => {
                report.changed_files.push(file.display_path.clone());
                (None, file_key.clone())
            }
            Err(FormatError::Parse {
                message,
                line,
                column,
            }) => {
                report.errors.push(DisplayedFormatError {
                    path: file.display_path.clone(),
                    line,
                    column,
                    message: message.clone(),
                });

                (
                    Some(FormatCacheData::ParseError(ParseCacheFailure {
                        message,
                        line,
                        column,
                    })),
                    file_key.clone(),
                )
            }
            Err(FormatError::Internal(message)) => return Err(anyhow!(message)),
        }
    } else {
        match format_source(&source, Some(&file.absolute_path), settings) {
            Ok(FormattedSource::Unchanged) => (Some(FormatCacheData::Unchanged), file_key.clone()),
            Ok(FormattedSource::Formatted(formatted)) => {
                report.changed_files.push(file.display_path.clone());
                match mode {
                    FormatMode::Write => fs::write(&file.absolute_path, formatted.as_bytes())?,
                    FormatMode::Check => {}
                    FormatMode::Diff => {
                        let mut stdout = io::stdout().lock();
                        write!(
                            &mut stdout,
                            "{}",
                            unified_diff(&file.display_path, &source, &formatted)
                        )?;
                    }
                }

                let cache_key = if mode.is_write() {
                    FileCacheKey::from_path(&file.absolute_path)?
                } else {
                    file_key.clone()
                };
                let cache_result = mode.is_write().then_some(FormatCacheData::Unchanged);
                (cache_result, cache_key)
            }
            Err(FormatError::Parse {
                message,
                line,
                column,
            }) => {
                report.errors.push(DisplayedFormatError {
                    path: file.display_path.clone(),
                    line,
                    column,
                    message: message.clone(),
                });

                (
                    Some(FormatCacheData::ParseError(ParseCacheFailure {
                        message,
                        line,
                        column,
                    })),
                    file_key.clone(),
                )
            }
            Err(FormatError::Internal(message)) => return Err(anyhow!(message)),
        }
    };

    if let Some(cache) = cache.as_mut()
        && let Some(cached_result) = cached_result
    {
        cache.insert(file.relative_path, cached_key, cached_result);
    }
    report.cache_misses += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::args::FileSelectionArgs;
    use crate::config::ConfigArguments;

    fn make_file_read_only(path: &Path) {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn format_args(no_cache: bool) -> FormatCommand {
        FormatCommand {
            files: vec![PathBuf::from(".")],
            check: false,
            diff: false,
            no_cache,
            stdin_filename: None,
            file_selection: FileSelectionArgs::default(),
            dialect: None,
            indent_style: None,
            indent_width: None,
            binary_next_line: false,
            no_binary_next_line: false,
            switch_case_indent: false,
            no_switch_case_indent: false,
            space_redirects: false,
            no_space_redirects: false,
            keep_padding: false,
            no_keep_padding: false,
            function_next_line: false,
            no_function_next_line: false,
            never_split: false,
            no_never_split: false,
            simplify: false,
            minify: false,
        }
    }

    #[test]
    fn no_op_formatter_does_not_parse_sources() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

        let report = run_format_with_cwd(
            &format_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Write,
        )
        .unwrap();

        assert_eq!(report.exit_status(FormatMode::Write), ExitStatus::Success);
        assert!(report.errors.is_empty());
        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
    }

    #[test]
    fn reuses_cached_results() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let first = run_format_with_cwd(
            &format_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Write,
        )
        .unwrap();
        let second = run_format_with_cwd(
            &format_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Write,
        )
        .unwrap();

        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(second.cache_misses, 0);
    }

    #[test]
    fn invalidates_cache_when_file_changes() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        fs::write(&script, "#!/bin/bash\necho ok\n").unwrap();

        let first = run_format_with_cwd(
            &format_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Write,
        )
        .unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);

        fs::write(&script, "#!/bin/bash\nif true\n").unwrap();
        make_file_read_only(&script);

        let second = run_format_with_cwd(
            &format_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Write,
        )
        .unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert!(second.errors.is_empty());
    }

    #[test]
    fn no_cache_does_not_write_cache_files() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let report = run_format_with_cwd(
            &format_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Write,
        )
        .unwrap();

        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }

    #[test]
    fn check_mode_treats_no_op_output_as_unchanged() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        let source = "#!/bin/bash\n echo ok\n";
        fs::write(&script, source).unwrap();

        let mut args = format_args(true);
        args.check = true;

        let report = run_format_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Check,
        )
        .unwrap();

        assert_eq!(report.exit_status(FormatMode::Check), ExitStatus::Success);
        assert!(report.changed_files.is_empty());
        assert_eq!(fs::read_to_string(&script).unwrap(), source);
    }

    #[test]
    fn check_mode_reuses_cache_for_already_formatted_files() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("script.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let mut args = format_args(false);
        args.check = true;

        let first = run_format_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Check,
        )
        .unwrap();
        let second = run_format_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Check,
        )
        .unwrap();

        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(second.cache_misses, 0);
    }

    #[test]
    fn skips_embedded_workflow_yaml_during_format() {
        let tempdir = tempdir().unwrap();
        fs::create_dir_all(tempdir.path().join(".github/workflows")).unwrap();
        fs::write(tempdir.path().join("script.sh"), "#!/bin/bash\necho ok\n").unwrap();
        fs::write(
            tempdir.path().join(".github/workflows/ci.yml"),
            r#"on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
"#,
        )
        .unwrap();

        let mut args = format_args(true);
        args.check = true;

        let report = run_format_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &tempdir.path().join("cache"),
            FormatMode::Check,
        )
        .unwrap();

        assert_eq!(report.exit_status(FormatMode::Check), ExitStatus::Success);
        assert!(report.errors.is_empty());
        assert!(report.changed_files.is_empty());
        assert_eq!(report.cache_misses, 1);
    }
}
