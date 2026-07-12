use std::thread;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId};

#[test]
fn disconnect_before_initialization_exits_cleanly() {
    let (server_connection, client_connection) = Connection::memory();
    drop(client_connection);

    shuck_server::run_connection(server_connection)
        .expect("disconnect before initialization should be a clean shutdown");
}

#[test]
fn disconnect_after_initialization_exits_cleanly() {
    let (server_connection, client_connection) = Connection::memory();
    let server_thread = thread::spawn(move || shuck_server::run_connection(server_connection));

    client_connection
        .sender
        .send(Message::Request(Request {
            id: RequestId::from(1),
            method: "initialize".to_owned(),
            params: serde_json::json!({ "capabilities": {} }),
        }))
        .expect("initialize request should send");

    let response = client_connection
        .receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("server should respond to initialization");
    assert!(matches!(response, Message::Response(response) if response.error.is_none()));

    client_connection
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            serde_json::json!({}),
        )))
        .expect("initialized notification should send");
    drop(client_connection);

    server_thread
        .join()
        .expect("server thread should join")
        .expect("disconnect after initialization should be a clean shutdown");
}
