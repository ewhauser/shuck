use lsp_types as types;
use lsp_types::notification as notif;

use crate::server::Result;
use crate::session::{Client, Session};

pub(crate) struct DidChangeConfiguration;

impl super::super::traits::NotificationHandler for DidChangeConfiguration {
    type NotificationType = notif::DidChangeConfiguration;
}

impl super::super::traits::SyncNotificationHandler for DidChangeConfiguration {
    fn run(
        _session: &mut Session,
        _client: &Client,
        _params: types::DidChangeConfigurationParams,
    ) -> Result<()> {
        tracing::debug!("Ignoring didChangeConfiguration until client settings are wired");
        Ok(())
    }
}
