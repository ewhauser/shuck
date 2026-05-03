use std::sync::{Arc, OnceLock};

use crate::{Client, session::ClientOptions};

#[derive(Clone, Debug, Default)]
pub(crate) struct ClientSettings;

#[derive(Clone, Debug, Default)]
pub struct ShuckSettings;

pub struct GlobalClientSettings {
    options: ClientOptions,
    settings: OnceLock<Arc<ClientSettings>>,
    #[allow(dead_code)]
    client: Client,
}

impl GlobalClientSettings {
    pub(super) fn new(options: ClientOptions, client: Client) -> Self {
        Self {
            options,
            settings: OnceLock::new(),
            client,
        }
    }

    fn settings_impl(&self) -> &Arc<ClientSettings> {
        self.settings.get_or_init(|| {
            let _ = &self.options;
            Arc::new(ClientSettings)
        })
    }

    pub(super) fn to_settings_arc(&self) -> Arc<ClientSettings> {
        self.settings_impl().clone()
    }
}
