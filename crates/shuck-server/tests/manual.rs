use std::thread;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    ClientCapabilities, CodeAction, CodeActionContext, CodeActionParams, DocumentDiagnosticParams,
    DocumentDiagnosticReport, DocumentDiagnosticReportResult, HoverParams, PartialResultParams,
    Position, Range, TextDocumentIdentifier, TextDocumentPositionParams, Url,
    WorkDoneProgressParams,
};

fn send_request(connection: &Connection, id: i32, method: &str, params: serde_json::Value) {
    connection
        .sender
        .send(Message::Request(Request {
            id: RequestId::from(id),
            method: method.to_owned(),
            params,
        }))
        .expect("request should send");
}

fn recv_response(connection: &Connection, id: i32) -> serde_json::Value {
    let response = recv_lsp_response(connection, id);
    assert!(
        response.error.is_none(),
        "unexpected LSP error: {:?}",
        response.error
    );
    response
        .result
        .expect("successful response should carry a result")
}

fn recv_lsp_response(connection: &Connection, id: i32) -> Response {
    loop {
        let message = connection
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("server should respond");
        match message {
            Message::Response(response) if response.id == RequestId::from(id) => {
                return response;
            }
            Message::Notification(_) => continue,
            Message::Request(request) => panic!(
                "unexpected server request during replay: {}",
                request.method
            ),
            Message::Response(_) => continue,
        }
    }
}

fn replay_capabilities() -> ClientCapabilities {
    serde_json::from_value(serde_json::json!({
        "general": {
            "positionEncodings": ["utf-16"]
        },
        "textDocument": {
            "diagnostic": {
                "dynamicRegistration": false,
                "relatedDocumentSupport": false
            },
            "codeAction": {
                "dataSupport": true,
                "resolveSupport": { "properties": ["edit"] }
            },
            "hover": {
                "contentFormat": ["markdown"]
            }
        },
        "workspace": {
            "applyEdit": true,
            "workspaceEdit": {
                "documentChanges": true
            },
            "workspaceFolders": true,
            "configuration": false
        }
    }))
    .expect("test client capabilities should deserialize")
}

#[test]
fn replays_a_small_lsp_session() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace_root = tempfile::tempdir().expect("tempdir should be created");
    let script_path = workspace_root.path().join("script.sh");
    let script_uri =
        Url::from_file_path(&script_path).expect("script path should convert to a URL");

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": replay_capabilities(),
            "rootUri": Url::from_file_path(workspace_root.path())
                .expect("workspace path should convert to a URL"),
            "initializationOptions": { "shuck": { "fixAll": true, "unsafeFixes": true } }
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(
        initialize["capabilities"]["documentFormattingProvider"],
        serde_json::json!(true)
    );
    assert_eq!(
        initialize["capabilities"]["documentRangeFormattingProvider"],
        serde_json::json!(true)
    );
    assert_eq!(
        initialize["capabilities"]["definitionProvider"],
        serde_json::json!(true)
    );
    assert_eq!(
        initialize["capabilities"]["referencesProvider"],
        serde_json::json!(true)
    );
    assert_eq!(
        initialize["capabilities"]["documentHighlightProvider"],
        serde_json::json!(true)
    );
    assert_eq!(
        initialize["capabilities"]["foldingRangeProvider"],
        serde_json::json!(true)
    );
    assert!(initialize["capabilities"]["completionProvider"].is_object());
    assert_eq!(
        initialize["capabilities"]["renameProvider"]["prepareProvider"],
        serde_json::json!(true)
    );
    assert_eq!(
        initialize["capabilities"]["selectionRangeProvider"],
        serde_json::json!(true)
    );

    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized notification should send");

    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "textDocument/didOpen".to_owned(),
            serde_json::json!({
                "textDocument": {
                    "uri": script_uri,
                    "languageId": "shellscript",
                    "version": 1,
                    "text": "foo=1\n",
                }
            }),
        )))
        .expect("didOpen notification should send");

    send_request(
        &client_connection,
        2,
        "textDocument/diagnostic",
        serde_json::to_value(DocumentDiagnosticParams {
            text_document: TextDocumentIdentifier {
                uri: script_uri.clone(),
            },
            identifier: None,
            previous_result_id: None,
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        })
        .expect("diagnostic params should serialize"),
    );
    let diagnostic_report: DocumentDiagnosticReportResult =
        serde_json::from_value(recv_response(&client_connection, 2))
            .expect("diagnostic response should deserialize");
    let DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(report)) =
        diagnostic_report
    else {
        panic!("expected a full diagnostic report");
    };
    assert_eq!(report.full_document_diagnostic_report.items.len(), 1);
    let diagnostics = report.full_document_diagnostic_report.items;

    send_request(
        &client_connection,
        3,
        "textDocument/codeAction",
        serde_json::to_value(CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: script_uri.clone(),
            },
            range: Range::new(Position::new(0, 0), Position::new(0, 3)),
            context: CodeActionContext {
                diagnostics: diagnostics.clone(),
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        })
        .expect("code action params should serialize"),
    );
    let actions: Vec<lsp_types::CodeActionOrCommand> =
        serde_json::from_value(recv_response(&client_connection, 3))
            .expect("code action response should deserialize");
    let fix_all = actions
        .into_iter()
        .filter_map(|entry| match entry {
            lsp_types::CodeActionOrCommand::CodeAction(action) => Some(action),
            lsp_types::CodeActionOrCommand::Command(_) => None,
        })
        .find(|action| {
            action
                .kind
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "source.fixAll.shuck")
        })
        .expect("fix-all action should be present");
    assert!(fix_all.edit.is_none());
    assert!(fix_all.data.is_some());

    send_request(
        &client_connection,
        4,
        "codeAction/resolve",
        serde_json::to_value(fix_all).expect("code action should serialize"),
    );
    let resolved: CodeAction = serde_json::from_value(recv_response(&client_connection, 4))
        .expect("resolved code action should deserialize");
    assert!(resolved.edit.is_some());

    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "textDocument/didChange".to_owned(),
            serde_json::json!({
                "textDocument": { "uri": script_uri, "version": 2 },
                "contentChanges": [{ "text": "#!/bin/bash\necho $foo  # shellcheck disable=SC2154\n" }],
            }),
        )))
        .expect("didChange notification should send");

    send_request(
        &client_connection,
        5,
        "textDocument/hover",
        serde_json::to_value(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: script_uri.clone(),
                },
                position: Position::new(1, 37),
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .expect("hover params should serialize"),
    );
    let hover = recv_response(&client_connection, 5);
    assert!(
        hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("Undefined Variable"))
    );

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");

    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

