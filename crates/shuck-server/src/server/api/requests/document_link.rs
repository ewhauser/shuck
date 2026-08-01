use lsp_types::{self as types, request as req};

use crate::session::{Client, DocumentSnapshot, RequestCancellationToken, Session};
use crate::workspace_functions::{
    WorkspaceFunctionContext, canonical_path, workspace_function_index,
};

pub(crate) struct DocumentLinks;

pub(crate) struct DocumentLinksSnapshot {
    document: Option<DocumentSnapshot>,
    workspace: WorkspaceFunctionContext,
}

impl super::RequestHandler for DocumentLinks {
    type RequestType = req::DocumentLinkRequest;
}

impl super::super::traits::BackgroundRequestHandler for DocumentLinks {
    type Snapshot = DocumentLinksSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::DocumentLinkParams,
        cancellation: RequestCancellationToken,
    ) -> crate::server::Result<Self::Snapshot> {
        Ok(DocumentLinksSnapshot {
            document: session.take_snapshot(params.text_document.uri.clone()),
            workspace: session.workspace_function_context(cancellation),
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        _client: &Client,
        _params: types::DocumentLinkParams,
    ) -> crate::server::Result<Option<Vec<types::DocumentLink>>> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        Ok(document_links(document, &snapshot.workspace))
    }
}

fn document_links(
    snapshot: DocumentSnapshot,
    workspace: &WorkspaceFunctionContext,
) -> Option<Vec<types::DocumentLink>> {
    let analysis = snapshot.analysis()?;
    let path = snapshot
        .query()
        .file_url()
        .to_file_path()
        .ok()
        .map(|path| canonical_path(&path))?;
    let index = workspace_function_index(workspace)?;
    if workspace.cancellation.is_cancelled() {
        return None;
    }
    let workspace_roots = workspace
        .workspace_roots
        .iter()
        .map(|root| canonical_path(root))
        .collect::<Vec<_>>();

    let mut links = analysis
        .semantic()
        .source_refs()
        .iter()
        .filter_map(|source_ref| {
            let (target_path, target_uri) = index.source_target(&path, source_ref.span)?;
            if !workspace_roots
                .iter()
                .any(|root| target_path.starts_with(root))
            {
                return None;
            }
            let span = source_ref
                .directive_path_span
                .unwrap_or(source_ref.path_span);
            Some(types::DocumentLink {
                range: crate::edit::to_lsp_range(
                    span.to_range(),
                    analysis.source(),
                    analysis.line_index(),
                    snapshot.encoding(),
                ),
                target: Some(target_uri.clone()),
                tooltip: None,
                data: None,
            })
        })
        .collect::<Vec<_>>();
    links.sort_by_key(|link| (link.range.start, link.range.end));
    Some(links)
}
