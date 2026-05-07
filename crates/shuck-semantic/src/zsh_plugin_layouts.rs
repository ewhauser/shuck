//! Filesystem layouts for supported zsh plugin frameworks.
//!
//! Source-closure plugin managers detect logical requests such as "load the
//! Prezto editor module." Layouts keep the corresponding path conventions next
//! to that plugin knowledge so CLI configuration only needs to supply roots.

use std::path::{Path, PathBuf};

use crate::{PluginFramework, PluginRequestKind};

/// Filesystem contract for a zsh plugin framework.
pub trait ZshPluginLayout: Sync {
    /// Framework represented by this layout.
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

struct OhMyZshLayout;
struct PreztoLayout;
struct ZdotLayout;
struct ZinitLayout;

static OH_MY_ZSH_LAYOUT: OhMyZshLayout = OhMyZshLayout;
static PREZTO_LAYOUT: PreztoLayout = PreztoLayout;
static ZDOT_LAYOUT: ZdotLayout = ZdotLayout;
static ZINIT_LAYOUT: ZinitLayout = ZinitLayout;
static ZSH_PLUGIN_LAYOUTS: [&dyn ZshPluginLayout; 4] = [
    &OH_MY_ZSH_LAYOUT,
    &PREZTO_LAYOUT,
    &ZDOT_LAYOUT,
    &ZINIT_LAYOUT,
];

/// Returns all built-in zsh plugin layouts.
pub fn zsh_plugin_layouts() -> &'static [&'static dyn ZshPluginLayout] {
    &ZSH_PLUGIN_LAYOUTS
}

/// Returns the built-in layout for a framework, when Shuck knows one.
pub fn layout_for_plugin_framework(
    framework: &PluginFramework,
) -> Option<&'static dyn ZshPluginLayout> {
    zsh_plugin_layouts()
        .iter()
        .copied()
        .find(|layout| &layout.framework() == framework)
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

impl ZshPluginLayout for OhMyZshLayout {
    fn framework(&self) -> PluginFramework {
        PluginFramework::OhMyZsh
    }

    fn root_keys(&self) -> &'static [&'static str] {
        &["oh-my-zsh"]
    }

    fn resolve_plugin_entrypoint(&self, root: &Path, name: &str) -> Option<PathBuf> {
        Some(
            root.join("plugins")
                .join(name)
                .join(format!("{name}.plugin.zsh")),
        )
    }

    fn resolve_theme_entrypoint(&self, root: &Path, name: &str) -> Option<PathBuf> {
        Some(root.join("themes").join(format!("{name}.zsh-theme")))
    }

    fn resolve_source_suffix(
        &self,
        root: &Path,
        source_path: &Path,
        candidate: &str,
    ) -> Option<PathBuf> {
        let normalized = candidate.replace('\\', "/");
        if normalized == "oh-my-zsh.sh" || normalized.ends_with("/oh-my-zsh.sh") {
            return Some(PathBuf::from("oh-my-zsh.sh"));
        }

        let source_in_framework = source_path.starts_with(root);
        let candidate_has_framework_anchor = path_text_has_oh_my_zsh_anchor(&normalized)
            || path_text_starts_with_path(&normalized, root);
        if !source_in_framework && !candidate_has_framework_anchor {
            return None;
        }

        suffix_after_last_marker(&normalized, ["/plugins/", "/themes/", "/lib/"])
    }
}

impl ZshPluginLayout for PreztoLayout {
    fn framework(&self) -> PluginFramework {
        PluginFramework::Prezto
    }

    fn root_keys(&self) -> &'static [&'static str] {
        &["prezto"]
    }

    fn resolve_plugin_entrypoint(&self, root: &Path, name: &str) -> Option<PathBuf> {
        Some(root.join("modules").join(name).join("init.zsh"))
    }

    fn resolve_theme_entrypoint(&self, _root: &Path, _name: &str) -> Option<PathBuf> {
        None
    }

    fn resolve_source_suffix(
        &self,
        _root: &Path,
        _source_path: &Path,
        _candidate: &str,
    ) -> Option<PathBuf> {
        None
    }
}

impl ZshPluginLayout for ZdotLayout {
    fn framework(&self) -> PluginFramework {
        PluginFramework::Zdot
    }

    fn root_keys(&self) -> &'static [&'static str] {
        &["zdot"]
    }

    fn resolve_plugin_entrypoint(&self, root: &Path, name: &str) -> Option<PathBuf> {
        Some(root.join("modules").join(name).join(format!("{name}.zsh")))
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
        if normalized == "zdot.zsh" || normalized.ends_with("/zdot.zsh") {
            return Some(PathBuf::from("zdot.zsh"));
        }

        let source_in_framework = source_path.starts_with(root);
        let candidate_has_framework_anchor =
            path_text_has_zdot_anchor(&normalized) || path_text_starts_with_path(&normalized, root);
        if !source_in_framework && !candidate_has_framework_anchor {
            return None;
        }

        suffix_after_last_marker(&normalized, ["/core/", "/modules/"])
    }
}

impl ZshPluginLayout for ZinitLayout {
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

fn suffix_after_last_marker<const N: usize>(path: &str, markers: [&str; N]) -> Option<PathBuf> {
    for marker in markers {
        if let Some(index) = path.rfind(marker) {
            let suffix = &path[index + 1..];
            if !suffix.is_empty() {
                return Some(PathBuf::from(suffix));
            }
        }
    }
    None
}

fn path_text_has_oh_my_zsh_anchor(path: &str) -> bool {
    path.contains("/.oh-my-zsh/") || path.contains("/oh-my-zsh/")
}

fn path_text_has_zdot_anchor(path: &str) -> bool {
    path.contains("/.zdot/") || path.contains("/zdot/")
}

fn path_text_has_zinit_anchor(path: &str) -> bool {
    path.contains("/.zinit/")
        || path.contains("/zinit/")
        || path.contains("/.zi/")
        || path.contains("/zi/")
}

fn path_text_starts_with_path(path: &str, root: &Path) -> bool {
    let root_text = root.to_string_lossy().replace('\\', "/");
    path == root_text
        || path
            .strip_prefix(&root_text)
            .is_some_and(|tail| tail.starts_with('/'))
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
