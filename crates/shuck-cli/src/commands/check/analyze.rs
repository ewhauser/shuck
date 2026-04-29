use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use shuck_indexer::Indexer;
use shuck_linter::{Applicability, LinterSettings, RuleSet, ShellCheckCodeMap, ShellDialect};
use shuck_parser::{
    Error as ParseError,
    parser::{ParseResult, Parser},
};

use super::cache::CheckCacheData;
use super::display::{display_lint_diagnostics, display_parse_error};
use super::embedded::analyze_embedded_file;
use super::settings::CompiledPerFileShellList;
use crate::commands::check_output::DisplayedDiagnostic;
use crate::commands::project_runner::PendingProjectFile;
use crate::discover::FileKind;

#[derive(Debug, Clone)]
pub(super) struct FileCheckResult {
    pub(super) file: crate::discover::DiscoveredFile,
    pub(super) file_key: shuck_cache::FileCacheKey,
    pub(super) cache_data: CheckCacheData,
    pub(super) diagnostics: Vec<DisplayedDiagnostic>,
    pub(super) fixes_applied: usize,
    pub(super) parse_failed: bool,
}
pub(super) fn analyze_file(
    pending: PendingProjectFile,
    base_linter_settings: &LinterSettings,
    per_file_shell: &CompiledPerFileShellList,
    shellcheck_map: &ShellCheckCodeMap,
    include_source: bool,
    fix_applicability: Option<Applicability>,
    fixable_rules: &RuleSet,
) -> Result<FileCheckResult> {
    match pending.file.kind {
        FileKind::Shell => analyze_shell_file(
            pending,
            base_linter_settings,
            per_file_shell,
            shellcheck_map,
            include_source,
            fix_applicability,
            fixable_rules,
        ),
        FileKind::Embedded => analyze_embedded_file(
            pending,
            base_linter_settings,
            shellcheck_map,
            include_source,
        ),
    }
}

