//! Zsh plugin manager adapters.
//!
//! A manager recognizes one family of zsh plugin/framework syntax and converts
//! it into source-closure data: logical plugin requests and deferred runtime
//! entrypoints. The source-closure engine consumes those common outputs without
//! knowing whether they came from Oh My Zsh, a custom manager, or generic zsh
//! runtime APIs.

mod generic_zsh_runtime;
mod oh_my_zsh;
mod prezto;
mod zdot;

use super::*;

pub(super) use oh_my_zsh::{dedup_plugin_requests, sorted_dependency_paths};

pub(super) struct PluginManagerContext<'a> {
    pub(super) model: &'a SemanticModel,
    pub(super) file: &'a File,
    pub(super) source: &'a str,
    pub(super) source_path: &'a Path,
    pub(super) plugin_resolver: &'a (dyn PluginResolver + Send + Sync),
}

pub(super) struct DeferredPluginRuntimeContext<'a> {
    pub(super) semantic: &'a SemanticModel,
    pub(super) analysis: &'a crate::SemanticAnalysis<'a>,
    pub(super) facts: &'a AstFacts,
    pub(super) source: &'a str,
    pub(super) scope: ScopeId,
    pub(super) synthetic_reads: &'a [SyntheticRead],
}

trait ZshPluginManager {
    fn is_active(&self, context: &PluginManagerContext<'_>) -> bool {
        context.model.shell_profile().dialect == ParseShellDialect::Zsh
    }

    fn is_active_for_deferred(&self, context: &DeferredPluginRuntimeContext<'_>) -> bool {
        context.semantic.shell_profile().dialect == ParseShellDialect::Zsh
    }

    fn collect_plugin_requests(&self, context: &PluginManagerContext<'_>) -> Vec<PluginRequest> {
        let _ = context;
        Vec::new()
    }

    fn collect_deferred_required_reads(
        &self,
        context: &DeferredPluginRuntimeContext<'_>,
    ) -> Vec<Name> {
        let _ = context;
        Vec::new()
    }
}

pub(super) fn collect_plugin_requests(
    model: &SemanticModel,
    file: &File,
    source: &str,
    source_path: &Path,
    plugin_resolver: &(dyn PluginResolver + Send + Sync),
) -> Vec<PluginRequest> {
    if model.shell_profile().dialect != ParseShellDialect::Zsh {
        return Vec::new();
    }

    let context = PluginManagerContext {
        model,
        file,
        source,
        source_path,
        plugin_resolver,
    };
    let managers: [&dyn ZshPluginManager; 4] = [
        &oh_my_zsh::OhMyZshPluginManager,
        &prezto::PreztoPluginManager,
        &zdot::ZdotPluginManager,
        &generic_zsh_runtime::GenericZshRuntimeManager,
    ];

    let mut requests = Vec::new();
    for manager in managers {
        if manager.is_active(&context) {
            requests.extend(manager.collect_plugin_requests(&context));
        }
    }
    dedup_plugin_requests(requests)
}

pub(super) fn deferred_zsh_entrypoint_required_reads(
    semantic: &SemanticModel,
    analysis: &crate::SemanticAnalysis<'_>,
    facts: &AstFacts,
    source: &str,
    scope: ScopeId,
    synthetic_reads: &[SyntheticRead],
) -> Vec<Name> {
    if semantic.shell_profile().dialect != ParseShellDialect::Zsh {
        return Vec::new();
    }

    let context = DeferredPluginRuntimeContext {
        semantic,
        analysis,
        facts,
        source,
        scope,
        synthetic_reads,
    };
    let managers: [&dyn ZshPluginManager; 2] = [
        &oh_my_zsh::OhMyZshPluginManager,
        &generic_zsh_runtime::GenericZshRuntimeManager,
    ];

    let mut reads = Vec::new();
    for manager in managers {
        if manager.is_active_for_deferred(&context) {
            reads.extend(manager.collect_deferred_required_reads(&context));
        }
    }
    reads.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    reads.dedup();
    reads
}

fn static_command_args(command: &SimpleCommand, source: &str) -> Option<Vec<String>> {
    command
        .args
        .iter()
        .map(|arg| static_word_text(arg, source).map(|text| text.into_owned()))
        .collect()
}

fn static_plugin_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut plugins = Vec::new();
    for name in names {
        let name = name.trim();
        if name.is_empty() || name.contains('/') || plugins.iter().any(|plugin| plugin == name) {
            continue;
        }
        plugins.push(name.to_owned());
    }
    plugins
}
