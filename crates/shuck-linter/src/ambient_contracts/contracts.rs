//! Compiled ambient-contract configuration and well-known contract registry.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use globset::{Glob, GlobMatcher};
use shuck_ast::Name;
use shuck_semantic::{
    ContractCertainty, FileContract, FunctionContract, PluginRequest, PluginRequestKind,
    ProvidedBinding, ProvidedBindingKind,
};

use super::AmbientContractCollector;
use crate::ShellDialect;

/// User-configurable ambient contract settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbientContractConfig {
    /// Whether well-known ambient contracts are enabled.
    pub well_known: bool,
    /// Built-in contract selectors to disable.
    pub disabled: Vec<String>,
    /// User-authored custom contracts.
    pub custom: Vec<AmbientContractSpec>,
}

impl Default for AmbientContractConfig {
    fn default() -> Self {
        Self {
            well_known: true,
            disabled: Vec::new(),
            custom: Vec::new(),
        }
    }
}

/// One user-authored ambient contract specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbientContractSpec {
    /// Stable contract identifier.
    pub id: String,
    /// Built-in contract selectors this contract replaces.
    pub replaces: Vec<String>,
    /// Activation that decides when the contract applies.
    pub when: AmbientContractActivation,
    /// Optional file globs that limit the contract to matching files.
    pub files: Vec<String>,
    /// Contract effects compiled into semantic file contracts.
    pub effects: AmbientContractEffects,
}

/// Contract activation type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmbientContractActivation {
    /// Apply to matching files without an additional runtime request.
    Always,
    /// Apply when a matching zsh plugin request is observed.
    ZshPlugin {
        /// Framework name such as `oh-my-zsh`.
        framework: String,
        /// Plugin name such as `tmux`.
        plugin: String,
    },
    /// Apply when a matching zsh theme request is observed.
    ZshTheme {
        /// Framework name such as `oh-my-zsh`.
        framework: String,
        /// Theme name such as `agnoster`.
        theme: String,
    },
}

/// User-facing contract effects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AmbientContractEffects {
    /// Names read from the caller environment when the contract activates.
    pub reads: Vec<String>,
    /// Exact names externally consumed by runtime behavior.
    pub consumes_names: Vec<String>,
    /// Name prefixes externally consumed by runtime behavior.
    pub consumes_prefixes: Vec<String>,
    /// Whether every non-local assignment in the file is externally consumed.
    pub consumes_all: bool,
    /// Variables provided by the contract.
    pub provides_variables: Vec<String>,
    /// Callable function names provided by the contract.
    pub provides_functions: Vec<String>,
    /// Function-specific caller reads and sets.
    pub functions: Vec<AmbientFunctionContractSpec>,
}

/// Function-specific contract effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbientFunctionContractSpec {
    /// Function name.
    pub name: String,
    /// Caller names read when the function runs.
    pub reads: Vec<String>,
    /// Caller-visible names the function may set.
    pub sets: Vec<String>,
}

/// Plugin-request contract effects split by imported facts and requesting-file consumption.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedAmbientRequestContracts {
    /// Imported ordered effects applied at the plugin request span.
    pub imported_contracts: Vec<FileContract>,
    /// File-scoped consumption effects applied to the requesting file.
    pub requesting_file_contract: FileContract,
}

/// Stable snapshot of the enabled ambient contract set for cache keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveAmbientContracts {
    /// Enabled built-in contract ids.
    pub well_known_ids: Vec<String>,
    /// Stable descriptors for enabled custom contracts.
    pub custom_descriptors: Vec<String>,
}

/// Compiled ambient contracts shared by file-entry and plugin-request analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAmbientContracts {
    project_root: PathBuf,
    enabled_file_contract_ids: Vec<&'static str>,
    enabled_request_contract_ids: Vec<&'static str>,
    custom_contracts: Vec<CompiledCustomContract>,
}

