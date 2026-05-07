//! Common contract for supported zsh plugin frameworks.
//!
//! Framework-specific source discovery lives in `source_closure`, but every
//! built-in framework also has to expose its filesystem layout through this
//! trait. Keeping those methods on the same implementation prevents adding a
//! detector without deciding how its requests resolve to files.

use std::path::{Path, PathBuf};

use crate::{PluginFramework, PluginRequest, PluginRequestKind};

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

    /// Returns plugin requests that should be loaded alongside `request`.
    fn dependent_plugin_requests(&self, _request: &PluginRequest) -> Vec<PluginRequest> {
        Vec::new()
    }

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

/// Resolves a plugin request beneath a configured root.
pub fn resolve_zsh_plugin_entrypoint(root: &Path, request: &PluginRequest) -> Option<PathBuf> {
    if let Some(layout) = crate::layout_for_plugin_framework(&request.framework) {
        return layout.resolve_entrypoint(root, request.kind, &request.name);
    }

    match (&request.framework, request.kind) {
        (PluginFramework::Other(_), PluginRequestKind::Plugin) => {
            let standalone = root.join(format!("{}.plugin.zsh", request.name));
            if standalone.is_file() {
                Some(standalone)
            } else {
                Some(
                    root.join("plugins")
                        .join(&request.name)
                        .join(format!("{}.plugin.zsh", request.name)),
                )
            }
        }
        (PluginFramework::Other(_), PluginRequestKind::Theme) => Some(
            root.join("themes")
                .join(format!("{}.zsh-theme", request.name)),
        ),
        _ => None,
    }
}

/// Returns the configuration root aliases for a built-in framework.
pub fn zsh_plugin_root_keys(framework: &PluginFramework) -> Option<&'static [&'static str]> {
    crate::layout_for_plugin_framework(framework).map(ZshPluginFramework::root_keys)
}

/// Resolves a zsh `source` candidate through configured plugin roots.
pub fn resolve_zsh_plugin_source_paths<'a, I>(
    roots: I,
    source_path: &Path,
    candidate: &str,
) -> Vec<PathBuf>
where
    I: IntoIterator<Item = (&'a str, &'a Path)>,
{
    let roots = roots.into_iter().collect::<Vec<_>>();
    let mut paths = Vec::new();
    for layout in crate::zsh_plugin_frameworks() {
        for root_key in layout.root_keys() {
            for (configured_key, root) in &roots {
                if configured_key != root_key {
                    continue;
                }
                if let Some(suffix) = layout.resolve_source_suffix(root, source_path, candidate) {
                    paths.push(root.join(suffix));
                }
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginRequest;
    use tempfile::tempdir;

    #[test]
    fn custom_plugin_requests_use_default_nested_layout() {
        let root = Path::new("/workspace/custom");
        let request = PluginRequest {
            framework: PluginFramework::Other("custom".to_owned()),
            kind: PluginRequestKind::Plugin,
            name: "prompt-tools".to_owned(),
            span: shuck_ast::Span::new(),
            explicit: true,
            root_hint: None,
        };

        assert_eq!(
            resolve_zsh_plugin_entrypoint(root, &request),
            Some(PathBuf::from(
                "/workspace/custom/plugins/prompt-tools/prompt-tools.plugin.zsh"
            ))
        );
    }

    #[test]
    fn custom_plugin_requests_prefer_standalone_entrypoint_when_present() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("zsh-autosuggestions.plugin.zsh"),
            "source zsh-autosuggestions.zsh\n",
        )
        .unwrap();
        let request = PluginRequest {
            framework: PluginFramework::Other("zsh-autosuggestions".to_owned()),
            kind: PluginRequestKind::Plugin,
            name: "zsh-autosuggestions".to_owned(),
            span: shuck_ast::Span::new(),
            explicit: false,
            root_hint: None,
        };

        assert_eq!(
            resolve_zsh_plugin_entrypoint(temp.path(), &request),
            Some(temp.path().join("zsh-autosuggestions.plugin.zsh"))
        );
    }

    #[test]
    fn custom_theme_requests_use_default_theme_layout() {
        let root = Path::new("/workspace/custom");
        let request = PluginRequest {
            framework: PluginFramework::Other("custom".to_owned()),
            kind: PluginRequestKind::Theme,
            name: "minimal".to_owned(),
            span: shuck_ast::Span::new(),
            explicit: true,
            root_hint: None,
        };

        assert_eq!(
            resolve_zsh_plugin_entrypoint(root, &request),
            Some(PathBuf::from("/workspace/custom/themes/minimal.zsh-theme"))
        );
    }

    #[test]
    fn source_paths_resolve_through_framework_roots() {
        let root = Path::new("/workspace/oh-my-zsh");

        assert_eq!(
            resolve_zsh_plugin_source_paths(
                [("oh-my-zsh", root)].into_iter(),
                Path::new("/workspace/app/.zshrc"),
                "$ZSH/oh-my-zsh.sh",
            ),
            vec![PathBuf::from("/workspace/oh-my-zsh/oh-my-zsh.sh")]
        );
    }
}
