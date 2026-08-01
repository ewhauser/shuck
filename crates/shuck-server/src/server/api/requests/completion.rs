use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot, RequestCancellationToken, Session};
use crate::workspace_functions::{
    WorkspaceFunctionContext, canonical_path, workspace_function_index,
};

pub(crate) struct Completion;

pub(crate) struct CompletionSnapshot {
    document: Option<DocumentSnapshot>,
    workspace: WorkspaceFunctionContext,
}

impl super::RequestHandler for Completion {
    type RequestType = req::Completion;
}

impl super::super::traits::BackgroundRequestHandler for Completion {
    type Snapshot = CompletionSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::CompletionParams,
        cancellation: RequestCancellationToken,
    ) -> crate::server::Result<Self::Snapshot> {
        let uri = params.text_document_position.text_document.uri.clone();
        Ok(CompletionSnapshot {
            document: session.take_snapshot(uri),
            workspace: session.workspace_function_context(cancellation),
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        client: &Client,
        params: types::CompletionParams,
    ) -> crate::server::Result<editor_features::CompletionResponse> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        let path = document
            .query()
            .file_url()
            .to_file_path()
            .ok()
            .map(|path| canonical_path(&path));
        editor_features::completion_with_sourced_functions(
            document,
            client,
            params,
            move |analysis, offset| {
                let Some(path) = path.as_deref() else {
                    return Vec::new();
                };
                let source_spans = analysis
                    .semantic()
                    .source_refs()
                    .iter()
                    .filter(|source_ref| {
                        analysis
                            .semantic()
                            .source_ref_visible_at_offset(source_ref, offset)
                    })
                    .map(|source_ref| source_ref.span)
                    .collect::<Vec<_>>();
                let Some(index) = workspace_function_index(&snapshot.workspace) else {
                    return Vec::new();
                };
                index.visible_sourced_functions(path, &source_spans)
            },
        )
    }
}