impl Default for ResolvedAmbientContracts {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            enabled_file_contract_ids: enabled_well_known_file_contract_ids(&[]),
            enabled_request_contract_ids: enabled_well_known_request_contract_ids(&[]),
            custom_contracts: Vec::new(),
        }
    }
}

impl ResolvedAmbientContracts {
    /// Compiles ambient contract settings for one project root.
    pub fn resolve(
        project_root: impl Into<PathBuf>,
        config: AmbientContractConfig,
    ) -> Result<Self> {
        let project_root = project_root.into();
        let mut disabled = config.disabled;
        disabled.extend(
            config
                .custom
                .iter()
                .flat_map(|contract| contract.replaces.iter().cloned()),
        );
        let custom_contracts = config
            .custom
            .into_iter()
            .map(|contract| CompiledCustomContract::resolve(&project_root, contract))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            project_root,
            enabled_file_contract_ids: if config.well_known {
                enabled_well_known_file_contract_ids(&disabled)
            } else {
                Vec::new()
            },
            enabled_request_contract_ids: if config.well_known {
                enabled_well_known_request_contract_ids(&disabled)
            } else {
                Vec::new()
            },
            custom_contracts,
        })
    }

    /// Returns a stable cache-key snapshot of the compiled contract set.
    pub fn effective(&self) -> EffectiveAmbientContracts {
        let mut well_known_ids = self
            .enabled_file_contract_ids
            .iter()
            .chain(self.enabled_request_contract_ids.iter())
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>();
        well_known_ids.sort();
        well_known_ids.dedup();

        let mut custom_descriptors = self
            .custom_contracts
            .iter()
            .map(CompiledCustomContract::effective_descriptor)
            .collect::<Vec<_>>();
        custom_descriptors.sort();

        EffectiveAmbientContracts {
            well_known_ids,
            custom_descriptors,
        }
    }

    pub(crate) fn collector<'a>(
        self: &Arc<Self>,
        source: &'a str,
        path: Option<&'a Path>,
        shell: ShellDialect,
    ) -> super::AmbientContractCollector<'a> {
        super::AmbientContractCollector::new(source, path, shell, Arc::clone(self))
    }

    pub(crate) fn collector_factory(self: &Arc<Self>) -> super::AmbientContractCollectorFactory {
        super::AmbientContractCollectorFactory::new(Arc::clone(self))
    }

    pub(crate) fn file_entry_contract(
        &self,
        collector: &AmbientContractCollector<'_>,
        shell: ShellDialect,
    ) -> Option<FileContract> {
        let path = collector.signals.path()?;
        let mut merged = FileContract::default();
        let mut matched = false;

        for id in &self.enabled_file_contract_ids {
            let contract = well_known_file_contract_by_id(id).expect("known ambient contract id");
            if (contract.matches)(collector, shell) {
                matched = true;
                (contract.apply)(&mut merged, collector);
            }
        }

        for contract in self
            .custom_contracts
            .iter()
            .filter(|contract| contract.matches_file(path.path(), shell))
        {
            matched = true;
            merge_contract(&mut merged, contract.file_contract.clone());
        }

        matched.then_some(merged)
    }

    /// Returns imported and requesting-file contract effects for one plugin request.
    pub fn request_contracts_for_plugin(
        &self,
        source_path: &Path,
        request: &PluginRequest,
    ) -> ResolvedAmbientRequestContracts {
        let mut resolved = ResolvedAmbientRequestContracts::default();
        let lower_path = source_path.to_string_lossy().to_ascii_lowercase();

        for id in &self.enabled_request_contract_ids {
            let contract =
                well_known_request_contract_by_id(id).expect("known ambient request contract id");
            if request_activation_matches(contract.activation, request) {
                if let Some(file_match) = contract.file_match
                    && !file_match(&lower_path)
                {
                    continue;
                }
                let imported_contract = (contract.imported_contract)();
                if !contract_is_empty(&imported_contract) {
                    resolved.imported_contracts.push(imported_contract);
                }
                merge_contract(
                    &mut resolved.requesting_file_contract,
                    (contract.requesting_file_contract)(),
                );
            }
        }

        for contract in self
            .custom_contracts
            .iter()
            .filter(|contract| contract.matches_request(source_path, &lower_path, request))
        {
            if !contract_is_empty(&contract.imported_contract) {
                resolved
                    .imported_contracts
                    .push(contract.imported_contract.clone());
            }
            merge_contract(
                &mut resolved.requesting_file_contract,
                contract.requesting_file_contract.clone(),
            );
        }

        resolved
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledCustomContract {
    id: String,
    when: AmbientContractActivation,
    files: Vec<CompiledPathMatcher>,
    file_contract: FileContract,
    imported_contract: FileContract,
    requesting_file_contract: FileContract,
}

impl CompiledCustomContract {
    fn resolve(project_root: &Path, contract: AmbientContractSpec) -> Result<Self> {
        let files = contract
            .files
            .into_iter()
            .map(CompiledPathMatcher::new)
            .collect::<Result<Vec<_>>>()?;
        let file_contract = file_entry_contract_from_effects(&contract.effects);
        let imported_contract = imported_contract_from_effects(&contract.effects);
        let requesting_file_contract = requesting_file_contract_from_effects(&contract.effects);

        Ok(Self {
            id: contract.id,
            when: contract.when,
            files: files
                .into_iter()
                .map(|matcher| matcher.with_project_root(project_root))
                .collect(),
            file_contract,
            imported_contract,
            requesting_file_contract,
        })
    }

    fn matches_file(&self, path: &Path, shell: ShellDialect) -> bool {
        matches!(self.when, AmbientContractActivation::Always)
            && self.files_match(path)
            && file_activation_shell_matches(&self.when, shell)
    }

    fn matches_request(
        &self,
        source_path: &Path,
        _lower_path: &str,
        request: &PluginRequest,
    ) -> bool {
        self.files_match(source_path) && request_activation_matches_custom(&self.when, request)
    }

    fn files_match(&self, path: &Path) -> bool {
        if self.files.is_empty() {
            return true;
        }

        let mut saw_positive = false;
        let mut matched_positive = false;

        for matcher in &self.files {
            if matcher.negated {
                if matcher.path_matches(path) {
                    return false;
                }
                continue;
            }

            saw_positive = true;
            matched_positive |= matcher.path_matches(path);
        }

        if saw_positive { matched_positive } else { true }
    }

    fn effective_descriptor(&self) -> String {
        format!(
            "id={};when={};files={};file={};imported={};requesting={}",
            self.id,
            activation_descriptor(&self.when),
            join_sorted(self.files.iter().map(|matcher| matcher.pattern.clone())),
            file_contract_descriptor(&self.file_contract),
            file_contract_descriptor(&self.imported_contract),
            file_contract_descriptor(&self.requesting_file_contract),
        )
    }
}

#[derive(Debug, Clone)]
struct CompiledPathMatcher {
    pattern: String,
    basename_matcher: GlobMatcher,
    relative_matcher: GlobMatcher,
    absolute_matcher: GlobMatcher,
    negated: bool,
    project_root: PathBuf,
}

impl PartialEq for CompiledPathMatcher {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern
            && self.negated == other.negated
            && self.project_root == other.project_root
    }
}

