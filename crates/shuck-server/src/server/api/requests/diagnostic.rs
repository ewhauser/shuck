use lsp_types::{self as types, request as req};
use types::{
    DocumentDiagnosticReportResult, FullDocumentDiagnosticReport,
    RelatedFullDocumentDiagnosticReport,
};

use crate::lint::generate_diagnostics;
use crate::server::Result;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct DocumentDiagnostic;

impl super::RequestHandler for DocumentDiagnostic {
    type RequestType = req::DocumentDiagnosticRequest;
}

impl super::BackgroundDocumentRequestHandler for DocumentDiagnostic {
    super::define_document_url!(params: &types::DocumentDiagnosticParams);

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        _client: &Client,
        _params: types::DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        Ok(DocumentDiagnosticReportResult::Report(
            types::DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: generate_diagnostics(&snapshot),
                },
            }),
        ))
    }
}
