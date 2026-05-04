use std::any::TypeId;
use std::fmt::Display;

use anyhow::{Context, anyhow};
use lsp_server::{ErrorCode, Message, Notification, RequestId, ResponseError};
use serde_json::Value;

use crate::Session;
use crate::server::{ConnectionSender, Event, MainLoopSender};

pub(crate) type ClientResponseHandler = Box<dyn FnOnce(&Client, lsp_server::Response) + Send>;

#[derive(Clone, Debug)]
pub struct Client {
    main_loop_sender: MainLoopSender,
    client_sender: ConnectionSender,
}

impl Client {
    pub fn new(main_loop_sender: MainLoopSender, client_sender: ConnectionSender) -> Self {
        Self {
            main_loop_sender,
            client_sender,
        }
    }

    pub(crate) fn send_request<R>(
        &self,
        session: &Session,
        params: R::Params,
        response_handler: impl FnOnce(&Client, R::Result) + Send + 'static,
    ) -> crate::Result<()>
    where
        R: lsp_types::request::Request,
    {
        let response_handler = Box::new(move |client: &Client, response: lsp_server::Response| {
            match (response.error, response.result) {
                (Some(err), _) => {
                    tracing::error!(
                        "Client request failed (method={} code={}): {}",
                        R::METHOD,
                        err.code,
                        err.message
                    );
                }
                (None, Some(response)) => match serde_json::from_value(response) {
                    Ok(response) => response_handler(client, response),
                    Err(error) => {
                        tracing::error!(
                            "Failed to deserialize client response for {}: {error}",
                            R::METHOD
                        );
                    }
                },
                (None, None) => {
                    if TypeId::of::<R::Result>() == TypeId::of::<()>() {
                        match serde_json::from_value(Value::Null) {
                            Ok(response) => response_handler(client, response),
                            Err(error) => {
                                tracing::error!(
                                    "Failed to deserialize unit client response for {}: {error}",
                                    R::METHOD
                                );
                            }
                        }
                    } else {
                        tracing::error!(
                            "Client response missing result and error for {}",
                            R::METHOD
                        );
                    }
                }
            }
        });

        let id = session
            .request_queue()
            .outgoing()
            .register(response_handler);
        self.client_sender
            .send(Message::Request(lsp_server::Request {
                id,
                method: R::METHOD.to_string(),
                params: serde_json::to_value(params).context("Failed to serialize params")?,
            }))
            .with_context(|| format!("Failed to send request {}", R::METHOD))?;
        Ok(())
    }

    pub(crate) fn send_notification<N>(&self, params: N::Params) -> crate::Result<()>
    where
        N: lsp_types::notification::Notification,
    {
        self.client_sender
            .send(Message::Notification(Notification::new(
                N::METHOD.to_string(),
                params,
            )))
            .map_err(|error| anyhow!("Failed to send notification {}: {error}", N::METHOD))
    }

    pub(crate) fn respond<R>(
        &self,
        id: &RequestId,
        result: crate::server::Result<R>,
    ) -> crate::Result<()>
    where
        R: serde::Serialize,
    {
        let response = match result {
            Ok(value) => lsp_server::Response::new_ok(id.clone(), value),
            Err(crate::server::Error { code, error }) => {
                lsp_server::Response::new_err(id.clone(), code as i32, error.to_string())
            }
        };

        self.main_loop_sender
            .send(Event::SendResponse(response))
            .map_err(|error| anyhow!("Failed to queue response {id}: {error}"))
    }

    pub(crate) fn respond_err(&self, id: RequestId, error: ResponseError) -> crate::Result<()> {
        self.main_loop_sender
            .send(Event::SendResponse(lsp_server::Response {
                id,
                result: None,
                error: Some(error),
            }))
            .map_err(|send_error| anyhow!("Failed to queue error response: {send_error}"))
    }

    pub(crate) fn show_message(
        &self,
        message: impl Display,
        message_type: lsp_types::MessageType,
    ) -> crate::Result<()> {
        self.send_notification::<lsp_types::notification::ShowMessage>(
            lsp_types::ShowMessageParams {
                typ: message_type,
                message: message.to_string(),
            },
        )
    }

    pub(crate) fn log_message(
        &self,
        message: impl Display,
        message_type: lsp_types::MessageType,
    ) -> crate::Result<()> {
        self.send_notification::<lsp_types::notification::LogMessage>(lsp_types::LogMessageParams {
            typ: message_type,
            message: message.to_string(),
        })
    }

    pub(crate) fn show_error_message(&self, message: impl Display) {
        if let Err(err) = self.show_message(message, lsp_types::MessageType::ERROR) {
            tracing::error!("Failed to send error message to client: {err}");
        }
    }

    pub(crate) fn cancel(&self, session: &mut Session, id: RequestId) -> crate::Result<()> {
        let method_name = session.request_queue_mut().incoming_mut().cancel(&id);
        if let Some(method_name) = method_name {
            tracing::debug!("Cancelled request id={id} method={method_name}");
            self.client_sender
                .send(Message::Response(lsp_server::Response {
                    id,
                    result: None,
                    error: Some(ResponseError {
                        code: ErrorCode::RequestCanceled as i32,
                        message: "request was cancelled by client".to_owned(),
                        data: None,
                    }),
                }))?;
        }
        Ok(())
    }
}
