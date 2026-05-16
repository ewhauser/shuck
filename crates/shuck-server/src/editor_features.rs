use std::collections::HashMap;

use anyhow::anyhow;
use lsp_server::ErrorCode;
use lsp_types as types;
use serde::{Deserialize, Serialize};
use shuck_semantic::{
    EditorCompletionKind, EditorCompletionOptions, EditorOccurrenceKind, EditorSymbolKind,
    RenameSet,
};

use crate::edit::RangeExt;
use crate::server::Error;
use crate::session::{Client, DocumentSnapshot};

pub(crate) type CompletionResponse = Option<types::CompletionResponse>;
pub(crate) type DefinitionResponse = Option<types::GotoDefinitionResponse>;
pub(crate) type ReferencesResponse = Option<Vec<types::Location>>;
pub(crate) type DocumentHighlightResponse = Option<Vec<types::DocumentHighlight>>;
pub(crate) type PrepareRenameResponse = Option<types::PrepareRenameResponse>;
pub(crate) type RenameResponse = Option<types::WorkspaceEdit>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum CompletionData {
    Symbol {
        symbol_kind: String,
        line: usize,
        column: usize,
    },
    RuntimeName,
    Builtin,
    Keyword,
}

pub(crate) fn completion(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::CompletionParams,
) -> crate::server::Result<CompletionResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let position = params.text_document_position.position;
    let offset = offset_for_position(&snapshot, source, analysis.line_index(), position);
    let options = snapshot.client_settings().completion();
    let completions = analysis.semantic().editor_query().completions_at_offset(
        source,
        analysis.indexer(),
        offset,
        EditorCompletionOptions {
            include_runtime_names: options.include_runtime_names,
            include_keywords: options.include_keywords,
        },
    );
    let Some(completions) = completions else {
        return Ok(None);
    };
    let range = crate::edit::to_lsp_range(
        completions.replacement_span.to_range(),
        source,
        analysis.line_index(),
        snapshot.encoding(),
    );
    let items = completions
        .items
        .into_iter()
        .map(|completion| {
            let data = match completion.kind {
                EditorCompletionKind::Variable | EditorCompletionKind::Function => completion
                    .definition_span
                    .map(|span| CompletionData::Symbol {
                        symbol_kind: completion_kind_label(completion.kind).to_owned(),
                        line: span.start.line,
                        column: span.start.column,
                    }),
                EditorCompletionKind::RuntimeName => Some(CompletionData::RuntimeName),
                EditorCompletionKind::Builtin => Some(CompletionData::Builtin),
                EditorCompletionKind::Keyword => Some(CompletionData::Keyword),
            };
            types::CompletionItem {
                label: completion.name.to_string(),
                kind: Some(to_lsp_completion_kind(completion.kind)),
                detail: Some(completion_kind_label(completion.kind).to_owned()),
                text_edit: Some(types::CompletionTextEdit::Edit(types::TextEdit::new(
                    range,
                    completion.name.to_string(),
                ))),
                data: data.and_then(|data| serde_json::to_value(data).ok()),
                ..types::CompletionItem::default()
            }
        })
        .collect::<Vec<_>>();
    Ok(Some(types::CompletionResponse::List(
        types::CompletionList {
            is_incomplete: false,
            items,
        },
    )))
}

pub(crate) fn resolve_completion_item(
    mut item: types::CompletionItem,
) -> crate::server::Result<types::CompletionItem> {
    let Some(data) = item
        .data
        .clone()
        .and_then(|value| serde_json::from_value::<CompletionData>(value).ok())
    else {
        return Ok(item);
    };
    let documentation = match data {
        CompletionData::Symbol {
            symbol_kind,
            line,
            column,
        } => format!("{symbol_kind} defined at line {line}, column {column}."),
        CompletionData::RuntimeName => "Runtime-provided shell name.".to_owned(),
        CompletionData::Builtin => "Shell builtin modeled by Shuck.".to_owned(),
        CompletionData::Keyword => "Shell keyword.".to_owned(),
    };
    item.documentation = Some(types::Documentation::MarkupContent(types::MarkupContent {
        kind: types::MarkupKind::Markdown,
        value: documentation,
    }));
    Ok(item)
}

