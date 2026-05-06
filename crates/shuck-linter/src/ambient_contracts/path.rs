//! Path helpers shared by ambient contract providers.
//!
//! These helpers intentionally operate on a lowercased display path. The
//! providers are heuristics for repository layout conventions, not filesystem
//! identity checks.

use std::path::Path;

pub(super) fn lower_path(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

pub(super) fn path_matches_any(lower_path: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| lower_path.contains(pattern))
}

pub(super) fn path_has_component(lower_path: &str, names: &[&str]) -> bool {
    lower_path
        .split('/')
        .any(|component| names.contains(&component))
}

pub(super) fn path_file_name(lower_path: &str) -> &str {
    lower_path.rsplit('/').next().unwrap_or(lower_path)
}
