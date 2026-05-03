use lsp_types as types;
use lsp_types::notification as notif;

use crate::server::Result;
use crate::server::api::LSPResult;
use crate::session::{Client, Session};

pub(crate) struct DidChangeWorkspace;

impl super::super::traits::NotificationHandler for DidChangeWorkspace {
    type NotificationType = notif::DidChangeWorkspaceFolders;
}

impl super::super::traits::SyncNotificationHandler for DidChangeWorkspace {
    fn run(
        session: &mut Session,
        client: &Client,
        params: types::DidChangeWorkspaceFoldersParams,
    ) -> Result<()> {
        for folder in params.event.removed {
            session
                .close_workspace_folder(&folder.uri)
                .with_failure_code(lsp_server::ErrorCode::InternalError)?;
        }
        for folder in params.event.added {
            session
                .open_workspace_folder(folder.uri, client)
                .with_failure_code(lsp_server::ErrorCode::InternalError)?;
        }
        Ok(())
    }
}
