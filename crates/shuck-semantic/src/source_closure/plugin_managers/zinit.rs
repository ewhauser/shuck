//! Zinit/Zi source layout support.
//!
//! Zinit's plugin syntax does not currently map to a single local plugin
//! entrypoint, but its bootstrap scripts have stable framework-relative paths.

use super::super::*;
use super::*;

pub(super) struct ZinitPluginManager;

impl ZshPluginManager for ZinitPluginManager {}

impl ZshPluginFramework for ZinitPluginManager {
    fn framework(&self) -> PluginFramework {
        PluginFramework::Zinit
    }

    fn root_keys(&self) -> &'static [&'static str] {
        &["zinit", "zi"]
    }

    fn resolve_plugin_entrypoint(&self, _root: &Path, _name: &str) -> Option<PathBuf> {
        None
    }

    fn resolve_theme_entrypoint(&self, _root: &Path, _name: &str) -> Option<PathBuf> {
        None
    }

    fn resolve_source_suffix(
        &self,
        root: &Path,
        source_path: &Path,
        candidate: &str,
    ) -> Option<PathBuf> {
        let normalized = candidate.replace('\\', "/");
        let source_in_framework = source_path.starts_with(root);
        let candidate_has_framework_anchor = path_text_has_zinit_anchor(&normalized)
            || path_text_starts_with_path(&normalized, root);
        if !source_in_framework && !candidate_has_framework_anchor && normalized != "zinit.zsh" {
            return None;
        }

        suffix_after_last_marker(&normalized, ["/share/", "/doc/"])
            .or_else(|| zinit_builtin_source_suffix(&normalized).map(PathBuf::from))
    }
}

fn path_text_has_zinit_anchor(path: &str) -> bool {
    path.contains("/.zinit/")
        || path.contains("/zinit/")
        || path.contains("/.zi/")
        || path.contains("/zi/")
}

fn zinit_builtin_source_suffix(path: &str) -> Option<&'static str> {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    match file_name {
        "zinit.zsh" => Some("zinit.zsh"),
        "zinit-additional.zsh" => Some("zinit-additional.zsh"),
        "zinit-autoload.zsh" => Some("zinit-autoload.zsh"),
        "zinit-install.zsh" => Some("zinit-install.zsh"),
        "zinit-side.zsh" => Some("zinit-side.zsh"),
        _ => None,
    }
}
