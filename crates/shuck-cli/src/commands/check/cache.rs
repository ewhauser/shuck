use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use shuck_cache::{CacheKey, CacheKeyHasher};
use shuck_linter::Applicability;

use super::CheckReport;
use super::settings::{EffectiveCheckSettings, ResolvedCheckSettings};
use crate::commands::check_output::{
    DisplayPosition, DisplaySpan, DisplayedApplicability, DisplayedDiagnostic,
    DisplayedDiagnosticKind, DisplayedEdit, DisplayedFix,
};
use crate::discover::{DiscoveredFile, FileKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CheckCacheSettings {
    effective: EffectiveCheckSettings,
    analyzed_paths: Vec<PathBuf>,
}

impl CheckCacheSettings {
    pub(super) fn new(settings: &ResolvedCheckSettings, files: &[DiscoveredFile]) -> Self {
        Self {
            effective: settings.effective.clone(),
            analyzed_paths: analyzed_shell_relative_paths(files),
        }
    }
}

impl CacheKey for CheckCacheSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"check-cache-settings");
        self.effective.cache_key(state);
        self.analyzed_paths.cache_key(state);
    }
}

fn analyzed_shell_relative_paths(files: &[DiscoveredFile]) -> Vec<PathBuf> {
    let mut paths = files
        .iter()
        .filter(|file| file.kind == FileKind::Shell)
        .map(|file| file.relative_path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CheckCacheData {
    pub(super) diagnostics: Vec<CachedDisplayedDiagnostic>,
    #[serde(default)]
    pub(super) parse_failed: bool,
}

impl CheckCacheData {
    pub(super) fn from_displayed(diagnostics: &[DisplayedDiagnostic], parse_failed: bool) -> Self {
        Self {
            diagnostics: diagnostics
                .iter()
                .map(CachedDisplayedDiagnostic::from_displayed)
                .collect(),
            parse_failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum CachedDisplayedDiagnosticKind {
    ParseError,
    Lint { code: String, severity: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CachedDisplayedDiagnostic {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    message: String,
    pub(super) kind: CachedDisplayedDiagnosticKind,
    fix: Option<CachedLintFix>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum CachedApplicability {
    Safe,
    Unsafe,
}

impl From<Applicability> for CachedApplicability {
    fn from(value: Applicability) -> Self {
        match value {
            Applicability::Safe => Self::Safe,
            Applicability::Unsafe => Self::Unsafe,
        }
    }
}

impl From<CachedApplicability> for DisplayedApplicability {
    fn from(value: CachedApplicability) -> Self {
        match value {
            CachedApplicability::Safe => Self::Safe,
            CachedApplicability::Unsafe => Self::Unsafe,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedLintFix {
    applicability: CachedApplicability,
    message: Option<String>,
    edits: Vec<CachedLintEdit>,
}

impl CachedLintFix {
    fn from_displayed(fix: &DisplayedFix) -> Self {
        Self {
            applicability: match fix.applicability {
                DisplayedApplicability::Safe => CachedApplicability::Safe,
                DisplayedApplicability::Unsafe => CachedApplicability::Unsafe,
            },
            message: fix.message.clone(),
            edits: fix
                .edits
                .iter()
                .map(CachedLintEdit::from_displayed)
                .collect(),
        }
    }

    fn to_displayed(&self) -> DisplayedFix {
        DisplayedFix {
            applicability: self.applicability.into(),
            message: self.message.clone(),
            edits: self
                .edits
                .iter()
                .map(CachedLintEdit::to_displayed)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedLintEdit {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    content: String,
}

impl CachedLintEdit {
    fn from_displayed(edit: &DisplayedEdit) -> Self {
        Self {
            start_line: edit.location.line,
            start_column: edit.location.column,
            end_line: edit.end_location.line,
            end_column: edit.end_location.column,
            content: edit.content.clone(),
        }
    }

    fn to_displayed(&self) -> DisplayedEdit {
        DisplayedEdit {
            location: DisplayPosition::new(self.start_line, self.start_column),
            end_location: DisplayPosition::new(self.end_line, self.end_column),
            content: self.content.clone(),
        }
    }
}

impl CachedDisplayedDiagnostic {
    fn from_displayed(diagnostic: &DisplayedDiagnostic) -> Self {
        Self {
            start_line: diagnostic.span.start.line,
            start_column: diagnostic.span.start.column,
            end_line: diagnostic.span.end.line,
            end_column: diagnostic.span.end.column,
            message: diagnostic.message.clone(),
            kind: match &diagnostic.kind {
                DisplayedDiagnosticKind::ParseError => CachedDisplayedDiagnosticKind::ParseError,
                DisplayedDiagnosticKind::Lint { code, severity } => {
                    CachedDisplayedDiagnosticKind::Lint {
                        code: code.clone(),
                        severity: severity.clone(),
                    }
                }
            },
            fix: diagnostic.fix.as_ref().map(CachedLintFix::from_displayed),
        }
    }
}
pub(super) fn push_cached_diagnostics(
    report: &mut CheckReport,
    path: &Path,
    relative_path: &Path,
    absolute_path: &Path,
    diagnostics: &[CachedDisplayedDiagnostic],
    source: Option<Arc<str>>,
) {
    for diagnostic in diagnostics {
        report.diagnostics.push(DisplayedDiagnostic {
            path: path.to_path_buf(),
            relative_path: relative_path.to_path_buf(),
            absolute_path: absolute_path.to_path_buf(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.start_line, diagnostic.start_column),
                DisplayPosition::new(diagnostic.end_line, diagnostic.end_column),
            ),
            message: diagnostic.message.clone(),
            kind: match &diagnostic.kind {
                CachedDisplayedDiagnosticKind::ParseError => DisplayedDiagnosticKind::ParseError,
                CachedDisplayedDiagnosticKind::Lint { code, severity } => {
                    DisplayedDiagnosticKind::Lint {
                        code: code.clone(),
                        severity: severity.clone(),
                    }
                }
            },
            fix: diagnostic.fix.as_ref().map(CachedLintFix::to_displayed),
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
    use crate::discover::{FileKind, normalize_path};
    use shuck_config::ConfigArguments;

    #[test]
    fn reuses_cached_results() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let second = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
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
    fn cache_key_includes_analyzed_path_set() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("main.sh"),
            "#!/bin/sh\n. ./helper.sh\nprintf '%s\\n' \"$from_helper\"\n",
        )
        .unwrap();
        fs::write(
            tempdir.path().join("helper.sh"),
            "#!/bin/sh\nfrom_helper=ok\n",
        )
        .unwrap();

        let mut narrow_args = check_args(false);
        narrow_args.paths = vec![PathBuf::from("main.sh")];
        narrow_args.rule_selection.select =
            Some(vec![RuleSelector::Rule(Rule::UntrackedSourceFile)]);

        let mut broad_args = narrow_args.clone();
        broad_args.paths = vec![PathBuf::from("main.sh"), PathBuf::from("helper.sh")];

        let narrow = run_check_with_cwd(
            &narrow_args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(narrow.cache_hits, 0);
        assert_eq!(narrow.cache_misses, 1);
        assert_eq!(diagnostic_codes(&narrow), vec!["C003"]);

        let broad = run_check_with_cwd(
            &broad_args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(broad.cache_hits, 0);
        assert_eq!(broad.cache_misses, 2);
        assert!(
            broad.diagnostics.is_empty(),
            "{:?}",
            diagnostic_codes(&broad)
        );

        let broad_again = run_check_with_cwd(
            &broad_args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(broad_again.cache_hits, 2);
        assert_eq!(broad_again.cache_misses, 0);
        assert!(
            broad_again.diagnostics.is_empty(),
            "{:?}",
            diagnostic_codes(&broad_again)
        );
    }

    #[test]
    fn invalidates_cache_when_file_changes() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        fs::write(&script, "#!/bin/bash\necho ok\n").unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
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
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert_eq!(second.diagnostics.len(), 1);
    }

    #[test]
    fn invalidates_cache_when_rule_options_change() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        fs::write(
            &script,
            "#!/bin/bash\ntarget=ok\nname=target\nprintf '%s\\n' \"${!name}\"\n",
        )
        .unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C001']\n",
        )
        .unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);
        assert_eq!(first.diagnostics.len(), 1);
        assert!(first.diagnostics[0].message.contains("target"));

        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C001']\n\n[lint.rule-options.c001]\ntreat-indirect-expansion-targets-as-used = true\n",
        )
        .unwrap();

        let second = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert!(second.diagnostics.is_empty());
    }

    #[test]
    fn invalidates_cache_when_c063_rule_options_change() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        fs::write(&script, "#!/bin/bash\nouter() {\n  inner() { :; }\n}\n").unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C063']\n",
        )
        .unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);
        assert!(first.diagnostics.is_empty());

        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C063']\n\n[lint.rule-options.c063]\nreport-unreached-nested-definitions = true\n",
        )
        .unwrap();

        let second = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert_eq!(second.diagnostics.len(), 1);
    }

    #[test]
    fn mixes_cache_hits_and_misses_in_a_single_run() {
        let tempdir = tempdir().unwrap();
        let first = tempdir.path().join("first.sh");
        let second = tempdir.path().join("second.sh");
        fs::write(&first, "#!/bin/bash\necho ok\n").unwrap();
        fs::write(&second, "#!/bin/bash\necho ok\n").unwrap();

        let cache_root = cache_root(tempdir.path());
        let initial = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root,
        )
        .unwrap();
        assert_eq!(initial.cache_hits, 0);
        assert_eq!(initial.cache_misses, 2);

        fs::write(&second, "#!/bin/bash\nif true\n").unwrap();

        let rerun = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root,
        )
        .unwrap();
        assert_eq!(rerun.cache_hits, 1);
        assert_eq!(rerun.cache_misses, 1);
        assert_eq!(rerun.diagnostics.len(), 1);
        assert_eq!(rerun.diagnostics[0].path, PathBuf::from("second.sh"));
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
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let second = run_check_with_cwd(
            &check_args_with_format(false, CheckOutputFormatArg::Full),
            &ConfigArguments::default(),
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
