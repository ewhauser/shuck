#![allow(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use std::num::NonZeroUsize;

pub use edit::{DocumentKey, PositionEncoding, TextDocument};
use lsp_types::CodeActionKind;
pub use server::Server;
pub use session::{Client, ClientOptions, DocumentQuery, DocumentSnapshot, Session};
pub use workspace::{Workspace, Workspaces};

mod edit;
mod fix;
mod format;
mod lint;
mod logging;
mod resolve;
mod server;
mod session;
mod workspace;

pub(crate) const SERVER_NAME: &str = "shuck";
pub(crate) const DIAGNOSTIC_NAME: &str = "Shuck";

pub(crate) const SOURCE_FIX_ALL_SHUCK: CodeActionKind = CodeActionKind::new("source.fixAll.shuck");

pub(crate) type Result<T> = anyhow::Result<T>;

pub(crate) fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn run() -> Result<()> {
    let four = NonZeroUsize::try_from(4usize)
        .map_err(|_| anyhow::anyhow!("failed to create non-zero worker count"))?;
    let worker_threads = std::thread::available_parallelism()
        .unwrap_or(four)
        .min(four);

    let (connection, io_threads) = server::ConnectionInitializer::stdio();
    let server_result = Server::new(worker_threads, connection)
        .map_err(|err| err.context("Failed to start server"))?
        .run();

    let io_result = io_threads.join();
    match (server_result, io_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(server), Ok(())) => Err(server),
        (Ok(()), Err(io)) => Err(anyhow::Error::new(io).context("IO thread error")),
        (Err(server), Err(io)) => Err(server.context(format!("IO thread error: {io}"))),
    }
}