fn open_document(connection: &Connection, uri: &Url, text: &str) {
    connection
        .sender
        .send(Message::Notification(Notification::new(
            "textDocument/didOpen".to_owned(),
            serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": "shellscript",
                    "version": 1,
                    "text": text,
                }
            }),
        )))
        .expect("didOpen should send");
}

fn change_document(connection: &Connection, uri: &Url, version: i32, text: &str) {
    connection
        .sender
        .send(Message::Notification(Notification::new(
            "textDocument/didChange".to_owned(),
            serde_json::json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }],
            }),
        )))
        .expect("didChange should send");
}

fn folding_ranges_for_encoding(encoding: &str, function_start: u64) {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    let script_path = workspace.path().join("folding.sh");
    let script_uri = Url::from_file_path(&script_path).unwrap();
    let source = ": '😀'; foo() {\n  if true; then\n    echo nested\n  fi\n}\n";
    let mut capabilities = serde_json::to_value(replay_capabilities()).unwrap();
    capabilities["general"]["positionEncodings"] = serde_json::json!([encoding]);
    capabilities["textDocument"]["foldingRange"] = serde_json::json!({
        "dynamicRegistration": false,
        "lineFoldingOnly": false,
    });

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": capabilities,
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(initialize["capabilities"]["positionEncoding"], encoding);
    assert_eq!(
        initialize["capabilities"]["foldingRangeProvider"],
        serde_json::json!(true)
    );
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &script_uri, source);

    send_request(
        &client_connection,
        2,
        "textDocument/foldingRange",
        serde_json::json!({ "textDocument": { "uri": script_uri } }),
    );
    let ranges = recv_response(&client_connection, 2);
    let ranges = ranges.as_array().expect("folding ranges should be a list");
    assert_eq!(ranges.len(), 2, "unexpected ranges: {ranges:#?}");
    assert_eq!(
        ranges[0],
        serde_json::json!({
            "startLine": 0,
            "startCharacter": function_start,
            "endLine": 3,
            "endCharacter": 4,
        })
    );
    assert_eq!(
        ranges[1],
        serde_json::json!({
            "startLine": 1,
            "startCharacter": 2,
            "endLine": 2,
            "endCharacter": 15,
        })
    );

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn folding_ranges_use_utf8_positions() {
    folding_ranges_for_encoding("utf-8", 10);
}

#[test]
fn folding_ranges_use_utf16_positions() {
    folding_ranges_for_encoding("utf-16", 8);
}

#[test]
fn folding_ranges_use_utf32_positions() {
    folding_ranges_for_encoding("utf-32", 7);
}

fn selection_ranges_for_encoding(encoding: &str, echo_start: u64, name_start: u64) {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    let script_path = workspace.path().join("script.sh");
    let script_uri = Url::from_file_path(&script_path).unwrap();
    let source = ": '😀'; echo \"${name:-$(printf '%s' value)}\"\n";
    std::fs::write(&script_path, source).unwrap();

    let mut capabilities = serde_json::to_value(replay_capabilities()).unwrap();
    capabilities["general"]["positionEncodings"] = serde_json::json!([encoding]);
    capabilities["textDocument"]["selectionRange"] = serde_json::json!({});
    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": capabilities,
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(initialize["capabilities"]["positionEncoding"], encoding);
    assert_eq!(initialize["capabilities"]["selectionRangeProvider"], true);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &script_uri, source);

    send_request(
        &client_connection,
        2,
        "textDocument/selectionRange",
        serde_json::json!({
            "textDocument": { "uri": script_uri },
            "positions": [
                { "line": 0, "character": name_start },
                { "line": 0, "character": echo_start },
            ],
        }),
    );
    let response: Vec<lsp_types::SelectionRange> =
        serde_json::from_value(recv_response(&client_connection, 2)).unwrap();
    assert_eq!(response.len(), 2);

    for (selection, expected_start, expected_end) in [
        (&response[0], name_start, name_start + 4),
        (&response[1], echo_start, echo_start + 4),
    ] {
        assert_eq!(
            selection.range.start,
            Position::new(0, expected_start as u32)
        );
        assert_eq!(selection.range.end, Position::new(0, expected_end as u32));

        let mut current = selection;
        while let Some(parent) = current.parent.as_deref() {
            assert_ne!(current.range, parent.range);
            assert!(parent.range.start <= current.range.start);
            assert!(current.range.end <= parent.range.end);
            current = parent;
        }
        assert_eq!(current.range.start, Position::new(0, 0));
        assert_eq!(current.range.end, Position::new(1, 0));
    }

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn selection_ranges_use_utf8_positions() {
    selection_ranges_for_encoding("utf-8", 10, 18);
}

#[test]
fn selection_ranges_use_utf16_positions() {
    selection_ranges_for_encoding("utf-16", 8, 16);
}

#[test]
fn selection_ranges_use_utf32_positions() {
    selection_ranges_for_encoding("utf-32", 7, 15);
}

