use anyhow::{Context, anyhow};
use lsp_server::ErrorCode;
use lsp_types as types;
use serde::{Deserialize, Serialize};

use crate::lint::{
    AssociatedDiagnosticData, associated_diagnostic_data, directive_edit_for_line,
    fix_all_document_edits, generate_diagnostics,
};
use crate::session::{Client, DocumentSnapshot, Session};

pub(crate) fn code_actions(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::CodeActionParams,
) -> crate::server::Result<Option<types::CodeActionResponse>> {
    let mut actions = Vec::new();
    let only = params.context.only.as_ref();
    let include_quickfix = wants_kind(only, &types::CodeActionKind::QUICKFIX);
    let include_fix_all = wants_kind(only, &crate::SOURCE_FIX_ALL_SHUCK);

    if include_quickfix {
        for diagnostic in diagnostics_for_range(&snapshot, &params.range) {
            let Some(data) = associated_diagnostic_data(&snapshot, &diagnostic) else {
                continue;
            };

            if should_offer_fix(&snapshot, &data) {
                actions.push(types::CodeActionOrCommand::CodeAction(diagnostic_fix_action(
                    &snapshot, &diagnostic, &data,
                )));
            }

            if let Some(edit) = data.directive_edit.clone() {
                actions.push(types::CodeActionOrCommand::CodeAction(
                    diagnostic_directive_action(&snapshot, &diagnostic, &data, edit),
                ));
            }
        }
    }

    if include_fix_all && snapshot.client_settings().fix_all() {
        let edits = fix_all_document_edits(
            &snapshot,
            if snapshot.client_settings().unsafe_fixes() {
                shuck_linter::Applicability::Unsafe
            } else {
                shuck_linter::Applicability::Safe
            },
        );
        if !edits.is_empty() {
            actions.push(types::CodeActionOrCommand::CodeAction(fix_all_action(
                &snapshot, edits,
            )?));
        }
    }

    Ok((!actions.is_empty()).then_some(actions))
}

pub(crate) fn resolve_code_action(
    session: &Session,
    _client: &Client,
    mut action: types::CodeAction,
) -> crate::server::Result<types::CodeAction> {
    if action.edit.is_some() {
        return Ok(action);
    }

    let Some(data) = action.data.clone() else {
        return Ok(action);
    };
    let resolved: ResolveCodeActionData =
        serde_json::from_value(data).context("deserialize code action resolve payload")?;
    let Some(snapshot) = session.take_snapshot(resolved.uri.clone()) else {
        return Ok(action);
    };

    let edits = match resolved.kind {
        ResolveCodeActionKind::FixAll => fix_all_document_edits(
            &snapshot,
            if resolved.include_unsafe {
                shuck_linter::Applicability::Unsafe
            } else {
                shuck_linter::Applicability::Safe
            },
        ),
    };
    if !edits.is_empty() {
        action.edit = Some(workspace_edit_for_document(&snapshot, edits));
    }
    Ok(action)
}

pub(crate) fn execute_command(
    session: &mut Session,
    client: &Client,
    params: types::ExecuteCommandParams,
) -> crate::server::Result<Option<serde_json::Value>> {
    match params.command.as_str() {
        "shuck.applyAutofix" => {
            let uri = command_uri(&params.arguments)?;
            let Some(snapshot) = session.take_snapshot(uri) else {
                return Ok(None);
            };
            let edits = fix_all_document_edits(
                &snapshot,
                if snapshot.client_settings().unsafe_fixes() {
                    shuck_linter::Applicability::Unsafe
                } else {
                    shuck_linter::Applicability::Safe
                },
            );
            apply_workspace_edit(session, client, "Shuck: apply autofix", &snapshot, edits)?;
            Ok(None)
        }
        "shuck.applyDirective" => {
            let args: ApplyDirectiveCommand = command_args(&params.arguments)?;
            let Some(snapshot) = session.take_snapshot(args.uri.clone()) else {
                return Ok(None);
            };
            let Some(edit) = directive_edit_for_line(&snapshot, args.line) else {
                return Ok(None);
            };
            apply_workspace_edit(
                session,
                client,
                "Shuck: disable for this line",
                &snapshot,
                vec![edit],
            )?;
            Ok(None)
        }
        "shuck.applyFormat" => {
            let uri = command_uri(&params.arguments)?;
            let Some(snapshot) = session.take_snapshot(uri) else {
                return Ok(None);
            };
            let edits = crate::format::format_document(
                snapshot.clone(),
                client,
                types::DocumentFormattingParams {
                    text_document: types::TextDocumentIdentifier {
                        uri: snapshot.query().file_url().clone(),
                    },
                    options: types::FormattingOptions {
                        tab_size: 8,
                        insert_spaces: false,
                        ..types::FormattingOptions::default()
                    },
                    work_done_progress_params: types::WorkDoneProgressParams::default(),
                },
            )?
            .unwrap_or_default();
            apply_workspace_edit(session, client, "Shuck: apply format", &snapshot, edits)?;
            Ok(None)
        }
        "shuck.printDebugInformation" => {
            tracing::info!(
                "shuck server state: open_documents={} workspace_roots={:?}",
                session.open_document_count(),
                session.workspace_roots()
            );
            Ok(None)
        }
        other => Err(crate::server::Error::new(
            anyhow!("unsupported executeCommand request: {other}"),
            ErrorCode::MethodNotFound,
        )),
    }
}

