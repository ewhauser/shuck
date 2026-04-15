use std::fs;
use std::io::{self, BufWriter};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use shuck_cache::{CacheKey, CacheKeyHasher};
use shuck_indexer::Indexer;
use shuck_linter::{
    LinterSettings, ShellCheckCodeMap, ShellDialect, SuppressionIndex, first_statement_line,
    parse_directives,
};
use shuck_parser::{
    Error as ParseError,
    parser::{ParseDiagnostic, ParseResult, ParseStatus, Parser},
};

use crate::ExitStatus;
use crate::args::CheckCommand;
use crate::cache::resolve_cache_root;
use crate::commands::check_output::{
    DisplayPosition, DisplaySpan, DisplayedDiagnostic, DisplayedDiagnosticKind, print_report_to,
};
use crate::commands::project_runner::{PendingProjectFile, prepare_project_runs};
use crate::discover::DiscoveryOptions;

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
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    code: String,
    severity: String,
    message: String,
}

impl CachedLintDiagnostic {
    fn from_diagnostic(diagnostic: &shuck_linter::Diagnostic) -> Self {
        Self {
            start_line: diagnostic.span.start.line,
            start_column: diagnostic.span.start.column,
            end_line: diagnostic.span.end.line,
            end_column: diagnostic.span.end.column,
            code: diagnostic.code().to_owned(),
            severity: diagnostic.severity.as_str().to_owned(),
            message: diagnostic.message.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct FileCheckResult {
    file: crate::discover::DiscoveredFile,
    file_key: shuck_cache::FileCacheKey,
    cache_data: CheckCacheData,
    diagnostics: Vec<DisplayedDiagnostic>,
}

pub(crate) fn check(args: CheckCommand, cache_dir: Option<&Path>) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let cache_root = resolve_cache_root(&cwd, cache_dir)?;
    let report = run_check_with_cwd(&args, &cwd, &cache_root)?;
    print_report(&report, args.output_format)?;
    Ok(report.exit_status())
}

fn print_report(
    report: &CheckReport,
    output_format: crate::args::CheckOutputFormatArg,
) -> Result<()> {
    let mut stdout = BufWriter::new(io::stdout().lock());
    print_report_to(
        &mut stdout,
        &report.diagnostics,
        output_format,
        colored::control::SHOULD_COLORIZE.should_colorize(),
    )?;
    Ok(())
}

fn run_check_with_cwd(args: &CheckCommand, cwd: &Path, cache_root: &Path) -> Result<CheckReport> {
    if args.fix || args.unsafe_fixes {
        return Err(anyhow!(
            "--fix and --unsafe-fixes are not supported until the analyzer is wired"
        ));
    }

    let include_source = matches!(args.output_format, crate::args::CheckOutputFormatArg::Full);
    let settings = EffectiveCheckSettings::default();
    let runs = prepare_project_runs::<CheckCacheData, EffectiveCheckSettings, _>(
        &args.paths,
        cwd,
        &DiscoveryOptions {
            parallel: true,
            cache_root: Some(cache_root.to_path_buf()),
            ..DiscoveryOptions::default()
        },
        cache_root,
        args.no_cache,
        b"project-cache-key",
        |_| Ok(settings.clone()),
    )?;
    let base_linter_settings = LinterSettings::default();
    let shellcheck_map = ShellCheckCodeMap::default();

    let mut report = CheckReport::default();

    for mut run in runs {
        let analyzed_paths = run
            .files
            .iter()
            .map(|file| file.absolute_path.clone())
            .collect::<Vec<_>>();
        let pending = run.take_pending_files(|file, cached| {
            report.cache_hits += 1;
            match cached {
                CheckCacheData::Success(diagnostics) => {
                    let source = (include_source && !diagnostics.is_empty())
                        .then(|| read_shared_source(&file.absolute_path))
                        .transpose()?;
                    push_cached_lint_diagnostics(
                        &mut report,
                        &file.display_path,
                        &diagnostics,
                        source,
                    );
                }
                CheckCacheData::ParseError(error) => {
                    let source = include_source
                        .then(|| read_shared_source(&file.absolute_path))
                        .transpose()?;
                    report.diagnostics.push(DisplayedDiagnostic {
                        path: file.display_path,
                        span: DisplaySpan::point(error.line, error.column),
                        message: error.message,
                        kind: DisplayedDiagnosticKind::ParseError,
                        source,
                    });
                }
            }
            Ok(())
        })?;

        let results = pending
            .into_par_iter()
            .map(|pending| {
                analyze_file(
                    pending,
                    &base_linter_settings
                        .clone()
                        .with_analyzed_paths(analyzed_paths.clone()),
                    &shellcheck_map,
                    include_source,
                )
            })
            .collect::<Vec<_>>();

        for result in results {
            let result = result?;
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

fn analyze_file(
    pending: PendingProjectFile,
    base_linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    include_source: bool,
) -> Result<FileCheckResult> {
    let source = read_shared_source(&pending.file.absolute_path)?;
    let inferred_shell = ShellDialect::infer(&source, Some(&pending.file.absolute_path));
    let parse_dialect = match inferred_shell {
        ShellDialect::Sh | ShellDialect::Dash | ShellDialect::Ksh => {
            shuck_parser::ShellDialect::Posix
        }
        ShellDialect::Mksh => shuck_parser::ShellDialect::Mksh,
        ShellDialect::Zsh => shuck_parser::ShellDialect::Zsh,
        ShellDialect::Unknown | ShellDialect::Bash => shuck_parser::ShellDialect::Bash,
    };

    let linter_settings = base_linter_settings.clone().with_shell(inferred_shell);
    let parse_result = Parser::with_dialect(&source, parse_dialect).parse();
    let lint_result = lint_parsed_output(
        &pending,
        &source,
        &parse_result,
        &linter_settings,
        shellcheck_map,
        include_source,
    );
    let handled_parse_diagnostic = linter_settings
        .rules
        .contains(shuck_linter::Rule::MissingFi)
        && parse_diagnostics_include_missing_fi(&parse_result.diagnostics);
    let (cache_data, diagnostics) = if parse_result.status == ParseStatus::Fatal
        && lint_result.1.is_empty()
        && !handled_parse_diagnostic
    {
        let ParseError::Parse {
            message,
            line,
            column,
        } = parse_result.strict_error();
        (
            CheckCacheData::ParseError(ParseCacheFailure {
                message: message.clone(),
                line,
                column,
            }),
            vec![DisplayedDiagnostic {
                path: pending.file.display_path.clone(),
                span: DisplaySpan::point(line, column),
                message,
                kind: DisplayedDiagnosticKind::ParseError,
                source: include_source.then_some(source.clone()),
            }],
        )
    } else {
        lint_result
    };

    Ok(FileCheckResult {
        file: pending.file,
        file_key: pending.file_key,
        cache_data,
        diagnostics,
    })
}

fn lint_parsed_output(
    pending: &PendingProjectFile,
    source: &str,
    parse_result: &ParseResult,
    linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    include_source: bool,
) -> (CheckCacheData, Vec<DisplayedDiagnostic>) {
    let indexer = Indexer::new(source, parse_result);
    let directives = parse_directives(source, indexer.comment_index(), shellcheck_map);
    let suppression_index = (!directives.is_empty()).then(|| {
        SuppressionIndex::new(
            &directives,
            &parse_result.file,
            first_statement_line(&parse_result.file).unwrap_or(u32::MAX),
        )
    });
    let diagnostics = shuck_linter::lint_file_at_path_with_parse_result(
        parse_result,
        source,
        &indexer,
        linter_settings,
        suppression_index.as_ref(),
        Some(&pending.file.absolute_path),
    );
    let diagnostic_source =
        (!diagnostics.is_empty() && include_source).then(|| Arc::<str>::from(source));

    (
        CheckCacheData::Success(
            diagnostics
                .iter()
                .map(CachedLintDiagnostic::from_diagnostic)
                .collect(),
        ),
        diagnostics
            .iter()
            .map(|diagnostic| DisplayedDiagnostic {
                path: pending.file.display_path.clone(),
                span: DisplaySpan::new(
                    DisplayPosition::new(diagnostic.span.start.line, diagnostic.span.start.column),
                    DisplayPosition::new(diagnostic.span.end.line, diagnostic.span.end.column),
                ),
                message: diagnostic.message.clone(),
                kind: DisplayedDiagnosticKind::Lint {
                    code: diagnostic.code().to_owned(),
                    severity: diagnostic.severity.as_str().to_owned(),
                },
                source: diagnostic_source.clone(),
            })
            .collect(),
    )
}

fn parse_diagnostics_include_missing_fi(parse_diagnostics: &[ParseDiagnostic]) -> bool {
    parse_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.starts_with("expected 'fi'"))
}

fn push_cached_lint_diagnostics(
    report: &mut CheckReport,
    path: &Path,
    diagnostics: &[CachedLintDiagnostic],
    source: Option<Arc<str>>,
) {
    for diagnostic in diagnostics {
        report.diagnostics.push(DisplayedDiagnostic {
            path: path.to_path_buf(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.start_line, diagnostic.start_column),
                DisplayPosition::new(diagnostic.end_line, diagnostic.end_column),
            ),
            message: diagnostic.message.clone(),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code.clone(),
                severity: diagnostic.severity.clone(),
            },
            source: source.clone(),
        });
    }
}

fn read_shared_source(path: &Path) -> Result<Arc<str>> {
    Ok(Arc::<str>::from(fs::read_to_string(path)?))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use tempfile::tempdir;

    use super::*;
    use crate::args::CheckOutputFormatArg;

    fn cache_root(cwd: &Path) -> PathBuf {
        cwd.join("cache")
    }

    fn make_file_read_only(path: &Path) {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn check_args_with_format(no_cache: bool, output_format: CheckOutputFormatArg) -> CheckCommand {
        CheckCommand {
            fix: false,
            unsafe_fixes: false,
            no_cache,
            output_format,
            paths: Vec::new(),
        }
    }

    fn check_args(no_cache: bool) -> CheckCommand {
        check_args_with_format(no_cache, CheckOutputFormatArg::Full)
    }

    #[test]
    fn reports_parse_errors() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(false),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.exit_status(), ExitStatus::Failure);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
    }

    #[test]
    fn reports_missing_fi_as_c035_lint() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("broken.sh"),
            "#!/bin/sh\nif true; then\n  :\n",
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(false),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.diagnostics.len(), 1);
        match &report.diagnostics[0].kind {
            DisplayedDiagnosticKind::Lint { code, .. } => assert_eq!(code, "C035"),
            other => panic!("expected lint diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn reuses_cached_results() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let second = run_check_with_cwd(
            &check_args(false),
            tempdir.path(),
            &cache_root(tempdir.path()),
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

        let first = run_check_with_cwd(
            &check_args(false),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);

        fs::write(&script, "#!/bin/bash\nif true\n").unwrap();
        make_file_read_only(&script);

        let second = run_check_with_cwd(
            &check_args(false),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert_eq!(second.diagnostics.len(), 1);
    }

    #[test]
    fn no_cache_does_not_write_cache_files() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }

    #[test]
    fn infers_shell_from_extension_for_local_rule() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("posix.sh"), "local foo=bar\n").unwrap();
        fs::write(tempdir.path().join("bashy.bash"), "local foo=bar\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let c014 = report
            .diagnostics
            .iter()
            .filter(|diagnostic| matches!(&diagnostic.kind, DisplayedDiagnosticKind::Lint { code, .. } if code == "C014"))
            .collect::<Vec<_>>();

        assert_eq!(c014.len(), 1);
        assert_eq!(c014[0].path, PathBuf::from("bashy.bash"));
    }

