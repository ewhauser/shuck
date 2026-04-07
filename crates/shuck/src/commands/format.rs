use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use shuck_cache::{CacheKey, CacheKeyHasher, FileCacheKey, PackageCache};
use shuck_formatter::{
    FormatError, FormattedSource, FormatterSettings, IndentStyle, LineEnding, QuoteStyle,
    format_source,
};
use similar::TextDiff;

use crate::ExitStatus;
use crate::args::FormatCommand;
use crate::discover::{DiscoveredFile, ProjectRoot};
use crate::format_resolver::resolve_format_files;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveFormatSettings {
    line_width: u16,
    indent_style: IndentStyle,
    quote_style: QuoteStyle,
    line_ending: LineEnding,
}

impl From<&FormatterSettings> for EffectiveFormatSettings {
    fn from(settings: &FormatterSettings) -> Self {
        Self {
            line_width: settings.line_width,
            indent_style: settings.indent_style,
            quote_style: settings.quote_style,
            line_ending: settings.line_ending,
        }
    }
}

impl CacheKey for EffectiveFormatSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-format-settings");
        state.write_u64(u64::from(self.line_width));
        state.write_u8(indent_style_key(self.indent_style));
        state.write_u8(quote_style_key(self.quote_style));
        state.write_u8(line_ending_key(self.line_ending));
    }
}

#[derive(Debug, Clone)]
struct ProjectCacheKey {
    canonical_project_root: PathBuf,
    settings: EffectiveFormatSettings,
}

impl CacheKey for ProjectCacheKey {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"project-format-cache-key");
        self.canonical_project_root.cache_key(state);
        self.settings.cache_key(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum FormatCacheData {
    Success,
    ParseError(ParseCacheFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ParseCacheFailure {
    message: String,
    line: usize,
    column: usize,
}

pub(crate) fn format(args: FormatCommand) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let mode = FormatMode::from_cli(&args);
    let report = run_format_with_cwd(&args, &cwd, mode)?;
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

fn run_format_with_cwd(args: &FormatCommand, cwd: &Path, mode: FormatMode) -> Result<FormatReport> {
    let files = resolve_format_files(args, cwd)?;
    let mut groups: BTreeMap<ProjectRoot, Vec<DiscoveredFile>> = BTreeMap::new();
    for file in files {
        groups
            .entry(file.project_root.clone())
            .or_default()
            .push(file);
    }

    let settings = FormatterSettings::default();
    let effective_settings = EffectiveFormatSettings::from(&settings);
    let mut report = FormatReport::default();

    for (project_root, files) in groups {
        let cache_key = ProjectCacheKey {
            canonical_project_root: project_root.canonical_root.clone(),
            settings: effective_settings.clone(),
        };
        let mut cache = if args.no_cache {
            None
        } else {
            Some(PackageCache::<FormatCacheData>::open(
                &project_root.storage_root,
                project_root.canonical_root.clone(),
                env!("CARGO_PKG_VERSION"),
                &cache_key,
            )?)
        };

        for file in files {
            let file_key = FileCacheKey::from_path(&file.absolute_path)?;
            if let Some(cache) = cache.as_mut()
                && let Some(cached) = cache.get(&file.relative_path, &file_key)
            {
                report.cache_hits += 1;
                if let FormatCacheData::ParseError(error) = cached {
                    report.errors.push(DisplayedFormatError {
                        path: file.display_path,
                        line: error.line,
                        column: error.column,
                        message: error.message,
                    });
                }
                continue;
            }

            let source = fs::read_to_string(&file.absolute_path)?;
            let (cached_result, cached_key) =
                match format_source(&source, Some(&file.absolute_path), &settings) {
                    Ok(FormattedSource::Unchanged) => (FormatCacheData::Success, file_key.clone()),
                    Ok(FormattedSource::Formatted(formatted)) => {
                        report.changed_files.push(file.display_path.clone());
                        match mode {
                            FormatMode::Write => {
                                fs::write(&file.absolute_path, formatted.as_bytes())?
                            }
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
                        (FormatCacheData::Success, cache_key)
                    }
                    Err(FormatError::Parse {
                        message,
                        line,
                        column,
                    }) => {
                        report.errors.push(DisplayedFormatError {
                            path: file.display_path,
                            line,
                            column,
                            message: message.clone(),
                        });

                        (
                            FormatCacheData::ParseError(ParseCacheFailure {
                                message,
                                line,
                                column,
                            }),
                            file_key.clone(),
                        )
                    }
                };

            if let Some(cache) = cache.as_mut() {
                cache.insert(file.relative_path, cached_key, cached_result);
            }
            report.cache_misses += 1;
        }

        if let Some(cache) = cache {
            cache.persist()?;
        }
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

const fn indent_style_key(style: IndentStyle) -> u8 {
    match style {
        IndentStyle::Space => 0,
        IndentStyle::Tab => 1,
    }
}

const fn quote_style_key(style: QuoteStyle) -> u8 {
    match style {
        QuoteStyle::Preserve => 0,
        QuoteStyle::Single => 1,
        QuoteStyle::Double => 2,
    }
}

const fn line_ending_key(line_ending: LineEnding) -> u8 {
    match line_ending {
        LineEnding::Auto => 0,
        LineEnding::Lf => 1,
        LineEnding::CrLf => 2,
        LineEnding::Native => 3,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::tempdir;

    use super::*;

    fn format_args(no_cache: bool) -> FormatCommand {
        FormatCommand {
            files: vec![PathBuf::from(".")],
            check: false,
            diff: false,
            no_cache,
            stdin_filename: None,
            exclude: Vec::new(),
            respect_gitignore: false,
            no_respect_gitignore: false,
            force_exclude: false,
            no_force_exclude: false,
        }
    }

    #[test]
    fn reports_parse_errors() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

        let report =
            run_format_with_cwd(&format_args(false), tempdir.path(), FormatMode::Write).unwrap();

        assert_eq!(report.exit_status(FormatMode::Write), ExitStatus::Error);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
    }

    #[test]
    fn reuses_cached_results() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let first =
            run_format_with_cwd(&format_args(false), tempdir.path(), FormatMode::Write).unwrap();
        let second =
            run_format_with_cwd(&format_args(false), tempdir.path(), FormatMode::Write).unwrap();

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

        let first =
            run_format_with_cwd(&format_args(false), tempdir.path(), FormatMode::Write).unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);

        std::thread::sleep(Duration::from_millis(5));
        fs::write(&script, "#!/bin/bash\nif true\n").unwrap();

        let second =
            run_format_with_cwd(&format_args(false), tempdir.path(), FormatMode::Write).unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert_eq!(second.errors.len(), 1);
    }

    #[test]
    fn no_cache_does_not_write_cache_files() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let report =
            run_format_with_cwd(&format_args(true), tempdir.path(), FormatMode::Write).unwrap();

        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }
}
