use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use lsp_server::ErrorCode;
use lsp_types as types;
use shuck_ast::Span;
use shuck_semantic::{
    BindingKind, CallFunctionId, CallNodeKind, EditorCallHierarchyTarget, EditorSymbolTarget,
    ExactFunctionRenameError,
};

use crate::edit::RangeExt;
use crate::editor_features;
use crate::server::Error;
use crate::session::{Client, DocumentSnapshot};
use crate::workspace_functions::{
    WorkspaceFunctionContext, WorkspaceFunctionIndex, canonical_path,
    fresh_workspace_function_index,
};

enum FunctionResolution {
    NotFunction,
    Unavailable,
    Resolved(ResolvedFunction),
}

struct ResolvedFunction {
    index: Arc<WorkspaceFunctionIndex>,
    target_path: PathBuf,
    target_node: CallNodeKind,
    declaration_span: Span,
    editable_span: Span,
    name: String,
}

pub(super) fn prepare(
    snapshot: DocumentSnapshot,
    workspace: WorkspaceFunctionContext,
    client: &Client,
    params: types::TextDocumentPositionParams,
) -> crate::server::Result<editor_features::PrepareRenameResponse> {
    if !snapshot.client_settings().rename().allow_cross_file {
        return editor_features::prepare_rename(snapshot, client, params);
    }
    let position = params.position;
    match resolve_function(&snapshot, &workspace, position)? {
        FunctionResolution::NotFunction => {
            editor_features::prepare_rename(snapshot, client, params)
        }
        FunctionResolution::Unavailable => Ok(None),
        FunctionResolution::Resolved(target) => {
            if !snapshot.resolved_client_capabilities().document_changes {
                return Ok(None);
            }
            if collect_rename_spans(&target, &workspace).is_err() {
                return Ok(None);
            }
            let Some(analysis) = snapshot.analysis() else {
                return Ok(None);
            };
            Ok(Some(types::PrepareRenameResponse::RangeWithPlaceholder {
                range: crate::edit::to_lsp_range(
                    target.editable_span.to_range(),
                    analysis.source(),
                    analysis.line_index(),
                    snapshot.encoding(),
                ),
                placeholder: target.name,
            }))
        }
    }
}

pub(super) fn rename(
    snapshot: DocumentSnapshot,
    workspace: WorkspaceFunctionContext,
    client: &Client,
    params: types::RenameParams,
) -> crate::server::Result<editor_features::RenameResponse> {
    if !snapshot.client_settings().rename().allow_cross_file {
        return editor_features::rename(snapshot, client, params);
    }
    let position = params.text_document_position.position;
    let target = match resolve_function(&snapshot, &workspace, position)? {
        FunctionResolution::NotFunction => {
            return editor_features::rename(snapshot, client, params);
        }
        FunctionResolution::Unavailable => {
            return Err(rename_error(
                "the function binding is ambiguous or unavailable in the current workspace index",
            ));
        }
        FunctionResolution::Resolved(target) => target,
    };
    if !snapshot.resolved_client_capabilities().document_changes {
        return Err(rename_error(
            "cross-file function rename requires client support for documentChanges",
        ));
    }
    if !editor_features::valid_function_name(&params.new_name) {
        return Err(Error::new(
            anyhow!("new name is not valid for a shell function"),
            ErrorCode::InvalidParams,
        ));
    }
    let spans = collect_rename_spans(&target, &workspace).map_err(rename_error)?;
    let edit = workspace_edit(&target.index, spans, &params.new_name);
    validate_workspace_snapshot(&target.index, &workspace).map_err(rename_error)?;
    Ok(Some(edit))
}