pub(crate) fn definition(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::GotoDefinitionParams,
) -> crate::server::Result<DefinitionResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let position = params.text_document_position_params.position;
    let offset = offset_for_position(&snapshot, source, analysis.line_index(), position);
    let locations = analysis
        .semantic()
        .editor_query()
        .definition_spans_at_offset(offset)
        .into_iter()
        .map(|span| location_for_span(&snapshot, source, analysis.line_index(), span))
        .collect::<Vec<_>>();
    Ok(match locations.as_slice() {
        [] => None,
        [location] => Some(types::GotoDefinitionResponse::Scalar(location.clone())),
        _ => Some(types::GotoDefinitionResponse::Array(locations)),
    })
}

pub(crate) fn references(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::ReferenceParams,
) -> crate::server::Result<ReferencesResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let position = params.text_document_position.position;
    let offset = offset_for_position(&snapshot, source, analysis.line_index(), position);
    let locations = analysis
        .semantic()
        .editor_query()
        .occurrences_at_offset(offset, params.context.include_declaration)
        .into_iter()
        .map(|occurrence| {
            location_for_span(&snapshot, source, analysis.line_index(), occurrence.span)
        })
        .collect::<Vec<_>>();
    Ok((!locations.is_empty()).then_some(locations))
}

pub(crate) fn document_highlight(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::DocumentHighlightParams,
) -> crate::server::Result<DocumentHighlightResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let position = params.text_document_position_params.position;
    let offset = offset_for_position(&snapshot, source, analysis.line_index(), position);
    let highlights = analysis
        .semantic()
        .editor_query()
        .occurrences_at_offset(offset, true)
        .into_iter()
        .map(|occurrence| types::DocumentHighlight {
            range: crate::edit::to_lsp_range(
                occurrence.span.to_range(),
                source,
                analysis.line_index(),
                snapshot.encoding(),
            ),
            kind: Some(match occurrence.kind {
                EditorOccurrenceKind::Read => types::DocumentHighlightKind::READ,
                EditorOccurrenceKind::Write => types::DocumentHighlightKind::WRITE,
            }),
        })
        .collect::<Vec<_>>();
    Ok((!highlights.is_empty()).then_some(highlights))
}

pub(crate) fn prepare_rename(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::TextDocumentPositionParams,
) -> crate::server::Result<PrepareRenameResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let offset = offset_for_position(&snapshot, source, analysis.line_index(), params.position);
    let Ok(rename) = analysis
        .semantic()
        .editor_query()
        .rename_set_at_offset(offset)
    else {
        return Ok(None);
    };
    Ok(Some(types::PrepareRenameResponse::RangeWithPlaceholder {
        range: crate::edit::to_lsp_range(
            rename.editable_span.to_range(),
            source,
            analysis.line_index(),
            snapshot.encoding(),
        ),
        placeholder: rename.name.to_string(),
    }))
}

pub(crate) fn rename(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::RenameParams,
) -> crate::server::Result<RenameResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let position = params.text_document_position.position;
    let offset = offset_for_position(&snapshot, source, analysis.line_index(), position);
    let rename = analysis
        .semantic()
        .editor_query()
        .rename_set_at_offset(offset)
        .map_err(|reason| {
            Error::new(
                anyhow!("rename is not available here: {reason:?}"),
                ErrorCode::InvalidRequest,
            )
        })?;
    if !new_name_is_valid(rename.kind, &params.new_name) {
        return Err(Error::new(
            anyhow!("new name is not valid for this symbol"),
            ErrorCode::InvalidParams,
        ));
    }
    Ok(Some(workspace_edit_for_rename(
        &snapshot,
        source,
        analysis.line_index(),
        &rename,
        &params.new_name,
    )))
}

