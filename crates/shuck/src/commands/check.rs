use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use shuck_cache::{CacheKey, CacheKeyHasher, FileCacheKey, PackageCache};
use shuck_indexer::Indexer;
use shuck_linter::{
    LinterSettings, ShellCheckCodeMap, ShellDialect, SuppressionIndex, first_statement_line,
    parse_directives,
};
use shuck_parser::{Error as ParseError, parser::Parser};

use crate::ExitStatus;
use crate::args::CheckCommand;
use crate::discover::{DiscoveredFile, DiscoveryOptions, ProjectRoot, discover_files};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplayedDiagnostic {
    path: PathBuf,
    line: usize,
    column: usize,
    message: String,
    kind: DisplayedDiagnosticKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DisplayedDiagnosticKind {
    ParseError,
    Lint { code: String, severity: String },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CheckReport {
    diagnostics: Vec<DisplayedDiagnostic>,
    cache_hits: usize,
    cache_misses: usize,
}

impl CheckReport {
    fn exit_status(&self) -> ExitStatus {
        if self.diagnostics.is_empty() {
            ExitStatus::Success
        } else {
            ExitStatus::Failure
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveCheckSettings {
    enabled_rules: Vec<String>,
}

impl Default for EffectiveCheckSettings {
    fn default() -> Self {
        let mut enabled_rules = LinterSettings::default()
            .rules
            .iter()
            .map(|rule| rule.code().to_owned())
            .collect::<Vec<_>>();
        enabled_rules.sort();

        Self { enabled_rules }
    }
}

impl CacheKey for EffectiveCheckSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-check-settings");
        self.enabled_rules.cache_key(state);
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
enum CheckCacheData {
    Success(Vec<CachedLintDiagnostic>),
    ParseError(ParseCacheFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ParseCacheFailure {
    message: String,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedLintDiagnostic {
    line: usize,
    column: usize,
    code: String,
    severity: String,
    message: String,
}

impl CachedLintDiagnostic {
    fn from_diagnostic(diagnostic: &shuck_linter::Diagnostic) -> Self {
        Self {
            line: diagnostic.span.start.line,
            column: diagnostic.span.start.column,
            code: diagnostic.code().to_owned(),
            severity: diagnostic.severity.as_str().to_owned(),
            message: diagnostic.message.clone(),
        }
    }
}

pub(crate) fn check(args: CheckCommand) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let report = run_check_with_cwd(&args, &cwd)?;
    print_report(&report)?;
    Ok(report.exit_status())
}

fn print_report(report: &CheckReport) -> Result<()> {
    let mut stdout = BufWriter::new(io::stdout().lock());
    for diagnostic in &report.diagnostics {
        match &diagnostic.kind {
            DisplayedDiagnosticKind::ParseError => writeln!(
                stdout,
                "{}:{}:{}: parse error {}",
                diagnostic.path.display(),
                diagnostic.line,
                diagnostic.column,
                diagnostic.message
            )?,
            DisplayedDiagnosticKind::Lint { code, severity } => writeln!(
                stdout,
                "{}:{}:{}: {}[{}] {}",
                diagnostic.path.display(),
                diagnostic.line,
                diagnostic.column,
                severity,
                code,
                diagnostic.message
            )?,
        }
    }
    Ok(())
}

fn run_check_with_cwd(args: &CheckCommand, cwd: &Path) -> Result<CheckReport> {
    if args.fix || args.unsafe_fixes {
        return Err(anyhow!(
            "--fix and --unsafe-fixes are not supported until the analyzer is wired"
        ));
    }

    let files = discover_files(&args.paths, cwd, &DiscoveryOptions::default())?;
    let mut groups: BTreeMap<ProjectRoot, Vec<DiscoveredFile>> = BTreeMap::new();
    for file in files {
        groups
            .entry(file.project_root.clone())
            .or_default()
            .push(file);
    }

    let settings = EffectiveCheckSettings::default();
    let base_linter_settings = LinterSettings::default();
    let shellcheck_map = ShellCheckCodeMap::default();

    let mut report = CheckReport::default();

    for (project_root, files) in groups {
        let cache_key = ProjectCacheKey {
            canonical_project_root: project_root.canonical_root.clone(),
            settings: settings.clone(),
        };
        let mut cache = if args.no_cache {
            None
        } else {
            Some(PackageCache::<CheckCacheData>::open(
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
                match cached {
                    CheckCacheData::Success(diagnostics) => {
                        push_cached_lint_diagnostics(&mut report, &file.display_path, &diagnostics);
                    }
                    CheckCacheData::ParseError(error) => {
                        report.diagnostics.push(DisplayedDiagnostic {
                            path: file.display_path,
                            line: error.line,
                            column: error.column,
                            message: error.message,
                            kind: DisplayedDiagnosticKind::ParseError,
                        });
                    }
                }
                continue;
            }

            let source = fs::read_to_string(&file.absolute_path)?;
            let inferred_shell = ShellDialect::infer(&source, Some(&file.absolute_path));
            let parse_dialect = match inferred_shell {
                ShellDialect::Sh | ShellDialect::Dash | ShellDialect::Ksh => {
                    shuck_parser::ShellDialect::Posix
                }
                ShellDialect::Mksh => shuck_parser::ShellDialect::Mksh,
                ShellDialect::Zsh => shuck_parser::ShellDialect::Zsh,
                ShellDialect::Unknown | ShellDialect::Bash => shuck_parser::ShellDialect::Bash,
            };
            let cached = match Parser::with_dialect(&source, parse_dialect).parse() {
                Ok(output) => {
                    let indexer = Indexer::new(&source, &output);
                    let directives =
                        parse_directives(&source, indexer.comment_index(), &shellcheck_map);
                    let suppression_index = (!directives.is_empty()).then(|| {
                        SuppressionIndex::new(
                            &directives,
                            &output.file,
                            first_statement_line(&output.file).unwrap_or(u32::MAX),
                        )
                    });
                    let linter_settings = base_linter_settings.clone().with_shell(inferred_shell);
                    let diagnostics = shuck_linter::lint_file_at_path(
                        &output.file,
                        &source,
                        &indexer,
                        &linter_settings,
                        suppression_index.as_ref(),
                        Some(&file.absolute_path),
                    );

                    for diagnostic in &diagnostics {
                        report.diagnostics.push(DisplayedDiagnostic {
                            path: file.display_path.clone(),
                            line: diagnostic.span.start.line,
                            column: diagnostic.span.start.column,
                            message: diagnostic.message.clone(),
                            kind: DisplayedDiagnosticKind::Lint {
                                code: diagnostic.code().to_owned(),
                                severity: diagnostic.severity.as_str().to_owned(),
                            },
                        });
                    }

                    CheckCacheData::Success(
                        diagnostics
                            .iter()
                            .map(CachedLintDiagnostic::from_diagnostic)
                            .collect(),
                    )
                }
                Err(ParseError::Parse {
                    message,
                    line,
                    column,
                }) => {
                    report.diagnostics.push(DisplayedDiagnostic {
                        path: file.display_path,
                        line,
                        column,
                        message: message.clone(),
                        kind: DisplayedDiagnosticKind::ParseError,
                    });

                    CheckCacheData::ParseError(ParseCacheFailure {
                        message,
                        line,
                        column,
                    })
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

    report.diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
            .then(left.message.cmp(&right.message))
    });

    Ok(report)
}

fn push_cached_lint_diagnostics(
    report: &mut CheckReport,
    path: &Path,
    diagnostics: &[CachedLintDiagnostic],
) {
    for diagnostic in diagnostics {
        report.diagnostics.push(DisplayedDiagnostic {
            path: path.to_path_buf(),
            line: diagnostic.line,
            column: diagnostic.column,
            message: diagnostic.message.clone(),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code.clone(),
                severity: diagnostic.severity.clone(),
            },
        });
    }
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
        assert_eq!(report.diagnostics.len(), 1);
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
        assert_eq!(second.diagnostics.len(), 1);
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

    #[test]
    fn infers_shell_from_extension_for_local_rule() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("posix.sh"), "local foo=bar\n").unwrap();
        fs::write(tempdir.path().join("bashy.bash"), "local foo=bar\n").unwrap();

        let report = run_check_with_cwd(&check_args(true), tempdir.path()).unwrap();
        let c014 = report
            .diagnostics
            .iter()
            .filter(|diagnostic| matches!(&diagnostic.kind, DisplayedDiagnosticKind::Lint { code, .. } if code == "C014"))
            .collect::<Vec<_>>();

        assert_eq!(c014.len(), 1);
        assert_eq!(c014[0].path, PathBuf::from("bashy.bash"));
    }
}