fn resolve_function(
    snapshot: &DocumentSnapshot,
    workspace: &WorkspaceFunctionContext,
    position: types::Position,
) -> crate::server::Result<FunctionResolution> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(FunctionResolution::Unavailable);
    };
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
        return Ok(FunctionResolution::NotFunction);
    };
    let Some(path) = snapshot
        .query()
        .file_url()
        .to_file_path()
        .ok()
        .map(|path| canonical_path(&path))
    else {
        return Ok(FunctionResolution::NotFunction);
    };

    let resolved = match target {
        EditorSymbolTarget::FunctionCall(call) => {
            let Some(index) = fresh_workspace_function_index(workspace) else {
                return Ok(FunctionResolution::Unavailable);
            };
            let Some(target) =
                index.resolve_call_site_exact(&path, call.name_span, &workspace.cancellation)
            else {
                return Ok(FunctionResolution::Unavailable);
            };
            let Some(declaration_span) = target.selection_span else {
                return Ok(FunctionResolution::Unavailable);
            };
            ResolvedFunction {
                index,
                target_path: target.path,
                target_node: target.node,
                declaration_span,
                editable_span: call.name_span,
                name: call.name.to_string(),
            }
        }
        EditorSymbolTarget::Binding(binding_id) => {
            let binding = analysis.semantic().binding(binding_id);
            if !matches!(binding.kind, BindingKind::FunctionDefinition) {
                return Ok(FunctionResolution::NotFunction);
            }
            let Some(index) = fresh_workspace_function_index(workspace) else {
                return Ok(FunctionResolution::Unavailable);
            };
            let Some(item) = analysis
                .semantic()
                .editor_query()
                .prepare_call_hierarchy(offset)
            else {
                return Ok(FunctionResolution::Unavailable);
            };
            local_function(index, path, item, binding.span)?
        }
        EditorSymbolTarget::Reference(reference_id) => {
            let Some(binding) = analysis.semantic().resolved_binding(reference_id) else {
                return Ok(FunctionResolution::NotFunction);
            };
            if !matches!(binding.kind, BindingKind::FunctionDefinition) {
                return Ok(FunctionResolution::NotFunction);
            }
            let Some(index) = fresh_workspace_function_index(workspace) else {
                return Ok(FunctionResolution::Unavailable);
            };
            let Some(item) = analysis
                .semantic()
                .editor_query()
                .prepare_call_hierarchy(offset)
            else {
                return Ok(FunctionResolution::Unavailable);
            };
            local_function(
                index,
                path,
                item,
                analysis.semantic().reference(reference_id).name_span,
            )?
        }
        EditorSymbolTarget::RuntimeName(_) => return Ok(FunctionResolution::NotFunction),
    };
    Ok(FunctionResolution::Resolved(resolved))
}

fn local_function(
    index: Arc<WorkspaceFunctionIndex>,
    path: PathBuf,
    item: shuck_semantic::EditorCallHierarchyItem,
    editable_span: Span,
) -> crate::server::Result<ResolvedFunction> {
    let EditorCallHierarchyTarget::Function(_) = item.target else {
        return Err(rename_error("the selected symbol is not a function"));
    };
    let Some(definition_span) = item.full_span else {
        return Err(rename_error("the function definition has no source span"));
    };
    let Some(declaration_span) = item.selection_span else {
        return Err(rename_error("the function name has no source span"));
    };
    Ok(ResolvedFunction {
        index,
        target_path: path,
        target_node: CallNodeKind::Function(CallFunctionId::new(
            item.name.clone(),
            definition_span,
        )),
        declaration_span,
        editable_span,
        name: item.name.to_string(),
    })
}

