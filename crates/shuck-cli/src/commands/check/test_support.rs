use std::fs;
use std::path::{Path, PathBuf};

use crate::args::{CheckCommand, CheckOutputFormatArg, FileSelectionArgs, RuleSelectionArgs};
use crate::commands::check::CheckReport;
use crate::commands::check::settings::CompiledPerFileShellList;
use crate::commands::check_output::{DisplaySpan, DisplayedDiagnostic, DisplayedDiagnosticKind};
use crate::commands::project_runner::PendingProjectFile;
use crate::discover::{FileKind, normalize_path};

pub(super) fn pending_project_file(path: &Path, project_root: &Path) -> PendingProjectFile {
    PendingProjectFile {
        file: crate::discover::DiscoveredFile {
            display_path: path.strip_prefix(project_root).unwrap().to_path_buf(),
            absolute_path: path.to_path_buf(),
            relative_path: path.strip_prefix(project_root).unwrap().to_path_buf(),
            project_root: crate::discover::ProjectRoot {
                storage_root: project_root.to_path_buf(),
                canonical_root: fs::canonicalize(project_root).unwrap(),
            },
            kind: FileKind::Shell,
        },
        file_key: shuck_cache::FileCacheKey::from_path(path).unwrap(),
    }
}

pub(super) fn empty_per_file_shell(project_root: &Path) -> CompiledPerFileShellList {
    CompiledPerFileShellList::resolve(project_root.to_path_buf(), []).unwrap()
}

pub(super) fn cache_root(cwd: &Path) -> PathBuf {
    cwd.join("cache")
}

fn diagnostic_paths(path: &str) -> (PathBuf, PathBuf, PathBuf) {
    let display = PathBuf::from(path);
    let relative = PathBuf::from(path);
    let absolute = PathBuf::from(format!("/tmp/{path}"));
    (display, relative, absolute)
}

pub(super) fn match_paths(canonical: &Path, resolved: &Path) -> Vec<PathBuf> {
    let mut paths = vec![canonical.to_path_buf(), normalize_path(resolved)];
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn watch_paths(canonical: &Path, resolved: &Path) -> Vec<PathBuf> {
    let mut paths = vec![canonical.to_path_buf(), normalize_path(resolved)];
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn make_file_read_only(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).unwrap();
}

pub(super) fn check_args_with_format(
    no_cache: bool,
    output_format: CheckOutputFormatArg,
) -> CheckCommand {
    CheckCommand {
        fix: false,
        unsafe_fixes: false,
        add_ignore: None,
        no_cache,
        output_format,
        watch: false,
        paths: Vec::new(),
        rule_selection: RuleSelectionArgs::default(),
        file_selection: FileSelectionArgs::default(),
        exit_zero: false,
        exit_non_zero_on_fix: false,
    }
}

pub(super) fn check_args(no_cache: bool) -> CheckCommand {
    check_args_with_format(no_cache, CheckOutputFormatArg::Full)
}

pub(super) fn lint_displayed_diagnostic(
    path: &str,
    span: DisplaySpan,
    message: &str,
    code: &str,
    severity: &str,
) -> DisplayedDiagnostic {
    let (path, relative_path, absolute_path) = diagnostic_paths(path);
    DisplayedDiagnostic {
        path,
        relative_path,
        absolute_path,
        span,
        message: message.to_owned(),
        kind: DisplayedDiagnosticKind::Lint {
            code: code.to_owned(),
            severity: severity.to_owned(),
        },
        fix: None,
        source: None,
    }
}

pub(super) fn parse_displayed_diagnostic(
    path: &str,
    span: DisplaySpan,
    message: &str,
) -> DisplayedDiagnostic {
    let (path, relative_path, absolute_path) = diagnostic_paths(path);
    DisplayedDiagnostic {
        path,
        relative_path,
        absolute_path,
        span,
        message: message.to_owned(),
        kind: DisplayedDiagnosticKind::ParseError,
        fix: None,
        source: None,
    }
}

pub(super) fn diagnostic_codes(report: &CheckReport) -> Vec<String> {
    report
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match &diagnostic.kind {
            DisplayedDiagnosticKind::Lint { code, .. } => Some(code.clone()),
            DisplayedDiagnosticKind::ParseError => None,
        })
        .collect()
}