impl Eq for CompiledPathMatcher {}

impl CompiledPathMatcher {
    fn new(pattern: String) -> Result<Self> {
        let mut matcher_pattern = pattern.clone();
        let negated = matcher_pattern.starts_with('!');
        if negated {
            matcher_pattern.drain(..1);
        }

        Ok(Self {
            pattern,
            basename_matcher: Glob::new(&matcher_pattern)
                .map_err(|err| anyhow!("invalid glob {matcher_pattern:?}: {err}"))?
                .compile_matcher(),
            relative_matcher: Glob::new(&matcher_pattern)
                .map_err(|err| anyhow!("invalid glob {matcher_pattern:?}: {err}"))?
                .compile_matcher(),
            absolute_matcher: Glob::new(&matcher_pattern)
                .map_err(|err| anyhow!("invalid glob {matcher_pattern:?}: {err}"))?
                .compile_matcher(),
            negated,
            project_root: PathBuf::new(),
        })
    }

    fn with_project_root(mut self, project_root: &Path) -> Self {
        self.project_root = project_root.to_path_buf();
        self
    }

    fn path_matches(&self, path: &Path) -> bool {
        let relative_path = path.strip_prefix(&self.project_root).unwrap_or(path);
        let file_name = relative_path.file_name().or_else(|| path.file_name());
        let Some(file_name) = file_name else {
            return false;
        };
        self.basename_matcher.is_match(file_name)
            || self.relative_matcher.is_match(relative_path)
            || self.absolute_matcher.is_match(path)
    }
}

