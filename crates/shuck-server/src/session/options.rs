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
    workspace_symbols_overrides: WorkspaceSymbolFeatureOptionsOverrides,
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
        }

        let raw = RawServerOptions::deserialize(deserializer)?;
        Ok(Self {
            workspace_symbols: raw
                .workspace_symbols
                .apply_to(WorkspaceSymbolFeatureOptions::default()),
            workspace_symbols_overrides: raw.workspace_symbols,
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

fn default_workspace_symbols_max_files() -> usize {
    5000
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
    pub(crate) fn from_value(value: serde_json::Value, _client: &Client) -> Self {
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
