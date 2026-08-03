use lsp_types::Url;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Deserializer};
use shuck_config::{FormatConfig, LintConfig, ShuckConfig};

use crate::session::settings::GlobalClientSettings;
use crate::{Client, logging};

pub(crate) type WorkspaceOptionsMap = FxHashMap<Url, ClientOptions>;

/// Global initialization options accepted by the Shuck LSP server.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GlobalOptions {
    #[serde(flatten)]
    client: ClientOptions,
    #[serde(default)]
    pub(crate) tracing: TracingOptions,
}

impl GlobalOptions {
    /// Resolve client-provided options into runtime global settings.
    pub fn into_settings(self, client: Client) -> GlobalClientSettings {
        GlobalClientSettings::new(self.client, client)
    }

    pub(crate) fn workspace_diagnostics_enabled(&self) -> bool {
        self.client.server.workspace_diagnostics.enabled
    }
}

/// Per-client or per-workspace Shuck options supplied through LSP settings.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientOptions {
    #[serde(default)]
    /// Lint configuration overrides.
    pub lint: Option<LintConfig>,
    #[serde(default)]
    /// Format configuration overrides.
    pub format: Option<FormatConfig>,
    #[serde(default)]
    /// Whether source-level fix-all actions are enabled.
    pub fix_all: Option<bool>,
    #[serde(default)]
    /// Whether unsafe fixes may be offered.
    pub unsafe_fixes: Option<bool>,
    #[serde(default)]
    /// Whether parser diagnostics should be shown.
    pub show_syntax_errors: Option<bool>,
    #[serde(default)]
    /// Server-only editor feature options.
    pub server: ServerOptions,
}

impl ClientOptions {
    pub(crate) fn to_config_overrides(&self) -> ShuckConfig {
        ShuckConfig {
            lint: self.lint.clone().unwrap_or_default(),
            format: self.format.clone().unwrap_or_default(),
            ..ShuckConfig::default()
        }
    }
}

/// Options for server-only editor features.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerOptions {
    /// Workspace-wide symbol search configuration.
    pub workspace_symbols: WorkspaceSymbolFeatureOptions,
    /// Completion configuration.
    pub completion: CompletionFeatureOptions,
    /// Rename configuration.
    pub rename: RenameFeatureOptions,
    /// Cross-file call hierarchy configuration.
    pub call_hierarchy: CallHierarchyFeatureOptions,
    /// Workspace-wide diagnostic pull configuration.
    pub workspace_diagnostics: WorkspaceDiagnosticsFeatureOptions,
    workspace_symbols_overrides: WorkspaceSymbolFeatureOptionsOverrides,
    completion_overrides: CompletionFeatureOptionsOverrides,
    rename_overrides: RenameFeatureOptionsOverrides,
    call_hierarchy_overrides: CallHierarchyFeatureOptionsOverrides,
    workspace_diagnostics_overrides: WorkspaceDiagnosticsFeatureOptionsOverrides,
}

impl ServerOptions {
    pub(crate) fn workspace_symbols_layered_over(
        &self,
        base: WorkspaceSymbolFeatureOptions,
    ) -> WorkspaceSymbolFeatureOptions {
        if self.workspace_symbols_overrides.has_overrides() {
            self.workspace_symbols_overrides.apply_to(base)
        } else if self.workspace_symbols != WorkspaceSymbolFeatureOptions::default() {
            self.workspace_symbols
        } else {
            base
        }
    }

    pub(crate) fn completion_layered_over(
        &self,
        base: CompletionFeatureOptions,
    ) -> CompletionFeatureOptions {
        if self.completion_overrides.has_overrides() {
            self.completion_overrides.apply_to(base)
        } else if self.completion != CompletionFeatureOptions::default() {
            self.completion
        } else {
            base
        }
    }

    pub(crate) fn rename_layered_over(&self, base: RenameFeatureOptions) -> RenameFeatureOptions {
        if self.rename_overrides.has_overrides() {
            self.rename_overrides.apply_to(base)
        } else if self.rename != RenameFeatureOptions::default() {
            self.rename
        } else {
            base
        }
    }

    pub(crate) fn call_hierarchy_layered_over(
        &self,
        base: CallHierarchyFeatureOptions,
    ) -> CallHierarchyFeatureOptions {
        if self.call_hierarchy_overrides.has_overrides() {
            self.call_hierarchy_overrides.apply_to(base)
        } else if self.call_hierarchy != CallHierarchyFeatureOptions::default() {
            self.call_hierarchy
        } else {
            base
        }
    }

