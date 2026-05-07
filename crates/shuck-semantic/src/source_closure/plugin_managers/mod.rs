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
mod zinit;

use super::*;
use crate::ZshPluginFramework;

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

trait ZshPluginManager: ZshPluginFramework {
    fn is_active(&self, context: &PluginManagerContext<'_>) -> bool {
        context.model.shell_profile().dialect == ParseShellDialect::Zsh
    }

    fn collect_plugin_requests(&self, context: &PluginManagerContext<'_>) -> Vec<PluginRequest> {
        let _ = context;
        Vec::new()
    }
}

trait ZshDeferredRuntimeManager {
    fn collect_deferred_required_reads(
        &self,
        context: &DeferredPluginRuntimeContext<'_>,
    ) -> Vec<Name> {
        let _ = context;
        Vec::new()
    }
}

static OH_MY_ZSH_PLUGIN_MANAGER: oh_my_zsh::OhMyZshPluginManager = oh_my_zsh::OhMyZshPluginManager;
static PREZTO_PLUGIN_MANAGER: prezto::PreztoPluginManager = prezto::PreztoPluginManager;
static ZDOT_PLUGIN_MANAGER: zdot::ZdotPluginManager = zdot::ZdotPluginManager;
static ZINIT_PLUGIN_MANAGER: zinit::ZinitPluginManager = zinit::ZinitPluginManager;

static ZSH_PLUGIN_MANAGERS: [&dyn ZshPluginManager; 4] = [
    &OH_MY_ZSH_PLUGIN_MANAGER,
    &PREZTO_PLUGIN_MANAGER,
    &ZDOT_PLUGIN_MANAGER,
    &ZINIT_PLUGIN_MANAGER,
];

static ZSH_PLUGIN_FRAMEWORKS: [&dyn ZshPluginFramework; 4] = [
    &OH_MY_ZSH_PLUGIN_MANAGER,
    &PREZTO_PLUGIN_MANAGER,
    &ZDOT_PLUGIN_MANAGER,
    &ZINIT_PLUGIN_MANAGER,
];

/// Returns all built-in zsh plugin framework implementations.
pub fn zsh_plugin_frameworks() -> &'static [&'static dyn ZshPluginFramework] {
    &ZSH_PLUGIN_FRAMEWORKS
}

/// Returns the built-in implementation for a framework, when Shuck knows one.
pub fn layout_for_plugin_framework(
    framework: &PluginFramework,
) -> Option<&'static dyn ZshPluginFramework> {
    zsh_plugin_frameworks()
        .iter()
        .copied()
        .find(|layout| &layout.framework() == framework)
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
    let mut requests = Vec::new();
    for manager in ZSH_PLUGIN_MANAGERS {
        if manager.is_active(&context) {
            requests.extend(manager.collect_plugin_requests(&context));
        }
    }
    let mut seen = requests
        .iter()
        .map(plugin_request_dependency_key)
        .collect::<FxHashSet<_>>();
    let mut index = 0;
    while index < requests.len() {
        let request = requests[index].clone();
        if let Some(manager) = manager_for_plugin_framework(&request.framework) {
            for dependency in manager.dependent_plugin_requests(&request) {
                if seen.insert(plugin_request_dependency_key(&dependency)) {
                    requests.push(dependency);
                }
            }
        }
        index += 1;
    }
    dedup_plugin_requests(requests)
}

fn plugin_request_dependency_key(
    request: &PluginRequest,
) -> (PluginFramework, PluginRequestKind, String, Option<PathBuf>) {
    (
        request.framework.clone(),
        request.kind,
        request.name.clone(),
        request.root_hint.clone(),
    )
}

fn manager_for_plugin_framework(
    framework: &PluginFramework,
) -> Option<&'static dyn ZshPluginManager> {
    ZSH_PLUGIN_MANAGERS
        .iter()
        .copied()
        .find(|manager| &manager.framework() == framework)
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
    let managers: [&dyn ZshDeferredRuntimeManager; 1] =
        [&generic_zsh_runtime::GenericZshRuntimeManager];

    let mut reads = Vec::new();
    for manager in managers {
        reads.extend(manager.collect_deferred_required_reads(&context));
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

fn path_text_starts_with_path(path: &str, root: &Path) -> bool {
    let root_text = root.to_string_lossy().replace('\\', "/");
    path == root_text
        || path
            .strip_prefix(&root_text)
            .is_some_and(|tail| tail.starts_with('/'))
}