fn analyze_shell_file(
    pending: PendingProjectFile,
    base_linter_settings: &LinterSettings,
    per_file_shell: &CompiledPerFileShellList,
    shellcheck_map: &ShellCheckCodeMap,
    include_source: bool,
    fix_applicability: Option<Applicability>,
    fixable_rules: &RuleSet,
) -> Result<FileCheckResult> {
    let mut source = read_shared_source(&pending.file.absolute_path)?;
    let inferred_shell = per_file_shell
        .shell_for_path(&pending.file.absolute_path)
        .unwrap_or_else(|| ShellDialect::infer(&source, Some(&pending.file.absolute_path)));
    let parse_dialect = inferred_shell.parser_dialect();

    let linter_settings = base_linter_settings.clone().with_shell(inferred_shell);
    let mut parse_result = Parser::with_dialect(&source, parse_dialect).parse();
    let mut diagnostics = collect_lint_diagnostics(
        &pending,
        &source,
        &parse_result,
        &linter_settings,
        shellcheck_map,
        &pending.file.absolute_path,
    );
    let mut fixes_applied = 0;

    if let Some(applicability) = fix_applicability {
        let fixable_diagnostics = diagnostics
            .iter()
            .filter(|diagnostic| fixable_rules.contains(diagnostic.rule))
            .cloned()
            .collect::<Vec<_>>();
        let applied = shuck_linter::apply_fixes(&source, &fixable_diagnostics, applicability);
        if applied.fixes_applied > 0 {
            source = Arc::<str>::from(applied.code);
            fs::write(&pending.file.absolute_path, &*source)?;
            parse_result = Parser::with_dialect(&source, parse_dialect).parse();
            diagnostics = collect_lint_diagnostics(
                &pending,
                &source,
                &parse_result,
                &linter_settings,
                shellcheck_map,
                &pending.file.absolute_path,
            );
            fixes_applied = applied.fixes_applied;
        }
    }

    let parse_failed = parse_result.is_err();
    let diagnostics = if parse_failed && diagnostics.is_empty() {
        let ParseError::Parse {
            message,
            line,
            column,
        } = parse_result.strict_error();
        vec![display_parse_error(
            &pending.file.display_path,
            &pending.file.relative_path,
            &pending.file.absolute_path,
            line,
            column,
            message,
            include_source.then_some(source.clone()),
        )]
    } else {
        display_lint_diagnostics(&pending, &source, &diagnostics, include_source)
    };
    let cache_data = CheckCacheData::from_displayed(&diagnostics, parse_failed);

    Ok(FileCheckResult {
        file: pending.file,
        file_key: pending.file_key,
        cache_data,
        diagnostics,
        fixes_applied,
        parse_failed,
    })
}
pub(super) fn collect_lint_diagnostics(
    _pending: &PendingProjectFile,
    source: &Arc<str>,
    parse_result: &ParseResult,
    linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    source_path: &Path,
) -> Vec<shuck_linter::Diagnostic> {
    let indexer = Indexer::new(source, parse_result);
    shuck_linter::lint_file(
        parse_result,
        source,
        &indexer,
        linter_settings,
        shellcheck_map,
        Some(source_path),
    )
}
pub(super) fn read_shared_source(path: &Path) -> Result<Arc<str>> {
    Ok(Arc::<str>::from(fs::read_to_string(path)?))
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
    fn reports_parse_errors() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.exit_status(false, false), ExitStatus::Failure);
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
            &ConfigArguments::default(),
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
    fn ignore_can_trigger_parse_error_fallback() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("broken.sh"),
            "#!/bin/sh\nif true; then\n  :\n",
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            ignore: vec![RuleSelector::Rule(Rule::MissingFi)],
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.diagnostics.len(), 1);
        assert!(matches!(
            report.diagnostics[0].kind,
            DisplayedDiagnosticKind::ParseError
        ));
    }

    #[test]
    fn reports_missing_fi_as_parse_error_when_parse_rule_is_disabled() {
        let tempdir = tempdir().unwrap();
        let broken_path = tempdir.path().join("broken.sh");
        fs::write(&broken_path, "#!/bin/sh\nif true; then\n  :\n").unwrap();

        let result = analyze_file(
            pending_project_file(&broken_path, tempdir.path()),
            &LinterSettings::for_rule(shuck_linter::Rule::UnusedAssignment)
                .with_analyzed_paths([broken_path.clone()]),
            &empty_per_file_shell(tempdir.path()),
            &ShellCheckCodeMap::default(),
            false,
            None,
            &RuleSet::all(),
        )
        .unwrap();

        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.cache_data.diagnostics.len(), 1);
        assert!(matches!(
            result.cache_data.diagnostics[0].kind,
            CachedDisplayedDiagnosticKind::ParseError
        ));
        match &result.diagnostics[0].kind {
            DisplayedDiagnosticKind::ParseError => {}
            other => panic!("expected parse error, got {other:?}"),
        }
        assert!(result.diagnostics[0].message.contains("expected 'fi'"));
    }

    #[test]
    fn infers_shell_from_extension_for_local_rule() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("posix.sh"), "local foo=bar\n").unwrap();
        fs::write(tempdir.path().join("bashy.bash"), "local foo=bar\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
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
    fn lint_diagnostics_share_the_original_source_arc_for_full_output() {
        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("warn.sh");
        fs::write(&path, "#!/bin/bash\nunused=1\necho ok\n").unwrap();

        let pending = pending_project_file(&path, tempdir.path());
        let source = read_shared_source(&path).unwrap();
        let parse_result = Parser::with_dialect(&source, shuck_parser::ShellDialect::Bash).parse();

        let diagnostics = collect_lint_diagnostics(
            &pending,
            &source,
            &parse_result,
            &LinterSettings::default(),
            &ShellCheckCodeMap::default(),
            &path,
        );
        let diagnostics = display_lint_diagnostics(&pending, &source, &diagnostics, true);

        let diagnostic_source = diagnostics[0]
            .source
            .as_ref()
            .expect("full output should retain source");
        assert!(Arc::ptr_eq(diagnostic_source, &source));
    }
}