struct WellKnownFileContract {
    id: &'static str,
    groups: &'static [&'static str],
    matches: fn(&AmbientContractCollector<'_>, ShellDialect) -> bool,
    apply: fn(&mut FileContract, &AmbientContractCollector<'_>),
}

#[derive(Clone, Copy)]
enum RequestActivation {
    ZshPlugin {
        framework: &'static str,
        plugin: &'static str,
    },
}

struct WellKnownRequestContract {
    id: &'static str,
    groups: &'static [&'static str],
    activation: RequestActivation,
    file_match: Option<fn(&str) -> bool>,
    imported_contract: fn() -> FileContract,
    requesting_file_contract: fn() -> FileContract,
}

const WELL_KNOWN_FILE_CONTRACTS: &[WellKnownFileContract] = &[
    WellKnownFileContract {
        id: "sourced/runtime",
        groups: &["sourced"],
        matches: super::sourced_runtime::matches_sourced_runtime_contract,
        apply: super::sourced_runtime::apply_sourced_runtime_contract,
    },
    WellKnownFileContract {
        id: "zsh/runtime",
        groups: &["zsh"],
        matches: super::zsh_runtime::matches_zsh_ambient_runtime_contract,
        apply: super::zsh_runtime::apply_zsh_ambient_runtime_contract,
    },
    WellKnownFileContract {
        id: "zsh/caller-scoped-arrays",
        groups: &["zsh"],
        matches: super::zsh_caller_arrays::matches_zsh_caller_scoped_array_contract,
        apply: super::zsh_caller_arrays::apply_zsh_caller_scoped_array_contract,
    },
    WellKnownFileContract {
        id: "zsh/config",
        groups: &["zsh"],
        matches: super::zsh_config::matches_zsh_config_contract,
        apply: super::zsh_config::apply_zsh_config_contract,
    },
    WellKnownFileContract {
        id: "zsh/module-metadata",
        groups: &["zsh"],
        matches: super::zsh_module_metadata::matches_zsh_module_metadata_contract,
        apply: super::zsh_module_metadata::apply_zsh_module_metadata_contract,
    },
];

const WELL_KNOWN_REQUEST_CONTRACTS: &[WellKnownRequestContract] = &[WellKnownRequestContract {
    id: "zsh/oh-my-zsh/plugin/tmux",
    groups: &["zsh", "zsh/oh-my-zsh", "zsh/oh-my-zsh/plugin"],
    activation: RequestActivation::ZshPlugin {
        framework: "oh-my-zsh",
        plugin: "tmux",
    },
    file_match: None,
    imported_contract: FileContract::default,
    requesting_file_contract: oh_my_zsh_tmux_requesting_file_contract,
}];

fn enabled_well_known_file_contract_ids(disabled: &[String]) -> Vec<&'static str> {
    WELL_KNOWN_FILE_CONTRACTS
        .iter()
        .filter(|contract| !selector_matches(disabled, contract.id, contract.groups))
        .map(|contract| contract.id)
        .collect()
}

fn enabled_well_known_request_contract_ids(disabled: &[String]) -> Vec<&'static str> {
    WELL_KNOWN_REQUEST_CONTRACTS
        .iter()
        .filter(|contract| !selector_matches(disabled, contract.id, contract.groups))
        .map(|contract| contract.id)
        .collect()
}

