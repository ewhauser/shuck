//! Fuzz target for bounded in-memory LSP protocol sessions.

#![no_main]

mod common;
mod lsp_common;

use std::thread;
use std::time::{Duration, Instant};

use libfuzzer_sys::{Corpus, fuzz_target};
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::Url;

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    run_protocol_sequence(input, data).expect("LSP protocol sequence should shut down cleanly");

    Corpus::Keep
});

fn run_protocol_sequence(source: &str, data: &[u8]) -> Result<(), String> {
    let encoding = lsp_common::encoding_from_byte(data.first().copied().unwrap_or_default());
    let language_id = lsp_common::language_id_from_byte(data.get(1).copied().unwrap_or_default());
    let file_name = lsp_common::file_name_for_language(language_id);
    let workspace_root =
        std::env::temp_dir().join(format!("shuck-lsp-protocol-fuzz-{}", std::process::id()));
    std::fs::create_dir_all(&workspace_root)
        .map_err(|error| format!("create fuzz workspace: {error}"))?;
    let root_uri = Url::from_file_path(&workspace_root)
        .map_err(|()| "failed to build workspace URI".to_owned())?;
    let script_uri = Url::from_file_path(workspace_root.join(file_name))
        .map_err(|()| "failed to build document URI".to_owned())?;

    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    send_request(
        &client_connection,
        1,
        "initialize",
        serde_json::json!({
            "capabilities": lsp_common::capabilities_from_byte(
                data.get(2).copied().unwrap_or_default(),
                encoding,
            ),
            "rootUri": root_uri,
            "initializationOptions": {
                "shuck": {
                    "fixAll": true,
                    "unsafeFixes": data.get(3).copied().unwrap_or_default() & 1 == 0,
                    "showSyntaxErrors": true,
                    "server": {
                        "workspaceSymbols": { "maxFiles": 0 }
                    }
                }
            }
        }),
    )?;
    recv_response(&client_connection, 1)?;
    send_notification(
        &client_connection,
        "initialized",
        serde_json::json!({}),
    )?;

    let mut current_source = source.to_owned();
    let mut version = 1i32;
    let mut open = true;
    send_did_open(
        &client_connection,
        &script_uri,
        language_id,
        version,
        &current_source,
    )?;

    let mut next_id = 2i32;
    let mut last_request_id = None;
    for chunk in data.chunks(8).take(12) {
        match chunk.first().copied().unwrap_or_default() % 15 {
            0 => {
                version += 1;
                current_source = lsp_common::replacement_from_bytes(source, chunk);
                send_notification(
                    &client_connection,
                    "textDocument/didChange",
                    serde_json::json!({
                        "textDocument": { "uri": script_uri, "version": version },
                        "contentChanges": [{ "text": current_source }],
                    }),
                )?;
            }
            1 => {
                version += 1;
                let range = lsp_common::range_from_bytes(&current_source, chunk, encoding);
                let text = lsp_common::replacement_from_bytes(source, chunk);
                send_notification(
                    &client_connection,
                    "textDocument/didChange",
                    serde_json::json!({
                        "textDocument": { "uri": script_uri, "version": version },
                        "contentChanges": [{ "range": range, "text": text }],
                    }),
                )?;
            }
            2 => {
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/diagnostic",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                    }),
                )?;
            }
            3 => {
                let position = lsp_common::position_from_bytes(&current_source, chunk, encoding);
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/hover",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                        "position": position,
                    }),
                )?;
            }
            4 => {
                let position = lsp_common::position_from_bytes(&current_source, chunk, encoding);
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/completion",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                        "position": position,
                    }),
                )?;
            }
            5 => {
                let range = lsp_common::range_from_bytes(&current_source, chunk, encoding);
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/codeAction",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                        "range": range,
                        "context": { "diagnostics": [] },
                    }),
                )?;
            }
            6 => {
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/documentSymbol",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                    }),
                )?;
            }
            7 => {
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/formatting",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                        "options": { "tabSize": 4, "insertSpaces": true },
                    }),
                )?;
            }
            8 => {
                let range = lsp_common::range_from_bytes(&current_source, chunk, encoding);
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/rangeFormatting",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                        "range": range,
                        "options": { "tabSize": 4, "insertSpaces": true },
                    }),
                )?;
            }
            9 => {
                let position = lsp_common::position_from_bytes(&current_source, chunk, encoding);
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/prepareRename",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                        "position": position,
                    }),
                )?;
            }
            10 => {
                let position = lsp_common::position_from_bytes(&current_source, chunk, encoding);
                send_document_request(
                    &client_connection,
                    &mut next_id,
                    &mut last_request_id,
                    "textDocument/rename",
                    &script_uri,
                    serde_json::json!({
                        "textDocument": { "uri": script_uri },
                        "position": position,
                        "newName": lsp_common::new_name_from_byte(chunk.get(1).copied().unwrap_or_default()),
                    }),
                )?;
            }
            11 => {
                let request_id = next_id;
                next_id += 1;
                last_request_id = Some(request_id);
                send_request(
                    &client_connection,
                    request_id,
                    "workspace/symbol",
                    serde_json::json!({
                        "query": lsp_common::workspace_query_from_byte(
                            chunk.get(1).copied().unwrap_or_default(),
                        ),
                    }),
                )?;
            }
            12 => {
                send_notification(
                    &client_connection,
                    "workspace/didChangeConfiguration",
                    serde_json::json!({
                        "settings": {
                            "shuck": {
                                "fixAll": true,
                                "unsafeFixes": chunk.get(1).copied().unwrap_or_default() & 1 == 0,
                                "showSyntaxErrors": chunk.get(2).copied().unwrap_or_default() & 1 == 0,
                            }
                        }
                    }),
                )?;
            }
            13 => {
                if let Some(id) = last_request_id {
                    send_notification(
                        &client_connection,
                        "$/cancelRequest",
                        serde_json::json!({ "id": id }),
                    )?;
                }
            }
            _ => {
                if open {
                    send_notification(
                        &client_connection,
                        "textDocument/didClose",
                        serde_json::json!({
                            "textDocument": { "uri": script_uri },
                        }),
                    )?;
                    open = false;
                } else {
                    version += 1;
                    send_did_open(
                        &client_connection,
                        &script_uri,
                        language_id,
                        version,
                        &current_source,
                    )?;
                    open = true;
                }
            }
        }
        drain_messages(&client_connection, Duration::from_millis(2))?;
    }

    let shutdown_id = next_id;
    send_request(
        &client_connection,
        shutdown_id,
        "shutdown",
        serde_json::Value::Null,
    )?;
    recv_response(&client_connection, shutdown_id)?;
    send_notification(&client_connection, "exit", serde_json::json!({}))?;

    server_thread
        .join()
        .map_err(|_| "server thread panicked".to_owned())?
        .map_err(|error| error.to_string())
}

