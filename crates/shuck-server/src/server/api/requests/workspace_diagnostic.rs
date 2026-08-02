use lsp_types::{self as types, request as req};

use crate::server::Result;
use crate::session::{Client, RequestCancellationToken, Session};
use crate::workspace_diagnostics::{self, WorkspaceDiagnosticContext};

pub(crate) struct WorkspaceDiagnostic;

impl super::RequestHandler for WorkspaceDiagnostic {
    type RequestType = req::WorkspaceDiagnosticRequest;
}

impl super::super::traits::BackgroundRequestHandler for WorkspaceDiagnostic {
    type Snapshot = WorkspaceDiagnosticContext;

    fn snapshot(
        session: &Session,
        _params: &types::WorkspaceDiagnosticParams,
        cancellation: RequestCancellationToken,
    ) -> Result<Self::Snapshot> {
        Ok(session.workspace_diagnostic_context(cancellation))
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        client: &Client,
        params: types::WorkspaceDiagnosticParams,
    ) -> Result<types::WorkspaceDiagnosticReportResult> {
        workspace_diagnostics::workspace_diagnostics(snapshot, client, &params)
    }
}
