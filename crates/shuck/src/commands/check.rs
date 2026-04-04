use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use shuck_cache::{CacheKey, CacheKeyHasher, FileCacheKey, PackageCache};
use shuck_syntax::{Dialect, ParseError, ParseMode, ParseOptions};

use crate::ExitStatus;
use crate::args::CheckCommand;
use crate::discover::{DiscoveredFile, ProjectRoot, discover_files};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplayedParseFailure {
    path: PathBuf,
    line: usize,
    column: usize,
    message: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CheckReport {
    failures: Vec<DisplayedParseFailure>,
    cache_hits: usize,
    cache_misses: usize,
}

impl CheckReport {
    fn exit_status(&self) -> ExitStatus {
        if self.failures.is_empty() {
            ExitStatus::Success
        } else {
            ExitStatus::Failure
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EffectiveCheckSettings {
    dialect: Dialect,
    mode: ParseMode,
}

impl Default for EffectiveCheckSettings {
    fn default() -> Self {
        Self {
            dialect: Dialect::Bash,
            mode: ParseMode::Strict,
        }
    }
}

impl CacheKey for EffectiveCheckSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-check-settings");
        state.write_str(self.dialect.as_str());
        state.write_str(self.mode.as_str());
    }
}

#[derive(Debug, Clone)]
struct ProjectCacheKey {
    canonical_project_root: PathBuf,
    settings: EffectiveCheckSettings,
}

impl CacheKey for ProjectCacheKey {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"project-cache-key");
        self.canonical_project_root.cache_key(state);
        self.settings.cache_key(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum ParseCacheData {
    Success,
    Error(ParseCacheFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ParseCacheFailure {
    message: String,
    line: usize,
    column: usize,
}

pub(crate) fn check(args: CheckCommand) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let report = run_check_with_cwd(&args, &cwd)?;
    print_report(&report)?;
    Ok(report.exit_status())
}

fn print_report(report: &CheckReport) -> Result<()> {
    let mut stdout = BufWriter::new(io::stdout().lock());
    for failure in &report.failures {
        writeln!(
            stdout,
            "{}:{}:{}: parse error {}",
            failure.path.display(),
            failure.line,
            failure.column,
            failure.message
        )?;
    }
    Ok(())
}

fn run_check_with_cwd(args: &CheckCommand, cwd: &Path) -> Result<CheckReport> {
    if args.fix || args.unsafe_fixes {
        return Err(anyhow!(
            "--fix and --unsafe-fixes are not supported until the analyzer is wired"
        ));
    }

    let files = discover_files(&args.paths, cwd)?;
    let mut groups: BTreeMap<ProjectRoot, Vec<DiscoveredFile>> = BTreeMap::new();
    for file in files {
        groups
            .entry(file.project_root.clone())
            .or_default()
            .push(file);
    }

    let settings = EffectiveCheckSettings::default();
    let parse_options = ParseOptions {
        dialect: settings.dialect,
        mode: settings.mode,
    };

    let mut report = CheckReport::default();

    for (project_root, files) in groups {
        let cache_key = ProjectCacheKey {
            canonical_project_root: project_root.canonical_root.clone(),
            settings,
        };
        let mut cache = if args.no_cache {
            None
        } else {
            Some(PackageCache::<ParseCacheData>::open(
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
                if let ParseCacheData::Error(error) = cached {
                    report.failures.push(DisplayedParseFailure {
                        path: file.display_path,
                        line: error.line,
                        column: error.column,
                        message: error.message,
                    });
                }
                continue;
            }

            let source = fs::read_to_string(&file.absolute_path)?;
            let cached = match shuck_syntax::parse(&source, parse_options) {
                Ok(_) => ParseCacheData::Success,
                Err(ParseError::Parse {
                    message,
                    line,
                    column,
                }) => {
                    report.failures.push(DisplayedParseFailure {
                        path: file.display_path,
                        line,
                        column,
                        message: message.clone(),
                    });
                    ParseCacheData::Error(ParseCacheFailure {
                        message,
                        line,
                        column,
                    })
                }
                Err(other) => {
                    return Err(anyhow!(
                        "failed to parse {}: {other}",
                        file.display_path.display()
                    ));
                }
            };

            if let Some(cache) = cache.as_mut() {
                cache.insert(file.relative_path, file_key, cached);
            }
            report.cache_misses += 1;
        }

        if let Some(cache) = cache {
            cache.persist()?;
        }
    }

    report.failures.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
    });

    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::tempdir;

    use super::*;

    fn check_args(no_cache: bool) -> CheckCommand {
        CheckCommand {
            fix: false,
            unsafe_fixes: false,
            no_cache,
            paths: Vec::new(),
        }
    }

    #[test]
    fn reports_parse_errors() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

        let report = run_check_with_cwd(&check_args(false), tempdir.path()).unwrap();

        assert_eq!(report.exit_status(), ExitStatus::Failure);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
    }

    #[test]
    fn reuses_cached_results() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let first = run_check_with_cwd(&check_args(false), tempdir.path()).unwrap();
        let second = run_check_with_cwd(&check_args(false), tempdir.path()).unwrap();

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

        let first = run_check_with_cwd(&check_args(false), tempdir.path()).unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);

        std::thread::sleep(Duration::from_millis(5));
        fs::write(&script, "#!/bin/bash\nif true\n").unwrap();

        let second = run_check_with_cwd(&check_args(false), tempdir.path()).unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert_eq!(second.failures.len(), 1);
    }

    #[test]
    fn no_cache_does_not_write_cache_files() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let report = run_check_with_cwd(&check_args(true), tempdir.path()).unwrap();

        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }
}
