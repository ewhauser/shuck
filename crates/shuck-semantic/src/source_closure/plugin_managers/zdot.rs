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
