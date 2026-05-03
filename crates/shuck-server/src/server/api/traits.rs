use anyhow::anyhow;
use lsp_server::ErrorCode;
use lsp_types::notification::Notification as LspNotification;
use lsp_types::request::Request;

use crate::server::Result;
use crate::session::{Client, DocumentSnapshot, Session};

pub(super) trait RequestHandler {
    type RequestType: Request;
    const METHOD: &'static str = <<Self as RequestHandler>::RequestType as Request>::METHOD;
}

pub(super) trait SyncRequestHandler: RequestHandler {
    fn run(
        session: &mut Session,
        client: &Client,
        params: <<Self as RequestHandler>::RequestType as Request>::Params,
    ) -> Result<<<Self as RequestHandler>::RequestType as Request>::Result>;
}

pub(super) trait BackgroundDocumentRequestHandler: RequestHandler {
    fn document_url(
        params: &<<Self as RequestHandler>::RequestType as Request>::Params,
    ) -> std::borrow::Cow<'_, lsp_types::Url>;

    fn run_without_snapshot(
        _client: &Client,
        _params: <<Self as RequestHandler>::RequestType as Request>::Params,
    ) -> Result<<<Self as RequestHandler>::RequestType as Request>::Result> {
        Err(crate::server::Error::new(
            anyhow!("text document URI does not point to an open document"),
            ErrorCode::InvalidRequest,
        ))
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: <<Self as RequestHandler>::RequestType as Request>::Params,
    ) -> Result<<<Self as RequestHandler>::RequestType as Request>::Result>;
}

pub(super) trait NotificationHandler {
    type NotificationType: LspNotification;
    const METHOD: &'static str =
        <<Self as NotificationHandler>::NotificationType as LspNotification>::METHOD;
}

pub(super) trait SyncNotificationHandler: NotificationHandler {
    fn run(
        session: &mut Session,
        client: &Client,
        params: <<Self as NotificationHandler>::NotificationType as LspNotification>::Params,
    ) -> Result<()>;
}