fn selector_matches(selectors: &[String], id: &str, groups: &[&str]) -> bool {
    selectors.iter().any(|selector| {
        selector == "*" || selector == id || groups.iter().any(|group| selector == group)
    })
}

fn well_known_file_contract_by_id(id: &str) -> Option<&'static WellKnownFileContract> {
    WELL_KNOWN_FILE_CONTRACTS
        .iter()
        .find(|contract| contract.id == id)
}

fn well_known_request_contract_by_id(id: &str) -> Option<&'static WellKnownRequestContract> {
    WELL_KNOWN_REQUEST_CONTRACTS
        .iter()
        .find(|contract| contract.id == id)
}

fn request_activation_matches(activation: RequestActivation, request: &PluginRequest) -> bool {
    match activation {
        RequestActivation::ZshPlugin { framework, plugin } => {
            request.kind == PluginRequestKind::Plugin
                && plugin_framework_name(&request.framework) == framework
                && request.name == plugin
        }
    }
}

fn request_activation_matches_custom(
    activation: &AmbientContractActivation,
    request: &PluginRequest,
) -> bool {
    match activation {
        AmbientContractActivation::Always => false,
        AmbientContractActivation::ZshPlugin { framework, plugin } => {
            request.kind == PluginRequestKind::Plugin
                && plugin_framework_name(&request.framework) == framework
                && &request.name == plugin
        }
        AmbientContractActivation::ZshTheme { framework, theme } => {
            request.kind == PluginRequestKind::Theme
                && plugin_framework_name(&request.framework) == framework
                && &request.name == theme
        }
    }
}

fn plugin_framework_name(framework: &shuck_semantic::PluginFramework) -> &str {
    match framework {
        shuck_semantic::PluginFramework::OhMyZsh => "oh-my-zsh",
        shuck_semantic::PluginFramework::Prezto => "prezto",
        shuck_semantic::PluginFramework::Zdot => "zdot",
        shuck_semantic::PluginFramework::Zinit => "zinit",
        shuck_semantic::PluginFramework::ExplicitFilesystem => "filesystem",
        shuck_semantic::PluginFramework::Other(other) => other.as_str(),
    }
}

fn file_activation_shell_matches(
    activation: &AmbientContractActivation,
    shell: ShellDialect,
) -> bool {
    match activation {
        AmbientContractActivation::Always => true,
        AmbientContractActivation::ZshPlugin { .. }
        | AmbientContractActivation::ZshTheme { .. } => shell == ShellDialect::Zsh,
    }
}

fn activation_descriptor(activation: &AmbientContractActivation) -> String {
    match activation {
        AmbientContractActivation::Always => "always".to_owned(),
        AmbientContractActivation::ZshPlugin { framework, plugin } => {
            format!("zsh_plugin:{framework}:{plugin}")
        }
        AmbientContractActivation::ZshTheme { framework, theme } => {
            format!("zsh_theme:{framework}:{theme}")
        }
    }
}

fn file_contract_descriptor(contract: &FileContract) -> String {
    let required_reads = join_sorted(
        contract
            .required_reads
            .iter()
            .map(|name| name.as_str().to_owned()),
    );
    let provided_bindings = join_sorted(contract.provided_bindings.iter().map(|binding| {
        format!(
            "{}:{:?}:{:?}:{:?}",
            binding.name.as_str(),
            binding.kind,
            binding.certainty,
            binding.file_entry_initialization
        )
    }));
    let provided_functions = join_sorted(
        contract
            .provided_functions
            .iter()
            .map(function_contract_descriptor),
    );
    let consumed_names = join_sorted(
        contract
            .externally_consumed_binding_names
            .iter()
            .map(|name| name.as_str().to_owned()),
    );
    let consumed_prefixes = join_sorted(
        contract
            .externally_consumed_binding_prefixes
            .iter()
            .map(|name| name.as_str().to_owned()),
    );

    format!(
        "reads=[{required_reads}]|bindings=[{provided_bindings}]|functions=[{provided_functions}]|all={}|names=[{consumed_names}]|prefixes=[{consumed_prefixes}]",
        contract.externally_consumed_bindings
    )
}

