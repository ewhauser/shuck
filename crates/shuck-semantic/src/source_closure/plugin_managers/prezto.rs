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