fn cross_file_rename_for_encoding(
    encoding: &str,
    name_start: u64,
    supports_document_changes: bool,
) {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    let lib = workspace.path().join("lib");
    std::fs::create_dir(&lib).unwrap();
    std::fs::write(
        workspace.path().join("shuck.toml"),
        "[lint]\nsource-paths = [\"lib\"]\n",
    )
    .unwrap();
    let target_path = lib.join("target.sh");
    let cycle_path = lib.join("cycle.sh");
    let main_path = workspace.path().join("main.sh");
    let caller_path = workspace.path().join("caller.sh");
    let shadow_path = workspace.path().join("shadow.sh");
    let unrelated_path = workspace.path().join("unrelated.sh");
    let target_source = "source cycle.sh\n: '😀'; foo() { :; }\n";
    let main_source = "# shuck: source=target.sh\nsource \"$DIR/target.sh\"\n: '😀'; foo\n";
    let caller_source = "source main.sh\nfoo\n";
    let shadow_source = "source main.sh\nfoo\nfoo() { :; }\nfoo\n";
    std::fs::write(&target_path, "stale_target() { :; }\n").unwrap();
    std::fs::write(&cycle_path, "source target.sh\n").unwrap();
    std::fs::write(&main_path, "stale_main\n").unwrap();
    std::fs::write(&caller_path, caller_source).unwrap();
    std::fs::write(&shadow_path, shadow_source).unwrap();
    std::fs::write(&unrelated_path, "foo() { :; }\nfoo\n").unwrap();

    let target_uri = Url::from_file_path(&target_path).unwrap();
    let main_uri = Url::from_file_path(&main_path).unwrap();
    let caller_uri = Url::from_file_path(std::fs::canonicalize(&caller_path).unwrap()).unwrap();
    let shadow_uri = Url::from_file_path(std::fs::canonicalize(&shadow_path).unwrap()).unwrap();
    let unrelated_uri =
        Url::from_file_path(std::fs::canonicalize(&unrelated_path).unwrap()).unwrap();

    let mut capabilities = serde_json::to_value(replay_capabilities()).unwrap();
    capabilities["general"]["positionEncodings"] = serde_json::json!([encoding]);
    capabilities["workspace"]["workspaceEdit"]["documentChanges"] =
        serde_json::json!(supports_document_changes);
    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": capabilities,
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
            "initializationOptions": {
                "shuck": { "server": { "rename": { "allowCrossFile": true } } }
            },
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(initialize["capabilities"]["positionEncoding"], encoding);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &target_uri, target_source);
    open_document(&client_connection, &main_uri, main_source);

    if !supports_document_changes {
        send_request(
            &client_connection,
            2,
            "textDocument/rename",
            serde_json::json!({
                "textDocument": { "uri": main_uri },
                "position": { "line": 2, "character": name_start },
                "newName": "unsafe_without_versions",
            }),
        );
        let unsupported = recv_lsp_response(&client_connection, 2);
        let unsupported_error = unsupported
            .error
            .expect("cross-file rename should require versioned document changes");
        assert!(
            unsupported_error
                .message
                .contains("requires client support")
        );
        send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
        let _ = recv_response(&client_connection, 99);
        client_connection
            .sender
            .send(Message::Notification(Notification::new(
                "exit".to_owned(),
                serde_json::json!({}),
            )))
            .expect("exit notification should send");
        server_thread
            .join()
            .expect("server thread should join")
            .expect("server should exit cleanly");
        return;
    }

    send_request(
        &client_connection,
        2,
        "textDocument/prepareRename",
        serde_json::json!({
            "textDocument": { "uri": main_uri },
            "position": { "line": 2, "character": name_start },
        }),
    );
    let prepared = recv_response(&client_connection, 2);
    assert_eq!(prepared["placeholder"], "foo");
    assert_eq!(
        prepared["range"],
        serde_json::json!({
            "start": { "line": 2, "character": name_start },
            "end": { "line": 2, "character": name_start + 3 },
        })
    );

    send_request(
        &client_connection,
        3,
        "textDocument/rename",
        serde_json::json!({
            "textDocument": { "uri": main_uri },
            "position": { "line": 2, "character": name_start },
            "newName": "renamed",
        }),
    );
    let rename = recv_response(&client_connection, 3);
    let documents = rename["documentChanges"]
        .as_array()
        .expect("cross-file rename should use documentChanges");
    assert_eq!(documents.len(), 4);
    let document = |uri: &Url| {
        documents
            .iter()
            .find(|document| document["textDocument"]["uri"] == uri.as_str())
            .unwrap_or_else(|| panic!("missing rename edits for {uri}: {rename:#}"))
    };
    let target = document(&target_uri);
    assert_eq!(target["textDocument"]["version"], 1);
    assert_eq!(target["edits"].as_array().unwrap().len(), 1);
    assert_eq!(
        target["edits"][0]["range"]["start"]["character"],
        name_start
    );
    let main = document(&main_uri);
    assert_eq!(main["textDocument"]["version"], 1);
    assert_eq!(main["edits"].as_array().unwrap().len(), 1);
    let caller = document(&caller_uri);
    assert!(caller["textDocument"]["version"].is_null());
    assert_eq!(caller["edits"].as_array().unwrap().len(), 1);
    let shadow = document(&shadow_uri);
    assert!(shadow["textDocument"]["version"].is_null());
    assert_eq!(shadow["edits"].as_array().unwrap().len(), 1);
    assert!(
        documents
            .iter()
            .all(|document| { document["textDocument"]["uri"] != unrelated_uri.as_str() })
    );

    std::fs::write(&unrelated_path, "source main.sh\nfoo\n").unwrap();
    send_request(
        &client_connection,
        4,
        "textDocument/rename",
        serde_json::json!({
            "textDocument": { "uri": main_uri },
            "position": { "line": 2, "character": name_start },
            "newName": "freshly_connected",
        }),
    );
    let refreshed = recv_response(&client_connection, 4);
    let refreshed_documents = refreshed["documentChanges"]
        .as_array()
        .expect("fresh rename should use documentChanges");
    let newly_connected = refreshed_documents
        .iter()
        .find(|document| document["textDocument"]["uri"] == unrelated_uri.as_str())
        .expect("fresh mutation index should include the newly connected closed file");
    assert_eq!(newly_connected["edits"].as_array().unwrap().len(), 1);

    std::fs::write(&unrelated_path, "foo() { :; }\nfoo\n").unwrap();
    change_document(
        &client_connection,
        &main_uri,
        2,
        "# shuck: source=target.sh\nsource \"$DIR/target.sh\"\n: '😀'; foo\nsource \"$dynamic\"\n",
    );
    send_request(
        &client_connection,
        5,
        "textDocument/rename",
        serde_json::json!({
            "textDocument": { "uri": main_uri },
            "position": { "line": 2, "character": name_start },
            "newName": "ambiguous_rejected",
        }),
    );
    let unresolved = recv_lsp_response(&client_connection, 5);
    let unresolved_error = unresolved
        .error
        .expect("unresolved source should fail rename");
    assert!(
        unresolved_error.message.contains("unresolved or unindexed")
            || unresolved_error
                .message
                .contains("ambiguous binding identity"),
        "unexpected unresolved-source error: {}",
        unresolved_error.message
    );

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn cross_file_rename_uses_utf8_positions() {
    cross_file_rename_for_encoding("utf-8", 10, true);
}

#[test]
fn cross_file_rename_uses_utf16_positions() {
    cross_file_rename_for_encoding("utf-16", 8, true);
}

#[test]
fn cross_file_rename_uses_utf32_positions() {
    cross_file_rename_for_encoding("utf-32", 7, true);
}

#[test]
fn cross_file_rename_requires_versioned_document_changes() {
    cross_file_rename_for_encoding("utf-16", 8, false);
}

