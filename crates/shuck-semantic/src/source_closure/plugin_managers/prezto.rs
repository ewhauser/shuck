//! Prezto module request discovery.
//!
//! Prezto loads modules through the `pmodload` helper, usually after reading
//! static `zstyle ':prezto:load' pmodule ...` configuration. This adapter turns
//! those static module names into common `PluginRequest`s; the resolver decides
//! where a module lives on disk.

use super::super::*;
use super::*;

pub(super) struct PreztoPluginManager;

impl ZshPluginManager for PreztoPluginManager {
    fn collect_plugin_requests(&self, context: &PluginManagerContext<'_>) -> Vec<PluginRequest> {
        collect_prezto_plugin_requests(context)
    }
}

impl ZshPluginFramework for PreztoPluginManager {
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

    fn dependent_plugin_requests(&self, request: &PluginRequest) -> Vec<PluginRequest> {
        if request.kind != PluginRequestKind::Plugin {
            return Vec::new();
        }
        let Some(framework_name) = prezto_external_module_framework(&request.name) else {
            return Vec::new();
        };
        vec![PluginRequest {
            framework: PluginFramework::Other(framework_name.to_owned()),
            kind: PluginRequestKind::Plugin,
            name: framework_name.to_owned(),
            span: request.span,
            explicit: false,
            root_hint: None,
        }]
    }
}

fn prezto_external_module_framework(module: &str) -> Option<&'static str> {
    match module {
        "autosuggestions" => Some("zsh-autosuggestions"),
        "syntax-highlighting" => Some("zsh-syntax-highlighting"),
        _ => None,
    }
}

fn collect_prezto_plugin_requests(context: &PluginManagerContext<'_>) -> Vec<PluginRequest> {
    let mut requests = Vec::new();
    for stmt in &context.file.body.stmts {
        let Command::Simple(command) = &stmt.command else {
            continue;
        };
        let Some(name) = static_word_text(&command.name, context.source) else {
            continue;
        };
        let modules = match name.as_ref() {
            "zstyle" => prezto_zstyle_module_names(command, context.source),
            "pmodload" => prezto_pmodload_module_names(command, context.source),
            _ => Vec::new(),
        };
        requests.extend(modules.into_iter().map(|module| PluginRequest {
            framework: PluginFramework::Prezto,
            kind: PluginRequestKind::Plugin,
            name: module,
            span: stmt.span,
            explicit: false,
            root_hint: None,
        }));
    }
    requests
}

fn prezto_zstyle_module_names(command: &SimpleCommand, source: &str) -> Vec<String> {
    let args = static_command_args(command, source);
    let Some(args) = args.as_deref() else {
        return Vec::new();
    };
    if args.first().is_some_and(|arg| arg.starts_with('-')) {
        return Vec::new();
    }
    let operands = args
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .map(String::as_str)
        .collect::<Vec<_>>();
    let [context, style, modules @ ..] = operands.as_slice() else {
        return Vec::new();
    };
    if *context != ":prezto:load" || *style != "pmodule" {
        return Vec::new();
    }
    static_plugin_names(modules.iter().copied())
}

fn prezto_pmodload_module_names(command: &SimpleCommand, source: &str) -> Vec<String> {
    let args = static_command_args(command, source);
    let Some(args) = args.as_deref() else {
        return Vec::new();
    };
    static_plugin_names(
        args.iter()
            .filter(|arg| !arg.starts_with('-'))
            .map(String::as_str),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_keys_match_config_name() {
        assert_eq!(PreztoPluginManager.root_keys(), &["prezto"]);
    }

    #[test]
    fn resolves_module_entrypoint() {
        let root = Path::new("/workspace/prezto");

        assert_eq!(
            PreztoPluginManager.resolve_plugin_entrypoint(root, "editor"),
            Some(PathBuf::from("/workspace/prezto/modules/editor/init.zsh"))
        );
        assert_eq!(
            PreztoPluginManager.resolve_entrypoint(root, PluginRequestKind::Plugin, "utility"),
            Some(PathBuf::from("/workspace/prezto/modules/utility/init.zsh"))
        );
    }

    #[test]
    fn does_not_resolve_themes_or_framework_sources() {
        let root = Path::new("/workspace/prezto");

        assert_eq!(
            PreztoPluginManager.resolve_theme_entrypoint(root, "sorin"),
            None
        );
        assert_eq!(
            PreztoPluginManager.resolve_entrypoint(root, PluginRequestKind::Theme, "sorin"),
            None
        );
        assert_eq!(
            PreztoPluginManager.resolve_source_suffix(
                root,
                Path::new("/workspace/prezto/init.zsh"),
                "/not-installed/prezto/modules/editor/init.zsh",
            ),
            None
        );
    }

    #[test]
    fn declares_external_module_dependencies() {
        let request = PluginRequest {
            framework: PluginFramework::Prezto,
            kind: PluginRequestKind::Plugin,
            name: "autosuggestions".to_owned(),
            span: Span::new(),
            explicit: false,
            root_hint: None,
        };

        assert_eq!(
            PreztoPluginManager.dependent_plugin_requests(&request),
            vec![PluginRequest {
                framework: PluginFramework::Other("zsh-autosuggestions".to_owned()),
                kind: PluginRequestKind::Plugin,
                name: "zsh-autosuggestions".to_owned(),
                span: request.span,
                explicit: false,
                root_hint: None,
            }]
        );
    }
}