fn workspace_edit_for_rename(
    snapshot: &DocumentSnapshot,
    source: &str,
    line_index: &shuck_indexer::LineIndex,
    rename: &RenameSet,
    new_name: &str,
) -> types::WorkspaceEdit {
    let mut edits = rename
        .spans
        .iter()
        .copied()
        .map(|span| {
            types::TextEdit::new(
                crate::edit::to_lsp_range(span.to_range(), source, line_index, snapshot.encoding()),
                new_name.to_owned(),
            )
        })
        .collect::<Vec<_>>();
    edits.sort_by(|left, right| {
        right
            .range
            .start
            .line
            .cmp(&left.range.start.line)
            .then_with(|| right.range.start.character.cmp(&left.range.start.character))
    });

    let uri = snapshot.query().file_url().clone();
    if snapshot.resolved_client_capabilities().document_changes {
        let edit = types::TextDocumentEdit {
            text_document: types::OptionalVersionedTextDocumentIdentifier {
                uri,
                version: Some(snapshot.query().document().version()),
            },
            edits: edits.into_iter().map(types::OneOf::Left).collect(),
        };
        return types::WorkspaceEdit {
            changes: None,
            document_changes: Some(types::DocumentChanges::Edits(vec![edit])),
            change_annotations: None,
        };
    }

    types::WorkspaceEdit {
        changes: Some(HashMap::from([(uri, edits)])),
        document_changes: None,
        change_annotations: None,
    }
}

fn location_for_span(
    snapshot: &DocumentSnapshot,
    source: &str,
    line_index: &shuck_indexer::LineIndex,
    span: shuck_ast::Span,
) -> types::Location {
    types::Location {
        uri: snapshot.query().file_url().clone(),
        range: crate::edit::to_lsp_range(span.to_range(), source, line_index, snapshot.encoding()),
    }
}

fn offset_for_position(
    snapshot: &DocumentSnapshot,
    source: &str,
    line_index: &shuck_indexer::LineIndex,
    position: types::Position,
) -> usize {
    usize::from(
        types::Range {
            start: position,
            end: position,
        }
        .to_text_range(source, line_index, snapshot.encoding())
        .start(),
    )
}

fn to_lsp_completion_kind(kind: EditorCompletionKind) -> types::CompletionItemKind {
    match kind {
        EditorCompletionKind::Variable | EditorCompletionKind::RuntimeName => {
            types::CompletionItemKind::VARIABLE
        }
        EditorCompletionKind::Function => types::CompletionItemKind::FUNCTION,
        EditorCompletionKind::Builtin => types::CompletionItemKind::FUNCTION,
        EditorCompletionKind::Keyword => types::CompletionItemKind::KEYWORD,
    }
}

fn completion_kind_label(kind: EditorCompletionKind) -> &'static str {
    match kind {
        EditorCompletionKind::Variable => "Variable",
        EditorCompletionKind::Function => "Function",
        EditorCompletionKind::Builtin => "Builtin",
        EditorCompletionKind::RuntimeName => "Runtime name",
        EditorCompletionKind::Keyword => "Keyword",
    }
}

fn new_name_is_valid(kind: EditorSymbolKind, name: &str) -> bool {
    match kind {
        EditorSymbolKind::Function => valid_function_name(name),
        EditorSymbolKind::Variable
        | EditorSymbolKind::Array
        | EditorSymbolKind::AssociativeArray
        | EditorSymbolKind::Declaration => valid_variable_name(name),
        EditorSymbolKind::RuntimeName => false,
    }
}

