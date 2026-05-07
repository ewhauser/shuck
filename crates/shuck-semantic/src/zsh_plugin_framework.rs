//! Common contract for supported zsh plugin frameworks.
//!
//! Framework-specific source discovery lives in `source_closure`, but every
//! built-in framework also has to expose its filesystem layout through this
//! trait. Keeping those methods on the same implementation prevents adding a
//! detector without deciding how its requests resolve to files.

use std::path::{Path, PathBuf};

use crate::{PluginFramework, PluginRequestKind};

/// Source and filesystem contract for a zsh plugin framework.
pub trait ZshPluginFramework: Sync {
    /// Framework represented by this implementation.
    fn framework(&self) -> PluginFramework;

    /// Configuration keys that can name roots for this framework.
    fn root_keys(&self) -> &'static [&'static str];

    /// Resolve a logical plugin/module name beneath a configured framework root.
    fn resolve_plugin_entrypoint(&self, root: &Path, name: &str) -> Option<PathBuf>;

    /// Resolve a logical theme name beneath a configured framework root.
    fn resolve_theme_entrypoint(&self, root: &Path, name: &str) -> Option<PathBuf>;

    /// Resolve a sourced candidate path to a suffix beneath a configured root.
    fn resolve_source_suffix(
        &self,
        root: &Path,
        source_path: &Path,
        candidate: &str,
    ) -> Option<PathBuf>;

    /// Resolve an entrypoint for a request kind beneath a configured root.
    fn resolve_entrypoint(
        &self,
        root: &Path,
        kind: PluginRequestKind,
        name: &str,
    ) -> Option<PathBuf> {
        match kind {
            PluginRequestKind::Plugin => self.resolve_plugin_entrypoint(root, name),
            PluginRequestKind::Theme => self.resolve_theme_entrypoint(root, name),
            PluginRequestKind::Entrypoint => Some(PathBuf::from(name)),
        }
    }
}

/// Converts a user-facing framework name or alias into a plugin framework.
pub fn zsh_plugin_framework_from_name(name: &str) -> PluginFramework {
    match name {
        "oh-my-zsh" => PluginFramework::OhMyZsh,
        "prezto" => PluginFramework::Prezto,
        "zdot" => PluginFramework::Zdot,
        "zinit" | "zi" => PluginFramework::Zinit,
        other => PluginFramework::Other(other.to_owned()),
    }
}
