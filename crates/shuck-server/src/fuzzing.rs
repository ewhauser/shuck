//! Fuzz-only harnesses for exercising private LSP server surfaces.

use std::path::PathBuf;

use anyhow::{Context, anyhow};
use crossbeam::channel;
use lsp_types as types;
use serde::Serialize;
use serde_json::json;

use crate::edit::DocumentVersion;
use crate::session::{ClientOptions, GlobalOptions};
use crate::{Client, PositionEncoding, Session, TextDocument, Workspace, Workspaces};

/// Input for the direct LSP request-surface fuzz harness.
#[derive(Clone, Debug)]
pub struct RequestSurfaceInput<'a> {
    /// Source text to open as an in-memory document.
    pub source: &'a str,
    /// LSP language identifier supplied by the client.
    pub language_id: &'a str,
    /// File name used for shell dialect inference.
    pub file_name: &'a str,
    /// Position encoding negotiated with the client.
    pub encoding: PositionEncoding,
    /// Client capabilities used to resolve response shapes.
    pub capabilities: types::ClientCapabilities,
    /// Client options layered over default Shuck settings.
    pub client_options: ClientOptions,
    /// Position used by hover, completion, navigation, and rename requests.
    pub position: types::Position,
    /// Range used by code action and range-formatting requests.
    pub range: types::Range,
    /// Candidate replacement name for rename requests.
    pub new_name: &'a str,
    /// Query string for workspace symbol requests.
    pub workspace_query: &'a str,
}

/// Final state after applying LSP text document changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDocumentState {
    /// Current document contents.
    pub contents: String,
    /// Current document version.
    pub version: DocumentVersion,
}

/// Apply LSP text document changes through the server document type.
pub fn apply_text_document_changes(
    source: &str,
    version: DocumentVersion,
    changes: Vec<types::TextDocumentContentChangeEvent>,
    new_version: DocumentVersion,
    encoding: PositionEncoding,
) -> TextDocumentState {
    let mut document =
        TextDocument::new(source.to_owned(), version).with_language_id("shellscript");
    document.apply_changes(changes, new_version, encoding);
    TextDocumentState {
        contents: document.contents().to_owned(),
        version: document.version(),
    }
}

/// Exercise diagnostics, fixes, editor features, symbols, and formatting.
pub fn exercise_request_surface(
    input: RequestSurfaceInput<'_>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let (mut session, client, _client_receiver, uri) = build_session(
        input.encoding,
        input.capabilities,
        input.client_options,
        input.file_name,
    )?;
    session.open_text_document(
        uri.clone(),
        TextDocument::new(input.source.to_owned(), 1).with_language_id(input.language_id),
    );
    let Some(snapshot) = session.take_snapshot(uri.clone()) else {
        return Err(anyhow!("opened fuzz document did not produce a snapshot"));
    };

    let mut outputs = Vec::new();
    let diagnostics = crate::lint::generate_diagnostics(&snapshot);
    record_value(&mut outputs, "textDocument/diagnostic", &diagnostics)?;

    let code_actions = crate::fix::code_actions(
        snapshot.clone(),
        &client,
        types::CodeActionParams {
            text_document: text_document_identifier(uri.clone()),
            range: input.range,
            context: types::CodeActionContext {
                diagnostics: diagnostics.clone(),
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: types::WorkDoneProgressParams::default(),
            partial_result_params: types::PartialResultParams::default(),
        },
    );
    if let Some(Some(actions)) =
        record_server_result(&mut outputs, "textDocument/codeAction", code_actions)?
    {
        for action in actions.into_iter().take(4) {
            let types::CodeActionOrCommand::CodeAction(action) = action else {
                continue;
            };
            let resolved = crate::fix::resolve_code_action(&session, &client, action);
            let _ = record_server_result(&mut outputs, "codeAction/resolve", resolved)?;
        }
    }

    let text_position = text_document_position(uri.clone(), input.position);
    let hover = crate::resolve::hover(
        snapshot.clone(),
        &client,
        types::HoverParams {
            text_document_position_params: text_position.clone(),
            work_done_progress_params: types::WorkDoneProgressParams::default(),
        },
    );
    let _ = record_server_result(&mut outputs, "textDocument/hover", hover)?;

    let completion = crate::editor_features::completion(
        snapshot.clone(),
        &client,
        types::CompletionParams {
            text_document_position: text_position.clone(),
            work_done_progress_params: types::WorkDoneProgressParams::default(),
            partial_result_params: types::PartialResultParams::default(),
            context: None,
        },
    );
    if let Some(Some(completion)) =
        record_server_result(&mut outputs, "textDocument/completion", completion)?
    {
        let items = match completion {
            types::CompletionResponse::Array(items) => items,
            types::CompletionResponse::List(list) => list.items,
        };
        for item in items.into_iter().take(4) {
            let resolved = crate::editor_features::resolve_completion_item(item);
            let _ = record_server_result(&mut outputs, "completionItem/resolve", resolved)?;
        }
    }

    let definition = crate::editor_features::definition(
        snapshot.clone(),
        &client,
        types::GotoDefinitionParams {
            text_document_position_params: text_position.clone(),
            work_done_progress_params: types::WorkDoneProgressParams::default(),
            partial_result_params: types::PartialResultParams::default(),
        },
    );
    let _ = record_server_result(&mut outputs, "textDocument/definition", definition)?;

    let references = crate::editor_features::references(
        snapshot.clone(),
        &client,
        types::ReferenceParams {
            text_document_position: text_position.clone(),
            work_done_progress_params: types::WorkDoneProgressParams::default(),
            partial_result_params: types::PartialResultParams::default(),
            context: types::ReferenceContext {
                include_declaration: true,
            },
        },
    );
    let _ = record_server_result(&mut outputs, "textDocument/references", references)?;

    let highlights = crate::editor_features::document_highlight(
        snapshot.clone(),
        &client,
        types::DocumentHighlightParams {
            text_document_position_params: text_position.clone(),
            work_done_progress_params: types::WorkDoneProgressParams::default(),
            partial_result_params: types::PartialResultParams::default(),
        },
    );
    let _ = record_server_result(&mut outputs, "textDocument/documentHighlight", highlights)?;

    let prepare_rename =
        crate::editor_features::prepare_rename(snapshot.clone(), &client, text_position.clone());
    let _ = record_server_result(&mut outputs, "textDocument/prepareRename", prepare_rename)?;

    let rename = crate::editor_features::rename(
        snapshot.clone(),
        &client,
        types::RenameParams {
            text_document_position: text_position,
            new_name: input.new_name.to_owned(),
            work_done_progress_params: types::WorkDoneProgressParams::default(),
        },
    );
    let _ = record_server_result(&mut outputs, "textDocument/rename", rename)?;

    let document_symbols = crate::symbols::document_symbols(
        snapshot.clone(),
        &client,
        types::DocumentSymbolParams {
            text_document: text_document_identifier(uri.clone()),
            work_done_progress_params: types::WorkDoneProgressParams::default(),
            partial_result_params: types::PartialResultParams::default(),
        },
    );
    let _ = record_server_result(
        &mut outputs,
        "textDocument/documentSymbol",
        document_symbols,
    )?;

    let workspace_symbols = crate::symbols::workspace_symbols(
        session.workspace_symbol_context(),
        &client,
        types::WorkspaceSymbolParams {
            query: input.workspace_query.to_owned(),
            work_done_progress_params: types::WorkDoneProgressParams::default(),
            partial_result_params: types::PartialResultParams::default(),
        },
    );
    let _ = record_server_result(&mut outputs, "workspace/symbol", workspace_symbols)?;

    let format_document = crate::format::format_document(
        snapshot.clone(),
        &client,
        types::DocumentFormattingParams {
            text_document: text_document_identifier(uri.clone()),
            options: types::FormattingOptions {
                tab_size: 4,
                insert_spaces: true,
                ..types::FormattingOptions::default()
            },
            work_done_progress_params: types::WorkDoneProgressParams::default(),
        },
    );
    let _ = record_server_result(&mut outputs, "textDocument/formatting", format_document)?;

    let format_range = crate::format::format_range(
        snapshot,
        &client,
        types::DocumentRangeFormattingParams {
            text_document: text_document_identifier(uri),
            range: input.range,
            options: types::FormattingOptions {
                tab_size: 4,
                insert_spaces: true,
                ..types::FormattingOptions::default()
            },
            work_done_progress_params: types::WorkDoneProgressParams::default(),
        },
    );
    let _ = record_server_result(&mut outputs, "textDocument/rangeFormatting", format_range)?;

    Ok(outputs)
}

