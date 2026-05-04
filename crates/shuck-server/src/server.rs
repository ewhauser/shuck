use lsp_server::Connection;
use lsp_types as types;
use lsp_types::InitializeParams;
use lsp_types::{
    ClientCapabilities, CodeActionKind, CodeActionOptions, DiagnosticOptions, OneOf,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    WorkDoneProgressOptions, WorkspaceFoldersServerCapabilities,
};
use std::num::NonZeroUsize;

pub(crate) use self::connection::ConnectionInitializer;
pub use self::connection::ConnectionSender;
use self::schedule::spawn_main_loop;
use crate::PositionEncoding;
pub use crate::server::main_loop::MainLoopSender;
pub(crate) use crate::server::main_loop::{Event, MainLoopReceiver};
use crate::session::{AllOptions, Client, Session};
use crate::workspace::Workspaces;
pub(crate) use api::Error;

mod api;
mod connection;
mod main_loop;
mod schedule;

pub(crate) type Result<T> = std::result::Result<T, api::Error>;

pub struct Server {
    connection: Connection,
    client_capabilities: ClientCapabilities,
    worker_threads: NonZeroUsize,
    main_loop_receiver: MainLoopReceiver,
    main_loop_sender: MainLoopSender,
    session: Session,
}

impl Server {
    pub(crate) fn new(
        worker_threads: NonZeroUsize,
        connection: ConnectionInitializer,
    ) -> crate::Result<Self> {
        let (id, init_params) = connection.initialize_start()?;
        let client_capabilities = init_params.capabilities;
        let position_encoding = Self::find_best_position_encoding(&client_capabilities);
        let server_capabilities = Self::server_capabilities(position_encoding);
        let connection = connection.initialize_finish(
            id,
            &server_capabilities,
            crate::SERVER_NAME,
            crate::version(),
        )?;

        let (main_loop_sender, main_loop_receiver) = crossbeam::channel::bounded(32);

        #[allow(deprecated)]
        let InitializeParams {
            initialization_options,
            root_path,
            root_uri,
            workspace_folders,
            ..
        } = init_params;

        let client = Client::new(main_loop_sender.clone(), connection.sender.clone());
        let AllOptions { global, workspace } = AllOptions::from_value(
            initialization_options.unwrap_or(serde_json::Value::Null),
            &client,
        );

        crate::logging::init_logging(
            global.tracing.log_level.unwrap_or_default(),
            global.tracing.log_file.as_deref(),
        );

        let workspaces = Workspaces::from_workspace_folders(
            workspace_folders,
            root_uri,
            root_path,
            workspace.unwrap_or_default(),
        )?;
        let global = global.into_settings(client.clone());

        Ok(Self {
            connection,
            client_capabilities: client_capabilities.clone(),
            worker_threads,
            main_loop_receiver,
            main_loop_sender,
            session: Session::new(
                &client_capabilities,
                position_encoding,
                global,
                &workspaces,
                &client,
            )?,
        })
    }

    pub fn run(mut self) -> crate::Result<()> {
        let panic_client = Client::new(
            self.main_loop_sender.clone(),
            self.connection.sender.clone(),
        );
        let _panic_hook = install_panic_hook(panic_client);
        spawn_main_loop(move || self.main_loop())?
            .join()
            .map_err(|_| anyhow::anyhow!("main loop thread panicked"))?
    }

    fn find_best_position_encoding(client_capabilities: &ClientCapabilities) -> PositionEncoding {
        client_capabilities
            .general
            .as_ref()
            .and_then(|general| general.position_encodings.as_ref())
            .and_then(|encodings| {
                encodings
                    .iter()
                    .filter_map(|encoding| PositionEncoding::try_from(encoding).ok())
                    .max()
            })
            .unwrap_or_default()
    }

    fn server_capabilities(position_encoding: PositionEncoding) -> types::ServerCapabilities {
        types::ServerCapabilities {
            position_encoding: Some(position_encoding.into()),
            code_action_provider: Some(types::CodeActionProviderCapability::Options(
                CodeActionOptions {
                    code_action_kinds: Some(
                        SupportedCodeAction::all()
                            .map(SupportedCodeAction::to_kind)
                            .collect(),
                    ),
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: Some(true),
                    },
                    resolve_provider: Some(true),
                },
            )),
            workspace: Some(types::WorkspaceServerCapabilities {
                workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                    supported: Some(true),
                    change_notifications: Some(OneOf::Left(true)),
                }),
                file_operations: None,
            }),
            document_formatting_provider: None,
            document_range_formatting_provider: None,
            diagnostic_provider: Some(types::DiagnosticServerCapabilities::Options(
                DiagnosticOptions {
                    identifier: Some(crate::DIAGNOSTIC_NAME.into()),
                    inter_file_dependencies: false,
                    workspace_diagnostics: false,
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: Some(true),
                    },
                },
            )),
            execute_command_provider: Some(types::ExecuteCommandOptions {
                commands: SupportedCommand::all()
                    .map(|command| command.identifier().to_string())
                    .collect(),
                work_done_progress_options: WorkDoneProgressOptions {
                    work_done_progress: Some(false),
                },
            }),
            hover_provider: Some(types::HoverProviderCapability::Simple(true)),
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::INCREMENTAL),
                    will_save: Some(false),
                    will_save_wait_until: Some(false),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SupportedCodeAction {
    QuickFix,
    SourceFixAll,
}