    pub(crate) fn workspace_diagnostics_layered_over(
        &self,
        base: WorkspaceDiagnosticsFeatureOptions,
    ) -> WorkspaceDiagnosticsFeatureOptions {
        if self.workspace_diagnostics_overrides.has_overrides() {
            self.workspace_diagnostics_overrides.apply_to(base)
        } else if self.workspace_diagnostics != WorkspaceDiagnosticsFeatureOptions::default() {
            self.workspace_diagnostics
        } else {
            base
        }
    }
}

impl<'de> Deserialize<'de> for ServerOptions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct RawServerOptions {
            #[serde(default)]
            workspace_symbols: WorkspaceSymbolFeatureOptionsOverrides,
            #[serde(default)]
            completion: CompletionFeatureOptionsOverrides,
            #[serde(default)]
            rename: RenameFeatureOptionsOverrides,
            #[serde(default)]
            call_hierarchy: CallHierarchyFeatureOptionsOverrides,
            #[serde(default)]
            workspace_diagnostics: WorkspaceDiagnosticsFeatureOptionsOverrides,
        }

        let raw = RawServerOptions::deserialize(deserializer)?;
        Ok(Self {
            workspace_symbols: raw
                .workspace_symbols
                .apply_to(WorkspaceSymbolFeatureOptions::default()),
            completion: raw.completion.apply_to(CompletionFeatureOptions::default()),
            rename: raw.rename.apply_to(RenameFeatureOptions::default()),
            call_hierarchy: raw
                .call_hierarchy
                .apply_to(CallHierarchyFeatureOptions::default()),
            workspace_diagnostics: raw
                .workspace_diagnostics
                .apply_to(WorkspaceDiagnosticsFeatureOptions::default()),
            workspace_symbols_overrides: raw.workspace_symbols,
            completion_overrides: raw.completion,
            rename_overrides: raw.rename,
            call_hierarchy_overrides: raw.call_hierarchy,
            workspace_diagnostics_overrides: raw.workspace_diagnostics,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WorkspaceSymbolFeatureOptionsOverrides {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    max_files: Option<usize>,
}

impl WorkspaceSymbolFeatureOptionsOverrides {
    fn has_overrides(self) -> bool {
        self.enabled.is_some() || self.max_files.is_some()
    }

    fn apply_to(self, base: WorkspaceSymbolFeatureOptions) -> WorkspaceSymbolFeatureOptions {
        WorkspaceSymbolFeatureOptions {
            enabled: self.enabled.unwrap_or(base.enabled),
            max_files: self.max_files.unwrap_or(base.max_files),
        }
    }
}

/// Configuration for `workspace/symbol`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSymbolFeatureOptions {
    /// Whether the workspace symbol index should serve requests.
    #[serde(default = "default_workspace_symbols_enabled")]
    pub enabled: bool,
    /// Maximum number of closed workspace files to index.
    #[serde(default = "default_workspace_symbols_max_files")]
    pub max_files: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CompletionFeatureOptionsOverrides {
    #[serde(default)]
    include_runtime_names: Option<bool>,
    #[serde(default)]
    include_keywords: Option<bool>,
}

impl CompletionFeatureOptionsOverrides {
    fn has_overrides(self) -> bool {
        self.include_runtime_names.is_some() || self.include_keywords.is_some()
    }

    fn apply_to(self, base: CompletionFeatureOptions) -> CompletionFeatureOptions {
        CompletionFeatureOptions {
            include_runtime_names: self
                .include_runtime_names
                .unwrap_or(base.include_runtime_names),
            include_keywords: self.include_keywords.unwrap_or(base.include_keywords),
        }
    }
}

/// Configuration for `textDocument/completion`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompletionFeatureOptions {
    /// Include runtime-provided parameter names.
    #[serde(default = "default_completion_include_runtime_names")]
    pub include_runtime_names: bool,
    /// Include shell keywords in command-position completion.
    #[serde(default = "default_completion_include_keywords")]
    pub include_keywords: bool,
}

impl Default for CompletionFeatureOptions {
    fn default() -> Self {
        Self {
            include_runtime_names: true,
            include_keywords: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RenameFeatureOptionsOverrides {
    #[serde(default)]
    allow_cross_file: Option<bool>,
}

impl RenameFeatureOptionsOverrides {
    fn has_overrides(self) -> bool {
        self.allow_cross_file.is_some()
    }

    fn apply_to(self, base: RenameFeatureOptions) -> RenameFeatureOptions {
        RenameFeatureOptions {
            allow_cross_file: self.allow_cross_file.unwrap_or(base.allow_cross_file),
        }
    }
}

/// Configuration for rename requests.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenameFeatureOptions {
    /// Allow rename edits outside the current document.
    #[serde(default = "default_cross_file_rename_enabled")]
    pub allow_cross_file: bool,
}

impl Default for RenameFeatureOptions {
    fn default() -> Self {
        Self {
            allow_cross_file: default_cross_file_rename_enabled(),
        }
    }
}

fn default_cross_file_rename_enabled() -> bool {
    true
}

impl Default for WorkspaceSymbolFeatureOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            max_files: 5000,
        }
    }
}