fn build_session(
    encoding: PositionEncoding,
    capabilities: types::ClientCapabilities,
    client_options: ClientOptions,
    file_name: &str,
) -> anyhow::Result<(
    Session,
    Client,
    channel::Receiver<lsp_server::Message>,
    types::Url,
)> {
    let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
    let (client_sender, client_receiver) = channel::unbounded();
    let client = Client::new(main_loop_sender, client_sender);
    let workspace_root = fuzz_workspace_root()?;
    let workspace_uri = types::Url::from_file_path(&workspace_root)
        .map_err(|()| anyhow!("failed to build fuzz workspace URI"))?;
    let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
    let global = GlobalOptions::default().into_settings(client.clone());
    let mut session = Session::new(&capabilities, encoding, global, &workspaces, &client)?;
    session.update_client_options(client_options);
    let uri = types::Url::from_file_path(workspace_root.join(file_name))
        .map_err(|()| anyhow!("failed to build fuzz document URI"))?;
    Ok((session, client, client_receiver, uri))
}

fn fuzz_workspace_root() -> anyhow::Result<PathBuf> {
    let root = std::env::temp_dir().join(format!("shuck-lsp-fuzz-{}", std::process::id()));
    std::fs::create_dir_all(&root)
        .with_context(|| format!("create fuzz workspace {}", root.display()))?;
    Ok(root)
}

fn text_document_identifier(uri: types::Url) -> types::TextDocumentIdentifier {
    types::TextDocumentIdentifier { uri }
}

fn text_document_position(
    uri: types::Url,
    position: types::Position,
) -> types::TextDocumentPositionParams {
    types::TextDocumentPositionParams {
        text_document: text_document_identifier(uri),
        position,
    }
}

fn record_server_result<T>(
    outputs: &mut Vec<serde_json::Value>,
    method: &str,
    result: crate::server::Result<T>,
) -> anyhow::Result<Option<T>>
where
    T: Serialize,
{
    match result {
        Ok(value) => {
            record_value(outputs, method, &value)?;
            Ok(Some(value))
        }
        Err(error) => {
            outputs.push(json!({
                "method": method,
                "ok": false,
                "code": error.code as i32,
                "message": error.to_string(),
            }));
            Ok(None)
        }
    }
}

fn record_value<T>(
    outputs: &mut Vec<serde_json::Value>,
    method: &str,
    value: &T,
) -> anyhow::Result<()>
where
    T: Serialize,
{
    outputs.push(json!({
        "method": method,
        "ok": true,
        "result": serde_json::to_value(value)?,
    }));
    Ok(())
}