fn collect_rename_spans(
    target: &ResolvedFunction,
    workspace: &WorkspaceFunctionContext,
) -> Result<BTreeMap<PathBuf, Vec<Span>>, String> {
    if !target.index.is_complete() {
        return Err(
            "workspace indexing is incomplete; retry after the workspace is fully indexed".into(),
        );
    }
    if workspace.cache.current_epoch() != workspace.epoch {
        return Err("workspace state changed during rename; retry the request".into());
    }
    let Some(rename) = target.index.exact_function_rename(
        &target.target_path,
        &target.target_node,
        &workspace.cancellation,
    ) else {
        return Err("the rename request was cancelled".into());
    };
    let rename = rename.map_err(|error| match error {
        ExactFunctionRenameError::AmbiguousReference => {
            "a same-named call in the source graph has ambiguous binding identity".to_owned()
        }
        ExactFunctionRenameError::IncompleteSourceGraph => {
            "the source graph contains an unresolved or unindexed source operation".to_owned()
        }
    })?;
    let roots = workspace
        .workspace_roots
        .iter()
        .map(|root| canonical_path(root))
        .collect::<Vec<_>>();
    for path in &rename.relevant_paths {
        if workspace.cancellation.is_cancelled() {
            return Err("the rename request was cancelled".into());
        }
        if !roots.iter().any(|root| path.starts_with(root)) {
            return Err(format!(
                "the source graph reaches a file outside the workspace: {}",
                path.display()
            ));
        }
        if target.index.file(path).is_none() {
            return Err(format!(
                "a source-connected file is not indexed: {}",
                path.display()
            ));
        }
    }

    let mut spans = BTreeMap::<PathBuf, Vec<Span>>::new();
    spans
        .entry(target.target_path.clone())
        .or_default()
        .push(target.declaration_span);
    for reference in rename.references {
        spans
            .entry(reference.path)
            .or_default()
            .push(reference.span);
    }
    for (path, spans) in &mut spans {
        spans.sort_by_key(|span| (span.start.offset(), span.end.offset()));
        spans.dedup();
        if spans
            .windows(2)
            .any(|pair| pair[0].end.offset() > pair[1].start.offset())
        {
            return Err(format!(
                "rename edits overlap in {}; no edits were produced",
                path.display()
            ));
        }
    }
    validate_workspace_snapshot(&target.index, workspace)?;
    Ok(spans)
}

fn validate_workspace_snapshot(
    index: &WorkspaceFunctionIndex,
    workspace: &WorkspaceFunctionContext,
) -> Result<(), String> {
    if workspace.cache.current_epoch() != workspace.epoch {
        return Err("workspace state changed during rename; retry the request".into());
    }
    let Some(closed_files) = index.validate_closed_files(&workspace.cancellation) else {
        return Err("the rename request was cancelled".into());
    };
    if let Err(path) = closed_files {
        return Err(format!(
            "an indexed file changed during rename: {}; retry the request",
            path.display()
        ));
    }
    if workspace.cache.current_epoch() != workspace.epoch {
        return Err("workspace state changed during rename; retry the request".into());
    }
    Ok(())
}

fn workspace_edit(
    index: &WorkspaceFunctionIndex,
    spans_by_path: BTreeMap<PathBuf, Vec<Span>>,
    new_name: &str,
) -> types::WorkspaceEdit {
    let mut document_edits = Vec::new();
    for (path, mut spans) in spans_by_path {
        let file = index
            .file(&path)
            .expect("rename spans should only reference validated indexed files");
        spans.sort_by_key(|span| std::cmp::Reverse((span.start.offset(), span.end.offset())));
        let edits = spans
            .into_iter()
            .map(|span| {
                types::TextEdit::new(
                    crate::edit::to_lsp_range(
                        span.to_range(),
                        file.source(),
                        file.line_index(),
                        index.encoding(),
                    ),
                    new_name.to_owned(),
                )
            })
            .collect::<Vec<_>>();
        document_edits.push(types::TextDocumentEdit {
            text_document: types::OptionalVersionedTextDocumentIdentifier {
                uri: file.editor_uri().clone(),
                version: file.version(),
            },
            edits: edits.into_iter().map(types::OneOf::Left).collect(),
        });
    }
    types::WorkspaceEdit {
        changes: None,
        document_changes: Some(types::DocumentChanges::Edits(document_edits)),
        change_annotations: None,
    }
}

fn rename_error(message: impl Into<String>) -> Error {
    Error::new(anyhow!(message.into()), ErrorCode::InvalidRequest)
}