fn document_links_follow_sources_for_encoding(
    encoding: &str,
    relative_start: u64,
    changed_start: u64,
) {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let outer = tempfile::tempdir().expect("tempdir should be created");
    let workspace = outer.path().join("workspace");
    let scripts = workspace.join("scripts");
    let lib = workspace.join("lib");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::create_dir(&lib).unwrap();
    std::fs::write(
        workspace.join("shuck.toml"),
        "[lint]\nsource-paths = [\"lib\"]\n",
    )
    .unwrap();
    std::fs::write(scripts.join("relative.sh"), ":\n").unwrap();
    std::fs::write(lib.join("configured.sh"), ":\n").unwrap();
    std::fs::write(scripts.join("cycle_a.sh"), "source cycle_b.sh\n").unwrap();
    std::fs::write(scripts.join("cycle_b.sh"), "source cycle_a.sh\n").unwrap();
    std::fs::write(outer.path().join("outside.sh"), ":\n").unwrap();

    let main_path = scripts.join("main.sh");
    let main_source = "# shuck: source=hinted.sh\nsource \"$DIR/hinted.sh\"\n: '😀'; . relative.sh\nsource configured.sh\nsource cycle_a.sh\nsource \"$dynamic\"\nsource missing.sh\nsource ../../outside.sh\n";
    std::fs::write(&main_path, "stale\n").unwrap();
    let main_uri = Url::from_file_path(&main_path).unwrap();
    let hinted_uri = Url::from_file_path(scripts.join("hinted.sh")).unwrap();
    let cycle_a_uri = Url::from_file_path(scripts.join("cycle_a.sh")).unwrap();
    let cycle_b_uri =
        Url::from_file_path(std::fs::canonicalize(scripts.join("cycle_b.sh")).unwrap()).unwrap();
    let relative_uri =
        Url::from_file_path(std::fs::canonicalize(scripts.join("relative.sh")).unwrap()).unwrap();
    let configured_uri =
        Url::from_file_path(std::fs::canonicalize(lib.join("configured.sh")).unwrap()).unwrap();

    let mut capabilities = serde_json::to_value(replay_capabilities()).unwrap();
    capabilities["general"]["positionEncodings"] = serde_json::json!([encoding]);
    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": capabilities,
            "rootUri": Url::from_file_path(&workspace).unwrap(),
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(initialize["capabilities"]["positionEncoding"], encoding);
    assert_eq!(
        initialize["capabilities"]["documentLinkProvider"]["resolveProvider"],
        false
    );
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &hinted_uri, "unsaved() { :; }\n");
    open_document(&client_connection, &cycle_a_uri, "source cycle_b.sh\n");
    open_document(&client_connection, &main_uri, main_source);

    send_request(
        &client_connection,
        2,
        "textDocument/documentLink",
        serde_json::json!({ "textDocument": { "uri": main_uri } }),
    );
    let links = recv_response(&client_connection, 2);
    let links = links.as_array().expect("document links should be a list");
    assert_eq!(
        links.len(),
        4,
        "dynamic, missing, and outside paths must be omitted"
    );
    assert_eq!(links[0]["target"], hinted_uri.as_str());
    assert_eq!(
        links[0]["range"],
        serde_json::json!({
            "start": { "line": 0, "character": 16 },
            "end": { "line": 0, "character": 25 },
        })
    );
    assert_eq!(links[1]["target"], relative_uri.as_str());
    assert_eq!(
        links[1]["range"],
        serde_json::json!({
            "start": { "line": 2, "character": relative_start },
            "end": { "line": 2, "character": relative_start + 11 },
        })
    );
    assert_eq!(links[2]["target"], configured_uri.as_str());
    assert_eq!(links[3]["target"], cycle_a_uri.as_str());

    send_request(
        &client_connection,
        3,
        "textDocument/documentLink",
        serde_json::json!({ "textDocument": { "uri": cycle_a_uri } }),
    );
    let cycle_links = recv_response(&client_connection, 3);
    assert_eq!(cycle_links.as_array().unwrap().len(), 1);
    assert_eq!(cycle_links[0]["target"], cycle_b_uri.as_str());

    let buffered_uri = Url::from_file_path(scripts.join("buffered.sh")).unwrap();
    open_document(&client_connection, &buffered_uri, "buffered() { :; }\n");
    change_document(
        &client_connection,
        &main_uri,
        2,
        ": '😀'; source buffered.sh\nsource missing_after_change.sh\n",
    );
    send_request(
        &client_connection,
        4,
        "textDocument/documentLink",
        serde_json::json!({ "textDocument": { "uri": main_uri } }),
    );
    let changed = recv_response(&client_connection, 4);
    let changed = changed.as_array().unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["target"], buffered_uri.as_str());
    assert_eq!(
        changed[0]["range"],
        serde_json::json!({
            "start": { "line": 0, "character": changed_start },
            "end": { "line": 0, "character": changed_start + 11 },
        })
    );

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn document_links_follow_sources_with_utf8_positions() {
    document_links_follow_sources_for_encoding("utf-8", 12, 17);
}

#[test]
fn document_links_follow_sources_with_utf16_positions() {
    document_links_follow_sources_for_encoding("utf-16", 10, 15);
}

#[test]
fn document_links_follow_sources_with_utf32_positions() {
    document_links_follow_sources_for_encoding("utf-32", 9, 14);
}