fn should_offer_fix(snapshot: &DocumentSnapshot, data: &AssociatedDiagnosticData) -> bool {
    !data.edits.is_empty()
        && shuck_linter::code_to_rule(&data.code)
            .is_some_and(|rule| snapshot.shuck_settings().fixable_rules().contains(rule))
        && (snapshot.client_settings().unsafe_fixes()
            || data.applicability == crate::lint::DiagnosticApplicability::Safe)
}

fn diagnostic_fix_action(
    snapshot: &DocumentSnapshot,
    diagnostic: &types::Diagnostic,
    data: &AssociatedDiagnosticData,
) -> types::CodeAction {
    types::CodeAction {
        title: format!("Shuck ({}): {}", data.code, data.title),
        kind: Some(types::CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(workspace_edit_for_document(snapshot, data.edits.clone())),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    }
}

fn diagnostic_directive_action(
    snapshot: &DocumentSnapshot,
    diagnostic: &types::Diagnostic,
    data: &AssociatedDiagnosticData,
    edit: types::TextEdit,
) -> types::CodeAction {
    types::CodeAction {
        title: format!("Shuck ({}): Disable for this line", data.code),
        kind: Some(types::CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(workspace_edit_for_document(snapshot, vec![edit])),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }
}

fn fix_all_action(
    snapshot: &DocumentSnapshot,
    edits: Vec<types::TextEdit>,
) -> crate::server::Result<types::CodeAction> {
    let mut action = types::CodeAction {
        title: "Shuck: Fix all auto-fixable issues".to_owned(),
        kind: Some(crate::SOURCE_FIX_ALL_SHUCK),
        diagnostics: None,
        edit: None,
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    };
    if snapshot
        .resolved_client_capabilities()
        .code_action_deferred_edit_resolution
    {
        action.data = Some(serde_json::to_value(ResolveCodeActionData {
            kind: ResolveCodeActionKind::FixAll,
            uri: snapshot.query().file_url().clone(),
            include_unsafe: snapshot.client_settings().unsafe_fixes(),
        })
        .map_err(anyhow::Error::new)?);
    } else {
        action.edit = Some(workspace_edit_for_document(snapshot, edits));
    }
    Ok(action)
}

fn workspace_edit_for_document(
    snapshot: &DocumentSnapshot,
    edits: Vec<types::TextEdit>,
) -> types::WorkspaceEdit {
    if snapshot.resolved_client_capabilities().document_changes {
        return types::WorkspaceEdit {
            changes: None,
            document_changes: Some(types::DocumentChanges::Edits(vec![
                types::TextDocumentEdit {
                    text_document: types::OptionalVersionedTextDocumentIdentifier {
                        uri: snapshot.query().file_url().clone(),
                        version: Some(snapshot.query().document().version()),
                    },
                    edits: edits.into_iter().map(types::OneOf::Left).collect(),
                },
            ])),
            change_annotations: None,
        };
    }

    let mut changes = std::collections::HashMap::new();
    changes.insert(snapshot.query().file_url().clone(), edits);
    types::WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }
}

fn apply_workspace_edit(
    session: &Session,
    client: &Client,
    label: &str,
    snapshot: &DocumentSnapshot,
    edits: Vec<types::TextEdit>,
) -> crate::server::Result<()> {
    if edits.is_empty() {
        return Ok(());
    }
    if !snapshot.resolved_client_capabilities().apply_edit {
        return Err(crate::server::Error::new(
            anyhow!("LSP client does not advertise workspace/applyEdit support"),
            ErrorCode::InvalidRequest,
        ));
    }

    client.send_request::<types::request::ApplyWorkspaceEdit>(
        session,
        types::ApplyWorkspaceEditParams {
            label: Some(label.to_owned()),
            edit: workspace_edit_for_document(snapshot, edits),
        },
        |_, response| {
            if !response.applied {
                tracing::warn!(
                    "Client rejected workspace edit: {}",
                    response.failure_reason.unwrap_or_else(|| "unknown reason".to_owned())
                );
            }
        },
    )?;
    Ok(())
}

fn wants_kind(only: Option<&Vec<types::CodeActionKind>>, expected: &types::CodeActionKind) -> bool {
    only.is_none_or(|kinds| kinds.iter().any(|kind| action_kind_matches(kind, expected)))
}

fn diagnostics_for_range(
    snapshot: &DocumentSnapshot,
    requested_range: &types::Range,
) -> Vec<types::Diagnostic> {
    generate_diagnostics(snapshot)
        .into_iter()
        .filter(|diagnostic| ranges_overlap(&diagnostic.range, requested_range))
        .collect()
}

fn action_kind_matches(
    requested: &types::CodeActionKind,
    provided: &types::CodeActionKind,
) -> bool {
    provided.as_str() == requested.as_str()
        || provided
            .as_str()
            .strip_prefix(requested.as_str())
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn ranges_overlap(left: &types::Range, right: &types::Range) -> bool {
    position_leq(left.start, right.end) && position_leq(right.start, left.end)
}

fn position_leq(left: types::Position, right: types::Position) -> bool {
    (left.line, left.character) <= (right.line, right.character)
}

fn command_uri(arguments: &[serde_json::Value]) -> crate::server::Result<lsp_types::Url> {
    if let Some(uri) = arguments
        .first()
        .and_then(|value| value.as_str())
        .and_then(|value| lsp_types::Url::parse(value).ok())
    {
        return Ok(uri);
    }

    #[derive(Deserialize)]
    struct UriArg {
        uri: lsp_types::Url,
    }

    let arg = arguments
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("missing executeCommand argument"))?;
    Ok(serde_json::from_value::<UriArg>(arg)
        .map_err(anyhow::Error::new)?
        .uri)
}

fn command_args<T: for<'de> Deserialize<'de>>(
    arguments: &[serde_json::Value],
) -> crate::server::Result<T> {
    let value = arguments
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("missing executeCommand argument"))?;
    Ok(serde_json::from_value(value).map_err(anyhow::Error::new)?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ResolveCodeActionKind {
    FixAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResolveCodeActionData {
    kind: ResolveCodeActionKind,
    uri: lsp_types::Url,
    include_unsafe: bool,
}

#[derive(Debug, Deserialize)]
struct ApplyDirectiveCommand {
    uri: lsp_types::Url,
    line: usize,
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_server::Message;
    use lsp_types::{
        CodeActionContext, CodeActionParams, ClientCapabilities, PartialResultParams, Position,
        Range, TextDocumentContentChangeEvent, TextDocumentIdentifier, Url,
        WorkDoneProgressParams,
    };

    use super::*;
    use crate::{
        ClientOptions, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace,
        Workspaces,
        lint::generate_diagnostics,
    };

    fn make_session(
        client_capabilities: ClientCapabilities,
        source: &str,
        language_id: &str,
        file_name: &str,
    ) -> (
        Session,
        Client,
        channel::Receiver<Message>,
        lsp_types::Url,
    ) {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-fix-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &client_capabilities,
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");
        session.update_client_options(ClientOptions {
            unsafe_fixes: Some(true),
            ..ClientOptions::default()
        });

        let path = workspace_root.join(file_name);
        let uri = Url::from_file_path(path).expect("test path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new(source.to_owned(), 1).with_language_id(language_id),
        );

        (session, client, client_receiver, uri)
    }

    fn extract_actions(
        response: types::CodeActionResponse,
    ) -> Vec<types::CodeAction> {
        response
            .into_iter()
            .map(|entry| match entry {
                types::CodeActionOrCommand::CodeAction(action) => action,
                types::CodeActionOrCommand::Command(command) => {
                    panic!("unexpected command response: {}", command.title)
                }
            })
            .collect()
    }

    fn first_edit_range(action: &types::CodeAction) -> Range {
        let edit = action
            .edit
            .as_ref()
            .expect("code action should include an edit");
        let document_changes = edit
            .document_changes
            .as_ref()
            .expect("workspace edit should use document changes");
        let types::DocumentChanges::Edits(edits) = document_changes else {
            panic!("workspace edit should contain document edits");
        };
        let text_edit = edits[0]
            .edits
            .first()
            .expect("workspace edit should contain at least one text edit");
        let types::OneOf::Left(text_edit) = text_edit else {
            panic!("workspace edit should contain plain text edits");
        };
        text_edit.range
    }

    fn deferred_capabilities() -> ClientCapabilities {
        serde_json::from_value(serde_json::json!({
            "textDocument": {
                "codeAction": {
                    "dataSupport": true,
                    "resolveSupport": { "properties": ["edit"] }
                }
            },
            "workspace": {
                "applyEdit": true,
                "workspaceEdit": {
                    "documentChanges": true
                }
            }
        }))
        .expect("test client capabilities should deserialize")
    }

    #[test]
    fn code_actions_include_quickfix_disable_and_fix_all() {
        let capabilities = deferred_capabilities();
        let (session, client, _client_receiver, uri) =
            make_session(capabilities, "foo=1\n", "shellscript", "script.sh");
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");
        let diagnostics = generate_diagnostics(&snapshot);

        let response = code_actions(
            snapshot,
            &client,
            CodeActionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                range: Range::new(Position::new(0, 0), Position::new(0, 3)),
                context: CodeActionContext {
                    diagnostics,
                    only: None,
                    trigger_kind: None,
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("code action request should succeed")
        .expect("violating document should produce actions");

        let actions = extract_actions(response);

        assert!(actions.iter().any(|action| action.title.contains("rename the unused assignment target")));
        assert!(actions.iter().any(|action| action.title.contains("Disable for this line")));
        let fix_all = actions
            .iter()
            .find(|action| action.kind == Some(crate::SOURCE_FIX_ALL_SHUCK))
            .expect("fix-all action should be present");
        assert!(fix_all.edit.is_none());
        assert!(fix_all.data.is_some());
    }

    #[test]
    fn code_actions_return_none_for_non_shell_documents() {
        let capabilities = deferred_capabilities();
        let (session, client, _client_receiver, uri) =
            make_session(capabilities, "# heading\n", "markdown", "README.md");
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let response = code_actions(
            snapshot,
            &client,
            CodeActionParams {
                text_document: TextDocumentIdentifier { uri },
                range: Range::new(Position::new(0, 0), Position::new(0, 3)),
                context: CodeActionContext {
                    diagnostics: Vec::new(),
                    only: None,
                    trigger_kind: None,
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("code action request should succeed");

        assert!(response.is_none());
    }

    #[test]
    fn code_action_resolve_materializes_deferred_fix_all_edit() {
        let capabilities = deferred_capabilities();
        let (session, client, _client_receiver, uri) =
            make_session(capabilities, "foo=1\n", "shellscript", "script.sh");
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");
        let diagnostics = generate_diagnostics(&snapshot);
        let response = code_actions(
            snapshot,
            &client,
            CodeActionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                range: Range::new(Position::new(0, 0), Position::new(0, 3)),
                context: CodeActionContext {
                    diagnostics,
                    only: Some(vec![crate::SOURCE_FIX_ALL_SHUCK]),
                    trigger_kind: None,
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("fix-all action request should succeed")
        .expect("fix-all action should be present");
        let action = match response.into_iter().next().expect("fix-all action expected") {
            types::CodeActionOrCommand::CodeAction(action) => action,
            types::CodeActionOrCommand::Command(_) => panic!("expected a code action"),
        };
        let expected_snapshot = session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot");
        let expected_edit = workspace_edit_for_document(
            &expected_snapshot,
            fix_all_document_edits(&expected_snapshot, shuck_linter::Applicability::Unsafe),
        );

        let resolved = resolve_code_action(&session, &client, action)
            .expect("resolve request should succeed");
        let edit = resolved.edit.expect("resolved action should include an edit");
        assert_eq!(edit, expected_edit);
    }

    #[test]
    fn apply_autofix_command_dispatches_workspace_edit_request() {
        let capabilities = deferred_capabilities();
        let (mut session, client, client_receiver, uri) =
            make_session(capabilities, "foo=1\n", "shellscript", "script.sh");

        execute_command(
            &mut session,
            &client,
            types::ExecuteCommandParams {
                command: "shuck.applyAutofix".to_owned(),
                arguments: vec![serde_json::Value::String(uri.to_string())],
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        )
        .expect("applyAutofix should succeed");

        let message = client_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("applyAutofix should send a workspace/applyEdit request");
        let Message::Request(request) = message else {
            panic!("expected a client request");
        };
        assert_eq!(request.method, "workspace/applyEdit");
    }

    #[test]
    fn code_actions_skip_rules_marked_unfixable_in_project_config() {
        let capabilities = deferred_capabilities();
        let workspace_root = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            workspace_root.path().join(".shuck.toml"),
            "[lint]\nunfixable = ['C001']\n",
        )
        .expect("config should be written");
        let script_path = workspace_root.path().join("script.sh");
        std::fs::write(&script_path, "foo=1\n").expect("source should be written");

        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_uri = Url::from_file_path(workspace_root.path())
            .expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &capabilities,
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");
        session.update_client_options(ClientOptions {
            unsafe_fixes: Some(true),
            ..ClientOptions::default()
        });

        let uri = Url::from_file_path(&script_path).expect("test path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("foo=1\n".to_owned(), 1).with_language_id("shellscript"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");
        let diagnostics = generate_diagnostics(&snapshot);

        let response = code_actions(
            snapshot,
            &client,
            CodeActionParams {
                text_document: TextDocumentIdentifier { uri },
                range: Range::new(Position::new(0, 0), Position::new(0, 3)),
                context: CodeActionContext {
                    diagnostics,
                    only: None,
                    trigger_kind: None,
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("code action request should succeed")
        .expect("violating document should still produce a disable action");

        let actions = extract_actions(response);

        assert!(actions
            .iter()
            .any(|action| action.title.contains("Disable for this line")));
        assert!(!actions
            .iter()
            .any(|action| action.title.contains("rename the unused assignment target")));
        assert!(!actions
            .iter()
            .any(|action| action.kind == Some(crate::SOURCE_FIX_ALL_SHUCK)));
    }

    #[test]
    fn code_actions_recompute_live_quickfix_and_disable_edits() {
        let capabilities = deferred_capabilities();
        let (mut session, client, _client_receiver, uri) =
            make_session(capabilities, "foo=1\n", "shellscript", "script.sh");
        let stale_snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");
        let stale_diagnostics = generate_diagnostics(&stale_snapshot);

        let key = session.key_from_url(uri.clone());
        session
            .update_text_document(
                &key,
                vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: "\nfoo=1\n".to_owned(),
                }],
                2,
            )
            .expect("text document update should succeed");

        let live_snapshot = session
            .take_snapshot(uri.clone())
            .expect("updated document should produce a snapshot");
        let response = code_actions(
            live_snapshot,
            &client,
            CodeActionParams {
                text_document: TextDocumentIdentifier { uri },
                range: Range::new(Position::new(1, 0), Position::new(1, 3)),
                context: CodeActionContext {
                    diagnostics: stale_diagnostics,
                    only: Some(vec![types::CodeActionKind::QUICKFIX]),
                    trigger_kind: None,
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("code action request should succeed")
        .expect("live diagnostic should still produce actions");

        let actions = extract_actions(response);
        let quickfix = actions
            .iter()
            .find(|action| action.title.contains("rename the unused assignment target"))
            .expect("quickfix action should be present");
        let disable = actions
            .iter()
            .find(|action| action.title.contains("Disable for this line"))
            .expect("disable action should be present");

        assert_eq!(first_edit_range(quickfix).start.line, 1);
        assert_eq!(first_edit_range(disable).start.line, 1);
        assert_eq!(
            quickfix
                .diagnostics
                .as_ref()
                .expect("quickfix should preserve associated diagnostic")[0]
                .range
                .start
                .line,
            1
        );
    }

    #[test]
    fn code_actions_match_parent_fix_all_kinds() {
        let capabilities = deferred_capabilities();
        let (session, client, _client_receiver, uri) =
            make_session(capabilities, "foo=1\n", "shellscript", "script.sh");

        for only in [
            types::CodeActionKind::SOURCE_FIX_ALL,
            types::CodeActionKind::SOURCE,
        ] {
            let snapshot = session
                .take_snapshot(uri.clone())
                .expect("test document should produce a snapshot");
            let response = code_actions(
                snapshot,
                &client,
                CodeActionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    range: Range::new(Position::new(0, 0), Position::new(0, 3)),
                    context: CodeActionContext {
                        diagnostics: Vec::new(),
                        only: Some(vec![only]),
                        trigger_kind: None,
                    },
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                },
            )
            .expect("code action request should succeed")
            .expect("fix-all action should be returned for parent kind filters");

            let actions = extract_actions(response);
            assert!(actions
                .iter()
                .any(|action| action.kind == Some(crate::SOURCE_FIX_ALL_SHUCK)));
        }
    }
}
