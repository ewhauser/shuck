use std::panic::UnwindSafe;

use anyhow::anyhow;
use lsp_server::{self as server, RequestId};
use lsp_types::{notification::Notification, request::Request};
use notifications as notification;
use requests as request;

use crate::server::Result;
use crate::server::schedule::{BackgroundSchedule, Task};
use crate::server::{
    api::traits::{
        BackgroundDocumentRequestHandler, NotificationHandler, RequestHandler,
        SyncNotificationHandler, SyncRequestHandler,
    },
    schedule,
};
use crate::session::{Client, Session};

mod diagnostics;
mod notifications;
mod requests;
mod traits;

macro_rules! define_document_url {
    ($params:ident: &$p:ty) => {
        fn document_url($params: &$p) -> std::borrow::Cow<'_, lsp_types::Url> {
            std::borrow::Cow::Borrowed(&$params.text_document.uri)
        }
    };
}

use define_document_url;

pub(super) fn request(req: server::Request) -> Task {
    let id = req.id.clone();

    match req.method.as_str() {
        request::CodeActions::METHOD => {
            background_request_task::<request::CodeActions>(req, BackgroundSchedule::Worker)
        }
        request::CodeActionResolve::METHOD => {
            sync_request_task::<request::CodeActionResolve>(req)
        }
        request::DocumentDiagnostic::METHOD => {
            background_request_task::<request::DocumentDiagnostic>(req, BackgroundSchedule::Worker)
        }
        request::ExecuteCommand::METHOD => sync_request_task::<request::ExecuteCommand>(req),
        request::Format::METHOD => {
            background_request_task::<request::Format>(req, BackgroundSchedule::Fmt)
        }
        request::FormatRange::METHOD => {
            background_request_task::<request::FormatRange>(req, BackgroundSchedule::Fmt)
        }
        request::Hover::METHOD => {
            background_request_task::<request::Hover>(req, BackgroundSchedule::Worker)
        }
        lsp_types::request::Shutdown::METHOD => sync_request_task::<request::ShutdownHandler>(req),
        method => {
            let result: Result<()> = Err(Error::new(
                anyhow!("Unknown request: {method}"),
                server::ErrorCode::MethodNotFound,
            ));
            return Task::immediate(id, result);
        }
    }
    .unwrap_or_else(|err| {
        Task::sync(move |_session, client| {
            client.show_error_message(
                "Shuck failed to handle a request from the editor. Check the logs for more details.",
            );
            respond_silent_error(
                id,
                client,
                lsp_server::ResponseError {
                    code: err.code as i32,
                    message: err.to_string(),
                    data: None,
                },
            );
        })
    })
}

pub(super) fn notification(notif: server::Notification) -> Task {
    match notif.method.as_str() {
        notification::DidChange::METHOD => sync_notification_task::<notification::DidChange>(notif),
        notification::DidChangeConfiguration::METHOD => {
            sync_notification_task::<notification::DidChangeConfiguration>(notif)
        }
        notification::DidChangeWatchedFiles::METHOD => {
            sync_notification_task::<notification::DidChangeWatchedFiles>(notif)
        }
        notification::DidChangeWorkspace::METHOD => {
            sync_notification_task::<notification::DidChangeWorkspace>(notif)
        }
        notification::DidClose::METHOD => sync_notification_task::<notification::DidClose>(notif),
        notification::DidOpen::METHOD => sync_notification_task::<notification::DidOpen>(notif),
        lsp_types::notification::Cancel::METHOD => {
            sync_notification_task::<notification::CancelNotificationHandler>(notif)
        }
        lsp_types::notification::SetTrace::METHOD => Ok(Task::nothing()),
        _ => Ok(Task::nothing()),
    }
    .unwrap_or_else(|err| {
        Task::sync(move |_session, client| {
            tracing::error!("Failed to handle notification: {err}");
            client.show_error_message(
                "Shuck failed to handle a notification from the editor. Check the logs for more details.",
            );
        })
    })
}

fn sync_request_task<R: SyncRequestHandler>(req: server::Request) -> Result<Task>
where
    <<R as RequestHandler>::RequestType as Request>::Params: UnwindSafe,
{
    let (id, params) = cast_request::<R>(req)?;
    Ok(Task::sync(move |session, client| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            R::run(session, client, params)
        }));

        let response = match result {
            Ok(result) => result,
            Err(error) => Err(Error::new(
                anyhow!(panic_message(&error).unwrap_or("request handler panicked".into())),
                lsp_server::ErrorCode::InternalError,
            )),
        };
        respond::<R>(&id, response, client);
    }))
}

