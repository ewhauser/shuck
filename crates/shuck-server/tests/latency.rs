use std::thread;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::{
    ClientCapabilities, DocumentDiagnosticParams, DocumentDiagnosticReport,
    DocumentDiagnosticReportResult, PartialResultParams, TextDocumentIdentifier, Url,
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
    loop {
        let message = connection
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("server should respond");
        match message {
            Message::Response(response) if response.id == RequestId::from(id) => {
                assert!(
                    response.error.is_none(),
                    "unexpected LSP error: {:?}",
                    response.error
                );
                return response
                    .result
                    .expect("successful response should carry a result");
            }
            Message::Notification(_) => continue,
            Message::Request(request) => {
                panic!(
                    "unexpected server request during latency test: {}",
                    request.method
                )
            }
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
            }
        },
        "workspace": {
            "workspaceFolders": true
        }
    }))
    .expect("test client capabilities should deserialize")
}

fn shell_source_of_size(target_bytes: usize) -> String {
    let mut source = String::from("#!/bin/bash\n");
    while source.len() < target_bytes {
        source.push_str("value=1\n");
    }
    source
}

#[test]
#[ignore = "manual performance measurement"]
fn measure_pull_diagnostics_round_trip() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    let workspace_root = tempfile::tempdir().expect("tempdir should be created");
    let script_path = workspace_root.path().join("latency.sh");
    let script_uri =
        Url::from_file_path(&script_path).expect("script path should convert to a URL");
    let source = shell_source_of_size(5 * 1024);

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": replay_capabilities(),
            "rootUri": Url::from_file_path(workspace_root.path())
                .expect("workspace path should convert to a URL"),
        }),
    );
    let _ = recv_response(&client_connection, 1);

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
                    "text": source,
                }
            }),
        )))
        .expect("didOpen notification should send");

    let start = Instant::now();
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
    let elapsed = start.elapsed();
    let DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(report)) =
        diagnostic_report
    else {
        panic!("expected a full diagnostic report");
    };

    println!(
        "pull diagnostics round-trip: {:.3} ms for {} bytes ({} diagnostics)",
        elapsed.as_secs_f64() * 1000.0,
        5 * 1024,
        report.full_document_diagnostic_report.items.len()
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
