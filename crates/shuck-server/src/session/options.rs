use lsp_types::Url;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use shuck_config::{FormatConfig, LintConfig, ShuckConfig, apply_config_overrides};

use crate::session::settings::GlobalClientSettings;
use crate::{Client, logging};

pub(crate) type WorkspaceOptionsMap = FxHashMap<Url, ClientOptions>;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GlobalOptions {
    #[serde(flatten)]
    client: ClientOptions,
    #[serde(default)]
    pub(crate) tracing: TracingOptions,
}

impl GlobalOptions {
    pub fn into_settings(self, client: Client) -> GlobalClientSettings {
        GlobalClientSettings::new(self.client, client)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientOptions {
    #[serde(default)]
    pub lint: Option<LintConfig>,
    #[serde(default)]
    pub format: Option<FormatConfig>,
    #[serde(default)]
    pub fix_all: Option<bool>,
    #[serde(default)]
    pub unsafe_fixes: Option<bool>,
    #[serde(default)]
    pub show_syntax_errors: Option<bool>,
}

impl ClientOptions {
    pub(crate) fn to_config_overrides(&self) -> ShuckConfig {
        ShuckConfig {
            lint: self.lint.clone().unwrap_or_default(),
            format: self.format.clone().unwrap_or_default(),
            ..ShuckConfig::default()
        }
    }

    pub(crate) fn merged(global: &Self, workspace: Option<&Self>) -> Self {
        let Some(workspace) = workspace else {
            return global.clone();
        };

        let mut config = global.to_config_overrides();
        apply_config_overrides(&mut config, workspace.to_config_overrides());

        Self {
            lint: (global.lint.is_some() || workspace.lint.is_some()).then_some(config.lint),
            format: (global.format.is_some() || workspace.format.is_some())
                .then_some(config.format),
            fix_all: workspace.fix_all.or(global.fix_all),
            unsafe_fixes: workspace.unsafe_fixes.or(global.unsafe_fixes),
            show_syntax_errors: workspace
                .show_syntax_errors
                .or(global.show_syntax_errors),
        }
    }
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
            let options = serde_json::from_value::<InitializationOptions>(value).unwrap_or_default();
            return Self {
                global: options.shuck,
                workspace: options.workspace,
            };
        }

        let global = serde_json::from_value::<GlobalOptions>(value).unwrap_or_default();
        Self {
            global,
            workspace: Some(WorkspaceOptionsMap::default()),
        }
    }
}