fn valid_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn valid_function_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '$' | '`'
                        | '\\'
                        | '"'
                        | '\''
                        | ';'
                        | '&'
                        | '|'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '{'
                        | '}'
                        | '['
                        | ']'
                        | '*'
                        | '?'
                        | '/'
                )
        })
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{
        ClientCapabilities, CompletionParams, PartialResultParams, Position, ReferenceContext,
        ReferenceParams, RenameParams, TextDocumentIdentifier, TextDocumentPositionParams, Url,
        WorkDoneProgressParams,
    };

    use super::*;
    use crate::{
        Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces,
    };

    fn make_snapshot(source: &str) -> (DocumentSnapshot, Client, Url) {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-editor-feature-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("session should initialize");
        let uri = Url::from_file_path(workspace_root.join("script.sh"))
            .expect("script path should convert to URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new(source.to_owned(), 1).with_language_id("shellscript"),
        );
        (
            session
                .take_snapshot(uri.clone())
                .expect("snapshot should exist"),
            client,
            uri,
        )
    }

    fn position_for_nth(source: &str, needle: &str, index: usize) -> Position {
        let offset = source
            .match_indices(needle)
            .nth(index)
            .map(|(offset, _)| offset)
            .expect("needle should exist");
        let prefix = &source[..offset];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
        let character = prefix
            .rsplit_once('\n')
            .map(|(_, tail)| tail.len())
            .unwrap_or(prefix.len()) as u32;
        Position { line, character }
    }

    fn text_position(uri: Url, position: Position) -> TextDocumentPositionParams {
        TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position,
        }
    }

    #[test]
    fn completion_returns_parameter_and_command_candidates() {
        let source = "build() { :; }\nname=1\nprintf '%s\\n' \"$\"\n";
        let (snapshot, client, uri) = make_snapshot(source);
        let parameter_position = Position {
            line: 2,
            character: 16,
        };
        let response = completion(
            snapshot.clone(),
            &client,
            CompletionParams {
                text_document_position: text_position(uri.clone(), parameter_position),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
                context: None,
            },
        )
        .expect("completion should succeed")
        .expect("completion should return items");
        let types::CompletionResponse::List(list) = response else {
            panic!("expected completion list");
        };
        assert!(list.items.iter().any(|item| item.label == "name"));

        let response = completion(
            snapshot,
            &client,
            CompletionParams {
                text_document_position: text_position(uri, Position::new(3, 0)),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
                context: None,
            },
        )
        .expect("completion should succeed")
        .expect("completion should return items");
        let types::CompletionResponse::List(list) = response else {
            panic!("expected completion list");
        };
        assert!(list.items.iter().any(|item| item.label == "build"));
        assert!(list.items.iter().any(|item| item.label == "printf"));
        assert!(list.items.iter().any(|item| item.label == "if"));
    }

    #[test]
    fn navigation_references_and_highlights_use_same_symbol_set() {
        let source = "name=1\necho \"$name\"\n";
        let (snapshot, client, uri) = make_snapshot(source);
        let reference_position = position_for_nth(source, "name", 1);

        let definition = definition(
            snapshot.clone(),
            &client,
            lsp_types::GotoDefinitionParams {
                text_document_position_params: text_position(uri.clone(), reference_position),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("definition should succeed")
        .expect("definition should resolve");
        let types::GotoDefinitionResponse::Scalar(location) = definition else {
            panic!("expected scalar definition");
        };
        assert_eq!(location.range.start, Position::new(0, 0));

        let references = references(
            snapshot.clone(),
            &client,
            ReferenceParams {
                text_document_position: text_position(uri.clone(), reference_position),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
                context: ReferenceContext {
                    include_declaration: false,
                },
            },
        )
        .expect("references should succeed")
        .expect("references should resolve");
        assert_eq!(references.len(), 1);

        let highlights = document_highlight(
            snapshot,
            &client,
            lsp_types::DocumentHighlightParams {
                text_document_position_params: text_position(uri, reference_position),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("highlights should succeed")
        .expect("highlights should resolve");
        assert_eq!(
            highlights
                .iter()
                .map(|highlight| highlight.kind)
                .collect::<Vec<_>>(),
            [
                Some(types::DocumentHighlightKind::WRITE),
                Some(types::DocumentHighlightKind::READ)
            ]
        );
    }

    #[test]
    fn prepare_rename_and_rename_return_same_file_edits() {
        let source = "name=1\necho \"$name\"\n";
        let (snapshot, client, uri) = make_snapshot(source);
        let reference_position = position_for_nth(source, "name", 1);

        let prepared = prepare_rename(
            snapshot.clone(),
            &client,
            text_position(uri.clone(), reference_position),
        )
        .expect("prepare rename should succeed")
        .expect("prepare rename should resolve");
        let types::PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } = prepared
        else {
            panic!("expected range with placeholder");
        };
        assert_eq!(placeholder, "name");

        let edit = rename(
            snapshot,
            &client,
            RenameParams {
                text_document_position: text_position(uri.clone(), reference_position),
                new_name: "other".to_owned(),
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        )
        .expect("rename should succeed")
        .expect("rename should return edits");
        let changes = edit.changes.expect("default client should use changes");
        let edits = changes.get(&uri).expect("uri should have edits");
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|edit| edit.new_text == "other"));
    }
}
