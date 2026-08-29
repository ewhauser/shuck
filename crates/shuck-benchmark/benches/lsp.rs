use std::thread::{self, JoinHandle};
use std::time::Duration;

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::{Position, Url};
use serde_json::{Value, json};
use shuck_benchmark::configure_benchmark_allocator;
use tempfile::TempDir;

configure_benchmark_allocator!();

const DOCUMENT_BYTES: usize = 5 * 1024;
const WORKSPACE_FILE_COUNT: usize = 32;
const WORKSPACE_FILE_BYTES: usize = 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

struct LspBenchClient {
    connection: Connection,
    server_thread: Option<JoinHandle<()>>,
    workspace: TempDir,
    document_uri: Option<Url>,
    next_request_id: i32,
    document_version: i32,
}

impl LspBenchClient {
    fn new(workspace: TempDir) -> Self {
        let (server_connection, connection) = Connection::memory();
        let server_thread = thread::spawn(move || {
            shuck_server::run_connection(server_connection)
                .expect("benchmark server should exit cleanly")
        });
        let root_uri = Url::from_file_path(workspace.path())
            .expect("benchmark workspace path should convert to a URL");
        let mut client = Self {
            connection,
            server_thread: Some(server_thread),
            workspace,
            document_uri: None,
            next_request_id: 1,
            document_version: 0,
        };

        let initialize = client.request(
            "initialize",
            json!({
                "capabilities": {
                    "general": { "positionEncodings": ["utf-16"] },
                    "textDocument": {
                        "completion": { "completionItem": {} },
                        "diagnostic": {
                            "dynamicRegistration": false,
                            "relatedDocumentSupport": false
                        },
                        "documentSymbol": {
                            "hierarchicalDocumentSymbolSupport": true
                        },
                        "hover": { "contentFormat": ["markdown"] }
                    },
                    "workspace": {
                        "configuration": false,
                        "workspaceFolders": true
                    }
                },
                "rootUri": root_uri,
            }),
        );
        assert_eq!(
            initialize["capabilities"]["textDocumentSync"]["change"],
            json!(2),
            "benchmark server should negotiate incremental document sync"
        );
        client.notify("initialized", json!({}));
        client
    }

    fn with_open_document(source: &str) -> Self {
        let workspace = tempfile::tempdir().expect("benchmark tempdir should be created");
        let document_uri = Url::from_file_path(workspace.path().join("benchmark.sh"))
            .expect("benchmark document path should convert to a URL");
        let mut client = Self::new(workspace);
        client.document_version = 1;
        client.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": document_uri,
                    "languageId": "shellscript",
                    "version": client.document_version,
                    "text": source,
                }
            }),
        );
        client.document_uri = Some(document_uri);
        client
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .expect("benchmark request IDs should not overflow");
        self.connection
            .sender
            .send(Message::Request(Request {
                id: RequestId::from(id),
                method: method.to_owned(),
                params,
            }))
            .expect("benchmark request should send");
        self.recv_response(id)
    }

    fn notify(&self, method: &str, params: Value) {
        self.connection
            .sender
            .send(Message::Notification(Notification::new(
                method.to_owned(),
                params,
            )))
            .expect("benchmark notification should send");
    }

    fn recv_response(&self, id: i32) -> Value {
        loop {
            let message = self
                .connection
                .receiver
                .recv_timeout(RESPONSE_TIMEOUT)
                .expect("benchmark server should respond");
            match message {
                Message::Response(response) if response.id == RequestId::from(id) => {
                    assert!(
                        response.error.is_none(),
                        "unexpected LSP benchmark error: {:?}",
                        response.error
                    );
                    return response
                        .result
                        .expect("successful benchmark response should have a result");
                }
                Message::Notification(_) | Message::Response(_) => {}
                Message::Request(request) => {
                    panic!(
                        "unexpected server request during LSP benchmark: {}",
                        request.method
                    );
                }
            }
        }
    }

    fn document_uri(&self) -> &Url {
        self.document_uri
            .as_ref()
            .expect("benchmark document should be open")
    }

    fn document_params(&self) -> Value {
        json!({ "textDocument": { "uri": self.document_uri() } })
    }

    fn diagnostic(&mut self) -> Value {
        let params = self.document_params();
        self.request("textDocument/diagnostic", params)
    }

    fn replace_document(&mut self, source: &str) {
        self.document_version += 1;
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {
                    "uri": self.document_uri(),
                    "version": self.document_version,
                },
                "contentChanges": [{ "text": source }],
            }),
        );
    }

    fn replace_one_character(&mut self, position: Position, replacement: char) {
        self.document_version += 1;
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {
                    "uri": self.document_uri(),
                    "version": self.document_version,
                },
                "contentChanges": [{
                    "range": {
                        "start": position,
                        "end": {
                            "line": position.line,
                            "character": position.character + 1,
                        }
                    },
                    "rangeLength": 1,
                    "text": replacement.to_string(),
                }],
            }),
        );
    }

    fn invalidate_workspace(&self) {
        let changed_uri = Url::from_file_path(self.workspace.path().join("file_000.sh"))
            .expect("benchmark workspace file should convert to a URL");
        self.notify(
            "workspace/didChangeWatchedFiles",
            json!({
                "changes": [{ "uri": changed_uri, "type": 2 }]
            }),
        );
    }

    fn shutdown(&mut self) {
        let Some(server_thread) = self.server_thread.take() else {
            return;
        };
        let _ = self.request("shutdown", Value::Null);
        self.notify("exit", json!({}));
        server_thread
            .join()
            .expect("benchmark server thread should join");
    }
}

