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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_keys_include_zinit_and_zi_aliases() {
        assert_eq!(ZinitPluginManager.root_keys(), &["zinit", "zi"]);
    }

    #[test]
    fn does_not_resolve_plugins_or_themes_to_oh_my_zsh_layouts() {
        let root = Path::new("/workspace/zinit");

        assert_eq!(
            ZinitPluginManager.resolve_plugin_entrypoint(root, "owner/repo"),
            None
        );
        assert_eq!(
            ZinitPluginManager.resolve_entrypoint(root, PluginRequestKind::Plugin, "owner/repo"),
            None
        );
        assert_eq!(
            ZinitPluginManager.resolve_theme_entrypoint(root, "agnoster"),
            None
        );
        assert_eq!(
            ZinitPluginManager.resolve_entrypoint(root, PluginRequestKind::Theme, "agnoster"),
            None
        );
    }

    #[test]
    fn resolves_bootstrap_source_suffixes_for_zinit_and_zi_anchors() {
        assert_eq!(
            ZinitPluginManager.resolve_source_suffix(
                Path::new("/workspace/zinit"),
                Path::new("/workspace/app/.zshrc"),
                "/not-installed/.zinit/bin/zinit.zsh",
            ),
            Some(PathBuf::from("zinit.zsh"))
        );
        assert_eq!(
            ZinitPluginManager.resolve_source_suffix(
                Path::new("/workspace/zi"),
                Path::new("/workspace/app/.zshrc"),
                "/not-installed/.zi/bin/zinit.zsh",
            ),
            Some(PathBuf::from("zinit.zsh"))
        );
    }

    #[test]
    fn resolves_framework_relative_share_and_doc_sources_only_inside_framework() {
        let root = Path::new("/workspace/zinit");

        assert_eq!(
            ZinitPluginManager.resolve_source_suffix(
                root,
                Path::new("/workspace/zinit/zinit.zsh"),
                "/opt/app/share/zinit-autoload.zsh",
            ),
            Some(PathBuf::from("share/zinit-autoload.zsh"))
        );
        assert_eq!(
            ZinitPluginManager.resolve_source_suffix(
                root,
                Path::new("/workspace/zinit/zinit.zsh"),
                "/opt/app/doc/zinit.1",
            ),
            Some(PathBuf::from("doc/zinit.1"))
        );
        assert_eq!(
            ZinitPluginManager.resolve_source_suffix(
                root,
                Path::new("/workspace/app/.zshrc"),
                "/opt/app/share/zinit-autoload.zsh",
            ),
            None
        );
    }

    #[test]
    fn resolves_known_builtin_script_names_when_framework_is_active() {
        let root = Path::new("/workspace/zinit");
        let source_path = Path::new("/workspace/zinit/zinit.zsh");

        for script in [
            "zinit.zsh",
            "zinit-additional.zsh",
            "zinit-autoload.zsh",
            "zinit-install.zsh",
            "zinit-side.zsh",
        ] {
            assert_eq!(
                ZinitPluginManager.resolve_source_suffix(root, source_path, script),
                Some(PathBuf::from(script)),
                "script: {script}"
            );
        }
    }
}