fn background_request_task<R: BackgroundDocumentRequestHandler>(
    req: server::Request,
    schedule: schedule::BackgroundSchedule,
) -> Result<Task>
where
    <<R as RequestHandler>::RequestType as Request>::Params: UnwindSafe,
{
    let (id, params) = cast_request::<R>(req)?;
    Ok(Task::background(schedule, move |session: &Session| {
        let cancellation_token = session
            .request_queue()
            .incoming()
            .cancellation_token(&id)
            .expect("request should be registered before scheduling");
        let Some(snapshot) = session.take_snapshot(R::document_url(&params).into_owned()) else {
            tracing::debug!(
                "Skipping {} because the document is no longer open",
                R::METHOD
            );
            return Box::new(move |client| {
                if cancellation_token.is_cancelled() {
                    return;
                }

                respond::<R>(&id, R::run_without_snapshot(client, params), client);
            });
        };
        Box::new(move |client| {
            if cancellation_token.is_cancelled() {
                return;
            }

            let result =
                std::panic::catch_unwind(|| R::run_with_snapshot(snapshot, client, params));
            let response = match result {
                Ok(result) => result,
                Err(error) => Err(Error::new(
                    anyhow!(panic_message(&error).unwrap_or("request handler panicked".into())),
                    lsp_server::ErrorCode::InternalError,
                )),
            };
            if cancellation_token.is_cancelled() {
                return;
            }
            respond::<R>(&id, response, client);
        })
    }))
}

fn sync_notification_task<N: SyncNotificationHandler>(notif: server::Notification) -> Result<Task> {
    let params = cast_notification::<N>(notif)?;
    Ok(Task::sync(move |session, client| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            N::run(session, client, params)
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                tracing::error!("Notification handler failed: {err}");
                client.show_error_message(
                    "Shuck encountered a problem. Check the logs for more details.",
                );
            }
            Err(error) => {
                tracing::error!(
                    "Notification handler panicked: {}",
                    panic_message(&error).unwrap_or("unknown panic".into())
                );
                client.show_error_message(
                    "Shuck encountered a panic. Check the logs for more details.",
                );
            }
        }
    }))
}

fn cast_request<Req>(
    request: server::Request,
) -> Result<(
    RequestId,
    <<Req as RequestHandler>::RequestType as Request>::Params,
)>
where
    Req: RequestHandler,
    <<Req as RequestHandler>::RequestType as Request>::Params: UnwindSafe,
{
    request.extract(Req::METHOD).map_err(|err| match err {
        json_err @ server::ExtractError::JsonError { .. } => Error::new(
            anyhow!("JSON parsing failure:\n{json_err}"),
            server::ErrorCode::InvalidParams,
        ),
        server::ExtractError::MethodMismatch(_) => unreachable!(),
    })
}

fn cast_notification<Notif>(
    notification: server::Notification,
) -> Result<<<Notif as NotificationHandler>::NotificationType as Notification>::Params>
where
    Notif: NotificationHandler,
{
    notification
        .extract(Notif::METHOD)
        .map_err(|err| match err {
            json_err @ server::ExtractError::JsonError { .. } => Error::new(
                anyhow!("JSON parsing failure:\n{json_err}"),
                server::ErrorCode::InvalidParams,
            ),
            server::ExtractError::MethodMismatch(_) => unreachable!(),
        })
}

fn respond<Req>(
    id: &RequestId,
    response: Result<<<Req as RequestHandler>::RequestType as Request>::Result>,
    client: &Client,
) where
    Req: RequestHandler,
    <<Req as RequestHandler>::RequestType as Request>::Result: serde::Serialize,
{
    if let Err(err) = client.respond(id, response) {
        tracing::error!("Failed to send response for {}: {err}", Req::METHOD);
    }
}

fn respond_silent_error(id: RequestId, client: &Client, error: lsp_server::ResponseError) {
    if let Err(send_error) = client.respond_err(id, error) {
        tracing::error!("Failed to send error response: {send_error}");
    }
}

fn panic_message(error: &Box<dyn std::any::Any + Send + 'static>) -> Option<String> {
    error.downcast_ref::<String>().cloned().or_else(|| {
        error
            .downcast_ref::<&'static str>()
            .map(|msg| (*msg).to_owned())
    })
}

#[derive(Debug)]
pub(crate) struct Error {
    pub(crate) code: lsp_server::ErrorCode,
    pub(crate) error: anyhow::Error,
}