impl Drop for LspBenchClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn document_source(target_bytes: usize) -> String {
    let mut source = String::from(
        "#!/usr/bin/env bash\n\
         global_value=ready\n\
         render_value() {\n\
           local input=$1\n\
           printf '%s\\n' \"$input\"\n\
         }\n",
    );
    let suffix = "render_value \"$global_value\"\nprintf '%s\\n' \"$global_\"\n";
    let mut index = 0usize;
    while source.len() + suffix.len() < target_bytes {
        let line = format!("render_value \"$global_value\" # probe {index}\n");
        let remaining = target_bytes - source.len() - suffix.len();
        if line.len() > remaining {
            if remaining == 1 {
                source.push('\n');
            } else {
                source.push('#');
                source.extend(std::iter::repeat_n('x', remaining - 2));
                source.push('\n');
            }
            break;
        }
        source.push_str(&line);
        index += 1;
    }
    source.push_str(suffix);
    assert_eq!(source.len(), target_bytes);
    source
}

fn position_of(source: &str, needle: &str, offset_in_needle: usize) -> Position {
    let offset = source.find(needle).expect("benchmark probe should exist") + offset_in_needle;
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |newline| newline + 1);
    Position::new(
        u32::try_from(line).expect("benchmark line should fit in u32"),
        u32::try_from(offset - line_start).expect("benchmark column should fit in u32"),
    )
}

fn create_workspace() -> TempDir {
    let workspace = tempfile::tempdir().expect("benchmark tempdir should be created");
    for index in 0..WORKSPACE_FILE_COUNT {
        let source = document_source(WORKSPACE_FILE_BYTES);
        std::fs::write(workspace.path().join(format!("file_{index:03}.sh")), source)
            .expect("benchmark workspace fixture should be written");
    }
    workspace
}

fn assert_full_diagnostic_report(report: &Value) {
    assert_eq!(report["kind"], "full");
    assert!(report["items"].is_array());
}

fn workspace_report_count(report: &Value) -> usize {
    report["items"]
        .as_array()
        .expect("workspace diagnostic response should contain items")
        .len()
}

fn bench_document_diagnostics(c: &mut Criterion) {
    let source = document_source(DOCUMENT_BYTES);
    let mut group = c.benchmark_group("lsp_document_diagnostics");
    group.throughput(Throughput::Bytes(source.len() as u64));

    let mut warm_client = LspBenchClient::with_open_document(&source);
    assert_full_diagnostic_report(&warm_client.diagnostic());
    group.bench_function("warm_cache_5_kib", |b| {
        b.iter(|| black_box(warm_client.diagnostic()));
    });
    warm_client.shutdown();

    let mut changed_client = LspBenchClient::with_open_document(&source);
    assert_full_diagnostic_report(&changed_client.diagnostic());
    group.bench_function("after_full_change_5_kib", |b| {
        b.iter(|| {
            changed_client.replace_document(&source);
            black_box(changed_client.diagnostic())
        });
    });
    changed_client.shutdown();

    group.finish();
}

