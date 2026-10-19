use anyhow::anyhow;
use crossbeam::select;
use lsp_server::Message;
use lsp_types::{
    self as types, DidChangeWatchedFilesRegistrationOptions, FileSystemWatcher,
    notification::Notification as _,
};

use crate::server::{api, schedule};
use crate::session::ClientResponseHandler;
use crate::{Server, session::Client};

pub type MainLoopSender = crossbeam::channel::Sender<Event>;
pub(crate) type MainLoopReceiver = crossbeam::channel::Receiver<Event>;

impl Server {
    pub(super) fn main_loop(&mut self) -> crate::Result<()> {
        let mut scheduler = schedule::Scheduler::new(self.worker_threads);
        while let Ok(next_event) = self.next_event() {
            let Some(next_event) = next_event else {
                return Ok(());
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
                                run_response_handler(handler, &client, &mut self.session, response);
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
                "LSP client does not support dynamic watched-file registration; closed-file index updates and config reloads are disabled"
            );
            return;
        }

        let watchers = watched_files();

        let register_options =
            match serde_json::to_value(DidChangeWatchedFilesRegistrationOptions { watchers }) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!("Failed to serialize workspace watcher registration: {error}");
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

        let response_handler = |_: &Client, session: &mut crate::Session, ()| {
            session.set_project_settings_cache_enabled(true);
            tracing::info!("Registered workspace file watcher");
        };

        if let Err(err) = client.send_request::<lsp_types::request::RegisterCapability>(
            &self.session,
            params,
            response_handler,
        ) {
            tracing::error!("Failed to register workspace file watcher: {err}");
        }
    }
}

/// Run the handler for a client response on the main thread, containing a
/// panic to that one response instead of letting it end the server.
fn run_response_handler(
    handler: ClientResponseHandler,
    client: &Client,
    session: &mut crate::Session,
    response: lsp_server::Response,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handler(client, session, response);
    }));
    if let Err(error) = result {
        tracing::error!(
            "Client response handler panicked: {}",
            api::panic_message(&error).unwrap_or_else(|| "unknown panic".into())
        );
    }
}

fn watched_files() -> Vec<FileSystemWatcher> {
    let mut watchers = vec![
        // Call-hierarchy and workspace-symbol indexes include every discovered
        // shell file, including extensionless shebang and zsh startup files.
        // Watch the workspace broadly so closed-file edits, creates, deletes,
        // and VCS updates invalidate those indexes.
        FileSystemWatcher {
            glob_pattern: types::GlobPattern::String("**/*".into()),
            kind: None,
        },
        // Keep explicit config globs because some client glob implementations
        // do not let a wildcard match leading-dot path components.
        FileSystemWatcher {
            glob_pattern: types::GlobPattern::String("**/.shuck.toml".into()),
            kind: None,
        },
        FileSystemWatcher {
            glob_pattern: types::GlobPattern::String("**/shuck.toml".into()),
            kind: None,
        },
    ];
    // The user-level global config lives outside the workspace, so the
    // relative globs above never cover it; watch its candidate paths
    // explicitly so projects using the global fallback observe edits.
    watchers.extend(
        shuck_config::global_config_candidate_paths()
            .iter()
            .filter_map(|path| path.to_str())
            .map(|path| FileSystemWatcher {
                glob_pattern: types::GlobPattern::String(path.into()),
                kind: None,
            }),
    );
    watchers
}

#[derive(Debug)]
pub enum Event {
    Message(lsp_server::Message),
    SendResponse(lsp_server::Response),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watched_files_include_all_workspace_paths_and_config_dotfiles() {
        let patterns = watched_files()
            .into_iter()
            .filter_map(|watcher| match watcher.glob_pattern {
                types::GlobPattern::String(pattern) => Some(pattern),
                types::GlobPattern::Relative(_) => None,
            })
            .collect::<Vec<_>>();
        assert!(patterns.iter().any(|pattern| pattern == "**/*"));
        assert!(patterns.iter().any(|pattern| pattern == "**/.shuck.toml"));
        assert!(patterns.iter().any(|pattern| pattern == "**/shuck.toml"));
    }

    #[test]
    fn panicking_response_handler_is_contained() {
        let (event_tx, _event_rx) = crossbeam::channel::unbounded();
        let (connection_tx, _connection_rx) = crossbeam::channel::unbounded();
        let client = Client::new(event_tx, connection_tx);
        let workspace_url = lsp_types::Url::from_file_path(std::env::temp_dir())
            .expect("temporary directory should convert to a file URL");
        let workspaces = crate::Workspaces::new(vec![crate::Workspace::default(workspace_url)]);
        let global = crate::GlobalOptions::default().into_settings(client.clone());
        let mut session = crate::Session::new(
            &lsp_types::ClientCapabilities::default(),
            crate::PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let handler: ClientResponseHandler = Box::new(|_, _, _| panic!("handler exploded"));
        run_response_handler(
            handler,
            &client,
            &mut session,
            lsp_server::Response::new_ok(lsp_server::RequestId::from(1), ()),
        );

        assert_eq!(session.open_document_count(), 0);
    }
}
