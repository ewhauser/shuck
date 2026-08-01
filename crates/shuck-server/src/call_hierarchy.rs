//! Cross-file call hierarchy (spec 025).
//!
//! Call hierarchy is the first consumer of the shared workspace function index.
//! Every workspace shell file contributes compact definitions, call sites, and
//! determinable source edges. Open buffers shadow disk, and exact definition
//! byte ranges keep same-named redefinitions distinct.

use std::path::PathBuf;

use lsp_types as types;
use shuck_ast::Name;
use shuck_semantic::{CallFunctionId, CallNodeKind, CrossFileCall, EditorSymbolTarget};

use crate::edit::RangeExt;
use crate::editor_features::{self, CallHierarchyData, CallHierarchyPrepareResponse};
use crate::session::{Client, DocumentSnapshot};
use crate::workspace_functions::{
    WorkspaceFunctionContext, WorkspaceFunctionIndex, canonical_path, workspace_function_index,
};

pub(crate) type IncomingResponse = Option<Vec<types::CallHierarchyIncomingCall>>;
pub(crate) type OutgoingResponse = Option<Vec<types::CallHierarchyOutgoingCall>>;

/// Prepares the function under the cursor, including functions resolved from a
/// statically sourced file.
pub(crate) fn prepare_call_hierarchy(
    context: WorkspaceFunctionContext,
    snapshot: DocumentSnapshot,
    client: &Client,
    params: types::CallHierarchyPrepareParams,
) -> crate::server::Result<CallHierarchyPrepareResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let position = params.text_document_position_params.position;
    let offset = usize::from(
        types::Range {
            start: position,
            end: position,
        }
        .to_text_range(source, analysis.line_index(), snapshot.encoding())
        .start(),
    );
    let target = analysis.semantic().editor_query().target_at_offset(offset);
    let Some(EditorSymbolTarget::FunctionCall(call)) = target else {
        return editor_features::prepare_call_hierarchy(snapshot, client, params);
    };
    let Some(path) = snapshot
        .query()
        .file_url()
        .to_file_path()
        .ok()
        .map(|path| canonical_path(&path))
    else {
        return editor_features::prepare_call_hierarchy(snapshot, client, params);
    };
    let Some(built) = workspace_function_index(&context) else {
        return Ok(None);
    };
    if let Some(target) = built.resolve_call_site(&path, call.name_span) {
        return Ok(item_for(&built, &target).map(|item| vec![item]));
    }

    // An indexed active document should resolve every proven local function
    // call. Retain the document-local answer if a configured file limit made
    // the workspace index partial before it reached this buffer.
    if call.binding.is_some() {
        return editor_features::prepare_call_hierarchy(snapshot, client, params);
    }
    Ok(None)
}

pub(crate) fn incoming_calls(
    context: WorkspaceFunctionContext,
    params: types::CallHierarchyIncomingCallsParams,
) -> crate::server::Result<IncomingResponse> {
    let Some((path, node)) = item_identity(&params.item) else {
        return Ok(None);
    };
    let CallNodeKind::Function(_) = node else {
        return Ok(Some(Vec::new()));
    };
    let Some(built) = workspace_function_index(&context) else {
        return Ok(None);
    };
    let calls = built
        .incoming(&path, &node)
        .into_iter()
        .filter_map(|call| {
            let from = item_for(&built, &call)?;
            Some(types::CallHierarchyIncomingCall {
                from_ranges: built.ranges_in(&call.path, &call.call_spans),
                from,
            })
        })
        .collect();
    Ok(Some(calls))
}

pub(crate) fn outgoing_calls(
    context: WorkspaceFunctionContext,
    params: types::CallHierarchyOutgoingCallsParams,
) -> crate::server::Result<OutgoingResponse> {
    let Some((path, node)) = item_identity(&params.item) else {
        return Ok(None);
    };
    let Some(built) = workspace_function_index(&context) else {
        return Ok(None);
    };
    let calls = built
        .outgoing(&path, &node)
        .into_iter()
        .filter_map(|call| {
            let to = item_for(&built, &call)?;
            Some(types::CallHierarchyOutgoingCall {
                from_ranges: built.ranges_in(&path, &call.call_spans),
                to,
            })
        })
        .collect();
    Ok(Some(calls))
}

/// The queried node's identity: its file path plus which exact node in that file.
fn item_identity(item: &types::CallHierarchyItem) -> Option<(PathBuf, CallNodeKind)> {
    let path = canonical_path(&item.uri.to_file_path().ok()?);
    let data = item
        .data
        .clone()
        .and_then(|value| serde_json::from_value::<CallHierarchyData>(value).ok());
    let node = match data {
        Some(CallHierarchyData::TopLevel) => CallNodeKind::TopLevel,
        Some(CallHierarchyData::Function {
            definition_start,
            definition_end,
        }) => CallNodeKind::Function(CallFunctionId {
            name: Name::from(item.name.as_str()),
            definition_start,
            definition_end,
        }),
        None if item.kind == types::SymbolKind::MODULE => CallNodeKind::TopLevel,
        None => return None,
    };
    Some((path, node))
}

fn item_for(
    index: &WorkspaceFunctionIndex,
    call: &CrossFileCall,
) -> Option<types::CallHierarchyItem> {
    let uri = index.file(&call.path)?.uri().clone();
    match &call.node {
        CallNodeKind::Function(function) => {
            let definition_span = call.def_span?;
            let range = index.range_of(&call.path, definition_span)?;
            let selection_range = call
                .selection_span
                .and_then(|span| index.range_of(&call.path, span))
                .unwrap_or(range);
            Some(crate::editor_features::call_hierarchy_function_item(
                function.name.to_string(),
                uri,
                definition_span,
                range,
                selection_range,
            ))
        }
        CallNodeKind::TopLevel => Some(crate::editor_features::call_hierarchy_top_level_item(uri)),
    }
}
