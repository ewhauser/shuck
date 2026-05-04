use lsp_server as lsp;

pub type ConnectionSender = crossbeam::channel::Sender<lsp::Message>;

pub(crate) struct ConnectionInitializer {
    connection: lsp::Connection,
}

impl ConnectionInitializer {
    pub(crate) fn stdio() -> (Self, lsp::IoThreads) {
        let (connection, threads) = lsp::Connection::stdio();
        (Self { connection }, threads)
    }

    pub(crate) fn from_connection(connection: lsp::Connection) -> Self {
        Self { connection }
    }

    pub(super) fn initialize_start(
        &self,
    ) -> crate::Result<(lsp::RequestId, lsp_types::InitializeParams)> {
        let (id, params) = self.connection.initialize_start()?;
        Ok((id, serde_json::from_value(params)?))
    }

    pub(super) fn initialize_finish(
        self,
        id: lsp::RequestId,
        server_capabilities: &lsp_types::ServerCapabilities,
        name: &str,
        version: &str,
    ) -> crate::Result<lsp_server::Connection> {
        self.connection.initialize_finish(
            id,
            serde_json::json!({
                "capabilities": server_capabilities,
                "serverInfo": {
                    "name": name,
                    "version": version
                }
            }),
        )?;
        Ok(self.connection)
    }
}