fn function_contract_descriptor(contract: &FunctionContract) -> String {
    let required_reads = join_sorted(
        contract
            .required_reads
            .iter()
            .map(|name| name.as_str().to_owned()),
    );
    let provided_bindings = join_sorted(contract.provided_bindings.iter().map(|binding| {
        format!(
            "{}:{:?}:{:?}:{:?}",
            binding.name.as_str(),
            binding.kind,
            binding.certainty,
            binding.file_entry_initialization
        )
    }));

    format!(
        "{}|reads=[{}]|bindings=[{}]",
        contract.name.as_str(),
        required_reads,
        provided_bindings
    )
}

fn join_sorted(values: impl IntoIterator<Item = String>) -> String {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort();
    values.join(",")
}

fn file_entry_contract_from_effects(effects: &AmbientContractEffects) -> FileContract {
    let mut contract = imported_contract_from_effects(effects);
    merge_contract(
        &mut contract,
        requesting_file_contract_from_effects(effects),
    );
    contract
}

fn imported_contract_from_effects(effects: &AmbientContractEffects) -> FileContract {
    let mut contract = FileContract::default();
    for name in &effects.reads {
        contract.add_required_read(Name::from(name.as_str()));
    }
    for name in &effects.provides_variables {
        contract.add_provided_binding(ProvidedBinding::new_file_entry_initialized(
            Name::from(name.as_str()),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
    for name in &effects.provides_functions {
        contract.add_provided_binding(ProvidedBinding::new(
            Name::from(name.as_str()),
            ProvidedBindingKind::Function,
            ContractCertainty::Definite,
        ));
    }
    for function in &effects.functions {
        let mut function_contract = FunctionContract::new(Name::from(function.name.as_str()));
        for name in &function.reads {
            function_contract.add_required_read(Name::from(name.as_str()));
        }
        for name in &function.sets {
            function_contract.add_provided_binding(ProvidedBinding::new(
                Name::from(name.as_str()),
                ProvidedBindingKind::Variable,
                ContractCertainty::Definite,
            ));
        }
        contract.add_provided_function(function_contract);
    }
    contract
}

fn requesting_file_contract_from_effects(effects: &AmbientContractEffects) -> FileContract {
    let mut contract = FileContract {
        externally_consumed_bindings: effects.consumes_all,
        ..FileContract::default()
    };
    for name in &effects.consumes_names {
        contract.add_externally_consumed_binding_name(Name::from(name.as_str()));
    }
    for prefix in &effects.consumes_prefixes {
        contract.add_externally_consumed_binding_prefix(Name::from(prefix.as_str()));
    }
    contract
}

fn oh_my_zsh_tmux_requesting_file_contract() -> FileContract {
    let mut contract = FileContract::default();
    contract.add_externally_consumed_binding_prefix(Name::from("ZSH_TMUX_"));
    contract
}

pub(crate) fn merge_contract(merged: &mut FileContract, contract: FileContract) {
    merged.externally_consumed_bindings |= contract.externally_consumed_bindings;
    for name in contract.required_reads {
        merged.add_required_read(name);
    }
    for name in contract.externally_consumed_binding_names {
        merged.add_externally_consumed_binding_name(name);
    }
    for binding in contract.provided_bindings {
        merged.add_provided_binding(binding);
    }
    for function in contract.provided_functions {
        merged.add_provided_function(function);
    }
    for prefix in contract.externally_consumed_binding_prefixes {
        merged.add_externally_consumed_binding_prefix(prefix);
    }
}

fn contract_is_empty(contract: &FileContract) -> bool {
    contract.required_reads.is_empty()
        && contract.provided_bindings.is_empty()
        && contract.provided_functions.is_empty()
        && !contract.externally_consumed_bindings
        && contract.externally_consumed_binding_names.is_empty()
        && contract.externally_consumed_binding_prefixes.is_empty()
}
