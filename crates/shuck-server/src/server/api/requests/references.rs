use lsp_types::{self as types, request as req};
use shuck_semantic::{CallFunctionId, CallNodeKind, EditorCallHierarchyTarget, EditorSymbolTarget};

use crate::edit::RangeExt;
use crate::editor_features;
use crate::session::{Client, DocumentSnapshot, RequestCancellationToken, Session};
use crate::workspace_functions::{
    WorkspaceFunctionContext, canonical_path, workspace_function_index,
};

pub(crate) struct References;

pub(crate) struct ReferencesSnapshot {
    document: Option<DocumentSnapshot>,
    workspace: WorkspaceFunctionContext,
}

impl super::RequestHandler for References {
    type RequestType = req::References;
}

impl super::super::traits::BackgroundRequestHandler for References {
    type Snapshot = ReferencesSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::ReferenceParams,
        cancellation: RequestCancellationToken,
    ) -> crate::server::Result<Self::Snapshot> {
        let uri = params.text_document_position.text_document.uri.clone();
        Ok(ReferencesSnapshot {
            document: session.take_snapshot(uri),
            workspace: session.workspace_function_context(cancellation),
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        client: &Client,
        params: types::ReferenceParams,
    ) -> crate::server::Result<editor_features::ReferencesResponse> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        references(document, snapshot.workspace, client, params)
    }
}

fn references(
    snapshot: DocumentSnapshot,
    workspace: WorkspaceFunctionContext,
    client: &Client,
    params: types::ReferenceParams,
) -> crate::server::Result<editor_features::ReferencesResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let position = params.text_document_position.position;
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
    let Some(target) = analysis.semantic().editor_query().target_at_offset(offset) else {
        return Ok(None);
    };
    let Some(path) = snapshot
        .query()
        .file_url()
        .to_file_path()
        .ok()
        .map(|path| canonical_path(&path))
    else {
        return editor_features::references(snapshot, client, params);
    };
    let (index, target_path, target_node, declaration) = match target {
        EditorSymbolTarget::FunctionCall(call) => {
            let Some(index) = workspace_function_index(&workspace) else {
                return Ok(None);
            };
            let Some(target) =
                index.resolve_call_site_exact(&path, call.name_span, &workspace.cancellation)
            else {
                return Ok(None);
            };
            let Some(declaration_span) = target.selection_span.or(target.def_span) else {
                return Ok(None);
            };
            let Some(file) = index.file(&target.path) else {
                return Ok(None);
            };
            let declaration = types::Location {
                uri: file.editor_uri().clone(),
                range: crate::edit::to_lsp_range(
                    declaration_span.to_range(),
                    file.source(),
                    file.line_index(),
                    snapshot.encoding(),
                ),
            };
            (index, target.path, target.node, declaration)
        }
        EditorSymbolTarget::Binding(_)
        | EditorSymbolTarget::Reference(_)
        | EditorSymbolTarget::RuntimeName(_) => {
            let Some(item) = analysis
                .semantic()
                .editor_query()
                .prepare_call_hierarchy(offset)
            else {
                return editor_features::references(snapshot, client, params);
            };
            let EditorCallHierarchyTarget::Function(_) = item.target else {
                return editor_features::references(snapshot, client, params);
            };
            let Some(definition_span) = item.full_span else {
                return Ok(None);
            };
            let Some(index) = workspace_function_index(&workspace) else {
                return Ok(None);
            };
            let declaration_span = item.selection_span.unwrap_or(definition_span);
            let node = CallNodeKind::Function(CallFunctionId::new(item.name, definition_span));
            let declaration = types::Location {
                uri: snapshot.query().file_url().clone(),
                range: crate::edit::to_lsp_range(
                    declaration_span.to_range(),
                    analysis.source(),
                    analysis.line_index(),
                    snapshot.encoding(),
                ),
            };
            (index, path.clone(), node, declaration)
        }
    };

    let Some(mut locations) = index.exact_function_reference_locations(
        &target_path,
        &target_node,
        &workspace.cancellation,
    ) else {
        return Ok(None);
    };
    if workspace.cancellation.is_cancelled() {
        return Ok(None);
    }
    if params.context.include_declaration && !locations.contains(&declaration) {
        locations.insert(0, declaration);
    }
    Ok((!locations.is_empty()).then_some(locations))
}