#[test]
fn cross_file_definition_uses_exact_workspace_binding_and_open_buffers() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    std::fs::write(
        workspace.path().join("shuck.toml"),
        "[lint]\nsource-paths = [\"lib\"]\n",
    )
    .unwrap();
    std::fs::create_dir(workspace.path().join("lib")).unwrap();
    let configured_path = workspace.path().join("lib/configured.sh");
    std::fs::write(&configured_path, "configured() { :; }\n").unwrap();

    let imported_path = workspace.path().join("imported.sh");
    std::fs::write(&imported_path, "stale() { :; }\n").unwrap();
    let imported_source = ": '😀'; imported() {\n  :\n}\n";

    let caller_path = workspace.path().join("caller.sh");
    std::fs::write(&caller_path, "stale_caller\n").unwrap();
    let caller_source = "# shuck: source=imported.sh\nsource \"$DIR/imported.sh\"\nprintf '😀'; imported\nimported() {\n  :\n}\nimported\nsource configured.sh\nconfigured\nsource \"$dynamic\"\nimported\nunknown\n";

    let imported_uri = Url::from_file_path(std::fs::canonicalize(&imported_path).unwrap()).unwrap();
    let configured_uri =
        Url::from_file_path(std::fs::canonicalize(&configured_path).unwrap()).unwrap();
    let caller_uri = Url::from_file_path(&caller_path).unwrap();
    let mut capabilities = serde_json::to_value(replay_capabilities()).unwrap();
    capabilities["general"]["positionEncodings"] = serde_json::json!(["utf-8"]);

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": capabilities,
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(
        initialize["capabilities"]["positionEncoding"],
        serde_json::json!("utf-8")
    );
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &imported_uri, imported_source);
    open_document(&client_connection, &caller_uri, caller_source);

    // The first call sees the sourced definition from the unsaved buffer. Its
    // UTF-8 range begins after a four-byte emoji on the same line.
    send_request(
        &client_connection,
        2,
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 2, "character": 15 },
        }),
    );
    let imported = recv_response(&client_connection, 2);
    assert_eq!(imported["uri"], serde_json::json!(imported_uri));
    assert_eq!(
        imported["range"]["start"],
        serde_json::json!({ "line": 0, "character": 10 })
    );

    // The later local definition replaces the imported one for subsequent
    // calls, while retaining the caller's open-document URI.
    send_request(
        &client_connection,
        3,
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 6, "character": 0 },
        }),
    );
    let local = recv_response(&client_connection, 3);
    assert_eq!(local["uri"], serde_json::json!(caller_uri));
    assert_eq!(
        local["range"]["start"],
        serde_json::json!({ "line": 3, "character": 0 })
    );

    // Literal source operands also honor configured source paths.
    send_request(
        &client_connection,
        4,
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 8, "character": 0 },
        }),
    );
    let configured = recv_response(&client_connection, 4);
    assert_eq!(configured["uri"], serde_json::json!(configured_uri));

    // A dynamic source may replace even a previously proven local function,
    // so navigation fails closed instead of returning the stale binding.
    send_request(
        &client_connection,
        5,
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 10, "character": 0 },
        }),
    );
    assert!(recv_response(&client_connection, 5).is_null());

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn sourced_function_completion_uses_order_shadowing_and_open_buffers() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    std::fs::write(
        workspace.path().join("shuck.toml"),
        "[lint]\nsource-paths = [\"vendor\"]\n",
    )
    .unwrap();
    std::fs::create_dir(workspace.path().join("vendor")).unwrap();
    std::fs::write(
        workspace.path().join("vendor/configured.sh"),
        "configured_function() { :; }\n",
    )
    .unwrap();
    let library_path = workspace.path().join("lib.sh");
    let caller_path = workspace.path().join("main.sh");
    std::fs::write(&library_path, "stale_disk_only() { :; }\n").unwrap();
    let completion_line = ": \"🦀\"; imp";
    let caller = format!(
        "imp\n# shuck: source=lib.sh\nsource \"$DIR/lib.sh\"\n{completion_line}\ndup() {{ :; }}\ndu\nlat\nsource later.sh\nsource configured.sh\ncon\necho \"$imp\"\nrun() {{ local imp\n}}\nlocal_scope() {{\n  source inner.sh\n  inn\n}}\n"
    );
    std::fs::write(&caller_path, &caller).unwrap();
    std::fs::write(
        workspace.path().join("later.sh"),
        "later_function() { :; }\n",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("inner.sh"),
        "inner_imported() { :; }\n",
    )
    .unwrap();
    let library_uri = Url::from_file_path(&library_path).unwrap();
    let caller_uri = Url::from_file_path(&caller_path).unwrap();
    let mut capabilities = serde_json::to_value(replay_capabilities()).unwrap();
    capabilities["general"]["positionEncodings"] = serde_json::json!(["utf-8"]);

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": capabilities,
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(initialize["capabilities"]["positionEncoding"], "utf-8");
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(
        &client_connection,
        &library_uri,
        "imported() { :; }\ndup() { echo sourced; }\n",
    );
    open_document(&client_connection, &caller_uri, &caller);

    send_request(
        &client_connection,
        2,
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 0, "character": 3 },
        }),
    );
    let before_source = recv_response(&client_connection, 2);
    assert!(
        before_source["items"]
            .as_array()
            .is_none_or(|items| items.iter().all(|item| item["label"] != "imported"))
    );

    send_request(
        &client_connection,
        3,
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 3, "character": completion_line.len() },
        }),
    );
    let after_source = recv_response(&client_connection, 3);
    let imported = after_source["items"]
        .as_array()
        .unwrap_or_else(|| panic!("completion response should be a list: {after_source:#}"))
        .iter()
        .find(|item| item["label"] == "imported")
        .expect("unsaved sourced function should complete");
    assert_eq!(imported["detail"], "Function (sourced)");
    assert_eq!(
        imported["textEdit"]["range"],
        serde_json::json!({
            "start": { "line": 3, "character": completion_line.len() - 3 },
            "end": { "line": 3, "character": completion_line.len() },
        })
    );

    send_request(
        &client_connection,
        4,
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 5, "character": 2 },
        }),
    );
    let shadowed = recv_response(&client_connection, 4);
    let duplicates = shadowed["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["label"] == "dup")
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0]["detail"], "Function");

    send_request(
        &client_connection,
        5,
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 6, "character": 3 },
        }),
    );
    let before_later_source = recv_response(&client_connection, 5);
    assert!(
        before_later_source["items"]
            .as_array()
            .is_none_or(|items| items.iter().all(|item| item["label"] != "later_function"))
    );

    send_request(
        &client_connection,
        6,
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 9, "character": 3 },
        }),
    );
    let configured = recv_response(&client_connection, 6);
    assert!(
        configured["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "configured_function")
    );

    for (id, line, character) in [(7, 10, 10), (8, 11, "run() { local imp".len())] {
        send_request(
            &client_connection,
            id,
            "textDocument/completion",
            serde_json::json!({
                "textDocument": { "uri": caller_uri },
                "position": { "line": line, "character": character },
            }),
        );
        let non_command = recv_response(&client_connection, id);
        assert!(
            non_command["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["label"] != "imported"),
            "sourced functions must not leak into parameter or declaration completion"
        );
    }

    send_request(
        &client_connection,
        9,
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 15, "character": 5 },
        }),
    );
    let function_local = recv_response(&client_connection, 9);
    assert!(
        function_local["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "inner_imported")
    );

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn cross_file_hover_uses_exact_workspace_binding_and_open_buffers() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    std::fs::write(
        workspace.path().join("shuck.toml"),
        "[lint]\nsource-paths = [\"lib\"]\n",
    )
    .unwrap();
    std::fs::create_dir(workspace.path().join("lib")).unwrap();
    let configured_path = workspace.path().join("lib/configured.sh");
    std::fs::write(&configured_path, "configured() { :; }\n").unwrap();

    let imported_path = workspace.path().join("imported.sh");
    std::fs::write(&imported_path, "stale() { :; }\n").unwrap();
    let imported_source = ": '😀'; imported() {\n  :\n}\n";

    let caller_path = workspace.path().join("caller.sh");
    let caller_source = "# shuck: source=imported.sh\nsource \"$DIR/imported.sh\"\nprintf '😀'; imported\nimported() {\n  :\n}\nimported\nsource configured.sh\nconfigured\nsource \"$dynamic\"\nimported\n";
    std::fs::write(&caller_path, caller_source).unwrap();

    let imported_uri = Url::from_file_path(std::fs::canonicalize(&imported_path).unwrap()).unwrap();
    let configured_uri =
        Url::from_file_path(std::fs::canonicalize(&configured_path).unwrap()).unwrap();
    let caller_uri = Url::from_file_path(&caller_path).unwrap();
    let mut capabilities = serde_json::to_value(replay_capabilities()).unwrap();
    capabilities["general"]["positionEncodings"] = serde_json::json!(["utf-8"]);

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": capabilities,
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(initialize["capabilities"]["positionEncoding"], "utf-8");
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &imported_uri, imported_source);
    open_document(&client_connection, &caller_uri, caller_source);

    send_request(
        &client_connection,
        2,
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 2, "character": 15 },
        }),
    );
    let imported = recv_response(&client_connection, 2);
    let imported_markdown = imported["contents"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("expected sourced function hover: {imported:#}"));
    assert!(imported_markdown.contains("### `imported`"));
    assert!(imported_markdown.contains("Function"));
    let rendered_imported_path = imported_uri
        .to_file_path()
        .expect("imported URI should round-trip to its display path");
    assert!(imported_markdown.contains(&rendered_imported_path.display().to_string()));
    assert!(
        imported_markdown.contains("line 1, column 8"),
        "unexpected hover content: {imported_markdown}"
    );
    assert_eq!(
        imported["range"],
        serde_json::json!({
            "start": { "line": 2, "character": 15 },
            "end": { "line": 2, "character": 23 },
        })
    );

    send_request(
        &client_connection,
        3,
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 6, "character": 0 },
        }),
    );
    let local = recv_response(&client_connection, 3);
    let local_markdown = local["contents"]["value"].as_str().unwrap();
    assert!(local_markdown.contains("Defined at line 4"));
    assert!(!local_markdown.contains(&imported_path.display().to_string()));

    send_request(
        &client_connection,
        4,
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 8, "character": 0 },
        }),
    );
    let configured = recv_response(&client_connection, 4);
    let rendered_configured_path = configured_uri
        .to_file_path()
        .expect("configured URI should round-trip to its display path");
    assert!(
        configured["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains(&rendered_configured_path.display().to_string()))
    );

    send_request(
        &client_connection,
        5,
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": caller_uri },
            "position": { "line": 10, "character": 0 },
        }),
    );
    assert!(recv_response(&client_connection, 5).is_null());

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn cross_file_references_preserve_binding_identity_and_open_buffers() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    std::fs::write(
        workspace.path().join("shuck.toml"),
        "[lint]\nsource-paths = [\"lib\"]\n",
    )
    .unwrap();
    std::fs::create_dir(workspace.path().join("lib")).unwrap();
    let target_path = workspace.path().join("lib/a.sh");
    let other_path = workspace.path().join("other.sh");
    let caller_path = workspace.path().join("caller.sh");
    let child_path = workspace.path().join("child.sh");
    let hinted_path = workspace.path().join("hinted.sh");
    let ambiguous_path = workspace.path().join("ambiguous.sh");
    std::fs::write(&target_path, "stale() { :; }\n").unwrap();
    std::fs::write(&other_path, "shared() { echo other; }\n").unwrap();
    let caller_source = "source a.sh\nsource child.sh\nshared\nsource other.sh\nshared\n";
    std::fs::write(&caller_path, caller_source).unwrap();
    std::fs::write(&child_path, "shared\n").unwrap();
    std::fs::write(
        &hinted_path,
        "# shuck: source=lib/a.sh\nsource \"$DIR/a.sh\"\nshared\n",
    )
    .unwrap();
    std::fs::write(
        &ambiguous_path,
        "source a.sh\nsource \"$dynamic\"\nshared\n",
    )
    .unwrap();

    let target_uri = Url::from_file_path(std::fs::canonicalize(&target_path).unwrap()).unwrap();
    let caller_uri = Url::from_file_path(&caller_path).unwrap();
    let child_uri = Url::from_file_path(std::fs::canonicalize(&child_path).unwrap()).unwrap();
    let hinted_uri = Url::from_file_path(std::fs::canonicalize(&hinted_path).unwrap()).unwrap();
    let ambiguous_uri =
        Url::from_file_path(std::fs::canonicalize(&ambiguous_path).unwrap()).unwrap();
    let other_uri = Url::from_file_path(std::fs::canonicalize(&other_path).unwrap()).unwrap();
    let mut capabilities = serde_json::to_value(replay_capabilities()).unwrap();
    capabilities["general"]["positionEncodings"] = serde_json::json!(["utf-8"]);

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": capabilities,
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(initialize["capabilities"]["positionEncoding"], "utf-8");
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &target_uri, ": '😀'; shared() { :; }\n");
    open_document(&client_connection, &caller_uri, caller_source);
    open_document(&client_connection, &child_uri, "shared\n");

    let reference_params = |uri: &Url, line, character, include_declaration| {
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": include_declaration },
        })
    };
    send_request(
        &client_connection,
        2,
        "textDocument/references",
        reference_params(&caller_uri, 2, 0, true),
    );
    let with_declaration = recv_response(&client_connection, 2);
    let locations = with_declaration
        .as_array()
        .unwrap_or_else(|| panic!("expected reference locations: {with_declaration:#}"));
    assert_eq!(locations.len(), 4);
    let target = locations
        .iter()
        .find(|location| location["uri"] == serde_json::json!(target_uri))
        .expect("open-buffer declaration should be included");
    assert_eq!(
        target["range"]["start"],
        serde_json::json!({ "line": 0, "character": 10 })
    );
    assert!(locations.iter().any(|location| {
        location["uri"] == serde_json::json!(caller_uri) && location["range"]["start"]["line"] == 2
    }));
    assert!(locations.iter().any(|location| {
        location["uri"] == serde_json::json!(child_uri) && location["range"]["start"]["line"] == 0
    }));
    assert!(locations.iter().any(|location| {
        location["uri"] == serde_json::json!(hinted_uri) && location["range"]["start"]["line"] == 2
    }));
    assert!(locations.iter().all(|location| {
        location["uri"] != serde_json::json!(ambiguous_uri)
            && location["uri"] != serde_json::json!(other_uri)
    }));

    send_request(
        &client_connection,
        3,
        "textDocument/references",
        reference_params(&child_uri, 0, 0, false),
    );
    let without_declaration = recv_response(&client_connection, 3);
    assert_eq!(without_declaration.as_array().map(Vec::len), Some(3));
    assert!(
        without_declaration
            .as_array()
            .unwrap()
            .iter()
            .all(|location| { location["uri"] != serde_json::json!(target_uri) })
    );

    send_request(
        &client_connection,
        4,
        "textDocument/references",
        reference_params(&target_uri, 0, 10, true),
    );
    assert_eq!(recv_response(&client_connection, 4), with_declaration);

    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "textDocument/didChange".to_owned(),
            serde_json::json!({
                "textDocument": { "uri": target_uri, "version": 2 },
                "contentChanges": [{ "text": "renamed() { :; }\n" }],
            }),
        )))
        .expect("didChange should send");
    send_request(
        &client_connection,
        5,
        "textDocument/references",
        reference_params(&caller_uri, 2, 0, true),
    );
    assert!(recv_response(&client_connection, 5).is_null());

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn prepare_call_hierarchy_resolves_sourced_cross_file_call() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    std::fs::write(workspace.path().join("a.sh"), "greet() {\n  echo hi\n}\n").unwrap();
    let caller = "greet() {\n  echo local\n}\n# shuck: source=a.sh\nsource \"$DIR/a.sh\"\ngreet\ngreet() {\n  echo final\n}\ngreet\nhandler=greet\n\"$handler\"\n";
    std::fs::write(workspace.path().join("b.sh"), caller).unwrap();
    let a_uri = Url::from_file_path(
        std::fs::canonicalize(workspace.path().join("a.sh"))
            .expect("definition path should canonicalize"),
    )
    .unwrap();
    let b_uri = Url::from_file_path(workspace.path().join("b.sh")).unwrap();
    let canonical_b_uri = Url::from_file_path(
        std::fs::canonicalize(workspace.path().join("b.sh"))
            .expect("caller path should canonicalize"),
    )
    .unwrap();

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": replay_capabilities(),
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let _ = recv_response(&client_connection, 1);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &b_uri, caller);

    send_request(
        &client_connection,
        2,
        "textDocument/prepareCallHierarchy",
        serde_json::json!({
            "textDocument": { "uri": b_uri },
            "position": { "line": 5, "character": 0 },
        }),
    );
    let prepared = recv_response(&client_connection, 2);
    let item = &prepared.as_array().expect("prepare should return items")[0];
    assert_eq!(item["name"], serde_json::json!("greet"));
    assert_eq!(item["uri"], serde_json::json!(a_uri));
    assert_eq!(
        item["range"],
        serde_json::json!({
            "start": { "line": 0, "character": 0 },
            "end": { "line": 3, "character": 0 },
        })
    );
    assert_eq!(
        item["selectionRange"],
        serde_json::json!({
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 5 },
        })
    );

    // A later local redefinition takes precedence over the sourced function,
    // and prepare must retain that exact definition rather than the first
    // same-named definition in the file.
    send_request(
        &client_connection,
        3,
        "textDocument/prepareCallHierarchy",
        serde_json::json!({
            "textDocument": { "uri": b_uri },
            "position": { "line": 9, "character": 0 },
        }),
    );
    let local_prepared = recv_response(&client_connection, 3);
    let local_item = &local_prepared
        .as_array()
        .expect("local prepare should return items")[0];
    assert_eq!(local_item["name"], serde_json::json!("greet"));
    assert_eq!(local_item["uri"], serde_json::json!(canonical_b_uri));
    assert_eq!(
        local_item["range"],
        serde_json::json!({
            "start": { "line": 6, "character": 0 },
            "end": { "line": 9, "character": 0 },
        })
    );
    assert_eq!(
        local_item["selectionRange"],
        serde_json::json!({
            "start": { "line": 6, "character": 0 },
            "end": { "line": 6, "character": 5 },
        })
    );

    // Runtime-only dispatch cannot be tied to a concrete function definition,
    // even though its eventual value happens to spell the sourced function.
    send_request(
        &client_connection,
        4,
        "textDocument/prepareCallHierarchy",
        serde_json::json!({
            "textDocument": { "uri": b_uri },
            "position": { "line": 11, "character": 2 },
        }),
    );
    assert!(recv_response(&client_connection, 4).is_null());

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn prepare_call_hierarchy_keeps_local_calls_for_non_file_uri() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    let uri = Url::parse("untitled:script.sh").expect("untitled URI should parse");
    let source = "greet() {\n  echo hi\n}\ngreet\n";

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": replay_capabilities(),
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let _ = recv_response(&client_connection, 1);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &uri, source);

    send_request(
        &client_connection,
        2,
        "textDocument/prepareCallHierarchy",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 0 },
        }),
    );
    let prepared = recv_response(&client_connection, 2);
    let item = &prepared.as_array().expect("prepare should return items")[0];
    assert_eq!(item["name"], serde_json::json!("greet"));
    assert_eq!(item["uri"], serde_json::json!(uri));

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn cross_file_call_hierarchy_spans_source_edges() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    // a.sh defines greet; b.sh sources a (lint=true) and calls greet inside
    // `run`; c.sh sources a (import only) and calls greet at top level.
    std::fs::write(workspace.path().join("a.sh"), "greet() {\n  echo hi\n}\n").unwrap();
    std::fs::write(
        workspace.path().join("b.sh"),
        "run() {\n  # shuck: source=a.sh lint=true\n  source \"$DIR/a.sh\"\n  greet\n}\nrun\n",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("c.sh"),
        "# shuck: source=a.sh\nsource \"$DIR/a.sh\"\ngreet\n",
    )
    .unwrap();
    let a_uri = Url::from_file_path(workspace.path().join("a.sh")).unwrap();
    let b_uri = Url::from_file_path(workspace.path().join("b.sh")).unwrap();

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": replay_capabilities(),
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let initialize = recv_response(&client_connection, 1);
    assert_eq!(
        initialize["capabilities"]["callHierarchyProvider"],
        serde_json::json!(true)
    );
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");

    open_document(&client_connection, &a_uri, "greet() {\n  echo hi\n}\n");

    // prepare on greet's definition in a.sh
    send_request(
        &client_connection,
        2,
        "textDocument/prepareCallHierarchy",
        serde_json::json!({
            "textDocument": { "uri": a_uri },
            "position": { "line": 0, "character": 0 },
        }),
    );
    let prepared = recv_response(&client_connection, 2);
    let greet_item = prepared.as_array().unwrap()[0].clone();
    assert_eq!(greet_item["name"], serde_json::json!("greet"));

    // incoming: callers across files — b.sh's `run` (lint=true edge) and
    // c.sh top level (import-only edge)
    send_request(
        &client_connection,
        3,
        "callHierarchy/incomingCalls",
        serde_json::json!({ "item": greet_item }),
    );
    let incoming = recv_response(&client_connection, 3);
    let mut callers: Vec<String> = incoming
        .as_array()
        .unwrap()
        .iter()
        .map(|call| {
            let from = &call["from"];
            format!(
                "{}:{}",
                from["uri"].as_str().unwrap().rsplit('/').next().unwrap(),
                from["name"].as_str().unwrap()
            )
        })
        .collect();
    callers.sort();
    assert_eq!(callers, vec!["b.sh:run".to_owned(), "c.sh:c.sh".to_owned()]);

    // outgoing from run in b.sh descends into a.sh's greet
    open_document(
        &client_connection,
        &b_uri,
        "run() {\n  # shuck: source=a.sh lint=true\n  source \"$DIR/a.sh\"\n  greet\n}\nrun\n",
    );
    send_request(
        &client_connection,
        4,
        "textDocument/prepareCallHierarchy",
        serde_json::json!({
            "textDocument": { "uri": b_uri },
            "position": { "line": 0, "character": 0 },
        }),
    );
    let run_item = recv_response(&client_connection, 4).as_array().unwrap()[0].clone();
    assert_eq!(run_item["name"], serde_json::json!("run"));
    send_request(
        &client_connection,
        5,
        "callHierarchy/outgoingCalls",
        serde_json::json!({ "item": run_item }),
    );
    let outgoing = recv_response(&client_connection, 5);
    let outgoing = outgoing.as_array().unwrap();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0]["to"]["name"], serde_json::json!("greet"));
    assert!(outgoing[0]["to"]["uri"].as_str().unwrap().ends_with("a.sh"));

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn cross_file_call_hierarchy_honors_configured_source_paths() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    // The helper lives under lib/, reachable from scripts/main.sh ONLY via the
    // configured [lint] source-paths root — not relative to the annotating file.
    std::fs::write(
        workspace.path().join("shuck.toml"),
        "[lint]\nsource-paths = [\"lib\"]\n",
    )
    .unwrap();
    std::fs::create_dir(workspace.path().join("lib")).unwrap();
    std::fs::create_dir(workspace.path().join("scripts")).unwrap();
    std::fs::write(
        workspace.path().join("lib/util.sh"),
        "greet() {\n  echo hi\n}\n",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("scripts/main.sh"),
        "run() {\n  # shuck: source=util.sh lint=true\n  source \"$X/util.sh\"\n  greet\n}\nrun\n",
    )
    .unwrap();
    let util_uri = Url::from_file_path(workspace.path().join("lib/util.sh")).unwrap();

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": replay_capabilities(),
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let _ = recv_response(&client_connection, 1);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &util_uri, "greet() {\n  echo hi\n}\n");

    send_request(
        &client_connection,
        2,
        "textDocument/prepareCallHierarchy",
        serde_json::json!({
            "textDocument": { "uri": util_uri },
            "position": { "line": 0, "character": 0 },
        }),
    );
    let greet_item = recv_response(&client_connection, 2).as_array().unwrap()[0].clone();
    assert_eq!(greet_item["name"], serde_json::json!("greet"));

    send_request(
        &client_connection,
        3,
        "callHierarchy/incomingCalls",
        serde_json::json!({ "item": greet_item }),
    );
    let incoming = recv_response(&client_connection, 3);
    let incoming = incoming.as_array().unwrap();
    // Without source-paths, main.sh's source=util.sh directive would not resolve and
    // greet would have no caller; with it, `run` shows up.
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0]["from"]["name"], serde_json::json!("run"));
    assert!(
        incoming[0]["from"]["uri"]
            .as_str()
            .unwrap()
            .ends_with("scripts/main.sh")
    );

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}