fn bench_incremental_diagnostics(c: &mut Criterion) {
    let source = document_source(DOCUMENT_BYTES);
    let edit_position = position_of(&source, "global_value=ready", "global_value=".len());
    let mut client = LspBenchClient::with_open_document(&source);
    assert_full_diagnostic_report(&client.diagnostic());
    let mut uppercase = true;
    let mut group = c.benchmark_group("lsp_incremental_diagnostics");
    group.throughput(Throughput::Bytes(source.len() as u64));
    group.bench_function("single_character_change_5_kib", |b| {
        b.iter(|| {
            client.replace_one_character(edit_position, if uppercase { 'R' } else { 'r' });
            uppercase = !uppercase;
            black_box(client.diagnostic())
        });
    });
    group.finish();
    client.shutdown();
}

fn bench_interactive_requests(c: &mut Criterion) {
    let source = document_source(DOCUMENT_BYTES);
    let hover_position = position_of(&source, "$global_value", 2);
    let completion_position = position_of(&source, "$global_\"", "$global_".len());
    let mut client = LspBenchClient::with_open_document(&source);
    assert_full_diagnostic_report(&client.diagnostic());
    let document_uri = client.document_uri().clone();
    let hover_params = json!({
        "textDocument": { "uri": document_uri },
        "position": hover_position,
    });
    let completion_params = json!({
        "textDocument": { "uri": document_uri },
        "position": completion_position,
    });
    let document_symbol_params = json!({
        "textDocument": { "uri": document_uri },
    });

    assert!(
        !client
            .request("textDocument/hover", hover_params.clone())
            .is_null()
    );
    assert!(
        !client
            .request("textDocument/completion", completion_params.clone())
            .is_null()
    );
    assert!(
        !client
            .request(
                "textDocument/documentSymbol",
                document_symbol_params.clone(),
            )
            .is_null()
    );

    let mut group = c.benchmark_group("lsp_interactive_warm");
    group.throughput(Throughput::Elements(1));
    group.bench_function("hover", |b| {
        b.iter(|| black_box(client.request("textDocument/hover", hover_params.clone())));
    });
    group.bench_function("completion", |b| {
        b.iter(|| black_box(client.request("textDocument/completion", completion_params.clone())));
    });
    group.bench_function("document_symbols", |b| {
        b.iter(|| {
            black_box(client.request(
                "textDocument/documentSymbol",
                document_symbol_params.clone(),
            ))
        });
    });
    group.finish();
    client.shutdown();
}

fn bench_workspace_diagnostics(c: &mut Criterion) {
    let mut client = LspBenchClient::new(create_workspace());
    let params = json!({ "previousResultIds": [] });
    let first_report = client.request("workspace/diagnostic", params.clone());
    assert_eq!(workspace_report_count(&first_report), WORKSPACE_FILE_COUNT);

    let mut group = c.benchmark_group("lsp_workspace_diagnostics");
    group.sample_size(20);
    group.throughput(Throughput::Bytes(
        (WORKSPACE_FILE_COUNT * WORKSPACE_FILE_BYTES) as u64,
    ));
    group.bench_function("warm_cache_32_files", |b| {
        b.iter(|| black_box(client.request("workspace/diagnostic", params.clone())));
    });
    group.bench_function("invalidated_cache_32_files", |b| {
        b.iter(|| {
            client.invalidate_workspace();
            black_box(client.request("workspace/diagnostic", params.clone()))
        });
    });
    group.finish();
    client.shutdown();
}

criterion_group!(
    benches,
    bench_document_diagnostics,
    bench_incremental_diagnostics,
    bench_interactive_requests,
    bench_workspace_diagnostics,
);
criterion_main!(benches);
