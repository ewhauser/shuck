use lsp_types::{self as types, request as req};
use shuck_semantic::EditorSymbolTarget;

use crate::edit::RangeExt;
use crate::editor_features;
use crate::session::{Client, DocumentSnapshot, RequestCancellationToken, Session};
use crate::workspace_functions::{
    WorkspaceFunctionContext, canonical_path, workspace_function_index,
};

pub(crate) struct Definition;

pub(crate) struct DefinitionSnapshot {
    document: Option<DocumentSnapshot>,
    workspace: WorkspaceFunctionContext,
}

impl super::RequestHandler for Definition {
    type RequestType = req::GotoDefinition;
}

impl super::super::traits::BackgroundRequestHandler for Definition {
    type Snapshot = DefinitionSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::GotoDefinitionParams,
        cancellation: RequestCancellationToken,
    ) -> crate::server::Result<Self::Snapshot> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        Ok(DefinitionSnapshot {
            document: session.take_snapshot(uri),
            workspace: session.workspace_function_context(cancellation),
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        client: &Client,
        params: types::GotoDefinitionParams,
    ) -> crate::server::Result<editor_features::DefinitionResponse> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        definition(document, snapshot.workspace, client, params)
    }
}

fn definition(
    snapshot: DocumentSnapshot,
    workspace: WorkspaceFunctionContext,
    client: &Client,
    params: types::GotoDefinitionParams,
) -> crate::server::Result<editor_features::DefinitionResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let position = params.text_document_position_params.position;
    let offset = usize::from(
        types::Range {
            start: position,
            end: position,
        }
        .to_text_range(
            analysis.source(),
            analysis.line_index(),
            snapshot.encoding(),
        )
        .start(),
    );
    let Some(EditorSymbolTarget::FunctionCall(call)) =
        analysis.semantic().editor_query().target_at_offset(offset)
    else {
        return editor_features::definition(snapshot, client, params);
    };
    let Some(path) = snapshot
        .query()
        .file_url()
        .to_file_path()
        .ok()
        .map(|path| canonical_path(&path))
    else {
        return editor_features::definition(snapshot, client, params);
    };
    let Some(index) = workspace_function_index(&workspace) else {
        return Ok(None);
    };
    if let Some(target) = index.resolve_call_site_exact(&path, call.name_span)
        && let Some(definition_span) = target.def_span
        && let Some(range) = index.range_of(&target.path, definition_span)
    {
        let uri = if target.path == path {
            snapshot.query().file_url().clone()
        } else {
            let Some(file) = index.file(&target.path) else {
                return Ok(None);
            };
            file.editor_uri().clone()
        };
        return Ok(Some(types::GotoDefinitionResponse::Scalar(
            types::Location { uri, range },
        )));
    }

    // A partial workspace index may omit an otherwise proven local binding.
    // Preserve the document-local answer only when no source operation could
    // have replaced it; without the active file's projected facts, any source
    // effect is conservatively treated as relevant.
    if call.binding.is_some()
        && !index.contains(&path)
        && analysis.semantic().source_refs().is_empty()
    {
        return editor_features::definition(snapshot, client, params);
    }
    Ok(None)
}
