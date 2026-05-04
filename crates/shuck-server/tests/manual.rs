use std::thread;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::{
    ClientCapabilities, CodeAction, CodeActionContext, CodeActionParams,
    DocumentDiagnosticParams, DocumentDiagnosticReport, DocumentDiagnosticReportResult,
    HoverParams, PartialResultParams, Position, Range, TextDocumentIdentifier,
    TextDocumentPositionParams, Url, WorkDoneProgressParams,
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
    loop {
        let message = connection
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("server should respond");
        match message {
            Message::Response(response) if response.id == RequestId::from(id) => {
                assert!(response.error.is_none(), "unexpected LSP error: {:?}", response.error);
                return response
                    .result
                    .expect("successful response should carry a result");
            }
            Message::Notification(_) => continue,
            Message::Request(request) => panic!("unexpected server request during replay: {}", request.method),
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
    let script_uri = Url::from_file_path(&script_path).expect("script path should convert to a URL");

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
    assert!(initialize["capabilities"]["documentFormattingProvider"].is_null());
    assert!(initialize["capabilities"]["documentRangeFormattingProvider"].is_null());

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
    let diagnostic_report: DocumentDiagnosticReportResult = serde_json::from_value(recv_response(&client_connection, 2))
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
    let actions: Vec<lsp_types::CodeActionOrCommand> = serde_json::from_value(recv_response(&client_connection, 3))
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
    assert!(hover["contents"]["value"]
        .as_str()
        .is_some_and(|value| value.contains("Undefined Variable")));

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