impl Error {
    pub(crate) fn new(error: anyhow::Error, code: lsp_server::ErrorCode) -> Self {
        Self { code, error }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for Error {}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Self::new(error, lsp_server::ErrorCode::InternalError)
    }
}

pub(crate) trait LSPResult<T> {
    fn with_failure_code(self, code: lsp_server::ErrorCode) -> Result<T>;
}

impl<T, E> LSPResult<T> for std::result::Result<T, E>
where
    E: Into<anyhow::Error>,
{
    fn with_failure_code(self, code: lsp_server::ErrorCode) -> Result<T> {
        self.map_err(|error| Error::new(error.into(), code))
    }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_server::{Message, Request as LspRequest, RequestId};
    use lsp_types::{
        ClientCapabilities, DidChangeTextDocumentParams, DocumentDiagnosticParams,
        DocumentDiagnosticReportResult, HoverParams, PartialResultParams, Position,
        TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentPositionParams, Url,
        VersionedTextDocumentIdentifier, WorkDoneProgressParams,
    };

    use super::*;
    use crate::server::Event;
    use crate::session::AllOptions;
    use crate::{PositionEncoding, Workspace, Workspaces};

    fn test_client() -> (Client, channel::Receiver<Event>, channel::Receiver<Message>) {
        let (event_tx, event_rx) = channel::unbounded();
        let (connection_tx, connection_rx) = channel::unbounded::<Message>();
        (Client::new(event_tx, connection_tx), event_rx, connection_rx)
    }

    fn test_session(client: &Client) -> Session {
        let workspace_url = Url::from_file_path(std::env::current_dir().unwrap())
            .expect("current directory should convert to a file URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_url)]);
        let AllOptions { global, .. } = AllOptions::from_value(serde_json::Value::Null, client);
        let global = global.into_settings(client.clone());

        Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::default(),
            global,
            &workspaces,
            client,
        )
        .expect("test session should initialize")
    }

    #[test]
    fn missing_snapshot_background_request_still_sends_a_response() {
        let (client, event_rx, _connection_rx) = test_client();
        let mut session = test_session(&client);
        let uri = Url::parse("file:///tmp/missing.sh").expect("test URI should parse");
        let request_id: RequestId = 1.into();
        session
            .request_queue_mut()
            .incoming_mut()
            .register(request_id.clone(), request::Hover::METHOD.to_string());

        let request = LspRequest {
            id: request_id.clone(),
            method: request::Hover::METHOD.to_string(),
            params: serde_json::to_value(HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: Position::new(0, 0),
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            })
            .expect("hover params should serialize"),
        };

        let task = background_request_task::<request::Hover>(request, BackgroundSchedule::Worker)
            .expect("hover request should schedule");
        task.run_for_test(&mut session, &client);

        let Event::SendResponse(response) = event_rx
            .try_recv()
            .expect("missing snapshot path should enqueue a response")
        else {
            panic!("expected queued response event");
        };

        assert_eq!(response.id, request_id);
        assert!(response.error.is_none());
        assert_eq!(response.result, Some(serde_json::Value::Null));
    }

    struct SlowHoverRequest;

    impl lsp_types::request::Request for SlowHoverRequest {
        type Params = HoverParams;
        type Result = Option<lsp_types::Hover>;
        const METHOD: &'static str = "shuck/testSlowHover";
    }

    struct SlowHover;

    impl RequestHandler for SlowHover {
        type RequestType = SlowHoverRequest;
    }

    impl BackgroundDocumentRequestHandler for SlowHover {
        fn document_url(params: &HoverParams) -> std::borrow::Cow<'_, lsp_types::Url> {
            std::borrow::Cow::Borrowed(&params.text_document_position_params.text_document.uri)
        }

        fn run_without_snapshot(
            _client: &Client,
            _params: HoverParams,
        ) -> Result<Option<lsp_types::Hover>> {
            Ok(None)
        }

        fn run_with_snapshot(
            _snapshot: crate::session::DocumentSnapshot,
            _client: &Client,
            _params: HoverParams,
        ) -> Result<Option<lsp_types::Hover>> {
            std::thread::sleep(std::time::Duration::from_millis(100));
            Ok(None)
        }
    }

    #[test]
    fn cancelled_background_request_drops_late_response() {
        let (client, event_rx, connection_rx) = test_client();
        let mut session = test_session(&client);
        let uri = Url::parse("file:///tmp/cancelled.sh").expect("test URI should parse");
        session.open_text_document(
            uri.clone(),
            crate::TextDocument::new("foo=1\n".to_owned(), 1).with_language_id("shellscript"),
        );
        let request_id: RequestId = 2.into();
        session
            .request_queue_mut()
            .incoming_mut()
            .register(request_id.clone(), SlowHover::METHOD.to_string());

        let request = LspRequest {
            id: request_id.clone(),
            method: SlowHover::METHOD.to_string(),
            params: serde_json::to_value(HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: Position::new(0, 0),
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            })
            .expect("hover params should serialize"),
        };

        let task = background_request_task::<SlowHover>(request, BackgroundSchedule::Worker)
            .expect("slow hover should schedule");
        let run = task
            .build_background_for_test(&session)
            .expect("expected a background task");
        let thread_client = client.clone();
        let handle = std::thread::spawn(move || run(&thread_client));

        std::thread::sleep(std::time::Duration::from_millis(10));
        client
            .cancel(&mut session, request_id.clone())
            .expect("cancel should succeed");
        handle.join().expect("slow hover thread should join");

        let response = loop {
            let cancellation = connection_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("cancel should send a response to the client");
            match cancellation {
                Message::Response(response) => break response,
                Message::Notification(_) => continue,
                Message::Request(request) => {
                    panic!("unexpected client request during cancellation test: {}", request.method)
                }
            }
        };
        assert_eq!(response.id, request_id);
        assert_eq!(
            response
                .error
                .expect("cancellation response should carry an error")
                .code,
            lsp_server::ErrorCode::RequestCanceled as i32
        );
        assert!(event_rx.try_recv().is_err());
    }

    struct SlowDiagnostic;

    impl RequestHandler for SlowDiagnostic {
        type RequestType = lsp_types::request::DocumentDiagnosticRequest;
    }

    impl BackgroundDocumentRequestHandler for SlowDiagnostic {
        super::define_document_url!(params: &DocumentDiagnosticParams);

        fn run_without_snapshot(
            _client: &Client,
            _params: DocumentDiagnosticParams,
        ) -> Result<DocumentDiagnosticReportResult> {
            request::DocumentDiagnostic::run_without_snapshot(_client, _params)
        }

        fn run_with_snapshot(
            snapshot: crate::session::DocumentSnapshot,
            client: &Client,
            params: DocumentDiagnosticParams,
        ) -> Result<DocumentDiagnosticReportResult> {
            std::thread::sleep(std::time::Duration::from_millis(100));
            request::DocumentDiagnostic::run_with_snapshot(snapshot, client, params)
        }
    }

    #[test]
    fn did_change_then_cancelled_diagnostic_request_drops_late_response() {
        let (client, event_rx, _connection_rx) = test_client();
        let mut session = test_session(&client);
        let uri = Url::parse("file:///tmp/cancelled-diagnostic.sh").expect("test URI should parse");
        session.open_text_document(
            uri.clone(),
            crate::TextDocument::new("foo=1\n".to_owned(), 1).with_language_id("shellscript"),
        );

        let change_notification = sync_notification_task::<notification::DidChange>(
            lsp_server::Notification::new(
                notification::DidChange::METHOD.to_owned(),
                serde_json::to_value(DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier {
                        uri: uri.clone(),
                        version: 2,
                    },
                    content_changes: vec![TextDocumentContentChangeEvent {
                        range: None,
                        range_length: None,
                        text: "bar=1\n".to_owned(),
                    }],
                })
                .expect("didChange params should serialize"),
            ),
        )
        .expect("didChange notification should schedule");
        change_notification.run_for_test(&mut session, &client);

        let request_id: RequestId = 3.into();
        session
            .request_queue_mut()
            .incoming_mut()
            .register(request_id.clone(), SlowDiagnostic::METHOD.to_string());

        let request = LspRequest {
            id: request_id.clone(),
            method: SlowDiagnostic::METHOD.to_string(),
            params: serde_json::to_value(DocumentDiagnosticParams {
                text_document: TextDocumentIdentifier { uri },
                identifier: None,
                previous_result_id: None,
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            })
            .expect("diagnostic params should serialize"),
        };

        let task = background_request_task::<SlowDiagnostic>(request, BackgroundSchedule::Worker)
            .expect("diagnostic request should schedule");
        let run = task
            .build_background_for_test(&session)
            .expect("expected a background task");
        let thread_client = client.clone();
        let handle = std::thread::spawn(move || run(&thread_client));

        std::thread::sleep(std::time::Duration::from_millis(10));
        client
            .cancel(&mut session, request_id.clone())
            .expect("cancel should succeed");
        handle.join().expect("diagnostic thread should join");
        assert!(event_rx.try_recv().is_err());
    }
}
