//! zdot module request discovery.
//!
//! zdot resolves logical module names through `zdot_load_module`, then sources
//! `modules/<name>/<name>.zsh` from its module search path. This adapter only
//! records static module loads; dynamic cache and hook execution stays in the
//! generic zsh/runtime layer.

use super::super::*;
use super::*;

pub(super) struct ZdotPluginManager;

impl ZshPluginManager for ZdotPluginManager {
    fn collect_plugin_requests(&self, context: &PluginManagerContext<'_>) -> Vec<PluginRequest> {
        collect_zdot_plugin_requests(context)
    }
}

impl ZshPluginFramework for ZdotPluginManager {
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

fn path_text_has_zdot_anchor(path: &str) -> bool {
    path.contains("/.zdot/") || path.contains("/zdot/")
}

fn collect_zdot_plugin_requests(context: &PluginManagerContext<'_>) -> Vec<PluginRequest> {
    let mut requests = Vec::new();
    for stmt in &context.file.body.stmts {
        let Command::Simple(command) = &stmt.command else {
            continue;
        };
        if static_word_text(&command.name, context.source).as_deref() != Some("zdot_load_module") {
            continue;
        }
        let args = static_command_args(command, context.source);
        let Some(args) = args.as_deref() else {
            continue;
        };
        let modules = static_plugin_names(
            args.iter()
                .filter(|arg| !arg.starts_with('-'))
                .map(String::as_str),
        );
        requests.extend(modules.into_iter().map(|module| PluginRequest {
            framework: PluginFramework::Zdot,
            kind: PluginRequestKind::Plugin,
            name: module,
            span: stmt.span,
            explicit: false,
            root_hint: None,
        }));
    }
    requests
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_keys_match_config_name() {
        assert_eq!(ZdotPluginManager.root_keys(), &["zdot"]);
    }

    #[test]
    fn resolves_module_entrypoint() {
        let root = Path::new("/workspace/zdot");

        assert_eq!(
            ZdotPluginManager.resolve_plugin_entrypoint(root, "fzf"),
            Some(PathBuf::from("/workspace/zdot/modules/fzf/fzf.zsh"))
        );
        assert_eq!(
            ZdotPluginManager.resolve_entrypoint(root, PluginRequestKind::Plugin, "tmux"),
            Some(PathBuf::from("/workspace/zdot/modules/tmux/tmux.zsh"))
        );
    }

    #[test]
    fn does_not_resolve_themes() {
        let root = Path::new("/workspace/zdot");

        assert_eq!(
            ZdotPluginManager.resolve_theme_entrypoint(root, "prompt"),
            None
        );
        assert_eq!(
            ZdotPluginManager.resolve_entrypoint(root, PluginRequestKind::Theme, "prompt"),
            None
        );
    }

    #[test]
    fn resolves_bootstrap_and_framework_relative_source_suffixes() {
        let root = Path::new("/workspace/zdot");

        assert_eq!(
            ZdotPluginManager.resolve_source_suffix(
                root,
                Path::new("/workspace/app/.zshrc"),
                "/not-installed/zdot/zdot.zsh",
            ),
            Some(PathBuf::from("zdot.zsh"))
        );
        assert_eq!(
            ZdotPluginManager.resolve_source_suffix(
                root,
                Path::new("/workspace/zdot/zdot.zsh"),
                "/opt/app/core/hooks.zsh",
            ),
            Some(PathBuf::from("core/hooks.zsh"))
        );
        assert_eq!(
            ZdotPluginManager.resolve_source_suffix(
                root,
                Path::new("/workspace/zdot/zdot.zsh"),
                "/opt/app/modules/fzf/fzf.zsh",
            ),
            Some(PathBuf::from("modules/fzf/fzf.zsh"))
        );
    }

    #[test]
    fn rejects_unanchored_source_suffixes_outside_framework() {
        assert_eq!(
            ZdotPluginManager.resolve_source_suffix(
                Path::new("/workspace/zdot"),
                Path::new("/workspace/app/.zshrc"),
                "/opt/app/core/hooks.zsh",
            ),
            None
        );
    }
}
