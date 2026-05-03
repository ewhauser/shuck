use anyhow::anyhow;
use crossbeam::select;
use lsp_server::Message;
use lsp_types::{
    self as types, DidChangeWatchedFilesRegistrationOptions, FileSystemWatcher,
    notification::Notification as _,
};

use crate::server::{api, schedule};
use crate::{Server, session::Client};

pub type MainLoopSender = crossbeam::channel::Sender<Event>;
pub(crate) type MainLoopReceiver = crossbeam::channel::Receiver<Event>;

impl Server {
    pub(super) fn main_loop(&mut self) -> crate::Result<()> {
        let mut scheduler = schedule::Scheduler::new(self.worker_threads);
        while let Ok(next_event) = self.next_event() {
            let Some(next_event) = next_event else {
                anyhow::bail!("client exited without proper shutdown sequence");
            };

            match next_event {
                Event::Message(msg) => {
                    let client = Client::new(
                        self.main_loop_sender.clone(),
                        self.connection.sender.clone(),
                    );

                    let task = match msg {
                        Message::Request(req) => {
                            self.session
                                .request_queue_mut()
                                .incoming_mut()
                                .register(req.id.clone(), req.method.clone());

                            if self.session.is_shutdown_requested() {
                                client.respond_err(
                                    req.id,
                                    lsp_server::ResponseError {
                                        code: lsp_server::ErrorCode::InvalidRequest as i32,
                                        message: "shutdown already requested".to_owned(),
                                        data: None,
                                    },
                                )?;
                                continue;
                            }

                            api::request(req)
                        }
                        Message::Notification(notification) => {
                            if notification.method == lsp_types::notification::Exit::METHOD {
                                if !self.session.is_shutdown_requested() {
                                    return Err(anyhow!(
                                        "received exit notification before shutdown request"
                                    ));
                                }
                                return Ok(());
                            }

                            if notification.method == lsp_types::notification::Initialized::METHOD {
                                self.on_initialized(&client);
                            }

                            api::notification(notification)
                        }
                        Message::Response(response) => {
                            if let Some(handler) = self
                                .session
                                .request_queue_mut()
                                .outgoing_mut()
                                .complete(&response.id)
                            {
                                handler(&client, response);
                            } else {
                                tracing::error!(
                                    "Received an unexpected response for request {}",
                                    response.id
                                );
                            }
                            continue;
                        }
                    };

                    scheduler.dispatch(task, &mut self.session, client);
                }
                Event::SendResponse(response) => {
                    if self
                        .session
                        .request_queue_mut()
                        .incoming_mut()
                        .complete(&response.id)
                        .is_some()
                    {
                        self.connection.sender.send(Message::Response(response))?;
                    } else {
                        tracing::trace!("Ignoring response for cancelled request {}", response.id);
                    }
                }
            }
        }

        Ok(())
    }

    fn next_event(&self) -> Result<Option<Event>, crossbeam::channel::RecvError> {
        select!(
            recv(self.connection.receiver) -> msg => Ok(msg.ok().map(Event::Message)),
            recv(self.main_loop_receiver) -> event => event.map(Some),
        )
    }

    fn on_initialized(&mut self, client: &Client) {
        let dynamic_registration = self
            .client_capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.did_change_watched_files)
            .and_then(|watched_files| watched_files.dynamic_registration)
            .unwrap_or_default();

        if !dynamic_registration {
            tracing::warn!(
                "LSP client does not support dynamic watched-file registration; config reloads are disabled"
            );
            return;
        }

        let register_options =
            match serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                watchers: vec![
                    FileSystemWatcher {
                        glob_pattern: types::GlobPattern::String("**/.shuck.toml".into()),
                        kind: None,
                    },
                    FileSystemWatcher {
                        glob_pattern: types::GlobPattern::String("**/shuck.toml".into()),
                        kind: None,
                    },
                ],
            }) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!("Failed to serialize config watcher registration: {error}");
                    return;
                }
            };

        let params = lsp_types::RegistrationParams {
            registrations: vec![lsp_types::Registration {
                id: "shuck-server-watch".into(),
                method: "workspace/didChangeWatchedFiles".into(),
                register_options: Some(register_options),
            }],
        };

        let response_handler = |_: &Client, ()| {
            tracing::info!("Registered configuration file watcher");
        };

        if let Err(err) = client.send_request::<lsp_types::request::RegisterCapability>(
            &self.session,
            params,
            response_handler,
        ) {
            tracing::error!("Failed to register configuration file watcher: {err}");
        }
    }
}

#[derive(Debug)]
pub enum Event {
    Message(lsp_server::Message),
    SendResponse(lsp_server::Response),
}
