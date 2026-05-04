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
        session: &mut Session,
        client: &Client,
        params: types::DidChangeConfigurationParams,
    ) -> Result<()> {
        let all_options = crate::session::AllOptions::from_value(params.settings, client);
        let global_settings = all_options.global.into_settings(client.clone());
        session.update_client_options(global_settings.options().clone());
        Ok(())
    }
}