impl SupportedCodeAction {
    fn all() -> impl Iterator<Item = Self> {
        [Self::QuickFix, Self::SourceFixAll].into_iter()
    }

    fn to_kind(self) -> CodeActionKind {
        match self {
            Self::QuickFix => CodeActionKind::QUICKFIX,
            Self::SourceFixAll => crate::SOURCE_FIX_ALL_SHUCK,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SupportedCommand {
    ApplyAutofix,
    ApplyDirective,
    PrintDebugInformation,
}

impl SupportedCommand {
    fn all() -> impl Iterator<Item = Self> {
        [
            Self::ApplyAutofix,
            Self::ApplyDirective,
            Self::PrintDebugInformation,
        ]
        .into_iter()
    }

    fn identifier(self) -> &'static str {
        match self {
            Self::ApplyAutofix => "shuck.applyAutofix",
            Self::ApplyDirective => "shuck.applyDirective",
            Self::PrintDebugInformation => "shuck.printDebugInformation",
        }
    }
}

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

struct PanicHookGuard {
    previous: Option<PanicHook>,
}

impl Drop for PanicHookGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::panic::set_hook(previous);
        }
    }
}

fn install_panic_hook(client: Client) -> PanicHookGuard {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        report_panic(&client, panic_info);
    }));
    PanicHookGuard {
        previous: Some(previous),
    }
}

fn report_panic(client: &Client, panic_info: &std::panic::PanicHookInfo<'_>) {
    let summary = panic_info
        .payload()
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            panic_info
                .payload()
                .downcast_ref::<&'static str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "unknown panic".to_owned());
    let location = panic_info.location().map(|location| {
        format!(
            "{}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        )
    });
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    emit_panic_report(client, &summary, location.as_deref(), &backtrace);
}

fn emit_panic_report(client: &Client, summary: &str, location: Option<&str>, backtrace: &str) {
    let location = location.unwrap_or("unknown location");
    let details = format!("Shuck server panicked at {location}: {summary}\n{backtrace}");
    tracing::error!("{details}");
    eprintln!("{details}");
    if let Err(error) = client.log_message(&details, lsp_types::MessageType::ERROR) {
        tracing::error!("Failed to send panic log message to client: {error}");
    }
    client.show_error_message(format!("Shuck server panicked: {summary}"));
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_server::Message;
    use lsp_types::notification::Notification;

    use super::*;
    use crate::Client;

    #[test]
    fn does_not_advertise_formatting_capabilities() {
        let capabilities = Server::server_capabilities(PositionEncoding::UTF16);
        assert!(capabilities.document_formatting_provider.is_none());
        assert!(capabilities.document_range_formatting_provider.is_none());
    }

    #[test]
    fn advertises_only_non_formatting_execute_commands() {
        let capabilities = Server::server_capabilities(PositionEncoding::UTF16);
        let commands = capabilities
            .execute_command_provider
            .expect("server should advertise execute commands")
            .commands;

        assert!(commands.contains(&"shuck.applyAutofix".to_owned()));
        assert!(commands.contains(&"shuck.applyDirective".to_owned()));
        assert!(commands.contains(&"shuck.printDebugInformation".to_owned()));
        assert!(!commands.contains(&"shuck.applyFormat".to_owned()));
    }

    #[test]
    fn panic_reports_are_sent_to_the_client() {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);

        emit_panic_report(&client, "boom", Some("test.rs:1:1"), "stack backtrace");

        let first = client_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("panic log notification should be sent");
        let second = client_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("panic showMessage notification should be sent");

        let messages = [first, second];
        assert!(messages.iter().any(|message| matches!(
            message,
            Message::Notification(notification)
                if notification.method == lsp_types::notification::LogMessage::METHOD
        )));
        assert!(messages.iter().any(|message| matches!(
            message,
            Message::Notification(notification)
                if notification.method == lsp_types::notification::ShowMessage::METHOD
        )));
    }
}
