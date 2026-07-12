#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! Language Server Protocol support for Shuck.
//!
//! The primary entrypoint is [`run`], which starts the server over standard
//! input and output. A small set of document/session types is also exposed for
//! integration tests and embedding scenarios that need an in-memory LSP server.

use std::num::NonZeroUsize;

pub use edit::{DocumentKey, PositionEncoding, TextDocument};
pub use lint::generate_diagnostics;
use lsp_types::CodeActionKind;
pub use server::Server;
pub use session::{Client, ClientOptions, DocumentQuery, DocumentSnapshot, GlobalOptions, Session};
pub use workspace::{Workspace, Workspaces};

mod analysis;
mod edit;
mod editor;
mod editor_features;
mod fix;
mod format;
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub mod fuzzing;
mod lint;
mod logging;
mod resolve;
mod server;
mod session;
mod symbols;
mod workspace;

pub(crate) const SERVER_NAME: &str = "shuck";
pub(crate) const DIAGNOSTIC_NAME: &str = "shuck";

pub(crate) const SOURCE_FIX_ALL_SHUCK: CodeActionKind = CodeActionKind::new("source.fixAll.shuck");

pub(crate) type Result<T> = anyhow::Result<T>;

pub(crate) fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Run the Shuck language server over standard input and output.
pub fn run() -> Result<()> {
    let four = NonZeroUsize::try_from(4usize)
        .map_err(|_| anyhow::anyhow!("failed to create non-zero worker count"))?;
    let worker_threads = std::thread::available_parallelism()
        .unwrap_or(four)
        .min(four);

    let (connection, io_threads) = server::ConnectionInitializer::stdio();
    let server_result = match start_server(worker_threads, connection)? {
        Some(server) => server.run(),
        None => Ok(()),
    };

    let io_result = io_threads.join();
    match (server_result, io_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(server), Ok(())) => Err(server),
        (Ok(()), Err(io)) => Err(anyhow::Error::new(io).context("IO thread error")),
        (Err(server), Err(io)) => Err(server.context(format!("IO thread error: {io}"))),
    }
}

#[doc(hidden)]
pub fn run_connection(connection: lsp_server::Connection) -> Result<()> {
    let four = NonZeroUsize::try_from(4usize)
        .map_err(|_| anyhow::anyhow!("failed to create non-zero worker count"))?;
    let worker_threads = std::thread::available_parallelism()
        .unwrap_or(four)
        .min(four);
    match start_server(
        worker_threads,
        server::ConnectionInitializer::from_connection(connection),
    )? {
        Some(server) => server.run(),
        None => Ok(()),
    }
}

fn start_server(
    worker_threads: NonZeroUsize,
    connection: server::ConnectionInitializer,
) -> Result<Option<Server>> {
    match Server::new(worker_threads, connection) {
        Ok(server) => Ok(Some(server)),
        Err(error) if is_disconnected(&error) => Ok(None),
        Err(error) => Err(error.context("Failed to start server")),
    }
}

fn is_disconnected(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<lsp_server::ProtocolError>()
        .is_some_and(lsp_server::ProtocolError::channel_is_disconnected)
}