fn default_workspace_symbols_enabled() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CallHierarchyFeatureOptionsOverrides {
    #[serde(default)]
    max_files: Option<usize>,
}

impl CallHierarchyFeatureOptionsOverrides {
    fn has_overrides(self) -> bool {
        self.max_files.is_some()
    }

    fn apply_to(self, base: CallHierarchyFeatureOptions) -> CallHierarchyFeatureOptions {
        CallHierarchyFeatureOptions {
            max_files: self.max_files.unwrap_or(base.max_files),
        }
    }
}

/// Configuration for cross-file call hierarchy.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyFeatureOptions {
    /// Maximum number of workspace files to index for the call graph.
    #[serde(default = "default_call_hierarchy_max_files")]
    pub max_files: usize,
}

impl Default for CallHierarchyFeatureOptions {
    fn default() -> Self {
        Self {
            max_files: default_call_hierarchy_max_files(),
        }
    }
}

fn default_call_hierarchy_max_files() -> usize {
    10_000
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDiagnosticsFeatureOptionsOverrides {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    max_files: Option<usize>,
    #[serde(default)]
    max_entries: Option<usize>,
    #[serde(default)]
    max_source_bytes: Option<usize>,
}

impl WorkspaceDiagnosticsFeatureOptionsOverrides {
    fn has_overrides(self) -> bool {
        self.enabled.is_some()
            || self.max_files.is_some()
            || self.max_entries.is_some()
            || self.max_source_bytes.is_some()
    }

    fn apply_to(
        self,
        base: WorkspaceDiagnosticsFeatureOptions,
    ) -> WorkspaceDiagnosticsFeatureOptions {
        WorkspaceDiagnosticsFeatureOptions {
            enabled: self.enabled.unwrap_or(base.enabled),
            max_files: self.max_files.unwrap_or(base.max_files),
            max_entries: self.max_entries.unwrap_or(base.max_entries),
            max_source_bytes: self.max_source_bytes.unwrap_or(base.max_source_bytes),
        }
    }
}

/// Configuration for bounded `workspace/diagnostic` requests.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiagnosticsFeatureOptions {
    /// Whether workspace diagnostic requests are advertised and served.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum number of workspace files returned by one request.
    #[serde(default = "default_workspace_diagnostics_max_files")]
    pub max_files: usize,
    /// Maximum number of filesystem entries visited by one request.
    #[serde(default = "default_workspace_diagnostics_max_entries")]
    pub max_entries: usize,
    /// Maximum source bytes read and analyzed by one request.
    #[serde(default = "default_workspace_diagnostics_max_source_bytes")]
    pub max_source_bytes: usize,
}

impl Default for WorkspaceDiagnosticsFeatureOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            max_files: default_workspace_diagnostics_max_files(),
            max_entries: default_workspace_diagnostics_max_entries(),
            max_source_bytes: default_workspace_diagnostics_max_source_bytes(),
        }
    }
}

fn default_workspace_diagnostics_max_files() -> usize {
    1_000
}

fn default_workspace_diagnostics_max_entries() -> usize {
    10_000
}

fn default_workspace_diagnostics_max_source_bytes() -> usize {
    32 * 1024 * 1024
}

fn default_workspace_symbols_max_files() -> usize {
    5000
}

fn default_completion_include_runtime_names() -> bool {
    true
}

fn default_completion_include_keywords() -> bool {
    true
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TracingOptions {
    pub(crate) log_file: Option<std::path::PathBuf>,
    pub(crate) log_level: Option<logging::LogLevel>,
}

#[derive(Debug, Default)]
pub(crate) struct AllOptions {
    pub(crate) global: GlobalOptions,
    pub(crate) workspace: Option<WorkspaceOptionsMap>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct InitializationOptions {
    #[serde(default)]
    shuck: GlobalOptions,
    #[serde(default)]
    workspace: Option<WorkspaceOptionsMap>,
}

impl AllOptions {
    pub(crate) fn from_value(value: serde_json::Value) -> Self {
        if value
            .as_object()
            .is_some_and(|object| object.contains_key("shuck"))
        {
            let options =
                serde_json::from_value::<InitializationOptions>(value).unwrap_or_default();
            return Self {
                global: options.shuck,
                workspace: options.workspace,
            };
        }

        let global = serde_json::from_value::<GlobalOptions>(value).unwrap_or_default();
        Self {
            global,
            workspace: None,
        }
    }
}