#[test]
fn call_hierarchy_round_trips_same_named_definition_identity() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace = tempfile::tempdir().expect("tempdir should be created");
    let script_path = workspace.path().join("script.sh");
    std::fs::write(&script_path, "stale() { :; }\n").unwrap();
    let script_uri = Url::from_file_path(&script_path).unwrap();
    let source = "first() { :; }\nsecond() { :; }\nworker() {\n  first\n}\nworker\nworker() {\n  second\n}\nworker\n";

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": replay_capabilities(),
            "rootUri": Url::from_file_path(workspace.path()).unwrap(),
        }),
    );
    let _ = recv_response(&client_connection, 1);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized should send");
    open_document(&client_connection, &script_uri, source);

    let mut prepared = Vec::new();
    for (request_id, line) in [(2, 2), (3, 6)] {
        send_request(
            &client_connection,
            request_id,
            "textDocument/prepareCallHierarchy",
            serde_json::json!({
                "textDocument": { "uri": script_uri },
                "position": { "line": line, "character": 0 },
            }),
        );
        let response = recv_response(&client_connection, request_id);
        prepared.push(response.as_array().unwrap()[0].clone());
    }

    assert_eq!(prepared[0]["range"]["start"]["line"], 2);
    assert_eq!(prepared[1]["range"]["start"]["line"], 6);
    assert_ne!(prepared[0]["data"], prepared[1]["data"]);

    for (request_id, item, expected_callee) in
        [(4, &prepared[0], "first"), (5, &prepared[1], "second")]
    {
        send_request(
            &client_connection,
            request_id,
            "callHierarchy/outgoingCalls",
            serde_json::json!({ "item": item }),
        );
        let response = recv_response(&client_connection, request_id);
        let outgoing = response.as_array().unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0]["to"]["name"], expected_callee);
    }

    for (request_id, item, expected_call_line) in [(6, &prepared[0], 5), (7, &prepared[1], 9)] {
        send_request(
            &client_connection,
            request_id,
            "callHierarchy/incomingCalls",
            serde_json::json!({ "item": item }),
        );
        let response = recv_response(&client_connection, request_id);
        let incoming = response.as_array().unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0]["fromRanges"].as_array().unwrap().len(), 1);
        assert_eq!(
            incoming[0]["fromRanges"][0]["start"]["line"],
            expected_call_line
        );
    }

    send_request(&client_connection, 99, "shutdown", serde_json::json!(null));
    let _ = recv_response(&client_connection, 99);
    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "exit".to_owned(),
            serde_json::json!({}),
        )))
        .expect("exit notification should send");
    server_thread
        .join()
        .expect("server thread should join")
        .expect("server should exit cleanly");
}
