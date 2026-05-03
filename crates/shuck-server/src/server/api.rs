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
            return Box::new(|_| {});
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
