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

        let InitializeParams {
            initialization_options,
            workspace_folders,
            ..
        } = init_params;

        let client = Client::new(main_loop_sender.clone(), connection.sender.clone());
        let AllOptions { global, workspace } = AllOptions::from_value(
            initialization_options.unwrap_or(serde_json::Value::Null),
            &client,
        );

        crate::logging::init_logging(global.tracing.log_level.unwrap_or_default(), None);

        let workspaces =
            Workspaces::from_workspace_folders(workspace_folders, workspace.unwrap_or_default())?;
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
            document_formatting_provider: Some(OneOf::Left(true)),
            document_range_formatting_provider: Some(OneOf::Left(true)),
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
}

impl SupportedCommand {
    fn all() -> impl Iterator<Item = Self> {
        [Self::ApplyAutofix].into_iter()
    }

    fn identifier(self) -> &'static str {
        match self {
            Self::ApplyAutofix => "shuck.applyAutofix",
        }
    }
}
