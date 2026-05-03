use lsp_types as types;
use lsp_types::notification as notif;

use crate::server::Result;
use crate::session::{Client, Session};

pub(crate) struct DidChangeWatchedFiles;

impl super::super::traits::NotificationHandler for DidChangeWatchedFiles {
    type NotificationType = notif::DidChangeWatchedFiles;
}

impl super::super::traits::SyncNotificationHandler for DidChangeWatchedFiles {
    fn run(
        session: &mut Session,
        client: &Client,
        params: types::DidChangeWatchedFilesParams,
    ) -> Result<()> {
        session.reload_settings(&params.changes, client);
        Ok(())
    }
}