    #[test]
    fn mixes_cache_hits_and_misses_in_a_single_run() {
        let tempdir = tempdir().unwrap();
        let first = tempdir.path().join("first.sh");
        let second = tempdir.path().join("second.sh");
        fs::write(&first, "#!/bin/bash\necho ok\n").unwrap();
        fs::write(&second, "#!/bin/bash\necho ok\n").unwrap();

        let cache_root = cache_root(tempdir.path());
        let initial = run_check_with_cwd(&check_args(false), tempdir.path(), &cache_root).unwrap();
        assert_eq!(initial.cache_hits, 0);
        assert_eq!(initial.cache_misses, 2);

        fs::write(&second, "#!/bin/bash\nif true\n").unwrap();

        let rerun = run_check_with_cwd(&check_args(false), tempdir.path(), &cache_root).unwrap();
        assert_eq!(rerun.cache_hits, 1);
        assert_eq!(rerun.cache_misses, 1);
        assert_eq!(rerun.diagnostics.len(), 1);
        assert_eq!(rerun.diagnostics[0].path, PathBuf::from("second.sh"));
    }

    #[test]
    fn sorts_diagnostics_deterministically_after_parallel_run() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("z.sh"), "#!/bin/bash\nif true\n").unwrap();
        fs::write(tempdir.path().join("a.bash"), "local foo=bar\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
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
        let report =
            run_check_with_cwd(&args, tempdir.path(), &cache_root(tempdir.path())).unwrap();

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

        let report = run_check_with_cwd(&check_args(false), tempdir.path(), &cache_root).unwrap();

        assert!(report.diagnostics.is_empty());
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }

    #[test]
    fn report_output_includes_ansi_styles_when_enabled() {
        colored::control::set_override(true);

        let report = CheckReport {
            diagnostics: vec![DisplayedDiagnostic {
                path: PathBuf::from("script.sh"),
                span: DisplaySpan::new(DisplayPosition::new(3, 14), DisplayPosition::new(3, 18)),
                message: "example message".to_owned(),
                kind: DisplayedDiagnosticKind::Lint {
                    code: "C014".to_owned(),
                    severity: "warning".to_owned(),
                },
                source: Some(Arc::<str>::from("echo ok\nvalue=$foo\nprintf '%s' $bar\n")),
            }],
            cache_hits: 0,
            cache_misses: 0,
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
            diagnostics: vec![DisplayedDiagnostic {
                path: PathBuf::from("script.sh"),
                span: DisplaySpan::point(2, 7),
                message: "unterminated construct".to_owned(),
                kind: DisplayedDiagnosticKind::ParseError,
                source: None,
            }],
            cache_hits: 0,
            cache_misses: 0,
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

    #[test]
    fn cached_diagnostics_retain_source_for_full_output() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("warn.sh"),
            "#!/bin/bash\nunused=1\necho ok\n",
        )
        .unwrap();

        let first = run_check_with_cwd(
            &check_args_with_format(false, CheckOutputFormatArg::Full),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let second = run_check_with_cwd(
            &check_args_with_format(false, CheckOutputFormatArg::Full),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(first.cache_misses, 1);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(second.diagnostics.len(), 1);
        assert_eq!(
            second.diagnostics[0].source.as_deref(),
            Some("#!/bin/bash\nunused=1\necho ok\n")
        );
    }
}