fn send_document_request(
    connection: &Connection,
    next_id: &mut i32,
    last_request_id: &mut Option<i32>,
    method: &str,
    _uri: &Url,
    params: serde_json::Value,
) -> Result<(), String> {
    let request_id = *next_id;
    *next_id += 1;
    *last_request_id = Some(request_id);
    send_request(connection, request_id, method, params)
}

fn send_did_open(
    connection: &Connection,
    uri: &Url,
    language_id: &str,
    version: i32,
    text: &str,
) -> Result<(), String> {
    send_notification(
        connection,
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": version,
                "text": text,
            }
        }),
    )
}

fn send_request(
    connection: &Connection,
    id: i32,
    method: &str,
    params: serde_json::Value,
) -> Result<(), String> {
    connection
        .sender
        .send(Message::Request(Request {
            id: RequestId::from(id),
            method: method.to_owned(),
            params,
        }))
        .map_err(|error| format!("send request {method}: {error}"))
}

fn send_notification(
    connection: &Connection,
    method: &str,
    params: serde_json::Value,
) -> Result<(), String> {
    connection
        .sender
        .send(Message::Notification(Notification::new(method.to_owned(), params)))
        .map_err(|error| format!("send notification {method}: {error}"))
}

fn recv_response(connection: &Connection, id: i32) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(format!("timed out waiting for response {id}"));
        }
        let timeout = deadline.saturating_duration_since(now).min(Duration::from_millis(25));
        match connection.receiver.recv_timeout(timeout) {
            Ok(Message::Response(response)) if response.id == RequestId::from(id) => {
                return Ok(());
            }
            Ok(message) => handle_server_message(connection, message)?,
            Err(crossbeam::channel::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(format!("receive response {id}: {error}")),
        }
    }
}

fn drain_messages(connection: &Connection, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        match connection.receiver.recv_timeout(Duration::from_millis(1)) {
            Ok(message) => handle_server_message(connection, message)?,
            Err(crossbeam::channel::RecvTimeoutError::Timeout) => return Ok(()),
            Err(error) => return Err(format!("drain server messages: {error}")),
        }
    }
    Ok(())
}

fn handle_server_message(connection: &Connection, message: Message) -> Result<(), String> {
    match message {
        Message::Request(request) => {
            let result = if request.method == "workspace/applyEdit" {
                serde_json::json!({ "applied": false })
            } else {
                serde_json::Value::Null
            };
            connection
                .sender
                .send(Message::Response(Response::new_ok(request.id, result)))
                .map_err(|error| format!("respond to server request: {error}"))?;
        }
        Message::Response(_) | Message::Notification(_) => {}
    }
    Ok(())
}
